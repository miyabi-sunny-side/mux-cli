use std::{
    io::{ErrorKind, Write},
    path::PathBuf,
    process::{Child, Command as ProcessCommand, Stdio},
};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Select, validate, and launch tmux projects",
    long_about = "With no command, mux lists saved projects in fzf and launches the selection.\n\
                  Use mux picker for the unified saved-and-live list intended for tmux display-popup.",
    after_help = "With no command, mux lists saved projects in fzf and launches the selection.\n\
                  mux picker lists saved projects and live sessions (● live / ○ stopped) for tmux display-popup.",
    args_conflicts_with_subcommands = true
)]
pub struct Cli {
    /// Override the selected project's tmux session name.
    #[arg(short = 't', long = "target", value_name = "NAME")]
    pub target: Option<String>,
    /// Override the selected project's top-level working directory.
    #[arg(short = 'c', long = "cwd", value_name = "DIR")]
    pub cwd: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Print a TOML project for the current tmux session.
    Snapshot,
    /// Save a TOML project from standard input.
    Save {
        name: String,
        #[arg(long)]
        force: bool,
    },
    /// Remove a saved project.
    Rm { name: String },
    /// List saved project names.
    Ls,
    /// Check whether one project can be launched.
    Check { name: String },
    /// Report safety and quality warnings.
    Lint { name: Option<String> },
    /// Update mux from the latest GitHub Release.
    Update,
    /// List saved projects and live sessions, then focus, restore, or close them.
    ///
    /// Intended to run from `tmux display-popup`. Requires fzf.
    /// Live sessions are marked ● and stopped saved projects are marked ○.
    /// Names starting with `_` are hidden. Rows are sorted in C locale order.
    /// Enter focuses a live session or restores a stopped project and exits.
    /// Space toggles (close confirm if live, restore without focus if stopped) and stays in the list.
    /// Esc exits.
    Picker,
    /// Snapshot the current tmux session and overwrite its normalized project file.
    SaveCurrent,
    /// Close the current tmux session without deleting its saved project file.
    Close,
    /// Launch or switch to a saved project without fzf.
    Open { name: String },
}

pub(crate) fn run_fzf(args: &[&str], lines: &[String]) -> Result<Option<String>> {
    let child = ProcessCommand::new("fzf")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("fzf is required for interactive project selection")?;
    finish_fzf(child, lines)
}

fn finish_fzf(mut child: Child, lines: &[String]) -> Result<Option<String>> {
    {
        let mut stdin = child.stdin.take().context("cannot open fzf stdin")?;
        for line in lines {
            if let Err(error) = writeln!(stdin, "{line}") {
                if error.kind() == ErrorKind::BrokenPipe {
                    break;
                }
                return Err(error.into());
            }
        }
    }
    let output = child.wait_with_output().context("cannot wait for fzf")?;
    if !output.status.success() {
        return Ok(None);
    }
    String::from_utf8(output.stdout)
        .context("fzf returned non-UTF-8 output")
        .map(Some)
}

pub fn select_with_fzf(names: &[String]) -> Result<Option<String>> {
    let Some(selected) = run_fzf(
        ["--prompt", "mux> ", "--height", "40%", "--reverse"].as_slice(),
        names,
    )?
    else {
        return Ok(None);
    };
    let selected = selected.trim_end_matches(['\r', '\n']);
    if selected.is_empty() {
        return Ok(None);
    }
    if !names.iter().any(|name| name == selected) {
        bail!("fzf returned an unknown project");
    }
    Ok(Some(selected.to_owned()))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn fzf_can_finish_before_reading_candidates() {
        for (script, expected) in [("exit 130", None), ("printf 'demo\\n'", Some("demo\n"))] {
            let mut child = ProcessCommand::new("/bin/sh")
                .args(["-c", script])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            // Keep the write end open while the reader exits, making the race deterministic.
            let stdin = child.stdin.take();
            child.wait().unwrap();
            child.stdin = stdin;
            assert_eq!(
                finish_fzf(child, &["demo".to_owned()]).unwrap().as_deref(),
                expected
            );
        }
    }

    #[test]
    fn launcher_overrides_accept_short_and_long_forms() {
        let short = Cli::try_parse_from(["mux", "-t", "work", "-c", "/tmp"]).unwrap();
        assert_eq!(short.target.as_deref(), Some("work"));
        assert_eq!(short.cwd.as_deref(), Some(Path::new("/tmp")));

        let long = Cli::try_parse_from(["mux", "--target", "work", "--cwd", "/tmp"]).unwrap();
        assert_eq!(long.target.as_deref(), Some("work"));
        assert_eq!(long.cwd.as_deref(), Some(Path::new("/tmp")));
    }

    #[test]
    fn launcher_overrides_conflict_with_subcommands_in_both_orders() {
        assert!(Cli::try_parse_from(["mux", "-t", "work", "check", "demo"]).is_err());
        assert!(Cli::try_parse_from(["mux", "check", "demo", "-t", "work"]).is_err());
        assert!(Cli::try_parse_from(["mux", "-c", "/tmp", "ls"]).is_err());
        assert!(Cli::try_parse_from(["mux", "ls", "-c", "/tmp"]).is_err());
        assert!(Cli::try_parse_from(["mux", "-t", "work", "open", "demo"]).is_err());
    }

    #[test]
    fn lifecycle_commands_parse_without_launcher_overrides() {
        assert!(matches!(
            Cli::try_parse_from(["mux", "picker"]).unwrap().command,
            Some(Command::Picker)
        ));
        assert!(matches!(
            Cli::try_parse_from(["mux", "save-current"])
                .unwrap()
                .command,
            Some(Command::SaveCurrent)
        ));
        assert!(matches!(
            Cli::try_parse_from(["mux", "close"]).unwrap().command,
            Some(Command::Close)
        ));
        let open = Cli::try_parse_from(["mux", "open", "demo"]).unwrap();
        assert!(matches!(
            open.command,
            Some(Command::Open { name }) if name == "demo"
        ));
        assert!(Cli::try_parse_from(["mux", "open"]).is_err());
    }
}
