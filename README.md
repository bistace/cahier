# Cahier

**Cahier** is a terminal session recorder and notebook written in Rust.

It runs your commands through your shell inside a PTY, captures command output, exit status, and execution time, and stores the session in a structured SQLite database. The result is a project-local notebook you can browse in a TUI and export to Markdown.

## Why Cahier

Standard shell history records command lines. Cahier records the working session around them:

- command text
- stdout and stderr
- exit codes
- execution duration
- annotations
- ordering and separators
- reusable snippets

This makes it useful for keeping development notes, reconstructing terminal sessions, and exporting clean project logs.

## Features

- Records shell commands with output, exit status, and duration.
- Stores session data in SQLite.
- Exports history as Markdown or plain command lists.
- Opens an interactive TUI to review, reorder, annotate, and delete entries.
- Supports project and global snippets.
- Redirects oversized command output to external files instead of bloating the database.
- Lets you skip persistence for a command with the `nr` prefix.
- Can import aliases from your interactive shell at startup.
- Uses restrictive Unix permissions for sensitive local files where supported.

## Installation

### From crates.io

```bash
cargo install cahier
```

### From source

```bash
git clone https://github.com/bistace/cahier.git
cd cahier
cargo install --path .
```

### Build a local binary

```bash
cargo build --release
cp target/release/cahier /usr/local/bin/
```

## Quick Start

Start Cahier in the project you want to track:

```bash
cahier
```

On first use, Cahier creates a project-local notebook under `./cahier_logs/`:

```text
cahier_logs/
|-- cahier.db
|-- cahier_history.txt
|-- outputs/
`-- env_state.json   # only used when restore_env is enabled
```

Run commands as usual inside the REPL. Later, export the notebook:

```bash
cahier export --output session.md
```

## Usage

### Start the REPL

`cahier` starts the REPL wrapper with the default output capture limit of 16384 bytes.

```bash
cahier
```

You can also start it explicitly and change the output threshold:

```bash
cahier start --max-output-size 1048576
```

Commands are executed through your current shell from `$SHELL` using `-c`, inside a pseudo-terminal. Cahier also provides built-ins for stateful shell-like behavior where needed.

### Built-in commands

The REPL supports these built-ins:

- `cd`
- `jobs`
- `fg`
- `alias`
- `unalias`
- `edit`
- `exit`

All other commands are executed through your shell.

### Skip recording for a command

Prefix a command with `nr` to execute it without storing it in the notebook or writing captured output to disk:

```bash
nr export API_KEY="12345"
nr echo "temporary secret"
```

### Export history

Export the recorded notebook as Markdown:

```bash
cahier export
```

Write the export to a file:

```bash
cahier export --output session_log.md
```

Export only the command lines:

```bash
cahier export --only-commands
```

Markdown export includes:

- optional annotations
- a status line in the form `(exit_code - duration_ms)`
- the command prefixed with `$`
- captured output, or a reference to an external output file

### Interactive editor

Open the TUI from the command line:

```bash
cahier edit
```

Or from inside the REPL:

```bash
edit
```

History view key bindings:

- `j` / `Down`: next entry
- `k` / `Up`: previous entry
- `Enter`: toggle fullscreen output view
- `p`: collapse or expand the preview pane
- `a`: annotate entry
- `b`: save selected command as a snippet
- `d`: delete entry
- `J`: move entry down
- `K`: move entry up
- `Space`: insert a separator
- `s`: send selected command back to the REPL
- `S`: open the snippet browser
- `q`: quit

Snippet browser key bindings:

- `j` / `Down`: next snippet
- `k` / `Up`: previous snippet
- `s`: send selected snippet back to the REPL
- `d`: delete snippet
- `q`: return to history view

When creating a snippet, Cahier lets you set:

- `name`
- `description`
- `scope` (`project` or `global`)
- `tags`

Project snippets are stored in the current notebook database. Global snippets are stored in `~/.cahier/snippets.db`.

## Storage Model

Cahier uses two storage scopes:

- Project-local data in `./cahier_logs/`
  - session database
  - REPL history file
  - redirected output files
  - optional persisted environment state
- User-level data in `~/.cahier/`
  - `config.json`
  - `snippets.db` for global snippets

This split keeps notebooks tied to the current project while allowing shared configuration and reusable snippets.

## Configuration

Cahier loads configuration from `~/.cahier/config.json`. If the file does not exist, Cahier creates it automatically.

Default configuration:

```json
{
  "ignored_outputs": [
    "nano",
    "vim",
    "vi",
    "emacs",
    "hx",
    "atom",
    "gedit",
    "geany",
    "kate",
    "kwrite",
    "nvim",
    "htop",
    "top",
    "atop",
    "less",
    "more",
    "man",
    "ssh",
    "tmux",
    "screen"
  ],
  "theme": "Solarized (dark)",
  "load_aliases": true,
  "restore_env": false
}
```

Configuration fields:

- `ignored_outputs`: commands whose output should not be captured and whose executions should not be persisted to the notebook.
- `theme`: syntax highlighting theme used by the REPL highlighter.
- `load_aliases`: whether Cahier should load aliases from your interactive shell at startup.
- `restore_env`: whether Cahier should persist environment state between sessions.

### `restore_env` behavior

When `restore_env` is enabled, Cahier stores environment variables to `./cahier_logs/env_state.json` and attempts to restore them, including `PWD`, on the next start.

When it is disabled, environment changes still persist for the lifetime of the current Cahier session, but they are not written for reuse in later sessions.

## Security and Privacy

- `nr ...` executes a command without logging it to the database.
- Oversized output is redirected to a file under `./cahier_logs/outputs/` when capture is enabled.
- Commands listed in `ignored_outputs` are executed without output capture and are not persisted to the notebook.
- On Unix, Cahier uses restrictive permissions for sensitive files and directories where it creates them:
  - notebook directories and output directories: `0700`
  - config, env-state, and redirected output files: `0600`

If you enable `restore_env`, be aware that environment values may be written to disk. Do not enable it casually on machines or workflows that handle sensitive secrets.

## Technical Notes

Cahier is built with:

- [`clap`](https://github.com/clap-rs/clap) for the CLI
- [`portable-pty`](https://github.com/wez/wezterm/tree/main/pty) for PTY-backed command execution
- [`reedline`](https://github.com/nushell/reedline) for the REPL editor, completion, and history integration
- [`ratatui`](https://github.com/ratatui/ratatui) for the interactive TUI
- [`rusqlite`](https://github.com/rusqlite/rusqlite) for SQLite persistence

## Development

Run the test suite with:

```bash
cargo test
```

## License

This project is licensed under the MIT License. See [LICENSE](LICENSE).
