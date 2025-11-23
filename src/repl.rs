use anyhow::Result;
use reedline::{ColumnarMenu, Emacs, FileBackedHistory, HistoryItem, KeyCode, KeyModifiers, Reedline, ReedlineEvent, ReedlineMenu, Signal};
use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::alias;
use crate::common::{HISTORY_FILENAME, MAX_HISTORY_ENTRIES, DB_FILENAME};
use crate::completion::CahierCompleter;
use crate::config::Config;
use crate::db;
use crate::executor::{self, Job};
use crate::highlighter::SyntectHighlighter;
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
    initial_command: Option<String>,
) -> Result<()> {
    println!("Cahier started.");
    
    // Resolve absolute path to database to ensure we can always connect to it
    // even after changing directories
    let db_path_buf = std::fs::canonicalize(DB_FILENAME)
        .unwrap_or_else(|_| std::path::PathBuf::from(DB_FILENAME));
    let db_path_str = db_path_buf.to_string_lossy().to_string();
    
    println!("Database: {}", db_path_str);
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

    let mut registry = Registry::new();
    registry.register(Box::new(command::CdCommand));
    registry.register(Box::new(command::JobsCommand));
    registry.register(Box::new(command::ExitCommand));
    registry.register(Box::new(command::FgCommand));
    registry.register(Box::new(command::AliasCommand));
    registry.register(Box::new(command::UnaliasCommand));
    registry.register(Box::new(command::EditCommand));

    // Load aliases from user shell if configured
    let aliases_map = if config.load_aliases {
        println!("Loading aliases...");
        let map = alias::load_aliases_from_shell(Duration::from_secs(2));
        println!("Loaded {} aliases.", map.len());
        map
    } else {
        HashMap::new()
    };
    let aliases = Arc::new(Mutex::new(aliases_map));

    let builtins = registry.command_names();

    let mut line_editor = Reedline::create()
        .with_history(history)
        .with_completer(Box::new(CahierCompleter::new(
            current_env.clone(),
            aliases.clone(),
            builtins
        )))
        .with_quick_completions(true)
        .with_menu(ReedlineMenu::EngineCompleter(Box::new(ColumnarMenu::default().with_name("completion_menu"))))
        .with_edit_mode(Box::new(edit_mode))
        .with_highlighter(Box::new(SyntectHighlighter::new(config.theme.clone())));
    let mut prompt = CahierPrompt::new();

    let mut jobs: Vec<Job> = Vec::new();
    
    // Add initial command to history if provided
    if let Some(cmd) = initial_command {
        let _ = line_editor.history_mut().save(HistoryItem::from_command_line(&cmd));
        // Also sync history to disk to ensure it persists
        let _ = line_editor.sync_history();
        // Try to run edit commands
        line_editor.run_edit_commands(&[reedline::EditCommand::InsertString(cmd)]);
    }
    
    let mut next_command: Option<String> = None;
    
    loop {
        let sig = line_editor.read_line(&prompt);
        match sig {
            Ok(Signal::Success(buffer)) => {
                let input = buffer.trim();
                if input.is_empty() {
                    continue;
                }

                let start_total = Instant::now();

                // Expand aliases
                let expanded_input_raw = alias::expand_alias(input, &aliases);
                
                // Check for nr prefix to skip logging
                let (expanded_input, should_log) = {
                    let trimmed = expanded_input_raw.trim_start();
                    if let Some(stripped) = trimmed.strip_prefix("nr ") {
                        // If we found 'nr', we need to try expanding aliases again
                        // because the command after 'nr' might be an alias
                        (alias::expand_alias(stripped, &aliases), false)
                    } else {
                        (expanded_input_raw, true)
                    }
                };
                
                // Check for built-in commands
                let args_owned = shlex::split(&expanded_input).unwrap_or_default();
                let args: Vec<&str> = args_owned.iter().map(|s| s.as_str()).collect();

                if let Some(cmd_name) = args.first() {
                    if let Some(cmd) = registry.get(cmd_name) {
                        let mut context = CommandContext {
                            db: &db,
                            current_env: &current_env,
                            jobs: &mut jobs,
                            pty_writer: &pty_writer,
                            max_output_size,
                            prompt: &mut prompt,
                            aliases: &aliases,
                            should_log,
                            db_path: &db_path_str,
                            next_command: &mut next_command,
                        };
                        
                        match cmd.execute(&args[1..], &mut context) {
                            Ok(CommandResult::Exit) => break,
                            Ok(CommandResult::Continue) => {
                                if let Some(cmd) = next_command.take() {
                                    let _ = line_editor.history_mut().save(HistoryItem::from_command_line(&cmd));
                                    let _ = line_editor.sync_history();
                                    line_editor.run_edit_commands(&[reedline::EditCommand::InsertString(cmd)]);
                                }
                                println!();
                                continue;
                            }
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
                // Use the expanded command name for this check
                let cmd_name = expanded_input.split_whitespace().next().unwrap_or("");
                let capture_output = !config.ignored_outputs.iter().any(|ignored| ignored == cmd_name);

                match executor::execute_in_pty(&expanded_input, max_output_size, &pty_writer, &current_env, capture_output, false)
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
                            aliases: &aliases,
                            should_log,
                            db_path: &db_path_str,
                            next_command: &mut next_command,
                        };
                        
                         // Log the ORIGINAL input
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

        // Handle pending command from edit
        if let Some(cmd) = next_command.take() {
            let _ = line_editor.history_mut().save(HistoryItem::from_command_line(&cmd));
             // Also sync history to disk to ensure it persists
            let _ = line_editor.sync_history();
            // Inject the command into the next prompt
            line_editor.run_edit_commands(&[reedline::EditCommand::InsertString(cmd)]);
        }
    }
    Ok(())
}
