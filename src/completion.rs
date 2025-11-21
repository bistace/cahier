use reedline::{Completer, Suggestion, Span};
use std::path::{Path, MAIN_SEPARATOR};
use std::fs;

pub struct FileCompleter;

impl FileCompleter {
    pub fn new() -> Self {
        Self
    }
}

impl Completer for FileCompleter {
    fn complete(&mut self, line: &str, pos: usize) -> Vec<Suggestion> {
        let (start, path_str) = find_word_at_pos(line, pos);
        
        let path = Path::new(path_str);
        
        let (dir, file_name) = if path_str.ends_with(MAIN_SEPARATOR) {
            (path, "")
        } else {
            match path.parent() {
                Some(parent) if !parent.as_os_str().is_empty() => (parent, path.file_name().and_then(|s| s.to_str()).unwrap_or("")),
                _ => (Path::new("."), path_str),
            }
        };
        
        let read_dir = match fs::read_dir(dir) {
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
            start = i + 1;
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
}
