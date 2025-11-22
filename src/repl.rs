use anyhow::Result;
use reedline::{ColumnarMenu, Emacs, FileBackedHistory, KeyCode, KeyModifiers, Reedline, ReedlineEvent, ReedlineMenu, Signal};
use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::common::{HISTORY_FILENAME, MAX_HISTORY_ENTRIES, DB_FILENAME};
use crate::completion::FileCompleter;
use crate::config::Config;
use crate::db;
use crate::executor::{self, Job};
use crate::prompt::CahierPrompt;
use crate::command::{self, Registry, CommandContext, CommandResult};

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
    config: Config,
) -> Result<()> {
    println!("Cahier started.");
    println!("Database: ./{}", DB_FILENAME);
    println!("Max output size: {} bytes", max_output_size);

    let history = Box::new(
        FileBackedHistory::with_file(MAX_HISTORY_ENTRIES, HISTORY_FILENAME.into())
            .map_err(|e| anyhow::anyhow!("Error creating history file: {:?}", e))?,
    );
    // Bind Tab to the completion menu
    let mut keybindings = reedline::default_emacs_keybindings();
    keybindings.add_binding(
        KeyModifiers::from_bits_truncate(0),
        KeyCode::Tab,
        ReedlineEvent::Menu("completion_menu".to_string()),
    );
    let edit_mode = Emacs::new(keybindings);

    // Initialize current environment
    let current_env: Arc<Mutex<HashMap<String, String>>> = 
        Arc::new(Mutex::new(std::env::vars().collect()));

    let mut line_editor = Reedline::create()
        .with_history(history)
        .with_completer(Box::new(FileCompleter::new(current_env.clone())))
        .with_menu(ReedlineMenu::EngineCompleter(Box::new(ColumnarMenu::default().with_name("completion_menu"))))
        .with_edit_mode(Box::new(edit_mode));
    let mut prompt = CahierPrompt::new();

    let mut jobs: Vec<Job> = Vec::new();
    
    let mut registry = Registry::new();
    registry.register(Box::new(command::CdCommand));
    registry.register(Box::new(command::JobsCommand));
    registry.register(Box::new(command::ExitCommand));
    registry.register(Box::new(command::FgCommand));

    loop {
        let sig = line_editor.read_line(&prompt);
        match sig {
            Ok(Signal::Success(buffer)) => {
                let input = buffer.trim();
                if input.is_empty() {
                    continue;
                }

                let start_total = Instant::now();

                // Check for built-in commands
                let args: Vec<&str> = input.split_whitespace().collect();
                if let Some(cmd_name) = args.first() {
                    if let Some(cmd) = registry.get(cmd_name) {
                        let mut context = CommandContext {
                            db: &db,
                            current_env: &current_env,
                            jobs: &mut jobs,
                            pty_writer: &pty_writer,
                            max_output_size,
                            prompt: &mut prompt,
                        };
                        
                        match cmd.execute(&args[1..], &mut context) {
                            Ok(CommandResult::Exit) => break,
                            Ok(CommandResult::Continue) => continue,
                            Err(e) => {
                                eprintln!("Error executing {}: {}", cmd_name, e);
                                prompt.set_last_success(false);
                                prompt.set_last_duration(Some(start_total.elapsed()));
                                continue;
                            }
                        }
                    }
                }

                // Execute command in PTY
                let start = Instant::now();

                // Check if command should have output captured
                let cmd_name = input.split_whitespace().next().unwrap_or("");
                let capture_output = !config.ignored_outputs.iter().any(|ignored| ignored == cmd_name);

                match executor::execute_in_pty(input, max_output_size, &pty_writer, &current_env, capture_output)
                {
                    Ok(res) => {
                         println!(); // Add newline between command output and next prompt
                         
                         let mut context = CommandContext {
                            db: &db,
                            current_env: &current_env,
                            jobs: &mut jobs,
                            pty_writer: &pty_writer,
                            max_output_size,
                            prompt: &mut prompt,
                        };
                        
                         if let Err(e) = command::handle_execution_result(res, start, input, &mut context) {
                             eprintln!("Error processing execution result: {}", e);
                             prompt.set_last_success(false);
                             prompt.set_last_duration(Some(start.elapsed()));
                         }
                    }
                    Err(e) => {
                        eprintln!("Execution error: {}", e);
                        prompt.set_last_success(false);
                        prompt.set_last_duration(Some(start.elapsed()));
                    }
                }
            }
            Ok(Signal::CtrlC) => {
                // Handle Ctrl+C at prompt - just continue to next prompt
                println!("^C");
                prompt.set_last_success(false);
                prompt.set_last_duration(None);
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
