use anyhow::Result;
use reedline::{
    ColumnarMenu, CommandLineSearch, Emacs, FileBackedHistory, HistoryItem, KeyCode, KeyModifiers,
    Reedline, ReedlineEvent, ReedlineMenu, SearchFilter, Signal,
};
use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::alias;
use crate::command::{self, CommandContext, CommandResult, Registry};
use crate::common::{self, MAX_HISTORY_ENTRIES};
use crate::completion::CahierCompleter;
use crate::config::Config;
use crate::db;
use crate::executor::{self, Job};
use crate::highlighter::SyntectHighlighter;
use crate::prompt::CahierPrompt;

/// Resolves the absolute path to the database
fn resolve_db_path() -> String {
    let db_path = common::db_path();
    let db_path_buf = std::fs::canonicalize(&db_path).unwrap_or(db_path);
    db_path_buf.to_string_lossy().to_string()
}

/// Initializes the Reedline editor with history, completion, keybindings, etc.
fn setup_line_editor(
    config: &Config,
    current_env: Arc<Mutex<HashMap<String, String>>>,
    aliases: Arc<Mutex<HashMap<String, String>>>,
    builtins: Vec<String>,
) -> Result<Reedline> {
    let history = Box::new(
        FileBackedHistory::with_file(MAX_HISTORY_ENTRIES, common::history_path())
            .map_err(|e| anyhow::anyhow!("Error creating history file: {:?}", e))?,
    );

    let mut keybindings = reedline::default_emacs_keybindings();
    keybindings.add_binding(
        KeyModifiers::from_bits_truncate(0),
        KeyCode::Tab,
        ReedlineEvent::Menu("completion_menu".to_string()),
    );
    let edit_mode = Emacs::new(keybindings);

    let line_editor = Reedline::create()
        .with_history(history)
        .with_completer(Box::new(CahierCompleter::new(
            current_env,
            aliases,
            builtins,
        )))
        .with_quick_completions(true)
        .with_menu(ReedlineMenu::EngineCompleter(Box::new(
            ColumnarMenu::default().with_name("completion_menu"),
        )))
        .with_edit_mode(Box::new(edit_mode))
        .with_highlighter(Box::new(SyntectHighlighter::new(config.theme.clone())));

    Ok(line_editor)
}

/// Processes the input string to handle aliases and the 'nr' prefix
fn process_input(input: &str, aliases: &Arc<Mutex<HashMap<String, String>>>) -> (String, bool) {
    // Expand aliases
    let expanded_input_raw = alias::expand_alias(input, aliases);

    // Check for nr prefix to skip logging
    let trimmed = expanded_input_raw.trim_start();
    if let Some(stripped) = trimmed.strip_prefix("nr ") {
        // If we found 'nr', we need to try expanding aliases again
        // because the command after 'nr' might be an alias
        (alias::expand_alias(stripped, aliases), false)
    } else {
        (expanded_input_raw, true)
    }
}

/// Executes an external command in the PTY
fn execute_external_command(
    input: &str,
    expanded_input: &str,
    should_log: bool,
    context: &mut CommandContext,
    config: &Config,
) -> Result<()> {
    let start = Instant::now();

    // Check if command should have output captured
    // Use the expanded command name for this check
    // Note: If should_log is false (due to 'nr' prefix), we also disable output capture
    // to ensure no persistent record (file or DB) is created.
    let cmd_name = expanded_input.split_whitespace().next().unwrap_or("");
    let is_ignored = config
        .ignored_outputs
        .iter()
        .any(|ignored| ignored == cmd_name);

    if is_ignored {
        context.should_log = false;
    }

    let capture_output = should_log && !is_ignored;

    match executor::execute_in_pty(
        expanded_input,
        context.max_output_size,
        context.pty_writer,
        context.current_env,
        capture_output,
        false,
    ) {
        Ok(res) => {
            println!(); // Add newline between command output and next prompt

            // Log the ORIGINAL input
            if let Err(e) = command::handle_execution_result(res, start, input, context) {
                eprintln!("Error processing execution result: {}", e);
                context.prompt.set_last_success(false);
                context.prompt.set_last_duration(Some(start.elapsed()));
            }
        }
        Err(e) => {
            eprintln!("Execution error: {}", e);
            context.prompt.set_last_success(false);
            context.prompt.set_last_duration(Some(start.elapsed()));
        }
    }
    Ok(())
}

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

    let db_path_str = resolve_db_path();

    println!("Database: {}", db_path_str);
    println!("Max output size: {} bytes", max_output_size);

    // Resolve absolute path for environment store
    let env_store_path = common::env_store_path();

    // Initialize current environment
    let mut env_map: HashMap<String, String> = std::env::vars().collect();

    if config.restore_env {
        println!("Restoring environment...");
        match crate::env_store::load_env(&env_store_path) {
            Ok(persisted_env) => {
                // Try to restore working directory first
                if let Some(pwd) = persisted_env.get("PWD") {
                    match std::env::set_current_dir(pwd) {
                        Ok(_) => {
                            // Successfully restored, merge all persisted variables
                            for (k, v) in persisted_env {
                                env_map.insert(k, v);
                            }
                        }
                        Err(e) => {
                            eprintln!(
                                "Warning: Failed to restore working directory ({}): {}",
                                pwd, e
                            );
                            // Directory doesn't exist, merge persisted vars but update PWD to current dir
                            for (k, v) in persisted_env {
                                env_map.insert(k, v);
                            }
                            // Update PWD to reflect actual current directory
                            if let Ok(cwd) = std::env::current_dir() {
                                env_map
                                    .insert("PWD".to_string(), cwd.to_string_lossy().to_string());
                            }
                        }
                    }
                } else {
                    // No PWD in persisted env, just merge
                    for (k, v) in persisted_env {
                        env_map.insert(k, v);
                    }
                }
            }
            Err(e) => {
                eprintln!("Failed to load persisted environment: {}", e);
            }
        }
    } else {
        // Ensure PWD in env map matches actual current dir when not restoring
        if let Ok(cwd) = std::env::current_dir() {
            env_map.insert("PWD".to_string(), cwd.to_string_lossy().to_string());
        }
    }

    let current_env: Arc<Mutex<HashMap<String, String>>> = Arc::new(Mutex::new(env_map));

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

    let mut line_editor =
        setup_line_editor(&config, current_env.clone(), aliases.clone(), builtins)?;

    let mut prompt = CahierPrompt::new();

    let mut jobs: Vec<Job> = Vec::new();

    // Track nr commands to remove from history before exit
    // These should stay in memory for the session but not be persisted
    let mut nr_commands: Vec<String> = Vec::new();

    // Add initial command to history if provided
    if let Some(cmd) = initial_command {
        let _ = line_editor
            .history_mut()
            .save(HistoryItem::from_command_line(&cmd));
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

                let (expanded_input, should_log) = process_input(input, &aliases);

                // Track nr commands for cleanup before exit
                if !should_log {
                    nr_commands.push(input.to_string());
                }

                // Check for built-in commands
                let args_owned = shlex::split(&expanded_input).unwrap_or_default();
                let args: Vec<&str> = args_owned.iter().map(|s| s.as_str()).collect();

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

                if let Some(cmd_name) = args.first() {
                    if let Some(cmd) = registry.get(cmd_name) {
                        match cmd.execute(&args[1..], &mut context) {
                            Ok(CommandResult::Exit) => break,
                            Ok(CommandResult::Continue) => {
                                if config.restore_env {
                                    if let Ok(env) = context.current_env.lock() {
                                        if let Err(e) =
                                            crate::env_store::save_env(&env, &env_store_path)
                                        {
                                            eprintln!("Failed to save environment: {}", e);
                                        }
                                    }
                                }

                                if let Some(cmd) = next_command.take() {
                                    let _ = line_editor
                                        .history_mut()
                                        .save(HistoryItem::from_command_line(&cmd));
                                    let _ = line_editor.sync_history();
                                    line_editor.run_edit_commands(&[
                                        reedline::EditCommand::InsertString(cmd),
                                    ]);
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

                execute_external_command(
                    input,
                    &expanded_input,
                    should_log,
                    &mut context,
                    &config,
                )?;

                if config.restore_env {
                    if let Ok(env) = context.current_env.lock() {
                        if let Err(e) = crate::env_store::save_env(&env, &env_store_path) {
                            eprintln!("Failed to save environment: {}", e);
                        }
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
            let _ = line_editor
                .history_mut()
                .save(HistoryItem::from_command_line(&cmd));
            // Also sync history to disk to ensure it persists
            let _ = line_editor.sync_history();
            // Inject the command into the next prompt
            line_editor.run_edit_commands(&[reedline::EditCommand::InsertString(cmd)]);
        }
    }

    // Clean up nr commands from history before exit
    // These were kept in memory for session history but should not persist
    for cmd in &nr_commands {
        let filter = SearchFilter::from_text_search(CommandLineSearch::Exact(cmd.clone()), None);
        if let Ok(results) = line_editor
            .history()
            .search(reedline::SearchQuery::last_with_search(filter))
        {
            if let Some(item) = results.first() {
                if let Some(id) = item.id {
                    let _ = line_editor.history_mut().delete(id);
                }
            }
        }
    }
    // Sync the cleaned history to disk
    let _ = line_editor.sync_history();

    Ok(())
}
