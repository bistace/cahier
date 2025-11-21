use anyhow::Result;

use crate::db;

/// Generates plain text output containing only commands from the database
pub fn generate_commands_text(db: &db::Database) -> Result<String> {
    let entries = db.get_entries()?;
    let mut text = String::new();
    for entry in entries {
        text.push_str(&entry.command);
        text.push('\n');
    }
    Ok(text)
}

/// Generates markdown-formatted export of command history with outputs
pub fn generate_markdown(db: &db::Database) -> Result<String> {
    let entries = db.get_entries()?;

    let mut md = String::new();
    md.push_str("# Cahier Export\n\n");

    for entry in entries {
        // Format: everything inside a single bash block
        md.push_str("```bash\n");

        // Status line: (exit_code - duration)
        let exit_code_str = entry
            .exit_code
            .map_or("?".to_string(), |c| c.to_string());
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
                md.push_str("\n");
            }
        }

        md.push_str("```\n\n");
    }

    Ok(md)
}

