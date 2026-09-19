# Sessions and project configuration

[Back to installation and first project](../README.md).

## Save and manage sessions

Inside a tmux session, capture its layout and review any warnings:

```sh
mux snapshot > project.toml
mux save my-project < project.toml
```

`save` refuses an existing name unless you pass `--force`.
`mux save-current` uses the normalized session name and overwrites its saved file.
`mux ls` lists saved names; `mux rm my-project` deletes only the saved definition.

Snapshots preserve window layout, focus, and working directories with limitations.
They do not save process state, command arguments, or unsaved editor buffers.
Only the command names `claude`, `codex`, `cursor-agent`, and `nvim` can be captured.
Other programs and panes with different working directories become empty shell commands.
Review warnings and the generated TOML before relying on a saved project.

`mux close` closes the current session and its processes, keeping the saved definition.
It asks how to proceed for unsaved or changed definitions, or if snapshot capture fails.
A definition matching its snapshot closes immediately.
This does not imply all application data is saved.

For a combined list of saved projects and live sessions, run inside tmux:

```sh
tmux display-popup -E 'mux picker'
```

The picker marks live sessions with `●` and stopped projects with `○`.
Enter focuses or restores the selection and exits.
Space closes a live session or restores a stopped project while keeping the list open.
The same close checks apply. Esc exits; names beginning with `_` are hidden.

## Project configuration

Files live in `$MUX_CONFIG` if set, otherwise `$XDG_CONFIG_HOME/mux` or `~/.config/mux`.
Each saved project is a `.toml` file. Names accept ASCII letters, digits, `_`, and `-`.
The file's name selects the definition; its `name` field names the tmux session.

| Field | Meaning |
|---|---|
| `name` | Required tmux session name |
| `root` | Required working directory; relative paths use the directory where mux runs |
| `startup_window` | Window name to focus; defaults to the first window |
| `startup_pane` | Initial pane position, starting at 1 |
| `windows[].name` | Required, unique window name |
| `windows[].panes` | Required list of shell commands; `""` opens an empty shell |
| `windows[].root` | Optional directory relative to the project root, or an absolute path |
| `windows[].focused_pane` | Pane position to focus in that window, starting at 1 |
| `windows[].layout` | Optional tmux layout string, including its checksum; obtainable from a snapshot |

Unknown fields and `..` components in relative window roots are rejected.
An absolute window root stays fixed when `-c` changes the project root.
Use explicit directory paths; shell variables in TOML paths are not expanded.

Pane commands run in your shell when a new session is created.
Only load definitions you trust. Lint warnings do not prevent commands from running.
`mux lint` checks all saved projects and exits nonzero when warnings are found.

