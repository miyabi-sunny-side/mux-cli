use std::{
    ffi::OsStr,
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, SystemTime},
};

struct TmuxServer {
    socket: String,
}

impl TmuxServer {
    fn new() -> Self {
        Self {
            socket: format!(
                "mux-snapshot-test-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .expect("system clock is after the Unix epoch")
                    .as_nanos()
            ),
        }
    }

    fn output<I, S>(&self, arguments: I) -> Output
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        Command::new("tmux")
            .args(["-L", &self.socket, "-f", "/dev/null"])
            .args(arguments)
            .env_remove("TMUX")
            .output()
            .expect("run tmux")
    }

    fn success<I, S>(&self, arguments: I) -> String
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.output(arguments);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .expect("tmux prints UTF-8")
            .trim()
            .to_owned()
    }
}

impl Drop for TmuxServer {
    fn drop(&mut self) {
        let _ = Command::new("tmux")
            .args(["-L", &self.socket, "kill-server"])
            .env_remove("TMUX")
            .status();
    }
}

struct SessionEvidence {
    first_layout: String,
    tmux_environment: String,
    tmux_pane: String,
}

fn create_windows(server: &TmuxServer, roots: [&Path; 3]) -> (String, String, String) {
    let first = server.success([
        "new-session",
        "-d",
        "-P",
        "-F",
        "#{window_id}:#{pane_id}",
        "-s",
        "snapshot session",
        "-n",
        "same",
        "-c",
        roots[0].to_str().expect("temp paths are UTF-8"),
    ]);
    let (first_window, first_pane) = first.split_once(':').expect("tmux prints window:pane");
    server.success([
        "set-window-option",
        "-t",
        first_window,
        "automatic-rename",
        "off",
    ]);
    let second_pane = server.success([
        "split-window",
        "-d",
        "-P",
        "-F",
        "#{pane_id}",
        "-t",
        first_window,
        "-c",
        roots[2].to_str().expect("temp paths are UTF-8"),
    ]);
    server.success(["select-pane", "-t", &second_pane]);

    let second = server.success([
        "new-window",
        "-d",
        "-P",
        "-F",
        "#{window_id}:#{pane_id}",
        "-t",
        "=snapshot session",
        "-n",
        "same",
        "-c",
        roots[1].to_str().expect("temp paths are UTF-8"),
    ]);
    let (second_window, _) = second.split_once(':').expect("tmux prints window:pane");
    server.success([
        "set-window-option",
        "-t",
        second_window,
        "automatic-rename",
        "off",
    ]);
    server.success(["rename-window", "-t", first_window, "same"]);
    server.success(["rename-window", "-t", second_window, "same"]);
    server.success(["select-window", "-t", second_window]);
    (first_window.to_owned(), first_pane.to_owned(), second_pane)
}

fn capture_session_evidence(
    server: &TmuxServer,
    temp: &Path,
    first_window: &str,
    first_pane: &str,
    second_pane: &str,
) -> SessionEvidence {
    let first_layout = server.success([
        "display-message",
        "-p",
        "-t",
        first_window,
        "#{window_layout}",
    ]);
    server.success(["resize-pane", "-Z", "-t", second_pane]);
    let zoomed_layout = server.success([
        "display-message",
        "-p",
        "-t",
        first_window,
        "#{window_layout}",
    ]);
    assert_eq!(
        zoomed_layout, first_layout,
        "zoom must not change the reported layout"
    );
    server.success(["resize-pane", "-Z", "-t", second_pane]);

    let env_path = temp.join("pane-environment");
    let command = format!(
        "printf '%s\\n%s\\n' \"$TMUX\" \"$TMUX_PANE\" > '{}'",
        env_path.display()
    );
    server.success(["send-keys", "-l", "-t", first_pane, "--", &command]);
    server.success(["send-keys", "-t", first_pane, "Enter"]);
    for _ in 0..100 {
        if env_path.is_file() {
            break;
        }
        thread::sleep(Duration::from_millis(25));
    }
    let environment = fs::read_to_string(&env_path).expect("read pane environment");
    let mut lines = environment.lines();
    let tmux_environment = lines.next().expect("pane environment has TMUX").to_owned();
    let tmux_pane = lines
        .next()
        .expect("pane environment has TMUX_PANE")
        .to_owned();
    assert!(lines.next().is_none(), "pane environment has extra lines");
    SessionEvidence {
        first_layout,
        tmux_environment,
        tmux_pane,
    }
}

fn assert_snapshot(source: &str, stderr: &[u8], roots: [&Path; 3], first_layout: &str) {
    let project: toml::Value = toml::from_str(source).expect("snapshot is valid TOML");
    assert_eq!(
        project["name"].as_str(),
        Some("snapshot-session"),
        "project name"
    );
    assert_eq!(project["root"].as_str(), roots[0].to_str(), "project root");
    assert_eq!(
        project["startup_window"].as_str(),
        Some("same-2"),
        "startup window"
    );
    assert_eq!(
        project["startup_pane"].as_integer(),
        Some(1),
        "startup pane"
    );
    let windows = project["windows"].as_array().expect("snapshot has windows");
    assert_eq!(windows.len(), 2, "window count");
    assert_eq!(
        windows[0]["name"].as_str(),
        Some("same"),
        "first window name"
    );
    assert_eq!(
        windows[0]["focused_pane"].as_integer(),
        Some(2),
        "first window focus"
    );
    assert_eq!(
        windows[0]["layout"].as_str(),
        Some(first_layout),
        "first window layout"
    );
    assert_eq!(
        windows[0]["panes"]
            .as_array()
            .expect("first window has panes")
            .len(),
        2,
        "first window pane count"
    );
    assert!(
        windows[0]["panes"]
            .as_array()
            .expect("first window has panes")
            .iter()
            .all(|pane| pane.as_str() == Some("")),
        "snapshot panes have no commands"
    );
    assert!(
        windows[0].get("root").is_none(),
        "first window inherits the project root"
    );
    assert_eq!(
        windows[1]["name"].as_str(),
        Some("same-2"),
        "second window name"
    );
    assert_eq!(
        windows[1]["root"].as_str(),
        Some("other"),
        "second window root"
    );
    let warnings = String::from_utf8_lossy(stderr);
    assert!(warnings.contains("keep names unique"), "{warnings}");
    assert!(warnings.contains("cwd differs"), "{warnings}");
}

fn save_and_check(source: &str, config: &Path) {
    let mut save = Command::new(env!("CARGO_BIN_EXE_mux"))
        .args(["save", "captured"])
        .env("MUX_CONFIG", config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn mux save");
    save.stdin
        .take()
        .expect("stdin was piped")
        .write_all(source.as_bytes())
        .expect("write snapshot to mux save");
    let saved = save.wait_with_output().expect("wait for mux save");
    assert!(
        saved.status.success(),
        "{}",
        String::from_utf8_lossy(&saved.stderr)
    );
    assert!(
        Command::new(env!("CARGO_BIN_EXE_mux"))
            .args(["check", "captured"])
            .env("MUX_CONFIG", config)
            .status()
            .expect("run mux check")
            .success(),
        "saved snapshot fails mux check"
    );
}

fn launch_saved_snapshot(server: &TmuxServer, temp: &Path, config: &Path, roots: [&Path; 2]) {
    let bin = temp.join("bin");
    fs::create_dir(&bin).expect("create fake bin directory");
    let fzf = bin.join("fzf");
    fs::write(
        &fzf,
        "#!/bin/sh\nwhile IFS= read -r _line; do :; done\nprintf 'captured\\n'\n",
    )
    .expect("write fake fzf");
    fs::set_permissions(&fzf, fs::Permissions::from_mode(0o755)).expect("make fake fzf executable");

    let real_tmux = Command::new("sh")
        .args(["-c", "command -v tmux"])
        .output()
        .expect("locate tmux");
    assert!(real_tmux.status.success(), "tmux is not installed");
    let real_tmux = String::from_utf8(real_tmux.stdout).expect("tmux path is UTF-8");
    let tmux = bin.join("tmux");
    fs::write(
        &tmux,
        format!(
            "#!/bin/sh\nif [ \"$1\" = attach-session ]; then exit 0; fi\nexec '{}' -L '{}' -f /dev/null \"$@\"\n",
            real_tmux.trim(),
            server.socket
        ),
    )
    .expect("write tmux wrapper");
    fs::set_permissions(&tmux, fs::Permissions::from_mode(0o755))
        .expect("make tmux wrapper executable");

    let launched = Command::new(env!("CARGO_BIN_EXE_mux"))
        .args(["-t", "roundtrip"])
        .env("MUX_CONFIG", config)
        .env("PATH", &bin)
        .env_remove("TMUX")
        .output()
        .expect("run mux launch");
    assert!(
        launched.status.success(),
        "{}",
        String::from_utf8_lossy(&launched.stderr)
    );

    for (window, expected) in [("same", roots[0]), ("same-2", roots[1])] {
        let paths = server.success([
            "list-panes",
            "-t",
            &format!("=roundtrip:{window}"),
            "-F",
            "#{pane_current_path}",
        ]);
        assert!(
            paths.lines().all(|path| path == expected.to_string_lossy()),
            "{paths}"
        );
    }

    let replacement = temp.join("replacement");
    let replacement_other = replacement.join("other");
    fs::create_dir(&replacement).expect("create replacement root");
    fs::create_dir(&replacement_other).expect("create nested replacement root");
    let overridden = Command::new(env!("CARGO_BIN_EXE_mux"))
        .args(["-t", "cwd-override", "-c"])
        .arg(&replacement)
        .env("MUX_CONFIG", config)
        .env("PATH", &bin)
        .env_remove("TMUX")
        .output()
        .expect("run mux with cwd override");
    assert!(
        overridden.status.success(),
        "{}",
        String::from_utf8_lossy(&overridden.stderr)
    );
    for (window, expected) in [
        ("same", replacement.as_path()),
        ("same-2", replacement_other.as_path()),
    ] {
        let paths = server.success([
            "list-panes",
            "-t",
            &format!("=cwd-override:{window}"),
            "-F",
            "#{pane_current_path}",
        ]);
        assert!(
            paths.lines().all(|path| path == expected.to_string_lossy()),
            "{paths}"
        );
    }
}

#[test]
#[ignore = "requires a real tmux binary"]
fn snapshot_round_trips_through_the_real_tmux_socket() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let other = root.join("other");
    let different = temp.path().join("different");
    for path in [&root, &other, &different] {
        fs::create_dir(path).unwrap();
    }
    let roots = [root.as_path(), other.as_path(), different.as_path()];
    let server = TmuxServer::new();
    let (first_window, first_pane, second_pane) = create_windows(&server, roots);
    let evidence = capture_session_evidence(
        &server,
        temp.path(),
        &first_window,
        &first_pane,
        &second_pane,
    );

    let snapshot = Command::new(env!("CARGO_BIN_EXE_mux"))
        .arg("snapshot")
        .env("TMUX", &evidence.tmux_environment)
        .env("TMUX_PANE", &evidence.tmux_pane)
        .output()
        .unwrap();
    assert!(
        snapshot.status.success(),
        "{}",
        String::from_utf8_lossy(&snapshot.stderr)
    );
    let source = String::from_utf8(snapshot.stdout).unwrap();
    assert_snapshot(&source, &snapshot.stderr, roots, &evidence.first_layout);
    let config = temp.path().join("config");
    save_and_check(&source, &config);
    launch_saved_snapshot(&server, temp.path(), &config, [&root, &other]);
}
