use anyhow::Result;
use crossterm::terminal;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::common::OUTPUT_DIR;

/// Executes a command in a PTY environment, capturing output and handling signals.
///
/// # Arguments
/// * `command` - The shell command to execute
/// * `max_output_size` - Maximum size in bytes before redirecting output to a file
/// * `pty_writer` - Shared writer for Ctrl+C signal handling
/// * `current_env` - Current environment variables to use and update
///
/// # Returns
/// A tuple of (output_string, exit_code, optional_output_file_path)
pub fn execute_in_pty(
    command: &str,
    max_output_size: usize,
    pty_writer: &Arc<Mutex<Option<Box<dyn Write + Send>>>>,
    current_env: &Arc<Mutex<HashMap<String, String>>>,
) -> Result<(String, Option<i32>, Option<String>)> {
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

    // Generate unique temporary file path for environment dump
    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S_%f");
    let env_dump_path = std::env::temp_dir().join(format!("cahier_env_{}", timestamp));

    // Wrap command with trap to capture environment on exit
    let wrapped_command = format!(
        "trap 'env -0 > \"{}\"' EXIT; {}",
        env_dump_path.display(),
        command
    );
    cmd.args(["-c", &wrapped_command]);

    // Clear and set environment from current_env
    cmd.env_clear();
    {
        let env = current_env.lock().unwrap();
        for (key, value) in env.iter() {
            cmd.env(key, value);
        }
    }

    // Set current directory
    cmd.cwd(std::env::current_dir()?);

    let mut child = pair.slave.spawn_command(cmd)?;

    // Drop slave to close the write-end of the pipe in this process
    drop(pair.slave);

    // Register the writer for Ctrl+C handling
    let writer = pair.master.take_writer()?;
    {
        let mut writer_opt = pty_writer.lock().unwrap();
        *writer_opt = Some(writer);
    }

    // Ensure we clear the writer when done (using a scope guard pattern)
    struct WriterGuard {
        pty_writer: Arc<Mutex<Option<Box<dyn Write + Send>>>>,
    }
    impl Drop for WriterGuard {
        fn drop(&mut self) {
            if let Ok(mut writer_opt) = self.pty_writer.lock() {
                *writer_opt = None;
            }
        }
    }
    let _guard = WriterGuard {
        pty_writer: Arc::clone(pty_writer),
    };

    let mut reader = pair.master.try_clone_reader()?;

    let mut buf = [0u8; 1024];
    let mut captured_output = Vec::new();
    let mut output_file: Option<File> = None;
    let mut output_filename: Option<String> = None;

    // Simple loop to read and print
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let data = &buf[..n];

                // Check if we need to redirect to file
                if captured_output.len() + n > max_output_size && output_file.is_none() {
                    // Create output directory
                    let output_dir = PathBuf::from(OUTPUT_DIR);
                    std::fs::create_dir_all(&output_dir)?;

                    // Generate unique filename with timestamp
                    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
                    let filename = format!("output_{}.txt", timestamp);
                    let filepath = output_dir.join(&filename);

                    // Create file and write existing buffer
                    let mut file = File::create(&filepath)?;
                    file.write_all(&captured_output)?;

                    // Store relative path
                    output_filename = Some(format!("{}/{}", OUTPUT_DIR, filename));
                    output_file = Some(file);

                    // Clear captured_output since it's now in the file
                    captured_output.clear();

                    // Add a message to captured_output for display
                    captured_output.extend_from_slice(
                        format!(
                            "[Output too large, redirected to {}]\n",
                            output_filename.as_ref().unwrap()
                        )
                        .as_bytes(),
                    );
                }

                // Write to file if redirected, otherwise accumulate in memory
                if let Some(ref mut file) = output_file {
                    file.write_all(data)?;
                } else {
                    captured_output.extend_from_slice(data);
                }

                // Always write to stdout to show user
                let _ = io::stdout().write_all(data);
                let _ = io::stdout().flush();
            }
            Err(_) => break,
        }
    }

    let exit_status = child.wait()?;
    let exit_code = if exit_status.success() {
        Some(0)
    } else {
        Some(1)
    };

    // Read and parse the environment dump
    if let Ok(env_data) = fs::read(&env_dump_path) {
        // Parse null-terminated environment variables into a temporary HashMap
        let mut new_env = HashMap::new();
        for entry in env_data.split(|&b| b == 0) {
            if entry.is_empty() {
                continue;
            }
            if let Ok(s) = std::str::from_utf8(entry) {
                if let Some(pos) = s.find('=') {
                    let key = s[..pos].to_string();
                    let value = s[pos + 1..].to_string();
                    new_env.insert(key, value);
                }
            }
        }

        // Only update current_env if we successfully parsed at least some variables
        // This prevents losing all environment variables if the dump is empty or invalid
        if !new_env.is_empty() {
            let mut env = current_env.lock().unwrap();
            *env = new_env;
        }

        // Clean up the temp file
        let _ = fs::remove_file(&env_dump_path);
    }

    Ok((
        String::from_utf8_lossy(&captured_output).to_string(),
        exit_code,
        output_filename,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execute_in_pty() {
        // simple echo
        let pty_writer = Arc::new(Mutex::new(None));
        let env = Arc::new(Mutex::new(std::env::vars().collect()));
        let (output, exit_code, output_file) =
            execute_in_pty("echo 'hello world'", 1024, &pty_writer, &env)
                .expect("failed to execute");
        assert!(output.contains("hello world"));
        assert_eq!(exit_code, Some(0));
        assert!(output_file.is_none());
    }

    #[test]
    fn test_execute_failure() {
        // command not found
        // bash -c "nonexistent" returns 127 (or non-zero)
        let pty_writer = Arc::new(Mutex::new(None));
        let env = Arc::new(Mutex::new(std::env::vars().collect()));
        let (_output, exit_code, _output_file) =
            execute_in_pty("nonexistent_command_123", 1024, &pty_writer, &env)
                .expect("failed to execute");
        // exit_code depends on shell, but should be Some(non-zero)
        assert_ne!(exit_code, Some(0));
    }

    #[test]
    fn test_output_redirection() {
        use std::path::PathBuf;
        
        // Use very small max_output_size to trigger redirection
        let pty_writer = Arc::new(Mutex::new(None));
        let env = Arc::new(Mutex::new(std::env::vars().collect()));
        
        // Generate output larger than 5 bytes
        let (output, exit_code, output_file) =
            execute_in_pty("echo 'This is a longer output'", 5, &pty_writer, &env)
                .expect("failed to execute");
        
        // Should have successful exit
        assert_eq!(exit_code, Some(0));
        
        // Output should contain the redirection message
        assert!(output.contains("[Output too large, redirected to"));
        
        // Should have an output file path
        assert!(output_file.is_some());
        
        let file_path = output_file.unwrap();
        assert!(file_path.starts_with(".cahier/outputs/"));
        assert!(file_path.ends_with(".txt"));
        
        // Verify the file exists and contains the actual output
        let full_path = PathBuf::from(&file_path);
        assert!(full_path.exists(), "Output file should exist: {:?}", full_path);
        
        let file_content = std::fs::read_to_string(&full_path)
            .expect("Should be able to read output file");
        assert!(file_content.contains("This is a longer output"));
        
        // Cleanup: remove the output file and directory
        std::fs::remove_file(&full_path).ok();
        // Try to remove the directory (will only succeed if empty)
        std::fs::remove_dir_all(".cahier").ok();
    }
}

