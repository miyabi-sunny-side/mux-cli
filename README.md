# mux

mux saves tmux projects as TOML and opens their windows and panes from the terminal.
Open a project by name, choose one with fzf, or capture the layout of a running session.
An existing session is focused without rebuilding it.

## Install

Install tmux first. Interactive selection also needs fzf.
Prebuilt binaries are available for Linux x86_64 and macOS ARM64 in
[GitHub Releases](https://github.com/miyabi-sunny-side/mux-cli/releases/latest).

For Linux x86_64, run these commands in an empty directory:

```sh
curl -fLO https://github.com/miyabi-sunny-side/mux-cli/releases/latest/download/mux-linux-x86_64.tar.gz
curl -fLO https://github.com/miyabi-sunny-side/mux-cli/releases/latest/download/mux-linux-x86_64.tar.gz.sha256
sha256sum -c mux-linux-x86_64.tar.gz.sha256
tar -xzf mux-linux-x86_64.tar.gz
mkdir -p "$HOME/.local/bin"
install -m 755 mux "$HOME/.local/bin/mux"
export PATH="$HOME/.local/bin:$PATH"
mux --version
```

Proceed with extraction only if the checksum succeeds.
On macOS, download `mux-macos-aarch64.tar.gz` and its `.sha256` file.
Use `shasum -a 256 -c` to verify it, then extract and install `mux` as above.
Add `~/.local/bin` to your shell's PATH to keep the command available in new terminals.

To build from source, use Rust 1.96 or later:

```sh
git clone https://github.com/miyabi-sunny-side/mux-cli.git
cd mux-cli
cargo install --path . --locked
```

## Open your first project

This example opens two shell panes rooted in your current directory.
Save it under a name that is not already used by a tmux session:

```sh
mux save demo <<'TOML'
name = "demo"
root = "."

[[windows]]
name = "shell"
panes = ["", ""]
TOML
mux check demo
mux lint demo
mux open demo
```

`check` validates the TOML and prints `demo: ok`.
`lint` also reports missing directories and command warnings.
`open` attaches from a regular terminal or switches clients from inside tmux.
Use tmux's usual detach command to leave the session running (default: Ctrl+b, then d).

Run `mux` to choose a saved project with fzf.
Launch overrides apply only to this selector and leave the saved file unchanged:

```sh
mux -t another-session -c /path/to/existing/project
```

Replace the path with an existing directory. These options cannot accompany subcommands.

## More operations

`mux ls` lists saved projects. `mux snapshot` captures a running tmux session.
Snapshots omit process state, arguments, and unsaved buffers.
Review their warnings before using them to restore a project.

See [session management and configuration](docs/usage.md) for more operations.
It covers the picker, close behavior, saved files, and TOML fields.
Pane commands execute in your shell; only open definitions you trust.

## Update and develop

`mux update` installs the latest GitHub release for the supported binary platforms.
It checks the archive checksum and candidate version before replacing the executable.
The executable's directory must be writable.

For development, clone the repository and run:

```sh
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --locked
cargo test --locked tmux::tests::real_tmux_session_is_built_without_user_configuration -- --ignored
cargo test --locked --test tmux_attach -- --ignored
cargo test --locked --test snapshot_tmux -- --ignored
```

The `--ignored` tests drive a real `tmux`, so install it first.
See the [release workflow](.github/workflows/release.yml) for packaging.

Licensed under [MIT](LICENSE).
