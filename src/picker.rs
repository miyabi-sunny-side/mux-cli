use std::{
    io::{self, BufRead, IsTerminal, Write},
    os::fd::AsRawFd,
    process::ExitCode,
};

use anyhow::{Context, Result, bail};

use crate::{
    config::ConfigDir,
    lint::lint_project,
    project::validate_project_name,
    tmux::{self, SessionRef, try_normalize_project_name},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PickerRow {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) live: bool,
    pub(crate) live_name: Option<String>,
    pub(crate) saved_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FzfKey {
    Enter,
    Space,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PickerAction {
    FocusLive(String),
    Restore { name: String, focus: bool },
    CloseLive(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClosePrompt {
    Immediate,
    Unsaved,
    Dirty,
    SnapshotFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloseOutcome {
    Closed,
    Cancelled,
}

pub(crate) fn run_picker(config: &ConfigDir) -> Result<ExitCode> {
    let mut first = true;
    loop {
        let rows = collect_rows(config)?;
        if rows.is_empty() {
            if first {
                bail!("no projects or sessions found");
            }
            return Ok(ExitCode::SUCCESS);
        }
        first = false;

        let Some((key, id)) = select_picker_row(&rows)? else {
            return Ok(ExitCode::SUCCESS);
        };
        let row = rows
            .iter()
            .find(|row| row.id == id)
            .with_context(|| format!("fzf returned an unknown project '{id}'"))?;

        match action_for(key, row)? {
            PickerAction::FocusLive(name) => {
                tmux::focus(&name)?;
                return Ok(ExitCode::SUCCESS);
            }
            PickerAction::Restore { name, focus: true } => {
                return open_saved(config, &name);
            }
            PickerAction::Restore { name, focus: false } => {
                ensure_saved(config, &name)?;
            }
            PickerAction::CloseLive(name) => {
                let session = tmux::session_by_name(&name)?;
                let _ = close_session(config, &session)?;
            }
        }
    }
}

pub(crate) fn run_save_current(config: &ConfigDir) -> Result<ExitCode> {
    let snapshot = tmux::snapshot()?;
    let name = snapshot.project_name.clone().context(
        "tmux session name cannot be converted to a mux project name; rename the session",
    )?;
    persist_snapshot(config, &snapshot, &name, &name)?;
    println!("{name}");
    Ok(ExitCode::SUCCESS)
}

pub(crate) fn run_close(config: &ConfigDir) -> Result<ExitCode> {
    let session = tmux::current_session()?;
    match close_session(config, &session)? {
        CloseOutcome::Closed => Ok(ExitCode::SUCCESS),
        CloseOutcome::Cancelled => Ok(ExitCode::FAILURE),
    }
}

pub(crate) fn run_open(config: &ConfigDir, name: &str) -> Result<ExitCode> {
    open_saved(config, name)
}

pub(crate) fn merge_picker_rows(saved: &[(String, String)], live: &[String]) -> Vec<PickerRow> {
    let mut rows = Vec::new();
    for (stem, project_name) in saved {
        if stem.starts_with('_') || project_name.starts_with('_') {
            continue;
        }
        rows.push(PickerRow {
            id: format!("s:{stem}"),
            label: stem.clone(),
            live: false,
            live_name: None,
            saved_name: Some(stem.clone()),
        });
    }

    for raw in live {
        if raw.starts_with('_') {
            continue;
        }
        let normalized = try_normalize_project_name(raw);
        if normalized
            .as_deref()
            .is_some_and(|name| name.starts_with('_'))
        {
            continue;
        }
        if let Some(index) = best_saved_match(&rows, saved, raw, normalized.as_deref()) {
            if rows[index].live {
                rows.push(live_row(raw));
            } else {
                rows[index].live = true;
                rows[index].live_name = Some(raw.clone());
            }
        } else {
            rows.push(live_row(raw));
        }
    }

    rows.sort_by(|left, right| left.label.as_bytes().cmp(right.label.as_bytes()));
    rows
}

pub(crate) fn format_picker_row(row: &PickerRow) -> String {
    format!(
        "{}\t{}\t{}",
        row.id,
        if row.live { "●" } else { "○" },
        row.label
    )
}

pub(crate) fn parse_fzf_expect(stdout: &str) -> Option<(FzfKey, String)> {
    let mut lines = stdout.lines();
    let key = match lines.next()? {
        "enter" => FzfKey::Enter,
        "space" => FzfKey::Space,
        _ => return None,
    };
    let row = lines.next().unwrap_or_default();
    let id = row.split('\t').next().unwrap_or_default();
    if id.is_empty() {
        return None;
    }
    Some((key, id.to_owned()))
}

pub(crate) fn action_for(key: FzfKey, row: &PickerRow) -> Result<PickerAction> {
    match (key, row.live) {
        (FzfKey::Enter, true) => row
            .live_name
            .clone()
            .map(PickerAction::FocusLive)
            .context("live picker row is missing a session name"),
        (FzfKey::Enter, false) => {
            saved_name(row).map(|name| PickerAction::Restore { name, focus: true })
        }
        (FzfKey::Space, true) => row
            .live_name
            .clone()
            .map(PickerAction::CloseLive)
            .context("live picker row is missing a session name"),
        (FzfKey::Space, false) => {
            saved_name(row).map(|name| PickerAction::Restore { name, focus: false })
        }
    }
}

pub(crate) fn close_prompt(saved: bool, snapshot_ok: bool, matches: bool) -> ClosePrompt {
    if !snapshot_ok {
        return ClosePrompt::SnapshotFailed;
    }
    if !saved {
        return ClosePrompt::Unsaved;
    }
    if matches {
        ClosePrompt::Immediate
    } else {
        ClosePrompt::Dirty
    }
}

pub(crate) fn parse_choice(answer: &str, accepted: &[char]) -> Option<char> {
    let answer = answer.trim().to_ascii_lowercase();
    let mut characters = answer.chars();
    match (characters.next(), characters.next()) {
        (Some(choice), None) if accepted.contains(&choice) => Some(choice),
        _ => None,
    }
}

fn saved_name(row: &PickerRow) -> Result<String> {
    row.saved_name
        .clone()
        .context("stopped picker row is missing a saved project")
}

fn live_row(raw: &str) -> PickerRow {
    PickerRow {
        id: format!("l:{raw}"),
        label: raw.to_owned(),
        live: true,
        live_name: Some(raw.to_owned()),
        saved_name: None,
    }
}

fn best_saved_match(
    rows: &[PickerRow],
    saved: &[(String, String)],
    raw: &str,
    normalized: Option<&str>,
) -> Option<usize> {
    let mut best: Option<(usize, i32, &str)> = None;
    for (index, row) in rows.iter().enumerate() {
        let Some(stem) = row.saved_name.as_deref() else {
            continue;
        };
        let project_name = saved
            .iter()
            .find(|(candidate, _)| candidate == stem)
            .map(|(_, name)| name.as_str());
        let score = live_saved_score(raw, normalized, stem, project_name);
        if score == 0 {
            continue;
        }
        let better = match best {
            None => true,
            Some((_, best_score, best_stem)) => {
                score > best_score
                    || (score == best_score && stem.as_bytes() < best_stem.as_bytes())
            }
        };
        if better {
            best = Some((index, score, stem));
        }
    }
    best.map(|(index, _, _)| index)
}

fn live_saved_score(
    raw: &str,
    normalized: Option<&str>,
    stem: &str,
    project_name: Option<&str>,
) -> i32 {
    if raw == stem {
        return 4;
    }
    if project_name == Some(raw) {
        return 3;
    }
    if normalized == Some(stem) {
        return 2;
    }
    if normalized.is_some() && normalized == project_name {
        return 1;
    }
    0
}

fn collect_rows(config: &ConfigDir) -> Result<Vec<PickerRow>> {
    let mut saved = Vec::new();
    for stem in config.list()? {
        let document = config.load(&stem)?;
        saved.push((stem, document.project.name));
    }
    let live = tmux::list_sessions()?;
    Ok(merge_picker_rows(&saved, &live))
}

fn select_picker_row(rows: &[PickerRow]) -> Result<Option<(FzfKey, String)>> {
    let lines: Vec<String> = rows.iter().map(format_picker_row).collect();
    let Some(stdout) = crate::cli::run_fzf(
        [
            "--ansi",
            "--no-sort",
            "--delimiter=\t",
            "--with-nth=2..",
            "--prompt",
            "mux> ",
            "--expect=enter,space",
        ]
        .as_slice(),
        &lines,
    )?
    else {
        return Ok(None);
    };
    Ok(parse_fzf_expect(&stdout))
}

fn open_saved(config: &ConfigDir, name: &str) -> Result<ExitCode> {
    let document = config.load(name)?;
    tmux::open(&document.project)?;
    Ok(ExitCode::SUCCESS)
}

fn ensure_saved(config: &ConfigDir, name: &str) -> Result<()> {
    let document = config.load(name)?;
    tmux::ensure_session(&document.project)
}

fn close_session(config: &ConfigDir, session: &SessionRef) -> Result<CloseOutcome> {
    let snapshot = tmux::snapshot_target(&session.id);
    let project_name = try_normalize_project_name(&session.raw_name);
    let saved = match project_name.as_deref() {
        Some(name) => config.find_saved(name)?,
        None => None,
    };
    let matches = match (&snapshot, &saved) {
        (Ok(snapshot), Some((_, document))) => {
            snapshot.document.to_toml().ok() == document.to_toml().ok()
        }
        _ => false,
    };
    let prompt = close_prompt(saved.is_some(), snapshot.is_ok(), matches);

    let action = match prompt {
        ClosePrompt::Immediate => Some('k'),
        ClosePrompt::Unsaved => choose(
            &format!(
                "Session {:?} is not saved.\n[s]ave and close\n[d]iscard and close\n[C]ancel\n> ",
                session.raw_name
            ),
            &['s', 'd'],
        )?,
        ClosePrompt::Dirty => choose(
            &format!(
                "Session {:?} differs from its saved project.\n[u]pdate and close\n[d]iscard changes and close\n[C]ancel\n> ",
                session.raw_name
            ),
            &['u', 'd'],
        )?,
        ClosePrompt::SnapshotFailed => {
            if let Err(error) = &snapshot {
                eprintln!(
                    "mux: cannot snapshot session {:?}: {error:#}",
                    session.raw_name
                );
            }
            choose("[d]iscard and close\n[C]ancel\n> ", &['d'])?
        }
    };

    match (prompt, action) {
        (ClosePrompt::Immediate, _)
        | (ClosePrompt::Unsaved | ClosePrompt::Dirty | ClosePrompt::SnapshotFailed, Some('d')) => {
            tmux::kill_session(&session.id)?;
            Ok(CloseOutcome::Closed)
        }
        (ClosePrompt::Unsaved, Some('s')) | (ClosePrompt::Dirty, Some('u')) => {
            let snapshot = snapshot?;
            let stem = match &saved {
                Some((stem, _)) => stem.clone(),
                None => resolve_save_name(&session.raw_name, project_name.as_deref())?,
            };
            let toml_name = snapshot
                .project_name
                .clone()
                .unwrap_or_else(|| stem.clone());
            persist_snapshot(config, &snapshot, &stem, &toml_name)?;
            tmux::kill_session(&session.id)?;
            Ok(CloseOutcome::Closed)
        }
        _ => Ok(CloseOutcome::Cancelled),
    }
}

fn resolve_save_name(raw_name: &str, project_name: Option<&str>) -> Result<String> {
    if let Some(name) = project_name {
        return Ok(name.to_owned());
    }
    eprint!(
        "session name {raw_name:?} cannot be converted to a mux project name; enter a name to save: "
    );
    io::stderr().flush()?;
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .context("cannot read project name")?;
    let name = answer.trim();
    if name.is_empty() {
        bail!("close cancelled");
    }
    validate_project_name(name)?;
    Ok(name.to_owned())
}

fn persist_snapshot(
    config: &ConfigDir,
    snapshot: &tmux::Snapshot,
    stem: &str,
    toml_name: &str,
) -> Result<()> {
    validate_project_name(stem)?;
    validate_project_name(toml_name)?;
    let mut document = snapshot.document.clone();
    toml_name.clone_into(&mut document.project.name);
    let source = document.to_toml()?;
    let parsed = crate::project::ProjectDocument::parse(&source)
        .context("generated snapshot TOML is invalid")?;
    for warning in &snapshot.warnings {
        eprintln!("warning: {warning}");
    }
    for warning in lint_project(&parsed) {
        eprintln!("warning: {warning}");
    }
    config.save(stem, source.as_bytes(), true)?;
    Ok(())
}

fn choose(prompt: &str, accepted: &[char]) -> Result<Option<char>> {
    eprint!("{prompt}");
    io::stderr().flush()?;
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let terminal_fd = input.is_terminal().then(|| input.as_raw_fd());
    let choice = read_choice(&mut input, terminal_fd, accepted)?;
    if terminal_fd.is_some() {
        match choice {
            Some(choice) => eprintln!("{choice}"),
            None => eprintln!(),
        }
    }
    Ok(choice)
}

fn read_choice<R: BufRead>(
    input: &mut R,
    terminal_fd: Option<i32>,
    accepted: &[char],
) -> Result<Option<char>> {
    if let Some(terminal_fd) = terminal_fd {
        let mut terminal = ImmediateInput::new(terminal_fd)?;
        let mut answer = [0];
        let read = input.read(&mut answer)?;
        terminal.restore()?;
        return Ok(parse_choice(
            std::str::from_utf8(&answer[..read]).unwrap_or_default(),
            accepted,
        ));
    }
    let mut answer = String::new();
    input.read_line(&mut answer)?;
    Ok(parse_choice(&answer, accepted))
}

struct ImmediateInput {
    fd: i32,
    original: libc::termios,
    restored: bool,
}

impl ImmediateInput {
    fn new(fd: i32) -> io::Result<Self> {
        let mut original = std::mem::MaybeUninit::uninit();
        // SAFETY: `fd` is a live terminal descriptor and `original` points to
        // writable storage for one termios value.
        if unsafe { libc::tcgetattr(fd, original.as_mut_ptr()) } == -1 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: tcgetattr succeeded and initialized the value.
        let original = unsafe { original.assume_init() };
        let mut immediate = original;
        immediate.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG);
        immediate.c_cc[libc::VMIN] = 1;
        immediate.c_cc[libc::VTIME] = 0;
        // SAFETY: `fd` remains live and `immediate` is a valid termios value
        // derived from the terminal's current settings.
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, std::ptr::from_ref(&immediate)) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            fd,
            original,
            restored: false,
        })
    }

    fn restore(&mut self) -> io::Result<()> {
        // SAFETY: the guard cannot outlive the input handle in `read_choice`,
        // so `fd` is live while restoring its captured settings.
        if unsafe { libc::tcsetattr(self.fd, libc::TCSANOW, std::ptr::from_ref(&self.original)) }
            == -1
        {
            return Err(io::Error::last_os_error());
        }
        self.restored = true;
        Ok(())
    }
}

impl Drop for ImmediateInput {
    fn drop(&mut self) {
        if !self.restored {
            // Best effort on early returns. The explicit path reports errors.
            unsafe {
                libc::tcsetattr(self.fd, libc::TCSANOW, std::ptr::from_ref(&self.original));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        label: &str,
        live: bool,
        live_name: Option<&str>,
        saved_name: Option<&str>,
    ) -> PickerRow {
        PickerRow {
            id: format!("s:{label}"),
            label: label.to_owned(),
            live,
            live_name: live_name.map(str::to_owned),
            saved_name: saved_name.map(str::to_owned),
        }
    }

    fn visible(row: &PickerRow) -> (String, bool, Option<String>) {
        (
            format!("{}\t{}", if row.live { "●" } else { "○" }, row.label),
            row.live,
            row.saved_name.clone(),
        )
    }

    #[test]
    fn merge_unions_skips_underscores_and_sorts_in_c_order() {
        let rows = merge_picker_rows(
            &[
                ("zebra".into(), "zebra".into()),
                ("alpha".into(), "alpha".into()),
                ("_hidden".into(), "hidden".into()),
                ("Beta".into(), "Beta".into()),
            ],
            &["middle".into(), "_tmp".into(), "alpha".into()],
        );
        let labels: Vec<_> = rows.iter().map(visible).collect();
        assert_eq!(
            labels,
            [
                ("○\tBeta".into(), false, Some("Beta".into())),
                ("●\talpha".into(), true, Some("alpha".into())),
                ("●\tmiddle".into(), true, None),
                ("○\tzebra".into(), false, Some("zebra".into())),
            ]
        );
    }

    #[test]
    fn merge_collapses_normalized_live_names_onto_saved_projects() {
        let rows = merge_picker_rows(
            &[("my-session-1".into(), "my-session-1".into())],
            &["my session.1".into()],
        );
        assert_eq!(rows.len(), 1);
        assert!(rows[0].live);
        assert_eq!(rows[0].id, "s:my-session-1");
        assert_eq!(rows[0].label, "my-session-1");
        assert_eq!(rows[0].live_name.as_deref(), Some("my session.1"));
        assert_eq!(rows[0].saved_name.as_deref(), Some("my-session-1"));
    }

    #[test]
    fn merge_keeps_alias_stems_and_duplicate_project_names_as_separate_rows() {
        let rows = merge_picker_rows(
            &[
                ("alias".into(), "runtime".into()),
                ("other".into(), "runtime".into()),
            ],
            &["runtime".into()],
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "s:alias");
        assert_eq!(rows[0].label, "alias");
        assert!(rows[0].live);
        assert_eq!(rows[0].live_name.as_deref(), Some("runtime"));
        assert_eq!(rows[1].id, "s:other");
        assert_eq!(rows[1].label, "other");
        assert!(!rows[1].live);
    }

    #[test]
    fn merge_keeps_normalized_colliding_live_sessions_separate() {
        let rows = merge_picker_rows(&[], &["a b".into(), "a-b".into()]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "l:a b");
        assert_eq!(rows[0].label, "a b");
        assert_eq!(rows[1].id, "l:a-b");
        assert_eq!(rows[1].label, "a-b");
    }

    #[test]
    fn unnormalizable_live_sessions_stay_as_their_raw_name() {
        let rows = merge_picker_rows(&[], &["日本語".into()]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "l:日本語");
        assert_eq!(rows[0].label, "日本語");
        assert!(rows[0].live);
        assert!(rows[0].saved_name.is_none());
    }

    #[test]
    fn fzf_expect_output_parses_enter_and_space() {
        assert_eq!(
            parse_fzf_expect("enter\ns:demo\t●\tdemo\n"),
            Some((FzfKey::Enter, "s:demo".into()))
        );
        assert_eq!(
            parse_fzf_expect("space\ns:other\t○\tother\n"),
            Some((FzfKey::Space, "s:other".into()))
        );
        assert_eq!(parse_fzf_expect("esc\ns:demo\t●\tdemo\n"), None);
        assert_eq!(parse_fzf_expect("enter\n\n"), None);
    }

    #[test]
    fn picker_actions_follow_enter_and_space_rules() {
        let live = row("demo", true, Some("demo session"), Some("demo"));
        let stopped = row("demo", false, None, Some("demo"));
        assert_eq!(
            action_for(FzfKey::Enter, &live).unwrap(),
            PickerAction::FocusLive("demo session".into())
        );
        assert_eq!(
            action_for(FzfKey::Enter, &stopped).unwrap(),
            PickerAction::Restore {
                name: "demo".into(),
                focus: true
            }
        );
        assert_eq!(
            action_for(FzfKey::Space, &live).unwrap(),
            PickerAction::CloseLive("demo session".into())
        );
        assert_eq!(
            action_for(FzfKey::Space, &stopped).unwrap(),
            PickerAction::Restore {
                name: "demo".into(),
                focus: false
            }
        );
    }

    #[test]
    fn close_prompt_covers_the_four_spec_states() {
        assert_eq!(close_prompt(true, true, true), ClosePrompt::Immediate);
        assert_eq!(close_prompt(false, true, false), ClosePrompt::Unsaved);
        assert_eq!(close_prompt(true, true, false), ClosePrompt::Dirty);
        assert_eq!(
            close_prompt(true, false, false),
            ClosePrompt::SnapshotFailed
        );
        assert_eq!(
            close_prompt(false, false, false),
            ClosePrompt::SnapshotFailed
        );
    }

    #[test]
    fn close_choices_accept_only_a_single_known_character() {
        assert_eq!(parse_choice("d\n", &['s', 'd']), Some('d'));
        assert_eq!(parse_choice(" S \n", &['s', 'd']), Some('s'));
        assert_eq!(parse_choice("", &['s', 'd']), None);
        assert_eq!(parse_choice("\n", &['s', 'd']), None);
        assert_eq!(parse_choice("\u{1b}", &['s', 'd']), None);
        assert_eq!(parse_choice("x\n", &['s', 'd']), None);
        assert_eq!(parse_choice("save\n", &['s', 'd']), None);
        assert_eq!(parse_choice("C\n", &['s', 'd']), None);
        assert_eq!(parse_choice("u\n", &['u', 'd']), Some('u'));
    }
}
