//! Constants shared across the application

use std::path::PathBuf;
use std::sync::OnceLock;

static BASE_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Main directory for cahier files
pub const CAHIER_DIR: &str = "cahier_logs";

/// Default database filename
pub const DB_FILENAME: &str = "cahier_logs/cahier.db";

/// Default history filename
pub const HISTORY_FILENAME: &str = "cahier_logs/cahier_history.txt";

/// Default maximum history entries
pub const MAX_HISTORY_ENTRIES: usize = 5000;

/// Default maximum output size before redirecting to file
pub const DEFAULT_MAX_OUTPUT_SIZE: usize = 16384;

/// Directory for storing large outputs
pub const OUTPUT_DIR: &str = "cahier_logs/outputs";

/// Directory for temporary files (e.g. env dumps)
pub const TEMP_DIR: &str = "cahier_logs/tmp";

/// File to store environment state
pub const ENV_STORE_FILENAME: &str = "cahier_logs/env_state.json";

/// Global Cahier configuration directory under the user's home directory
pub const GLOBAL_CONFIG_DIR: &str = ".cahier";

/// Global database filename for reusable snippets
pub const GLOBAL_SNIPPETS_DB_FILENAME: &str = ".cahier/snippets.db";

/// Initializes the session base directory once (defaults to the initial working directory).
pub fn init_base_dir() -> PathBuf {
    let base = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    set_base_dir(base)
}

/// Sets the base directory for this session if it hasn't been set yet.
pub fn set_base_dir(base: PathBuf) -> PathBuf {
    BASE_DIR.get_or_init(|| base).clone()
}

/// Returns the base directory for this session.
pub fn base_dir() -> PathBuf {
    BASE_DIR
        .get_or_init(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .clone()
}

/// Returns the absolute path to the cahier log directory for this session.
pub fn cahier_dir() -> PathBuf {
    base_dir().join(CAHIER_DIR)
}

/// Returns the absolute path to the database file for this session.
pub fn db_path() -> PathBuf {
    base_dir().join(DB_FILENAME)
}

/// Returns the absolute path to the history file for this session.
pub fn history_path() -> PathBuf {
    base_dir().join(HISTORY_FILENAME)
}

/// Returns the absolute path to the outputs directory for this session.
pub fn output_dir() -> PathBuf {
    base_dir().join(OUTPUT_DIR)
}

/// Returns the absolute path to the temp directory for this session.
pub fn temp_dir() -> PathBuf {
    base_dir().join(TEMP_DIR)
}

/// Returns the absolute path to the env store file for this session.
pub fn env_store_path() -> PathBuf {
    base_dir().join(ENV_STORE_FILENAME)
}

/// Returns the absolute path to the global Cahier configuration directory.
pub fn global_config_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(GLOBAL_CONFIG_DIR)
}

/// Returns the absolute path to the global snippets database.
pub fn global_snippets_db_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(GLOBAL_SNIPPETS_DB_FILENAME)
}
