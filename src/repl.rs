use anyhow::Result;
use reedline::{DefaultPrompt, FileBackedHistory, Reedline, Signal};
use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::common::{HISTORY_FILENAME, MAX_HISTORY_ENTRIES};
use crate::db;
use crate::executor;

/// Runs the interactive REPL loop
///
/// # Arguments
/// * `db` - Database instance for logging commands
/// * `max_output_size` - Maximum output size before redirecting to file
/// * `pty_writer` - Shared writer for Ctrl+C signal handling
pub fn run_repl(
    db: db::Database,
    max_output_size: usize,
    pty_writer: Arc<Mutex<Option<Box<dyn Write + Send>>>>,
) -> Result<()> {
    println!("Cahier started.");
    println!("Database: ./cahier.db");
    println!("Max output size: {} bytes", max_output_size);

    let history = Box::new(
        FileBackedHistory::with_file(MAX_HISTORY_ENTRIES, HISTORY_FILENAME.into())
            .map_err(|e| anyhow::anyhow!("Error creating history file: {:?}", e))?,
    );
    let mut line_editor = Reedline::create().with_history(history);
    let prompt = DefaultPrompt::default();

    // Initialize current environment
    let mut current_env: HashMap<String, String> = std::env::vars().collect();

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
                        db.log_entry(input, "", Some(0), 0, None)?;
                        // Update PWD in current_env
                        if let Ok(cwd) = std::env::current_dir() {
                            if let Some(cwd_str) = cwd.to_str() {
                                current_env.insert("PWD".to_string(), cwd_str.to_string());
                            }
                        }
                    }
                    continue;
                }

                // Execute command
                let start = Instant::now();
                match executor::execute_in_pty(input, max_output_size, &pty_writer, &mut current_env)
                {
                    Ok((output, exit_code, output_file)) => {
                        let duration = start.elapsed();

                        // Save to DB
                        db.log_entry(
                            input,
                            &output,
                            exit_code,
                            duration.as_millis(),
                            output_file.as_deref(),
                        )?;
                    }
                    Err(e) => {
                        eprintln!("Execution error: {}", e);
                    }
                }
            }
            Ok(Signal::CtrlC) => {
                // Handle Ctrl+C at prompt - just continue to next prompt
                println!("^C");
                continue;
            }
            Ok(Signal::CtrlD) => {
                // Handle Ctrl+D - exit the REPL
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

