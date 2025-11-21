use anyhow::Result;
use reedline::{DefaultPrompt, FileBackedHistory, Reedline, Signal};
use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::common::{HISTORY_FILENAME, MAX_HISTORY_ENTRIES};
use crate::db;
use crate::executor;

/// Handles the 'cd' command by changing directory and updating the environment
///
/// # Arguments
/// * `path` - The directory path to change to
/// * `current_env` - The environment HashMap to update with the new PWD
///
/// # Returns
/// Ok(()) if successful, Err if the directory change fails
pub fn handle_cd(path: &str, current_env: &mut HashMap<String, String>) -> Result<()> {
    std::env::set_current_dir(path)?;
    
    // Update PWD in current_env
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(cwd_str) = cwd.to_str() {
            current_env.insert("PWD".to_string(), cwd_str.to_string());
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
                    if let Err(e) = handle_cd(path, &mut current_env) {
                        eprintln!("Error changing directory: {}", e);
                    } else {
                        // Log cd command as well, though output is empty
                        db.log_entry(input, "", Some(0), 0, None)?;
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
        let mut env: HashMap<String, String> = std::env::vars().collect();
        
        // Change to the test directory
        let test_dir_str = test_dir.to_str().unwrap();
        handle_cd(test_dir_str, &mut env)?;
        
        // Verify current directory changed
        let current_dir = std::env::current_dir()?;
        assert_eq!(current_dir, test_dir);
        
        // Verify PWD environment variable was updated
        assert!(env.contains_key("PWD"));
        let pwd = env.get("PWD").unwrap();
        assert_eq!(PathBuf::from(pwd), test_dir);
        
        // Test error case: try to cd to non-existent directory
        let result = handle_cd("/this/directory/does/not/exist/cahier_test_xyz", &mut env);
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
        
        let mut env: HashMap<String, String> = std::env::vars().collect();
        
        // Change to base directory
        handle_cd(test_base.to_str().unwrap(), &mut env)?;
        assert_eq!(std::env::current_dir()?, test_base);
        
        // Change to subdirectory using relative path
        handle_cd("subdir", &mut env)?;
        assert_eq!(std::env::current_dir()?, test_sub);
        
        // Go back up using relative path
        handle_cd("..", &mut env)?;
        assert_eq!(std::env::current_dir()?, test_base);
        
        // Restore and cleanup
        std::env::set_current_dir(&original_dir)?;
        std::fs::remove_dir_all(&test_base)?;
        
        Ok(())
    }
}
