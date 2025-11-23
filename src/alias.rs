use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::process::Stdio;

/// Loads aliases from the user's shell configuration.
/// 
/// This function attempts to launch an interactive shell to read aliases.
/// It has a timeout to prevent hanging if the shell startup is slow.
pub fn load_aliases_from_shell(timeout: Duration) -> HashMap<String, String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".into());
    
    // We spawn the command and wait with timeout
    // Since std::process::Command doesn't support timeout easily on wait() without external crates or threads,
    // we'll use a simple thread-based timeout mechanism or just rely on the fact that `alias` should be fast.
    // However, if .bashrc is slow, it will hang.
    // For robustness, we can spawn a thread.
    
    let (tx, rx) = std::sync::mpsc::channel();
    
    std::thread::spawn(move || {
        let result = std::process::Command::new(&shell)
            .arg("-i")
            .arg("-c")
            .arg("alias")
            .stdout(Stdio::piped())
            .stderr(Stdio::null()) // Ignore stderr
            .spawn();
            
        match result {
            Ok(child) => {
                let result = child.wait_with_output();
                let _ = tx.send(result);
            }
            Err(e) => {
                // Just send error via channel if we can construct one, or log it.
                // For simplicity we just return early.
                eprintln!("Failed to spawn shell for aliases: {}", e);
            }
        }
    });
    
    if let Ok(Ok(output)) = rx.recv_timeout(timeout) {
         parse_aliases(&String::from_utf8_lossy(&output.stdout))
    } else {
        eprintln!("Timeout or error loading aliases from shell.");
        HashMap::new()
    }
}

fn parse_aliases(output: &str) -> HashMap<String, String> {
    let mut aliases = HashMap::new();
    for line in output.lines() {
        let line = line.trim();
        // Try to strip "alias " prefix (bash), otherwise take whole line (zsh/others output format varies)
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
            if !name.contains(char::is_whitespace) && !name.is_empty() {
                aliases.insert(name.to_string(), value.to_string());
            }
        }
    }
    aliases
}

/// Expands aliases in the input string.
/// Returns the expanded string.
pub fn expand_alias(input: &str, aliases_lock: &Arc<Mutex<HashMap<String, String>>>) -> String {
    let mut current_input = input.to_string();
    let mut expanded_cmds = HashSet::new();
    
    // Handle poisoned lock gracefully
    let aliases = match aliases_lock.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    
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





