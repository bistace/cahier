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
use crate::executor::{self, ExecutionResult, Job};
use crate::prompt::CahierPrompt;

/// Handles the 'cd' command by changing directory and updating the environment
///
/// # Arguments
/// * `path` - The directory path to change to
/// * `current_env` - The environment HashMap to update with the new PWD
///
/// # Returns
/// Ok(()) if successful, Err if the directory change fails
pub fn handle_cd(path: &str, current_env: &Arc<Mutex<HashMap<String, String>>>) -> Result<()> {
    std::env::set_current_dir(path)?;
    
    // Update PWD in current_env
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(cwd_str) = cwd.to_str() {
            let mut env = current_env.lock().unwrap();
            env.insert("PWD".to_string(), cwd_str.to_string());
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
                    if let Err(e) = handle_cd(path, &current_env) {
                        eprintln!("Error changing directory: {}", e);
                        prompt.set_last_success(false);
                    } else {
                        // Log cd command as well, though output is empty
                        db.log_entry(input, "", Some(0), 0, None)?;
                        prompt.set_last_success(true);
                    }
                    continue;
                }

                // Handle 'jobs'
                if input == "jobs" {
                    for (i, job) in jobs.iter().enumerate() {
                        println!("[{}] {} {}", i + 1, job.command, if i == jobs.len() - 1 { "+" } else { "" });
                    }
                    prompt.set_last_success(true);
                    continue;
                }

                // Handle 'fg'
                if input == "fg" || input.starts_with("fg ") {
                    if jobs.is_empty() {
                        eprintln!("fg: current: no such job");
                        prompt.set_last_success(false);
                        continue;
                    }

                    let job_index = if input == "fg" {
                        jobs.len() - 1
                    } else {
                        let arg = input.strip_prefix("fg ").unwrap().trim();
                        if let Ok(n) = arg.parse::<usize>() {
                             if n > 0 && n <= jobs.len() {
                                 n - 1
                             } else {
                                 eprintln!("fg: {}: no such job", arg);
                                 prompt.set_last_success(false);
                                 continue;
                             }
                        } else {
                            eprintln!("fg: invalid job specifier");
                            prompt.set_last_success(false);
                            continue;
                        }
                    };

                    let job = jobs.remove(job_index);
                    let job_command = job.command.clone();
                    println!("{}", job.command);

                    let start = Instant::now();
                    match executor::resume_job(job, max_output_size, &pty_writer, &current_env) {
                        Ok(res) => {
                            handle_execution_result(res, start, &job_command, &db, &mut jobs, &mut prompt)?;
                        }
                        Err(e) => {
                            eprintln!("Error resuming job: {}", e);
                            prompt.set_last_success(false);
                        }
                    }
                    continue;
                }

                // Execute command
                let start = Instant::now();

                // Check if command should have output captured
                let cmd_name = input.split_whitespace().next().unwrap_or("");
                let capture_output = !config.ignored_outputs.iter().any(|ignored| ignored == cmd_name);

                match executor::execute_in_pty(input, max_output_size, &pty_writer, &current_env, capture_output)
                {
                    Ok(res) => {
                         println!(); // Add newline between command output and next prompt
                         if let Err(e) = handle_execution_result(res, start, input, &db, &mut jobs, &mut prompt) {
                             eprintln!("Error processing execution result: {}", e);
                             prompt.set_last_success(false);
                         }
                    }
                    Err(e) => {
                        eprintln!("Execution error: {}", e);
                        prompt.set_last_success(false);
                    }
                }
            }
            Ok(Signal::CtrlC) => {
                // Handle Ctrl+C at prompt - just continue to next prompt
                println!("^C");
                // Ctrl+C at prompt is usually considered "abort", so maybe red?
                // But user didn't run a failing command. Let's keep previous status or default to false?
                // Bash keeps it 130 usually. Let's set to false for visual feedback of "aborted".
                prompt.set_last_success(false);
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

fn handle_execution_result(
    res: ExecutionResult,
    start_time: Instant,
    input: &str,
    db: &db::Database,
    jobs: &mut Vec<Job>,
    prompt: &mut CahierPrompt
) -> Result<()> {
    match res {
        ExecutionResult::Completed { output, exit_code, output_file } => {
            let duration = start_time.elapsed();
            db.log_entry(
                input,
                &output,
                exit_code,
                duration.as_millis(),
                output_file.as_deref(),
            )?;
            
            if let Some(code) = exit_code {
                prompt.set_last_success(code == 0);
            } else {
                prompt.set_last_success(false);
            }
        }
        ExecutionResult::Suspended(mut job) => {
             let id = jobs.len() + 1;
             job.id = id;
             println!("\n[{}] Stopped  {}", id, job.command);
             jobs.push(job);
             prompt.set_last_success(true);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_handle_cd() -> Result<()> {
        // Store original directory to restore later
        let original_dir = std::env::current_dir()?;
        
        // Create a temporary directory for testing
        let temp_dir = std::env::temp_dir();
        let test_dir = temp_dir.join(format!("cahier_test_{}", 
            chrono::Utc::now().format("%Y%m%d_%H%M%S_%f")));
        std::fs::create_dir_all(&test_dir)?;
        
        // Initialize environment
        let env = Arc::new(Mutex::new(std::env::vars().collect()));
        
        // Change to the test directory
        let test_dir_str = test_dir.to_str().unwrap();
        handle_cd(test_dir_str, &env)?;
        
        // Verify current directory changed
        let current_dir = std::env::current_dir()?;
        assert_eq!(current_dir, test_dir);
        
        // Verify PWD environment variable was updated
        {
            let env_map = env.lock().unwrap();
            assert!(env_map.contains_key("PWD"));
            let pwd = env_map.get("PWD").unwrap();
            assert_eq!(PathBuf::from(pwd), test_dir);
        }
        
        // Test error case: try to cd to non-existent directory
        let result = handle_cd("/this/directory/does/not/exist/cahier_test_xyz", &env);
        assert!(result.is_err());
        
        // Restore original directory
        std::env::set_current_dir(&original_dir)?;
        
        // Cleanup test directory
        std::fs::remove_dir_all(&test_dir)?;
        
        Ok(())
    }

    #[test]
    fn test_handle_cd_relative_path() -> Result<()> {
        let original_dir = std::env::current_dir()?;
        
        // Create nested test directories
        let temp_dir = std::env::temp_dir();
        let test_base = temp_dir.join(format!("cahier_test_base_{}", 
            chrono::Utc::now().format("%Y%m%d_%H%M%S_%f")));
        let test_sub = test_base.join("subdir");
        std::fs::create_dir_all(&test_sub)?;
        
        let env = Arc::new(Mutex::new(std::env::vars().collect()));
        
        // Change to base directory
        handle_cd(test_base.to_str().unwrap(), &env)?;
        assert_eq!(std::env::current_dir()?, test_base);
        
        // Change to subdirectory using relative path
        handle_cd("subdir", &env)?;
        assert_eq!(std::env::current_dir()?, test_sub);
        
        // Go back up using relative path
        handle_cd("..", &env)?;
        assert_eq!(std::env::current_dir()?, test_base);
        
        // Restore and cleanup
        std::env::set_current_dir(&original_dir)?;
        std::fs::remove_dir_all(&test_base)?;
        
        Ok(())
    }
}
