use anyhow::Result;
use clap::Parser;
use crossterm::terminal;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use reedline::{DefaultPrompt, FileBackedHistory, Reedline, Signal};
use std::io::{self, Read, Write};
use std::time::Instant;

mod db;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Optional name for this session
    #[arg(short, long)]
    session: Option<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let db = db::Database::init("cahier.db")?;
    let session_id = db.create_session(args.session)?;

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
