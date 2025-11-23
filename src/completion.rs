use reedline::{Completer, Suggestion, Span};
use std::collections::HashMap;
use std::path::{Path, PathBuf, MAIN_SEPARATOR};
use std::fs;
use std::sync::{Arc, Mutex};

pub struct FileCompleter {
    env_vars: Arc<Mutex<HashMap<String, String>>>,
}

impl FileCompleter {
    pub fn new(env_vars: Arc<Mutex<HashMap<String, String>>>) -> Self {
        Self { env_vars }
    }

    fn expand_path(&self, path_str: &str) -> PathBuf {
        // Handle tilde expansion
        if let Some(stripped) = path_str.strip_prefix('~') {
            if stripped.is_empty() || stripped.starts_with(MAIN_SEPARATOR) {
                if let Some(home) = dirs::home_dir() {
                    return home.join(stripped.trim_start_matches(MAIN_SEPARATOR));
                }
            }
        }

        // Handle variable expansion
        if let Some(stripped) = path_str.strip_prefix('$') {
            if let Some(idx) = stripped.find(MAIN_SEPARATOR) {
                let var = &stripped[..idx];
                let rest = &stripped[idx..];
                
                if let Ok(env) = self.env_vars.lock() {
                    if let Some(val) = env.get(var) {
                        return PathBuf::from(val).join(rest.trim_start_matches(MAIN_SEPARATOR));
                    }
                }
            } else {
                if let Ok(env) = self.env_vars.lock() {
                    if let Some(val) = env.get(stripped) {
                        return PathBuf::from(val);
                    }
                }
            }
        }

        PathBuf::from(path_str)
    }
}

impl Completer for FileCompleter {
    fn complete(&mut self, line: &str, pos: usize) -> Vec<Suggestion> {
        let (start, path_str) = find_word_at_pos(line, pos);
        
        // Check if we're completing a variable (starts with $ and has no separator)
        if path_str.starts_with('$') && !path_str.contains(MAIN_SEPARATOR) {
            if let Some(var_prefix) = path_str.strip_prefix('$') {
                // Remove the '$'
                let mut suggestions = Vec::new();
                
                if let Ok(env) = self.env_vars.lock() {
                    for (key, _value) in env.iter() {
                        if key.starts_with(var_prefix) {
                            suggestions.push(Suggestion {
                                value: format!("${}", key),
                                description: None,
                                extra: None,
                                span: Span { start, end: pos },
                                append_whitespace: true,
                            });
                        }
                    }
                }
                
                return suggestions;
            }
        }
        
        // Otherwise, do file completion
        let path = Path::new(path_str);
        
        let (dir, file_name) = if path_str.ends_with(MAIN_SEPARATOR) {
            (path, "")
        } else {
            match path.parent() {
                Some(parent) if !parent.as_os_str().is_empty() => (parent, path.file_name().and_then(|s| s.to_str()).unwrap_or("")),
                _ => (Path::new("."), path_str),
            }
        };
        
        // Expand directory for searching, but keep original `dir` for suggestions
        let search_dir = self.expand_path(dir.to_str().unwrap_or(""));

        let read_dir = match fs::read_dir(search_dir) {
            Ok(d) => d,
            Err(_) => return vec![],
        };

        let mut suggestions = Vec::new();
        
        for entry in read_dir.flatten() {
            let path = entry.path();
            let name = match path.file_name().and_then(|s| s.to_str()) {
                Some(n) => n,
                None => continue,
            };
            
            if name.starts_with(file_name) {
                 let value = if dir == Path::new(".") {
                     name.to_string()
                 } else {
                     let mut p = dir.to_path_buf();
                     p.push(name);
                     p.to_string_lossy().to_string()
                 };
                 
                 let is_dir = path.is_dir();
                 let (value, append_whitespace) = if is_dir {
                     (format!("{}{}", value, MAIN_SEPARATOR), false)
                 } else {
                     (value, true)
                 };
                 
                 suggestions.push(Suggestion {
                     value,
                     description: None,
                     extra: None,
                     span: Span { start, end: pos },
                     append_whitespace,
                 });
            }
        }
        
        suggestions
    }
}

fn find_word_at_pos(line: &str, pos: usize) -> (usize, &str) {
    let mut start = 0;
    for (i, c) in line.char_indices() {
        if i >= pos {
            break;
        }
        if c.is_whitespace() {
            start = i + c.len_utf8();
        }
    }
    if start > pos {
        start = pos;
    }
    (start, &line[start..pos])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_word_at_pos() {
        let line = "ls /tmp/fi";
        let pos = 10;
        let (start, word) = find_word_at_pos(line, pos);
        assert_eq!(start, 3);
        assert_eq!(word, "/tmp/fi");
        
        let line = "cd subdir";
        let pos = 9;
        let (start, word) = find_word_at_pos(line, pos);
        assert_eq!(start, 3);
        assert_eq!(word, "subdir");
        
        let line = "command";
        let pos = 7;
        let (start, word) = find_word_at_pos(line, pos);
        assert_eq!(start, 0);
        assert_eq!(word, "command");
    }

    #[test]
    fn test_find_word_at_pos_multibyte_whitespace() {
        // Use non-breaking space (U+00A0), which is 2 bytes in UTF-8
        let line = format!("ls\u{00A0}/tmp/fi");
        let pos = line.len();
        let (start, word) = find_word_at_pos(&line, pos);
        // "ls" are 2 bytes, NBSP is at byte index 2 and len_utf8() = 2 -> start should be 4
        assert_eq!(start, 4);
        assert_eq!(word, "/tmp/fi");
    }
}
