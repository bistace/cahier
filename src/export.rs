use anyhow::Result;

use crate::db;

/// Generates plain text output containing only commands from the database
pub fn generate_commands_text(db: &db::Database) -> Result<String> {
    let mut text = String::new();
    db.iterate_entries(|entry| {
        if entry.is_separator {
            return Ok(());
        }
        text.push_str(&entry.command);
        text.push('\n');
        Ok(())
    })?;
    Ok(text)
}

/// Generates markdown-formatted export of command history with outputs
pub fn generate_markdown(db: &db::Database) -> Result<String> {
    let mut md = String::new();
    md.push_str("# Cahier Export\n\n");

    db.iterate_entries(|entry| {
        if entry.is_separator {
            md.push_str("\n---\n\n");
            return Ok(());
        }

        // Annotation (if present)
        if let Some(annotation) = &entry.annotation {
            if !annotation.is_empty() {
                md.push_str(&format!("{}\n\n", annotation));
            }
        }

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
            md.push_str(&format!(
                "[Output stored in external file: {}]\n",
                output_file
            ));
        } else if !entry.output.is_empty() {
            let clean_output = strip_ansi_escapes::strip(&entry.output);
            md.push_str(&String::from_utf8_lossy(&clean_output));
            if !entry.output.ends_with('\n') {
                md.push('\n');
            }
        }

        md.push_str("```\n\n");
        Ok(())
    })?;

    Ok(md)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_commands_text() -> Result<()> {
        let db = db::Database::init_memory()?;

        // Populate with sample commands
        db.log_entry("echo hello", "hello\n", Some(0), 100, None)?;
        db.log_entry("ls -la", "file1\nfile2\n", Some(0), 50, None)?;
        db.log_entry("cat missing.txt", "No such file", Some(1), 25, None)?;

        let text = generate_commands_text(&db)?;

        // Verify it contains all commands
        assert!(text.contains("echo hello"));
        assert!(text.contains("ls -la"));
        assert!(text.contains("cat missing.txt"));

        // Verify format: each command on its own line
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "echo hello");
        assert_eq!(lines[1], "ls -la");
        assert_eq!(lines[2], "cat missing.txt");

        Ok(())
    }

    #[test]
    fn test_generate_markdown() -> Result<()> {
        let db = db::Database::init_memory()?;

        // Populate with various entry types
        db.log_entry("echo hello", "hello world\n", Some(0), 100, None)?;
        db.log_entry("failing_cmd", "error message", Some(1), 250, None)?;
        db.log_entry(
            "large_output",
            "[Output too large, redirected to .cahier/outputs/output_123.txt]\n",
            Some(0),
            500,
            Some(".cahier/outputs/output_123.txt"),
        )?;

        let md = generate_markdown(&db)?;

        // Verify title
        assert!(md.contains("# Cahier Export"));

        // Verify first entry
        assert!(md.contains("(0 - 100ms)"));
        assert!(md.contains("$ echo hello"));
        assert!(md.contains("hello world"));

        // Verify second entry with failure
        assert!(md.contains("(1 - 250ms)"));
        assert!(md.contains("$ failing_cmd"));
        assert!(md.contains("error message"));

        // Verify third entry with output file reference
        assert!(md.contains("(0 - 500ms)"));
        assert!(md.contains("$ large_output"));
        assert!(md.contains("[Output stored in external file: .cahier/outputs/output_123.txt]"));

        // Verify markdown structure (code blocks)
        assert!(md.contains("```bash\n"));
        let bash_count = md.matches("```bash").count();
        assert_eq!(bash_count, 3); // One for each entry

        Ok(())
    }

    #[test]
    fn test_generate_markdown_with_annotation() -> Result<()> {
        let db = db::Database::init_memory()?;

        // Add entry
        db.log_entry("echo annotated", "output\n", Some(0), 100, None)?;

        // Get the ID of the entry we just added
        let entries = db.get_all_entries_ordered()?;
        let id = entries[0].id;

        // Add annotation
        db.update_annotation(id, "This is a test annotation".to_string())?;

        let md = generate_markdown(&db)?;

        // Verify annotation is present
        assert!(md.contains("This is a test annotation"));

        Ok(())
    }

    #[test]
    fn test_generate_markdown_empty_output() -> Result<()> {
        let db = db::Database::init_memory()?;

        // Entry with empty output
        db.log_entry("cd /tmp", "", Some(0), 10, None)?;

        let md = generate_markdown(&db)?;

        // Should still have the command
        assert!(md.contains("$ cd /tmp"));
        assert!(md.contains("(0 - 10ms)"));

        Ok(())
    }

    #[test]
    fn test_generate_markdown_with_separator() -> Result<()> {
        let db = db::Database::init_memory()?;

        // Add entries and separator
        db.log_entry("echo before", "output\n", Some(0), 100, None)?;
        db.insert_separator(2)?; // Rank 2
        db.log_entry("echo after", "output\n", Some(0), 100, None)?;

        let md = generate_markdown(&db)?;

        assert!(md.contains("$ echo before"));
        assert!(md.contains("\n---\n\n"));
        assert!(md.contains("$ echo after"));

        Ok(())
    }
}
