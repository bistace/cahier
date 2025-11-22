use anyhow::Result;
use cahier::executor;
use std::sync::{Arc, Mutex};

#[test]
fn test_executor_simple_command() -> Result<()> {
    let pty_writer = Arc::new(Mutex::new(None));
    let current_env = Arc::new(Mutex::new(std::env::vars().collect()));
    
    // Run simple echo
    let res = executor::execute_in_pty(
        "echo 'integration test passed'",
        1024,
        &pty_writer,
        &current_env,
        true,
        true
    )?;
    
    match res {
        executor::ExecutionResult::Completed { output, exit_code, .. } => {
            assert_eq!(exit_code, Some(0));
            assert!(output.contains("integration test passed"));
        }
        _ => panic!("Expected completed execution"),
    }
    
    Ok(())
}

#[test]
fn test_executor_large_output() -> Result<()> {
    let pty_writer = Arc::new(Mutex::new(None));
    let current_env = Arc::new(Mutex::new(std::env::vars().collect()));
    
    // Run command that produces large output (larger than 100 bytes)
    let large_string = "a".repeat(200);
    let cmd = format!("echo '{}'", large_string);
    
    // Set max output size to 100 bytes
    let res = executor::execute_in_pty(
        &cmd,
        100,
        &pty_writer,
        &current_env,
        true,
        true
    )?;
    
    match res {
        executor::ExecutionResult::Completed { output, exit_code, output_file } => {
            assert_eq!(exit_code, Some(0));
            // Output should be redirected
            assert!(output_file.is_some());
            assert!(output.contains("Output too large"));
            
            // Cleanup output file
            if let Some(path) = output_file {
                if std::path::Path::new(&path).exists() {
                    std::fs::remove_file(path)?;
                }
            }
        }
        _ => panic!("Expected completed execution"),
    }
    
    Ok(())
}

