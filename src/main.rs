use anyhow::Result;
use clap::{Parser, Subcommand};
use crossterm::terminal;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use reedline::{DefaultPrompt, FileBackedHistory, Reedline, Signal};
use std::io::{self, Read, Write};
use std::time::Instant;

mod db;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Option<Commands>,
    
    /// Optional name for this session (used when starting default mode)
    #[arg(short, long)]
    session: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start a new cahier session (default)
    Start {
        #[arg(short, long)]
        session: Option<String>,
    },
    /// Export a session to markdown
    Export {
        /// The ID of the session to export (defaults to latest)
        #[arg(short, long)]
        id: Option<i64>,
        
        /// Output file path (default: stdout)
        #[arg(short, long)]
        output: Option<String>,

        /// Export only the commands (plain text)
        #[arg(long)]
        only_commands: bool,
    },
    /// List all sessions
    List,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let db = db::Database::init("cahier.db")?;

    match args.command {
        Some(Commands::Export { id, output, only_commands }) => {
            let session_id = match id {
                Some(id) => id,
                None => db.get_last_session_id()?.ok_or_else(|| anyhow::anyhow!("No sessions found"))?,
            };
            
            let content = if only_commands {
                generate_commands_text(&db, session_id)?
            } else {
                generate_markdown(&db, session_id)?
            };
            
            if let Some(path) = output {
                std::fs::write(path, content)?;
            } else {
                println!("{}", content);
            }
            return Ok(());
        }
        Some(Commands::List) => {
            let sessions = db.list_sessions()?;
            println!("{:<5} | {:<25} | {}", "ID", "Start Time", "Name");
            println!("{:-<5} | {:-<25} | {:-<20}", "", "", "");
            for s in sessions {
                println!("{:<5} | {:<25} | {}", s.id, s.start_time, s.name.unwrap_or_default());
            }
            return Ok(());
        }
        Some(Commands::Start { session }) => {
            run_repl(db, session)?;
        }
        None => {
            // Default behavior: start REPL
            run_repl(db, args.session)?;
        }
    }

    Ok(())
}

fn generate_commands_text(db: &db::Database, session_id: i64) -> Result<String> {
    let entries = db.get_entries(session_id)?;
    let mut text = String::new();
    for entry in entries {
        text.push_str(&entry.command);
        text.push('\n');
    }
    Ok(text)
}

fn generate_markdown(db: &db::Database, session_id: i64) -> Result<String> {
    let session = db.get_session(session_id)?;
    let entries = db.get_entries(session_id)?;
    
    let mut md = String::new();
    md.push_str(&format!("# Cahier Session: {}\n\n", session.name.unwrap_or_else(|| format!("Session {}", session_id))));
    md.push_str(&format!("**Date:** {}\n\n", session.start_time));
    
    for entry in entries {
        md.push_str(&format!("### `{}`\n", entry.cwd));
        md.push_str("```bash\n");
        md.push_str(&format!("$ {}\n", entry.command));
        md.push_str("```\n\n");
        
        if !entry.output.is_empty() {
            md.push_str("```\n");
            // Clean up output? For now, just raw.
            // Stripping ANSI codes might be good here for readability in markdown viewers.
            let clean_output = strip_ansi_escapes::strip(&entry.output);
            md.push_str(&String::from_utf8_lossy(&clean_output));
            md.push_str("\n```\n\n");
        }
        
        md.push_str(&format!("*Time: {} | Exit Code: {:?} | Duration: {}ms*\n\n---\n\n", 
            entry.timestamp.format("%H:%M:%S"), 
            entry.exit_code, 
            entry.duration_ms
        ));
    }
    
    Ok(md)
}

fn run_repl(db: db::Database, session_name: Option<String>) -> Result<()> {
    let session_id = db.create_session(session_name)?;

    println!("Cahier session started. (Session ID: {})", session_id);
    println!("Database: ./cahier.db");

    let history = Box::new(
        FileBackedHistory::with_file(5000, "cahier_history.txt".into())
            .map_err(|e| anyhow::anyhow!("Error creating history file: {:?}", e))?
    );
    let mut line_editor = Reedline::create().with_history(history);
    let prompt = DefaultPrompt::default();

    loop {
        let sig = line_editor.read_line(&prompt);
        match sig {
            Ok(Signal::Success(buffer)) => {
                let input = buffer.trim();
                if input.is_empty() {
                    continue;
                }

                if input == "exit" {
                    break;
                }

                // Handle 'cd' manually
                if input.starts_with("cd ") {
                    let path = input.strip_prefix("cd ").unwrap().trim();
                    if let Err(e) = std::env::set_current_dir(path) {
                        eprintln!("Error changing directory: {}", e);
                    } else {
                         // Log cd command as well, though output is empty
                        let cwd = std::env::current_dir()?.to_string_lossy().to_string();
                        db.log_entry(session_id, input, "", &cwd, Some(0), 0)?;
                    }
                    continue;
                }

                // Execute command
                let start = Instant::now();
                match execute_in_pty(input) {
                    Ok((output, exit_code)) => {
                        let duration = start.elapsed();
                        let cwd = std::env::current_dir()?.to_string_lossy().to_string();
                        
                        // Save to DB
                        db.log_entry(
                            session_id,
                            input,
                            &output,
                            &cwd,
                            exit_code,
                            duration.as_millis(),
                        )?;
                    }
                    Err(e) => {
                        eprintln!("Execution error: {}", e);
                    }
                }
            }
            Ok(Signal::CtrlD) | Ok(Signal::CtrlC) => {
                break;
            }
            Err(e) => {
                eprintln!("Error: {:?}", e);
                break;
            }
        }
    }
    Ok(())
}

fn execute_in_pty(command: &str) -> Result<(String, Option<i32>)> {
    let pty_system = native_pty_system();

    // Get terminal size for the PTY to match current term
    let (cols, rows) = terminal::size().unwrap_or((80, 24));
    let pair = pty_system.openpty(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    })?;

    // Use the user's shell or default to sh
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".into());
    let mut cmd = CommandBuilder::new(shell);
    cmd.args(["-c", command]);

    // Inherit environment variables
    for (key, value) in std::env::vars() {
        cmd.env(key, value);
    }
    
    // Set current directory
    cmd.cwd(std::env::current_dir()?);

    let mut child = pair.slave.spawn_command(cmd)?;
    
    // Drop slave to close the write-end of the pipe in this process
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader()?;
    
    let mut buf = [0u8; 1024];
    let mut captured_output = Vec::new();
    
    // Simple loop to read and print
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let data = &buf[..n];
                captured_output.extend_from_slice(data);
                // Write to stdout directly to show user
                let _ = io::stdout().write_all(data);
                let _ = io::stdout().flush();
            }
            Err(_) => break, 
        }
    }

    let exit_status = child.wait()?;
    let exit_code = if exit_status.success() { Some(0) } else { Some(1) }; 

    Ok((String::from_utf8_lossy(&captured_output).to_string(), exit_code))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execute_in_pty() {
        // simple echo
        let (output, exit_code) = execute_in_pty("echo 'hello world'").expect("failed to execute");
        assert!(output.contains("hello world"));
        assert_eq!(exit_code, Some(0));
    }

    #[test]
    fn test_execute_failure() {
        // command not found
        // bash -c "nonexistent" returns 127 (or non-zero)
        let (_output, exit_code) = execute_in_pty("nonexistent_command_123").expect("failed to execute");
        // exit_code depends on shell, but should be Some(non-zero)
        assert_ne!(exit_code, Some(0));
    }
}
