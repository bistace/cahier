use anyhow::{Context, Result};
use rusqlite::{params, Connection};

pub struct Database {
    conn: Connection,
}

pub struct Entry {
    #[allow(dead_code)]
    pub id: i64,
    pub command: String,
    pub output: String,
    pub exit_code: Option<i32>,
    pub duration_ms: i64,
    pub output_file: Option<String>,
}

impl Database {
    pub fn init(path: &str) -> Result<Self> {
        let conn = Connection::open(path).context("Failed to open database")?;
        Self::setup_schema(&conn).context("Failed to setup schema")?;
        Ok(Self { conn })
    }

    #[cfg(test)]
    pub fn init_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().context("Failed to open in-memory database")?;
        Self::setup_schema(&conn)?;
        Ok(Self { conn })
    }

    fn setup_schema(conn: &Connection) -> Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS entries (
                id INTEGER PRIMARY KEY,
                command TEXT NOT NULL,
                output TEXT NOT NULL,
                exit_code INTEGER,
                duration_ms INTEGER,
                output_file TEXT
            )",
            [],
        ).context("Failed to create entries table")?;
        
        // Migrate existing table if output_file column doesn't exist
        let column_exists: Result<i32, _> = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('entries') WHERE name='output_file'",
            [],
            |row| row.get(0),
        );
        
        if let Ok(0) = column_exists {
            conn.execute("ALTER TABLE entries ADD COLUMN output_file TEXT", []).context("Failed to add output_file column")?;
        }

        // Attempt to drop legacy columns if they exist (SQLite 3.35.0+)
        // We ignore errors because they might not exist or SQLite version might be old
        let _ = conn.execute("ALTER TABLE entries DROP COLUMN cwd", []);
        let _ = conn.execute("ALTER TABLE entries DROP COLUMN timestamp", []);
        
        Ok(())
    }

    pub fn log_entry(
        &self,
        command: &str,
        output: &str,
        exit_code: Option<i32>,
        duration_ms: u128,
        output_file: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO entries (command, output, exit_code, duration_ms, output_file)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                command,
                output,
                exit_code,
                duration_ms as i64,
                output_file
            ],
        ).context("Failed to insert log entry")?;
        Ok(())
    }

    #[cfg(test)]
    pub fn count_entries(&self) -> Result<i64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM entries",
            [],
            |row| row.get(0),
        ).context("Failed to count entries")?;
        Ok(count)
    }

    pub fn iterate_entries<F>(&self, mut callback: F) -> Result<()>
    where
        F: FnMut(Entry) -> Result<()>,
    {
        let mut stmt = self.conn.prepare(
            "SELECT id, command, output, exit_code, duration_ms, output_file 
             FROM entries ORDER BY id ASC"
        ).context("Failed to prepare iteration statement")?;
        let entry_iter = stmt.query_map([], |row| {
            Ok(Entry {
                id: row.get(0)?,
                command: row.get(1)?,
                output: row.get(2)?,
                exit_code: row.get(3)?,
                duration_ms: row.get(4)?,
                output_file: row.get(5)?,
            })
        }).context("Failed to query entries")?;

        for entry in entry_iter {
            callback(entry.context("Failed to read entry row")?)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_db_workflow() -> Result<()> {
        let db = Database::init_memory()?;
        
        db.log_entry("echo hello", "hello", Some(0), 100, None)?;
        
        assert_eq!(db.count_entries()?, 1);
        Ok(())
    }

    #[test]
    fn test_iterate_entries() -> Result<()> {
        let db = Database::init_memory()?;
        
        // Log multiple entries with different data
        db.log_entry("echo hello", "hello\n", Some(0), 100, None)?;
        db.log_entry("cat file.txt", "file content", Some(0), 250, None)?;
        db.log_entry("failing_command", "error output", Some(1), 50, Some(".cahier/outputs/output_123.txt"))?;
        
        let mut entries = Vec::new();
        db.iterate_entries(|entry| {
            entries.push(entry);
            Ok(())
        })?;
        
        assert_eq!(entries.len(), 3);
        
        // Verify first entry
        assert_eq!(entries[0].command, "echo hello");
        assert_eq!(entries[0].output, "hello\n");
        assert_eq!(entries[0].exit_code, Some(0));
        assert_eq!(entries[0].duration_ms, 100);
        assert_eq!(entries[0].output_file, None);
        
        // Verify second entry
        assert_eq!(entries[1].command, "cat file.txt");
        assert_eq!(entries[1].output, "file content");
        assert_eq!(entries[1].exit_code, Some(0));
        assert_eq!(entries[1].duration_ms, 250);
        
        // Verify third entry with output file
        assert_eq!(entries[2].command, "failing_command");
        assert_eq!(entries[2].exit_code, Some(1));
        assert_eq!(entries[2].output_file, Some(".cahier/outputs/output_123.txt".to_string()));
        
        Ok(())
    }

    #[test]
    fn test_schema_migration() -> Result<()> {
        // Create a connection with old schema (missing output_file column)
        let conn = Connection::open_in_memory()?;
        
        // Create old schema without output_file column
        conn.execute(
            "CREATE TABLE entries (
                id INTEGER PRIMARY KEY,
                command TEXT NOT NULL,
                output TEXT NOT NULL,
                exit_code INTEGER,
                duration_ms INTEGER
            )",
            [],
        )?;
        
        // Insert data with old schema
        conn.execute(
            "INSERT INTO entries (command, output, exit_code, duration_ms)
             VALUES (?1, ?2, ?3, ?4)",
            params!["test command", "test output", 0, 100],
        )?;
        
        // Verify column doesn't exist yet
        let column_count_before: i32 = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('entries') WHERE name='output_file'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(column_count_before, 0);
        
        // Run migration
        Database::setup_schema(&conn)?;
        
        // Verify column now exists
        let column_count_after: i32 = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('entries') WHERE name='output_file'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(column_count_after, 1);
        
        // Verify existing data is preserved
        let count: i32 = conn.query_row(
            "SELECT COUNT(*) FROM entries",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(count, 1);
        
        // Verify we can query the old data with new schema
        let command: String = conn.query_row(
            "SELECT command FROM entries WHERE id=1",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(command, "test command");
        
        Ok(())
    }
}
