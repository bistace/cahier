use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;

const CONFIG_DIR: &str = ".cahier";
const CONFIG_FILE: &str = "config.json";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    #[serde(default = "default_ignored_outputs")]
    pub ignored_outputs: Vec<String>,
}

fn default_ignored_outputs() -> Vec<String> {
    vec![
        "nano".to_string(),
        "vim".to_string(),
        "vi".to_string(),
        "emacs".to_string(),
        "hx".to_string(),
        "atom".to_string(),
        "gedit".to_string(),
        "geany".to_string(),
        "kate".to_string(),
        "kwrite".to_string(),
        "nvim".to_string(),
        "htop".to_string(),
        "top".to_string(),
        "atop".to_string(),
        "less".to_string(),
        "more".to_string(),
        "man".to_string(),
        "ssh".to_string(),
        "tmux".to_string(),
        "screen".to_string(),
    ]
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ignored_outputs: default_ignored_outputs(),
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        let home_dir = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Could not find home directory"))?;
        let config_path = home_dir.join(CONFIG_DIR).join(CONFIG_FILE);

        if !config_path.exists() {
            // Create default config if it doesn't exist
            let default_config = Config::default();
            
            // Ensure directory exists
            if let Some(parent) = config_path.parent() {
                fs::create_dir_all(parent)?;
            }
            
            let json = serde_json::to_string_pretty(&default_config)?;
            
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                let mut options = fs::OpenOptions::new();
                options.write(true).create(true).truncate(true).mode(0o600);
                let mut file = options.open(&config_path)?;
                use std::io::Write;
                file.write_all(json.as_bytes())?;
            }
            #[cfg(not(unix))]
            {
                fs::write(&config_path, json)?;
            }
            
            return Ok(default_config);
        }

        let content = fs::read_to_string(&config_path)?;
        let config: Config = serde_json::from_str(&content)?;
        
        Ok(config)
    }
}

