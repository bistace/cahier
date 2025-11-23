# Cahier

**Cahier** (French for "notebook") is a powerful terminal session recorder and manager written in Rust. It wraps your shell interactions, recording not just the commands you run, but their output, exit codes, execution duration, and more into a structured SQLite database.

Unlike standard shell history which only saves command strings, Cahier preserves the full context of your work, allowing you to generate accurate Markdown reports of your terminal sessions, which is perfect for documenting tutorials, debugging sessions, or keeping a lab notebook of your computational work.

## Features

- **Full Session Recording**: Captures standard output (stdout/stderr), exit codes, and timing for every command.
- **Structured Storage**: Uses a local SQLite database for reliable, queryable storage.
- **Markdown Export**: Generate Markdown logs of your session with a single command.
- **Smart Output Handling**:
  - Automatically redirects excessive output to external files to keep the database clean.
  - Configurable "ignore list" for interactive tools (vim, nano, htop, ssh, ...) to prevent capturing garbage output.
- **Privacy Control**: Prefix any command with `nr` (no-record) to execute it without logging (e.g., `nr echo secret`).
- **Modern REPL Experience**: Built on `reedline`, offering syntax highlighting and file/command autocompletion.
- **Alias Support**: Automatically loads aliases from your shell configuration.

## Installation

### From Source

Ensure you have Rust and Cargo installed.

```bash
git clone https://github.com/yourusername/cahier.git
cd cahier
cargo install --path .
```

Or build manually:

```bash
cargo build --release
cp target/release/cahier /usr/local/bin/
```

## Usage

### Starting a Session

Simply run `cahier` to start the REPL wrapper. It acts like a normal shell.

```bash
cahier
```

You can specify a maximum output capture size (in bytes) before it offloads to a file (default is 16KB):

```bash
cahier start --max-output-size 1048576  # 1MB limit
```

### Commands

Inside Cahier, use your shell commands as usual.

- **Prevent Logging**: Use the `nr` prefix to skip recording a specific command.
  ```bash
  nr export API_KEY="12345"
  ```

### Exporting History

Export your recorded history to a Markdown file. This is useful for creating documentation or sharing logs.

```bash
# Print Markdown to stdout
cahier export

# Save to a file
cahier export --output session_log.md

# Export only the commands (plain text)
cahier export --only-commands
```

## Configuration

Cahier creates a configuration file at `~/.cahier/config.json`. You can customize the behavior by editing this file.

**Default Configuration:**

```json
{
  "ignored_outputs": [
    "nano", "vim", "nvim", "htop", "ssh", "less", "man", "tmux"
  ],
  "theme": "Solarized (dark)",
  "load_aliases": true
}
```

- **`ignored_outputs`**: A list of command names whose output should not be captured (e.g., text editors, interactive TUI tools).
- **`theme`**: Syntax highlighting theme (e.g., "Solarized (dark)", "Solarized (light)", "InspiredGitHub").
- **`load_aliases`**: Whether to import aliases from your parent shell (bash/zsh/etc).

## Technical Architecture

Cahier combines several powerful Rust crates to provide a seamless experience:

- **[Reedline](https://github.com/nushell/reedline)**: Provides the line editor, history, syntax highlighting, and completion engine.
- **[Portable PTY](https://github.com/wez/wezterm/tree/main/pty)**: Creates a pseudo-terminal to execute commands accurately, preserving color codes and formatting.
- **[Rusqlite](https://github.com/rusqlite/rusqlite)**: Interfaces with the SQLite database for robust data persistence.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

1. Fork the project
2. Create your feature branch (`git checkout -b feature/AmazingFeature`)
3. Commit your changes (`git commit -m 'Add some AmazingFeature'`)
4. Push to the branch (`git push origin feature/AmazingFeature`)
5. Open a Pull Request

## 📄 License

This project is licensed under the MIT License.

