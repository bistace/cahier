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
        .with_quick_completions(true)
        .with_menu(ReedlineMenu::EngineCompleter(Box::new(ColumnarMenu::default().with_name("completion_menu"))))
        .with_edit_mode(Box::new(edit_mode))
        .with_highlighter(Box::new(SyntectHighlighter::new(config.theme.clone())));
    let mut prompt = CahierPrompt::new();

    let mut jobs: Vec<Job> = Vec::new();
    
    let mut registry = Registry::new();
    registry.register(Box::new(command::CdCommand));
    registry.register(Box::new(command::JobsCommand));
    registry.register(Box::new(command::ExitCommand));
    registry.register(Box::new(command::FgCommand));
    registry.register(Box::new(command::AliasCommand));
    registry.register(Box::new(command::UnaliasCommand));

    // Load aliases from user shell
    println!("Loading aliases...");
    let aliases = Arc::new(Mutex::new(load_aliases()));
    println!("Loaded {} aliases.", aliases.lock().unwrap().len());

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
                let expanded_input = expand_alias(input, &aliases);
                
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
                // Use the expanded command name for this check? Or original?
                // Usually config matches expanded name (actual binary name).
                // But user might have "alias myvi=vi".
                // If user ignored "vi", and runs "myvi", it expands to "vi".
                // So checking expanded command name is correct.
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
                        };
                        
                         // Log the ORIGINAL input or expanded? 
                         // Bash logs original. Let's log original.
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

fn load_aliases() -> HashMap<String, String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".into());
    // Use interactive mode to load aliases from rc files
    // We use -i to force interactive mode which loads .bashrc/.zshrc
    // We use -c to execute 'alias' and exit
    let output = std::process::Command::new(&shell)
        .arg("-i")
        .arg("-c")
        .arg("alias")
        .output();

    let mut aliases = HashMap::new();
    if let Ok(output) = output {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let line = line.trim();
            // Try to strip "alias " prefix (bash), otherwise take whole line (zsh)
            let content = line.strip_prefix("alias ").unwrap_or(line);
            
            if let Some((name, value)) = content.split_once('=') {
                let name = name.trim();
                let value = value.trim();
                
                // Strip quotes if present
                let value = if value.len() >= 2 && ((value.starts_with('\'') && value.ends_with('\'')) ||
                             (value.starts_with('"') && value.ends_with('"'))) {
                    &value[1..value.len()-1]
                } else {
                    value
                };
                
                // Only add if name looks valid (no spaces)
                if !name.contains(char::is_whitespace) {
                    aliases.insert(name.to_string(), value.to_string());
                }
            }
        }
    }
    aliases
}

fn expand_alias(input: &str, aliases_lock: &Arc<Mutex<HashMap<String, String>>>) -> String {
    let mut current_input = input.to_string();
    let mut expanded_cmds = std::collections::HashSet::new();
    let aliases = aliases_lock.lock().unwrap();
    
    // Prevent infinite loops
    for _ in 0..10 {
        let input_clone = current_input.clone();
        let trimmed = input_clone.trim_start();
        
        let first_word_end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
        let first_word = &trimmed[..first_word_end];
        
        if first_word.is_empty() {
            break;
        }

        if let Some(replacement) = aliases.get(first_word) {
             if expanded_cmds.contains(first_word) {
                 break; 
             }
             expanded_cmds.insert(first_word.to_string());
             
             let rest = &trimmed[first_word_end..];
             current_input = format!("{}{}", replacement, rest);
        } else {
            break;
        }
    }
    current_input
}
