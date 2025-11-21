use anyhow::Result;
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
        let conn = Connection::open(path)?;
        Self::setup_schema(&conn)?;
        Ok(Self { conn })
    }

    #[cfg(test)]
    pub fn init_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
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
        )?;
        
        // Migrate existing table if output_file column doesn't exist
        let column_exists: Result<i32, _> = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('entries') WHERE name='output_file'",
            [],
            |row| row.get(0),
        );
        
        if let Ok(0) = column_exists {
            conn.execute("ALTER TABLE entries ADD COLUMN output_file TEXT", [])?;
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
        )?;
        Ok(())
    }

    #[cfg(test)]
    pub fn count_entries(&self) -> Result<i64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM entries",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    pub fn get_entries(&self) -> Result<Vec<Entry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, command, output, exit_code, duration_ms, output_file 
             FROM entries ORDER BY id ASC"
        )?;
        let entry_iter = stmt.query_map([], |row| {
            Ok(Entry {
                id: row.get(0)?,
                command: row.get(1)?,
                output: row.get(2)?,
                exit_code: row.get(3)?,
                duration_ms: row.get(4)?,
                output_file: row.get(5)?,
            })
        })?;

        let mut entries = Vec::new();
        for entry in entry_iter {
            entries.push(entry?);
        }
        Ok(entries)
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
}
