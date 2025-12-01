use reedline::{Completer, Span, Suggestion};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf, MAIN_SEPARATOR};
use std::sync::{Arc, Mutex};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

pub struct CahierCompleter {
    env_vars: Arc<Mutex<HashMap<String, String>>>,
    aliases: Arc<Mutex<HashMap<String, String>>>,
    builtins: Vec<String>,
    external_commands: Vec<String>,
}

impl CahierCompleter {
    pub fn new(
        env_vars: Arc<Mutex<HashMap<String, String>>>,
        aliases: Arc<Mutex<HashMap<String, String>>>,
        builtins: Vec<String>,
    ) -> Self {
        let external_commands = Self::scan_path_commands();
        Self {
            env_vars,
            aliases,
            builtins,
            external_commands,
        }
    }

    fn scan_path_commands() -> Vec<String> {
        let path_var = std::env::var("PATH").unwrap_or_default();
        let mut commands = HashSet::new();

        for path_str in std::env::split_paths(&path_var) {
            if let Ok(entries) = fs::read_dir(path_str) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    // Check if it is a file and executable
                    if path.is_file() {
                        #[cfg(unix)]
                        {
                            if let Ok(metadata) = path.metadata() {
                                if metadata.permissions().mode() & 0o111 != 0 {
                                    if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                                        commands.insert(name.to_string());
                                    }
                                }
                            }
                        }
                        #[cfg(not(unix))]
                        {
                            // On Windows/other, check extension or assume executable if it's in PATH
                            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                                commands.insert(name.to_string());
                            }
                        }
                    }
                }
            }
        }

        let mut cmd_vec: Vec<String> = commands.into_iter().collect();
        cmd_vec.sort();
        cmd_vec
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
            } else if let Ok(env) = self.env_vars.lock() {
                if let Some(val) = env.get(stripped) {
                    return PathBuf::from(val);
                }
            }
        }

        PathBuf::from(path_str)
    }
}

impl Completer for CahierCompleter {
    fn complete(&mut self, line: &str, pos: usize) -> Vec<Suggestion> {
        let (start, word) = find_word_at_pos(line, pos);

        // Check if we are at command position
        // Logic: if the text before the current word contains no non-whitespace characters, we are at command position.
        // However, we must respect that `start` is the index of the word start.
        let prefix = &line[..start];
        let is_command_pos = prefix.trim().is_empty();

        if is_command_pos {
            let mut suggestions = Vec::new();
            let mut seen = HashSet::new();

            // 1. Aliases
            if let Ok(aliases) = self.aliases.lock() {
                for (name, value) in aliases.iter() {
                    if name.starts_with(word) && !seen.contains(name) {
                        suggestions.push(Suggestion {
                            value: name.clone(),
                            description: Some(format!("Alias: {}", value)),
                            extra: None,
                            span: Span { start, end: pos },
                            append_whitespace: true,
                        });
                        seen.insert(name.clone());
                    }
                }
            }

            // 2. Builtins
            for name in &self.builtins {
                if name.starts_with(word) && !seen.contains(name) {
                    suggestions.push(Suggestion {
                        value: name.clone(),
                        description: Some("Builtin".to_string()),
                        extra: None,
                        span: Span { start, end: pos },
                        append_whitespace: true,
                    });
                    seen.insert(name.clone());
                }
            }

            // 3. External Commands
            for name in &self.external_commands {
                if name.starts_with(word) && !seen.contains(name) {
                    suggestions.push(Suggestion {
                        value: name.clone(),
                        description: Some("Command".to_string()),
                        extra: None,
                        span: Span { start, end: pos },
                        append_whitespace: true,
                    });
                    seen.insert(name.clone());
                }
            }

            if !suggestions.is_empty() {
                return suggestions;
            }
        }

        // Check if we're completing a variable (starts with $ and has no separator)
        if word.starts_with('$') && !word.contains(MAIN_SEPARATOR) {
            if let Some(var_prefix) = word.strip_prefix('$') {
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
        let path = Path::new(word);

        let filename_offset = word
            .rfind(|c| c == MAIN_SEPARATOR || c == '/')
            .map(|i| i + 1)
            .unwrap_or(0);

        let (dir, file_name) = if word.ends_with(MAIN_SEPARATOR) {
            (path, "")
        } else {
            match path.parent() {
                Some(parent) if !parent.as_os_str().is_empty() => (
                    parent,
                    path.file_name().and_then(|s| s.to_str()).unwrap_or(""),
                ),
                _ => (Path::new("."), word),
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
                let mut value = name.to_string();

                let is_dir = path.is_dir();
                let append_whitespace;

                if is_dir {
                    value.push(MAIN_SEPARATOR);
                    append_whitespace = false;
                } else {
                    append_whitespace = true;
                }

                suggestions.push(Suggestion {
                    value,
                    description: None,
                    extra: None,
                    span: Span {
                        start: start + filename_offset,
                        end: pos,
                    },
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
        let line = "ls\u{00A0}/tmp/fi".to_string();
        let pos = line.len();
        let (start, word) = find_word_at_pos(&line, pos);
        // "ls" are 2 bytes, NBSP is at byte index 2 and len_utf8() = 2 -> start should be 4
        assert_eq!(start, 4);
        assert_eq!(word, "/tmp/fi");
    }
}
