use std::collections::HashMap;
use std::fs;
use std::path::Path;
use anyhow::{Context, Result};

/// Saves the current environment variables to a JSON file.
pub fn save_env(env: &HashMap<String, String>, path: &Path) -> Result<()> {
    let json = serde_json::to_string_pretty(env)
        .context("Failed to serialize environment variables")?;
    
    // Ensure directory exists
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .context("Failed to create directory for environment store")?;
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true).mode(0o600);
        let mut file = options.open(path)
            .context("Failed to open environment store file")?;
        use std::io::Write;
        file.write_all(json.as_bytes())
            .context("Failed to write to environment store file")?;
    }
    #[cfg(not(unix))]
    {
        fs::write(path, json)
            .context("Failed to write environment store file")?;
    }

    Ok(())
}

/// Loads environment variables from the JSON file.
pub fn load_env(path: &Path) -> Result<HashMap<String, String>> {
    if !path.exists() {
        return Ok(HashMap::new());
    }

    let content = fs::read_to_string(path)
        .context("Failed to read environment store file")?;
        
    let env: HashMap<String, String> = serde_json::from_str(&content)
        .context("Failed to deserialize environment variables")?;

    Ok(env)
}
