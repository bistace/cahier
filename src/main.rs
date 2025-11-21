use anyhow::Result;
use clap::{Parser, Subcommand};
use crossterm::terminal;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use reedline::{DefaultPrompt, FileBackedHistory, Reedline, Signal};
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

mod db;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start cahier REPL (default)
    Start {
        /// Maximum output size in bytes before redirecting to file (default: 16384)
        #[arg(long, default_value = "16384")]
        max_output_size: usize,
    },
    /// Export history to markdown
    Export {
        /// Output file path (default: stdout)
        #[arg(short, long)]
        output: Option<String>,

        /// Export only the commands (plain text)
        #[arg(long)]
        only_commands: bool,
    },
}

fn main() -> Result<()> {
    let args = Args::parse();
    let db = db::Database::init("cahier.db")?;

    match args.command {
        Some(Commands::Export { output, only_commands }) => {
            let content = if only_commands {
                generate_commands_text(&db)?
            } else {
                generate_markdown(&db)?
            };
            
            if let Some(path) = output {
                std::fs::write(path, content)?;
            } else {
                println!("{}", content);
            }
            return Ok(());
        }
        Some(Commands::Start { max_output_size }) => {
            // Setup PTY writer state for Ctrl+C handling
            let pty_writer: Arc<Mutex<Option<Box<dyn Write + Send>>>> = Arc::new(Mutex::new(None));
            
            // Register Ctrl+C handler
            let writer_clone = Arc::clone(&pty_writer);
            ctrlc::set_handler(move || {
                if let Ok(mut writer_opt) = writer_clone.lock() {
                    if let Some(writer) = writer_opt.as_mut() {
                        // Send Ctrl+C (ETX) to the running command
                        let _ = writer.write_all(&[3]);
                        let _ = writer.flush();
                    }
                    // If no writer, do nothing (at prompt)
                }
            })?;
            
            run_repl(db, max_output_size, pty_writer)?;
        }
        None => {
            // Default behavior: start REPL with default max_output_size
            // Setup PTY writer state for Ctrl+C handling
            let pty_writer: Arc<Mutex<Option<Box<dyn Write + Send>>>> = Arc::new(Mutex::new(None));
            
            // Register Ctrl+C handler
            let writer_clone = Arc::clone(&pty_writer);
            ctrlc::set_handler(move || {
                if let Ok(mut writer_opt) = writer_clone.lock() {
                    if let Some(writer) = writer_opt.as_mut() {
                        // Send Ctrl+C (ETX) to the running command
                        let _ = writer.write_all(&[3]);
                        let _ = writer.flush();
                    }
                    // If no writer, do nothing (at prompt)
                }
            })?;
            
            run_repl(db, 16384, pty_writer)?;
        }
    }

    Ok(())
}

fn generate_commands_text(db: &db::Database) -> Result<String> {
    let entries = db.get_entries()?;
    let mut text = String::new();
    for entry in entries {
        text.push_str(&entry.command);
        text.push('\n');
    }
    Ok(text)
}

fn generate_markdown(db: &db::Database) -> Result<String> {
    let entries = db.get_entries()?;
    
    let mut md = String::new();
    md.push_str("# Cahier Export\n\n");
    
    for entry in entries {
        // Format: everything inside a single bash block
        md.push_str("```bash\n");
        
        // Status line: (exit_code - duration)
        let exit_code_str = entry.exit_code.map_or("?".to_string(), |c| c.to_string());
        md.push_str(&format!("({} - {}ms)\n", exit_code_str, entry.duration_ms));
        
        // Command line with $ prefix
        md.push_str(&format!("$ {}\n", entry.command));
        
        // Output (if present)
        if let Some(output_file) = entry.output_file {
            // Reference the external file
            md.push_str(&format!("[Output stored in external file: {}]\n", output_file));
        } else if !entry.output.is_empty() {
            let clean_output = strip_ansi_escapes::strip(&entry.output);
            md.push_str(&String::from_utf8_lossy(&clean_output));
            if !entry.output.ends_with('\n') {
                md.push_str("\n");
            }
        }
        
        md.push_str("```\n\n");
    }
    
    Ok(md)
}

fn run_repl(db: db::Database, max_output_size: usize, pty_writer: Arc<Mutex<Option<Box<dyn Write + Send>>>>) -> Result<()> {
    println!("Cahier started.");
    println!("Database: ./cahier.db");
    println!("Max output size: {} bytes", max_output_size);

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
                        db.log_entry(input, "", &cwd, Some(0), 0, None)?;
                    }
                    continue;
                }

                // Execute command
                let start = Instant::now();
                match execute_in_pty(input, max_output_size, &pty_writer) {
                    Ok((output, exit_code, output_file)) => {
                        let duration = start.elapsed();
                        let cwd = std::env::current_dir()?.to_string_lossy().to_string();
                        
                        // Save to DB
                        db.log_entry(
                            input,
                            &output,
                            &cwd,
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

fn execute_in_pty(command: &str, max_output_size: usize, pty_writer: &Arc<Mutex<Option<Box<dyn Write + Send>>>>) -> Result<(String, Option<i32>, Option<String>)> {
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
                    // Create .cahier/outputs directory
                    let output_dir = PathBuf::from(".cahier/outputs");
                    std::fs::create_dir_all(&output_dir)?;
                    
                    // Generate unique filename with timestamp
                    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
                    let filename = format!("output_{}.txt", timestamp);
                    let filepath = output_dir.join(&filename);
                    
                    // Create file and write existing buffer
                    let mut file = File::create(&filepath)?;
                    file.write_all(&captured_output)?;
                    
                    // Store relative path
                    output_filename = Some(format!(".cahier/outputs/{}", filename));
                    output_file = Some(file);
                    
                    // Clear captured_output since it's now in the file
                    captured_output.clear();
                    
                    // Add a message to captured_output for display
                    captured_output.extend_from_slice(
                        format!("[Output too large, redirected to {}]\n", output_filename.as_ref().unwrap())
                            .as_bytes()
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
    let exit_code = if exit_status.success() { Some(0) } else { Some(1) }; 

    Ok((String::from_utf8_lossy(&captured_output).to_string(), exit_code, output_filename))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execute_in_pty() {
        // simple echo
        let pty_writer = Arc::new(Mutex::new(None));
        let (output, exit_code, output_file) = execute_in_pty("echo 'hello world'", 1024, &pty_writer).expect("failed to execute");
        assert!(output.contains("hello world"));
        assert_eq!(exit_code, Some(0));
        assert!(output_file.is_none());
    }

    #[test]
    fn test_execute_failure() {
        // command not found
        // bash -c "nonexistent" returns 127 (or non-zero)
        let pty_writer = Arc::new(Mutex::new(None));
        let (_output, exit_code, _output_file) = execute_in_pty("nonexistent_command_123", 1024, &pty_writer).expect("failed to execute");
        // exit_code depends on shell, but should be Some(non-zero)
        assert_ne!(exit_code, Some(0));
    }
}
