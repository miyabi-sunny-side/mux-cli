use std::{
    env, fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

fn mux(config: &Path, arguments: &[&str], stdin: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mux"));
    command
        .args(arguments)
        .env("MUX_CONFIG", config)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if stdin.is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = command.spawn().unwrap();
    if let Some(stdin) = stdin {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
    }
    child.wait_with_output().unwrap()
}

fn project_toml(root: &Path, name: &str) -> String {
    format!(
        "name = \"{name}\"\nroot = \"{}\"\nstartup_window = \"shell\"\nstartup_pane = 1\n\n[[windows]]\nname = \"shell\"\nfocused_pane = 1\npanes = [\"\"]\n",
        root.display()
    )
}

fn install_snapshot_tmux(bin: &Path) {
    fs::create_dir(bin).unwrap();
    let tmux = bin.join("tmux");
    fs::write(
        &tmux,
        r#"#!/bin/sh
if [ "${MUX_TEST_FAIL:-}" = 1 ] && [ "$*" = 'display-message -p -t @2 #{window_layout}' ]; then
  printf '%s\n' 'injected tmux failure' >&2
  exit 1
fi
case "$*" in
'display-message -p -t %7 #{session_id}') printf '%s\n' '$1' ;;
'display-message -p -t $1 #{session_name}') printf '%s\n' 'demo session.1' ;;
'list-windows -t $1 -F #{window_id}') printf '%s\n' '@1' '@2' ;;
'display-message -p -t $1 #{window_id}') printf '%s\n' '@2' ;;
'display-message -p -t $1 #{default-shell}') printf '%s\n' '/bin/zsh' ;;
'display-message -p -t @1 #{window_name}') printf '%s\n' 'work' ;;
'display-message -p -t @2 #{window_name}') printf '%s\n' 'work' ;;
'display-message -p -t @1 #{window_layout}'|'display-message -p -t @2 #{window_layout}') printf '%s\n' '020a,80x24,0,0{40x24,0,0,1,39x24,41,0,2}' ;;
'list-panes -t @1 -F #{pane_id}') printf '%s\n' '%7' '%8' ;;
'list-panes -t @2 -F #{pane_id}') printf '%s\n' '%9' '%10' ;;
'display-message -p -t @1 #{pane_id}') printf '%s\n' '%8' ;;
'display-message -p -t @2 #{pane_id}') printf '%s\n' '%9' ;;
'display-message -p -t %7 #{pane_current_path}'|'display-message -p -t %8 #{pane_current_path}') printf '%s\n' "$MUX_TEST_ROOT" ;;
'display-message -p -t %9 #{pane_current_path}') printf '%s\n' "$MUX_TEST_OTHER" ;;
'display-message -p -t %10 #{pane_current_path}') printf '%s\n' "$MUX_TEST_DIFFERENT" ;;
'display-message -p -t %7 #{pane_current_command}') printf '%s\n' 'zsh' ;;
'display-message -p -t %8 #{pane_current_command}') printf '%s\n' 'claude' ;;
'display-message -p -t %9 #{pane_current_command}') printf '%s\n' 'node' ;;
'display-message -p -t %10 #{pane_current_command}') printf '%s\n' 'nvim' ;;
*) printf 'unexpected tmux arguments: %s\n' "$*" >&2; exit 1 ;;
esac
"#,
    )
    .unwrap();
    fs::set_permissions(&tmux, fs::Permissions::from_mode(0o755)).unwrap();
}

fn install_launcher_commands(bin: &Path) {
    fs::create_dir(bin).unwrap();
    let fzf = bin.join("fzf");
    fs::write(
        &fzf,
        "#!/bin/sh\nwhile IFS= read -r _line; do :; done\nprintf 'demo\\n'\n",
    )
    .unwrap();
    fs::set_permissions(&fzf, fs::Permissions::from_mode(0o755)).unwrap();

    let tmux = bin.join("tmux");
    fs::write(
        &tmux,
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$MUX_TEST_LOG"
case "$1" in
  has-session) exit 1 ;;
  new-session) printf '%s\n' '$1' ;;
  list-windows) printf '%s\n' '@1' ;;
  new-window) printf '%s\n' '@2' ;;
  list-panes)
    case "$*" in
      *' -t @1 '*) printf '%s\n' '%1' ;;
      *) printf '%s\n' '%3' ;;
    esac
    ;;
  split-window) printf '%s\n' '%2' ;;
esac
exit 0
"#,
    )
    .unwrap();
    fs::set_permissions(&tmux, fs::Permissions::from_mode(0o755)).unwrap();
}

fn fake_snapshot(bin: &Path, config: &Path, roots: [&Path; 3], fail: bool) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mux"));
    command
        .arg("snapshot")
        .env("MUX_CONFIG", config)
        .env("PATH", bin)
        .env("TMUX", "test-socket,1,0")
        .env("TMUX_PANE", "%7")
        .env("MUX_TEST_ROOT", roots[0])
        .env("MUX_TEST_OTHER", roots[1])
        .env("MUX_TEST_DIFFERENT", roots[2]);
    if fail {
        command.env("MUX_TEST_FAIL", "1");
    }
    command.output().unwrap()
}

#[test]
fn save_check_list_lint_and_remove_round_trip() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config");
    let source = project_toml(temp.path(), "demo");

    assert!(
        mux(&config, &["save", "demo"], Some(&source))
            .status
            .success()
    );
    assert!(config.join("demo.toml").is_file());

    let existing = mux(&config, &["save", "demo"], Some(&source));
    assert!(!existing.status.success());
    assert!(String::from_utf8_lossy(&existing.stderr).contains("--force"));
    assert!(
        mux(&config, &["save", "demo", "--force"], Some(&source))
            .status
            .success()
    );

    let check = mux(&config, &["check", "demo"], None);
    assert!(check.status.success());
    assert_eq!(String::from_utf8(check.stdout).unwrap(), "demo: ok\n");
    let list = mux(&config, &["ls"], None);
    assert!(list.status.success());
    assert_eq!(String::from_utf8(list.stdout).unwrap(), "demo\n");
    assert!(mux(&config, &["lint", "demo"], None).status.success());

    assert!(mux(&config, &["rm", "demo"], None).status.success());
    assert!(!mux(&config, &["rm", "demo"], None).status.success());
}

#[test]
fn save_rejects_invalid_input_without_writing() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config");
    let invalid_cases = [
        "not = [\"valid\"",
        "name = \"x\"\nroot = \"/tmp\"\nunknown = true\nwindows = []\n",
        "name = \"x\"\nroot = \"/tmp\"\nstartup_pane = 2\n[[windows]]\nname = \"one\"\npanes = [\"\"]\n",
    ];
    for source in invalid_cases {
        let output = mux(&config, &["save", "bad"], Some(source));
        assert!(!output.status.success(), "{source}");
        assert!(!config.join("bad.toml").exists());
    }
    fs::create_dir_all(&config).unwrap();
    fs::write(config.join("kept.toml"), b"original").unwrap();
    assert!(
        !mux(&config, &["save", "kept", "--force"], Some("invalid = ["))
            .status
            .success()
    );
    assert_eq!(fs::read(config.join("kept.toml")).unwrap(), b"original");
    assert!(
        !mux(
            &config,
            &["save", "../bad"],
            Some(&project_toml(temp.path(), "bad"))
        )
        .status
        .success()
    );
}

#[test]
fn lint_warnings_do_not_block_save_but_make_lint_fail() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config");
    let source = format!(
        "name = \"warn\"\nroot = \"{}\"\n[[windows]]\nname = \"shell\"\npanes = [\"claude --resume\"]\n",
        temp.path().display()
    );
    let save = mux(&config, &["save", "warn"], Some(&source));
    assert!(save.status.success());
    assert!(String::from_utf8_lossy(&save.stderr).contains("warning:"));
    let lint = mux(&config, &["lint", "warn"], None);
    assert_eq!(lint.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&lint.stderr).contains("has arguments"));
    assert_eq!(mux(&config, &["lint"], None).status.code(), Some(1));
}

#[test]
fn missing_projects_and_empty_config_have_distinct_contracts() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("missing");
    let list = mux(&config, &["ls"], None);
    assert!(list.status.success());
    assert!(list.stdout.is_empty());
    assert!(!mux(&config, &["check", "none"], None).status.success());
    assert!(!mux(&config, &["lint", "none"], None).status.success());
    let select = mux(&config, &[], None);
    assert!(!select.status.success());
    assert!(String::from_utf8_lossy(&select.stderr).contains("no projects found"));
}

#[test]
fn snapshot_requires_a_current_tmux_pane_without_writing_config() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config");
    let output = Command::new(env!("CARGO_BIN_EXE_mux"))
        .arg("snapshot")
        .env("MUX_CONFIG", &config)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("inside tmux"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!config.exists());
}

#[test]
fn snapshot_outputs_valid_composable_toml_and_reports_loss_as_warnings() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let other = root.join("other");
    let different = temp.path().join("different");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&other).unwrap();
    fs::create_dir(&different).unwrap();
    let bin = temp.path().join("bin");
    install_snapshot_tmux(&bin);
    let config = temp.path().join("config");
    let roots = [root.as_path(), other.as_path(), different.as_path()];
    let snapshot = fake_snapshot(&bin, &config, roots, false);

    assert!(
        snapshot.status.success(),
        "{}",
        String::from_utf8_lossy(&snapshot.stderr)
    );
    assert!(!config.exists());
    let source = String::from_utf8(snapshot.stdout).unwrap();
    let project: toml::Value = toml::from_str(&source).unwrap();
    assert_eq!(project["name"].as_str(), Some("demo-session-1"));
    assert_eq!(
        project["root"].as_str(),
        Some(root.to_string_lossy().as_ref())
    );
    assert_eq!(project["startup_window"].as_str(), Some("work-2"));
    assert_eq!(project["startup_pane"].as_integer(), Some(1));
    let windows = project["windows"].as_array().unwrap();
    assert_eq!(windows[0]["name"].as_str(), Some("work"));
    assert_eq!(windows[0]["focused_pane"].as_integer(), Some(2));
    assert_eq!(windows[0]["panes"][0].as_str(), Some(""));
    assert_eq!(windows[0]["panes"][1].as_str(), Some("claude"));
    assert!(windows[0].get("root").is_none());
    assert_eq!(windows[1]["name"].as_str(), Some("work-2"));
    assert_eq!(windows[1]["root"].as_str(), Some("other"));
    assert_eq!(windows[1]["panes"][0].as_str(), Some(""));
    assert_eq!(windows[1]["panes"][1].as_str(), Some(""));
    let warnings = String::from_utf8(snapshot.stderr).unwrap();
    assert!(warnings.contains("normalized"), "{warnings}");
    assert!(warnings.contains("keep names unique"), "{warnings}");
    assert!(
        warnings.contains("command \"node\" was omitted"),
        "{warnings}"
    );
    assert!(warnings.contains("cwd differs"), "{warnings}");

    let saved = mux(&config, &["save", "captured"], Some(&source));
    assert!(
        saved.status.success(),
        "{}",
        String::from_utf8_lossy(&saved.stderr)
    );
    assert!(mux(&config, &["check", "captured"], None).status.success());

    let failed = fake_snapshot(&bin, &config, roots, true);
    assert!(!failed.status.success());
    assert!(failed.stdout.is_empty());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("injected tmux failure"));
}

#[test]
fn launcher_overrides_name_and_root_without_rewriting_the_template() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config");
    let original = temp.path().join("original");
    let replacement = temp.path().join("replacement");
    fs::create_dir_all(original.join("nested")).unwrap();
    fs::create_dir_all(replacement.join("nested")).unwrap();
    fs::create_dir(&config).unwrap();
    let source = format!(
        "name = \"demo\"\nroot = \"{}\"\n[[windows]]\nname = \"main\"\npanes = [\"\", \"\"]\n[[windows]]\nname = \"nested\"\nroot = \"nested\"\npanes = [\"\", \"\"]\n",
        original.display()
    );
    let project_path = config.join("demo.toml");
    fs::write(&project_path, &source).unwrap();

    let bin = temp.path().join("bin");
    install_launcher_commands(&bin);
    let log = temp.path().join("tmux.log");
    let output = Command::new(env!("CARGO_BIN_EXE_mux"))
        .args(["-t", "runtime", "-c", "replacement"])
        .current_dir(temp.path())
        .env("MUX_CONFIG", &config)
        .env("MUX_TEST_LOG", &log)
        .env("PATH", &bin)
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let replacement = replacement.canonicalize().unwrap();
    let calls = fs::read_to_string(&log).unwrap();
    assert!(calls.contains("has-session -t =runtime"), "{calls}");
    assert!(
        calls.contains(&format!(
            "new-session -d -P -F #{{session_id}} -s runtime -n main -c {}",
            replacement.display()
        )),
        "{calls}"
    );
    assert!(
        calls.contains(&format!(
            "new-window -d -P -F #{{window_id}} -t $1 -n nested -c {}/nested",
            replacement.display()
        )),
        "{calls}"
    );
    let cwd_arguments: Vec<_> = calls
        .lines()
        .filter_map(|line| line.rsplit_once(" -c ").map(|(_, root)| root))
        .collect();
    assert_eq!(
        cwd_arguments
            .iter()
            .filter(|root| **root == replacement.to_string_lossy())
            .count(),
        2,
        "{calls}"
    );
    assert_eq!(
        cwd_arguments
            .iter()
            .filter(|root| **root == format!("{}/nested", replacement.display()))
            .count(),
        2,
        "{calls}"
    );
    assert!(calls.contains("attach-session -t =runtime"), "{calls}");
    assert_eq!(fs::read_to_string(project_path).unwrap(), source);
}

#[test]
fn launcher_rejects_invalid_overrides_before_contacting_tmux() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config");
    fs::create_dir(&config).unwrap();
    fs::write(config.join("demo.toml"), project_toml(temp.path(), "demo")).unwrap();
    let bin = temp.path().join("bin");
    install_launcher_commands(&bin);
    fs::remove_file(bin.join("tmux")).unwrap();

    for arguments in [vec!["-t", "invalid/name"], vec!["-c", "definitely-missing"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_mux"))
            .args(arguments)
            .current_dir(temp.path())
            .env("MUX_CONFIG", &config)
            .env("PATH", &bin)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("cannot run tmux"));
    }
}

#[test]
fn non_toml_files_are_ignored() {
    let temp = tempfile::tempdir().unwrap();
    let source = project_toml(temp.path(), "same");
    fs::write(temp.path().join("same.yml"), &source).unwrap();
    fs::write(temp.path().join("same.yaml"), &source).unwrap();
    let output = mux(temp.path(), &["ls"], None);
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
}

#[test]
fn toml_is_loaded_and_force_overwritten_in_place() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("demo.toml");
    fs::write(&path, project_toml(temp.path(), "demo")).unwrap();
    assert!(mux(temp.path(), &["check", "demo"], None).status.success());

    let replacement = project_toml(temp.path(), "replacement");
    assert!(
        mux(
            temp.path(),
            &["save", "demo", "--force"],
            Some(&replacement)
        )
        .status
        .success()
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), replacement);
}

fn install_lifecycle_commands(bin: &Path, fzf: Option<&str>) {
    fs::create_dir(bin).unwrap();
    if let Some(script) = fzf {
        let path = bin.join("fzf");
        fs::write(&path, script).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let tmux = bin.join("tmux");
    fs::write(
        &tmux,
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$MUX_TEST_LOG"
if [ "${MUX_TEST_FAIL:-}" = 1 ] && [ "$*" = 'display-message -p -t @2 #{window_layout}' ]; then
  printf '%s\n' 'injected tmux failure' >&2
  exit 1
fi
if [ "${MUX_SNAPSHOT:-}" = 1 ]; then
  case "$*" in
  'list-sessions -F #{session_name}')
    if [ -n "${MUX_LIVE_SESSIONS:-}" ]; then printf '%s\n' "$MUX_LIVE_SESSIONS"; fi
    ;;
  'display-message -p -t %7 #{session_id}'|'display-message -p -t $1 #{session_id}'|'display-message -p -t =demo #{session_id}'|'display-message -p -t =demo session.1 #{session_id}'|'display-message -p -t =live #{session_id}') printf '%s\n' '$1' ;;
  'display-message -p -t $1 #{session_name}') printf '%s\n' "${MUX_SESSION_NAME:-demo session.1}" ;;
  'list-windows -t $1 -F #{window_id}') printf '%s\n' '@1' '@2' ;;
  'display-message -p -t $1 #{window_id}') printf '%s\n' '@2' ;;
  'display-message -p -t $1 #{default-shell}') printf '%s\n' '/bin/zsh' ;;
  'display-message -p -t @1 #{window_name}') printf '%s\n' 'work' ;;
  'display-message -p -t @2 #{window_name}') printf '%s\n' 'work' ;;
  'display-message -p -t @1 #{window_layout}'|'display-message -p -t @2 #{window_layout}') printf '%s\n' '020a,80x24,0,0{40x24,0,0,1,39x24,41,0,2}' ;;
  'list-panes -t @1 -F #{pane_id}') printf '%s\n' '%7' '%8' ;;
  'list-panes -t @2 -F #{pane_id}') printf '%s\n' '%9' '%10' ;;
  'display-message -p -t @1 #{pane_id}') printf '%s\n' '%8' ;;
  'display-message -p -t @2 #{pane_id}') printf '%s\n' '%9' ;;
  'display-message -p -t %7 #{pane_current_path}'|'display-message -p -t %8 #{pane_current_path}') printf '%s\n' "$MUX_TEST_ROOT" ;;
  'display-message -p -t %9 #{pane_current_path}') printf '%s\n' "$MUX_TEST_OTHER" ;;
  'display-message -p -t %10 #{pane_current_path}') printf '%s\n' "$MUX_TEST_DIFFERENT" ;;
  'display-message -p -t %7 #{pane_current_command}') printf '%s\n' 'zsh' ;;
  'display-message -p -t %8 #{pane_current_command}') printf '%s\n' 'claude' ;;
  'display-message -p -t %9 #{pane_current_command}') printf '%s\n' 'node' ;;
  'display-message -p -t %10 #{pane_current_command}') printf '%s\n' 'nvim' ;;
  esac
  exit 0
fi
case "$1" in
  list-sessions)
    if [ -n "${MUX_LIVE_SESSIONS:-}" ]; then printf '%s\n' "$MUX_LIVE_SESSIONS"; fi
    ;;
  has-session) exit "${MUX_HAS_SESSION:-1}" ;;
  new-session) printf '%s\n' '$1' ;;
  list-windows) printf '%s\n' '@1' ;;
  new-window) printf '%s\n' '@2' ;;
  list-panes)
    case "$*" in
      *' -t @1 '*) printf '%s\n' '%1' ;;
      *) printf '%s\n' '%3' ;;
    esac
    ;;
  split-window) printf '%s\n' '%2' ;;
esac
exit 0
"#,
    )
    .unwrap();
    fs::set_permissions(&tmux, fs::Permissions::from_mode(0o755)).unwrap();
}

fn lifecycle_mux(
    config: &Path,
    bin: &Path,
    log: &Path,
    roots: [&Path; 3],
    arguments: &[&str],
    stdin: Option<&str>,
) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mux"));
    command
        .args(arguments)
        .env("MUX_CONFIG", config)
        .env("PATH", bin)
        .env("MUX_TEST_LOG", log)
        .env("TMUX", "test-socket,1,0")
        .env("TMUX_PANE", "%7")
        .env("MUX_TEST_ROOT", roots[0])
        .env("MUX_TEST_OTHER", roots[1])
        .env("MUX_TEST_DIFFERENT", roots[2])
        .env("MUX_SNAPSHOT", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if stdin.is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = command.spawn().unwrap();
    if let Some(stdin) = stdin {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
    }
    child.wait_with_output().unwrap()
}

fn prepend_path(bin: &Path) -> PathBuf {
    env::join_paths(
        std::iter::once(bin.to_path_buf())
            .chain(env::split_paths(&env::var_os("PATH").unwrap_or_default())),
    )
    .unwrap()
    .into()
}

fn lifecycle_roots(temp: &Path) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let root = temp.join("root");
    let other = root.join("other");
    let different = temp.join("different");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&other).unwrap();
    fs::create_dir(&different).unwrap();
    (root, other, different)
}

#[test]
fn help_documents_picker_and_default_selection() {
    let output = Command::new(env!("CARGO_BIN_EXE_mux"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("picker"), "{help}");
    assert!(help.contains("save-current"), "{help}");
    assert!(help.contains("close"), "{help}");
    assert!(help.contains("open"), "{help}");
    assert!(help.contains("display-popup"), "{help}");
    assert!(help.contains("saved projects"), "{help}");
}

#[test]
fn open_launches_a_named_project_without_fzf() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config");
    fs::create_dir(&config).unwrap();
    fs::write(config.join("demo.toml"), project_toml(temp.path(), "demo")).unwrap();
    let bin = temp.path().join("bin");
    install_launcher_commands(&bin);
    fs::remove_file(bin.join("fzf")).unwrap();
    let log = temp.path().join("tmux.log");
    let output = Command::new(env!("CARGO_BIN_EXE_mux"))
        .args(["open", "demo"])
        .env("MUX_CONFIG", &config)
        .env("MUX_TEST_LOG", &log)
        .env("PATH", &bin)
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let calls = fs::read_to_string(&log).unwrap();
    assert!(calls.contains("has-session -t =demo"), "{calls}");
    assert!(calls.contains("new-session"), "{calls}");
    assert!(calls.contains("attach-session -t =demo"), "{calls}");
}

#[test]
fn save_current_force_writes_the_normalized_session_name() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config");
    fs::create_dir(&config).unwrap();
    fs::write(
        config.join("demo-session-1.toml"),
        project_toml(temp.path(), "stale"),
    )
    .unwrap();
    let (root, other, different) = lifecycle_roots(temp.path());
    let bin = temp.path().join("bin");
    install_lifecycle_commands(&bin, None);
    let log = temp.path().join("tmux.log");
    let output = lifecycle_mux(
        &config,
        &bin,
        &log,
        [root.as_path(), other.as_path(), different.as_path()],
        &["save-current"],
        None,
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        "demo-session-1"
    );
    let saved = fs::read_to_string(config.join("demo-session-1.toml")).unwrap();
    assert!(saved.contains("name = \"demo-session-1\""), "{saved}");
    assert!(!saved.contains("stale"), "{saved}");
}

#[test]
fn close_discards_an_unsaved_session_and_does_not_write_toml() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config");
    let (root, other, different) = lifecycle_roots(temp.path());
    let bin = temp.path().join("bin");
    install_lifecycle_commands(&bin, None);
    let log = temp.path().join("tmux.log");
    let output = lifecycle_mux(
        &config,
        &bin,
        &log,
        [root.as_path(), other.as_path(), different.as_path()],
        &["close"],
        Some("d\n"),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let calls = fs::read_to_string(&log).unwrap();
    assert!(calls.contains("kill-session -t $1"), "{calls}");
    assert!(!config.join("demo-session-1.toml").exists());
}

#[test]
fn close_matching_saved_session_is_immediate() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config");
    fs::create_dir(&config).unwrap();
    let (root, other, different) = lifecycle_roots(temp.path());
    let bin = temp.path().join("bin");
    install_lifecycle_commands(&bin, None);
    let log = temp.path().join("tmux.log");
    let roots = [root.as_path(), other.as_path(), different.as_path()];
    let saved = lifecycle_mux(&config, &bin, &log, roots, &["save-current"], None);
    assert!(
        saved.status.success(),
        "{}",
        String::from_utf8_lossy(&saved.stderr)
    );
    fs::write(&log, []).unwrap();
    let output = lifecycle_mux(&config, &bin, &log, roots, &["close"], None);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let calls = fs::read_to_string(&log).unwrap();
    assert!(calls.contains("kill-session -t $1"), "{calls}");
}

#[test]
fn close_dirty_session_updates_or_cancels_from_stdin() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config");
    fs::create_dir(&config).unwrap();
    fs::write(
        config.join("demo-session-1.toml"),
        project_toml(temp.path(), "demo-session-1"),
    )
    .unwrap();
    let (root, other, different) = lifecycle_roots(temp.path());
    let bin = temp.path().join("bin");
    install_lifecycle_commands(&bin, None);
    let log = temp.path().join("tmux.log");
    let roots = [root.as_path(), other.as_path(), different.as_path()];

    let cancelled = lifecycle_mux(&config, &bin, &log, roots, &["close"], Some("\n"));
    assert_eq!(cancelled.status.code(), Some(1));
    assert!(!fs::read_to_string(&log).unwrap().contains("kill-session"));

    let discarded = lifecycle_mux(&config, &bin, &log, roots, &["close"], Some("x\n"));
    assert_eq!(discarded.status.code(), Some(1));

    fs::write(&log, []).unwrap();
    let updated = lifecycle_mux(&config, &bin, &log, roots, &["close"], Some("u\n"));
    assert!(
        updated.status.success(),
        "{}",
        String::from_utf8_lossy(&updated.stderr)
    );
    let saved = fs::read_to_string(config.join("demo-session-1.toml")).unwrap();
    assert!(
        saved.contains("startup_window = \"work-2\"") && saved.contains("name = \"work\""),
        "{saved}"
    );
    assert!(!saved.contains("startup_window = \"shell\""), "{saved}");
    assert!(
        fs::read_to_string(&log)
            .unwrap()
            .contains("kill-session -t $1")
    );
}

#[test]
fn close_dirty_update_rewrites_the_alias_stem_only() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config");
    fs::create_dir(&config).unwrap();
    let stale = project_toml(temp.path(), "demo-session-1");
    fs::write(config.join("alias.toml"), &stale).unwrap();
    let (root, other, different) = lifecycle_roots(temp.path());
    let bin = temp.path().join("bin");
    install_lifecycle_commands(&bin, None);
    let log = temp.path().join("tmux.log");
    let roots = [root.as_path(), other.as_path(), different.as_path()];

    let updated = lifecycle_mux(&config, &bin, &log, roots, &["close"], Some("u\n"));
    assert!(
        updated.status.success(),
        "{}",
        String::from_utf8_lossy(&updated.stderr)
    );
    let alias = fs::read_to_string(config.join("alias.toml")).unwrap();
    assert!(
        alias.contains("startup_window = \"work-2\"")
            && alias.contains("name = \"demo-session-1\""),
        "{alias}"
    );
    assert!(!config.join("demo-session-1.toml").exists());
    assert!(
        fs::read_to_string(&log)
            .unwrap()
            .contains("kill-session -t $1")
    );
}

#[test]
fn close_snapshot_failure_discards_without_writing() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config");
    fs::create_dir(&config).unwrap();
    fs::write(
        config.join("demo-session-1.toml"),
        project_toml(temp.path(), "demo-session-1"),
    )
    .unwrap();
    let (root, other, different) = lifecycle_roots(temp.path());
    let bin = temp.path().join("bin");
    install_lifecycle_commands(&bin, None);
    let log = temp.path().join("tmux.log");
    let mut command = Command::new(env!("CARGO_BIN_EXE_mux"));
    command
        .arg("close")
        .env("MUX_CONFIG", &config)
        .env("PATH", &bin)
        .env("MUX_TEST_LOG", &log)
        .env("TMUX", "test-socket,1,0")
        .env("TMUX_PANE", "%7")
        .env("MUX_TEST_ROOT", &root)
        .env("MUX_TEST_OTHER", &other)
        .env("MUX_TEST_DIFFERENT", &different)
        .env("MUX_TEST_FAIL", "1")
        .env("MUX_SNAPSHOT", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    child.stdin.take().unwrap().write_all(b"d\n").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        fs::read_to_string(&log)
            .unwrap()
            .contains("kill-session -t $1")
    );
    assert_eq!(
        fs::read_to_string(config.join("demo-session-1.toml")).unwrap(),
        project_toml(temp.path(), "demo-session-1")
    );
}

#[test]
fn picker_enter_restores_a_stopped_project_and_focuses() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config");
    fs::create_dir(&config).unwrap();
    fs::write(config.join("demo.toml"), project_toml(temp.path(), "demo")).unwrap();
    let (root, other, different) = lifecycle_roots(temp.path());
    let bin = temp.path().join("bin");
    let input = temp.path().join("fzf-input");
    install_lifecycle_commands(
        &bin,
        Some(
            "#!/bin/sh\ncat > \"$MUX_FZF_INPUT\"\nprintf '%s\\n' \"${MUX_FZF_KEY}\"\nprintf '%s\\n' \"${MUX_FZF_ROW}\"\n",
        ),
    );
    let log = temp.path().join("tmux.log");
    let output = Command::new(env!("CARGO_BIN_EXE_mux"))
        .arg("picker")
        .env("MUX_CONFIG", &config)
        .env("PATH", prepend_path(&bin))
        .env("MUX_TEST_LOG", &log)
        .env("MUX_FZF_INPUT", &input)
        .env("MUX_FZF_KEY", "enter")
        .env("MUX_FZF_ROW", "s:demo\t○\tdemo")
        .env("TMUX", "test-socket,1,0")
        .env("MUX_TEST_ROOT", &root)
        .env("MUX_TEST_OTHER", &other)
        .env("MUX_TEST_DIFFERENT", &different)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let listed = fs::read_to_string(&input).unwrap();
    assert_eq!(listed, "s:demo\t○\tdemo\n");
    let calls = fs::read_to_string(&log).unwrap();
    assert!(calls.contains("new-session"), "{calls}");
    assert!(calls.contains("switch-client -t =demo"), "{calls}");
}

#[test]
fn picker_enter_focuses_a_live_session_without_creating() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config");
    fs::create_dir(&config).unwrap();
    let (root, other, different) = lifecycle_roots(temp.path());
    let bin = temp.path().join("bin");
    let input = temp.path().join("fzf-input");
    install_lifecycle_commands(
        &bin,
        Some(
            "#!/bin/sh\ncat > \"$MUX_FZF_INPUT\"\nprintf '%s\\n' \"${MUX_FZF_KEY}\"\nprintf '%s\\n' \"${MUX_FZF_ROW}\"\n",
        ),
    );
    let log = temp.path().join("tmux.log");
    let output = Command::new(env!("CARGO_BIN_EXE_mux"))
        .arg("picker")
        .env("MUX_CONFIG", &config)
        .env("PATH", prepend_path(&bin))
        .env("MUX_TEST_LOG", &log)
        .env("MUX_FZF_INPUT", &input)
        .env("MUX_FZF_KEY", "enter")
        .env("MUX_FZF_ROW", "l:live\t●\tlive")
        .env("MUX_LIVE_SESSIONS", "live")
        .env("TMUX", "test-socket,1,0")
        .env("MUX_TEST_ROOT", &root)
        .env("MUX_TEST_OTHER", &other)
        .env("MUX_TEST_DIFFERENT", &different)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read_to_string(&input).unwrap(), "l:live\t●\tlive\n");
    let calls = fs::read_to_string(&log).unwrap();
    assert!(calls.contains("switch-client -t =live"), "{calls}");
    assert!(!calls.contains("new-session"), "{calls}");
}

#[test]
fn picker_space_restores_without_focus_and_keeps_listing() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config");
    fs::create_dir(&config).unwrap();
    fs::write(config.join("demo.toml"), project_toml(temp.path(), "demo")).unwrap();
    let (root, other, different) = lifecycle_roots(temp.path());
    let bin = temp.path().join("bin");
    let input = temp.path().join("fzf-input");
    let count = temp.path().join("fzf-n");
    install_lifecycle_commands(
        &bin,
        Some(
            r#"#!/bin/sh
n=0
if [ -f "$MUX_FZF_N" ]; then n=$(cat "$MUX_FZF_N"); fi
n=$((n+1))
printf '%s\n' "$n" > "$MUX_FZF_N"
cat > "$MUX_FZF_INPUT.$n"
if [ "$n" = 1 ]; then
  printf '%s\n' space
  printf '%s\n' 's:demo	○	demo'
else
  exit 130
fi
"#,
        ),
    );
    let log = temp.path().join("tmux.log");
    let output = Command::new(env!("CARGO_BIN_EXE_mux"))
        .arg("picker")
        .env("MUX_CONFIG", &config)
        .env("PATH", prepend_path(&bin))
        .env("MUX_TEST_LOG", &log)
        .env("MUX_FZF_INPUT", &input)
        .env("MUX_FZF_N", &count)
        .env("TMUX", "test-socket,1,0")
        .env("MUX_TEST_ROOT", &root)
        .env("MUX_TEST_OTHER", &other)
        .env("MUX_TEST_DIFFERENT", &different)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let calls = fs::read_to_string(&log).unwrap();
    assert!(calls.contains("new-session"), "{calls}");
    assert!(!calls.contains("switch-client"), "{calls}");
    assert!(!calls.contains("attach-session"), "{calls}");
    assert_eq!(fs::read_to_string(&count).unwrap().trim(), "2");
}

#[test]
fn picker_skips_underscore_names_and_sorts_in_c_order() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config");
    fs::create_dir(&config).unwrap();
    fs::write(
        config.join("zebra.toml"),
        project_toml(temp.path(), "zebra"),
    )
    .unwrap();
    fs::write(
        config.join("alpha.toml"),
        project_toml(temp.path(), "alpha"),
    )
    .unwrap();
    fs::write(
        config.join("_hidden.toml"),
        project_toml(temp.path(), "hidden"),
    )
    .unwrap();
    let (root, other, different) = lifecycle_roots(temp.path());
    let bin = temp.path().join("bin");
    let input = temp.path().join("fzf-input");
    install_lifecycle_commands(
        &bin,
        Some("#!/bin/sh\ncat > \"$MUX_FZF_INPUT\"\nexit 130\n"),
    );
    let log = temp.path().join("tmux.log");
    let output = Command::new(env!("CARGO_BIN_EXE_mux"))
        .arg("picker")
        .env("MUX_CONFIG", &config)
        .env("PATH", prepend_path(&bin))
        .env("MUX_TEST_LOG", &log)
        .env("MUX_FZF_INPUT", &input)
        .env("MUX_LIVE_SESSIONS", "middle\n_tmp")
        .env("TMUX", "test-socket,1,0")
        .env("MUX_TEST_ROOT", &root)
        .env("MUX_TEST_OTHER", &other)
        .env("MUX_TEST_DIFFERENT", &different)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(&input).unwrap(),
        "s:alpha\t○\talpha\nl:middle\t●\tmiddle\ns:zebra\t○\tzebra\n"
    );
}

#[test]
fn selector_alone_requires_fzf_and_cancellation_is_silent() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("demo.toml"),
        project_toml(temp.path(), "demo"),
    )
    .unwrap();

    let missing = Command::new(env!("CARGO_BIN_EXE_mux"))
        .env("MUX_CONFIG", temp.path())
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("fzf is required"));

    let bin = temp.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let fzf = bin.join("fzf");
    fs::write(&fzf, "#!/bin/sh\nexit 130\n").unwrap();
    fs::set_permissions(&fzf, fs::Permissions::from_mode(0o755)).unwrap();
    let cancelled = Command::new(env!("CARGO_BIN_EXE_mux"))
        .env("MUX_CONFIG", temp.path())
        .env("PATH", &bin)
        .output()
        .unwrap();
    assert_eq!(cancelled.status.code(), Some(130));
    assert!(cancelled.stderr.is_empty());
}

#[test]
fn selector_switches_inside_tmux_and_attaches_outside() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("demo.toml"),
        project_toml(temp.path(), "demo"),
    )
    .unwrap();
    let bin = temp.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let fzf = bin.join("fzf");
    fs::write(
        &fzf,
        "#!/bin/sh\nwhile IFS= read -r _line; do :; done\nprintf 'demo\\n'\n",
    )
    .unwrap();
    fs::set_permissions(&fzf, fs::Permissions::from_mode(0o755)).unwrap();
    let tmux = bin.join("tmux");
    fs::write(
        &tmux,
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$MUX_TEST_LOG\"\nexit 0\n",
    )
    .unwrap();
    fs::set_permissions(&tmux, fs::Permissions::from_mode(0o755)).unwrap();
    let log = temp.path().join("tmux.log");

    let inside = Command::new(env!("CARGO_BIN_EXE_mux"))
        .env("MUX_CONFIG", temp.path())
        .env("MUX_TEST_LOG", &log)
        .env("PATH", &bin)
        .env("TMUX", "test")
        .output()
        .unwrap();
    assert!(
        inside.status.success(),
        "{}",
        String::from_utf8_lossy(&inside.stderr)
    );
    let calls = fs::read_to_string(&log).unwrap();
    assert!(calls.contains("has-session -t =demo"), "{calls}");
    assert!(calls.contains("switch-client -t =demo"), "{calls}");

    fs::write(&log, "").unwrap();
    let outside = Command::new(env!("CARGO_BIN_EXE_mux"))
        .env("MUX_CONFIG", temp.path())
        .env("MUX_TEST_LOG", &log)
        .env("PATH", &bin)
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert!(
        outside.status.success(),
        "{}",
        String::from_utf8_lossy(&outside.stderr)
    );
    let calls = fs::read_to_string(&log).unwrap();
    assert!(calls.contains("attach-session -t =demo"), "{calls}");
}
