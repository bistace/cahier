//! Constants shared across the application

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
