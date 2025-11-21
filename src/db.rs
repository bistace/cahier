use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};

pub struct Database {
    conn: Connection,
}

pub struct Entry {
    #[allow(dead_code)]
    pub id: i64,
    pub command: String,
    pub output: String,
    pub cwd: String,
    pub timestamp: DateTime<Utc>,
    pub exit_code: Option<i32>,
    pub duration_ms: i64,
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
                cwd TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                exit_code INTEGER,
                duration_ms INTEGER
            )",
            [],
        )?;
        Ok(())
    }

    pub fn log_entry(
        &self,
        command: &str,
        output: &str,
        cwd: &str,
        exit_code: Option<i32>,
        duration_ms: u128,
    ) -> Result<()> {
        let timestamp = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO entries (command, output, cwd, timestamp, exit_code, duration_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                command,
                output,
                cwd,
                timestamp,
                exit_code,
                duration_ms as i64
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
            "SELECT id, command, output, cwd, timestamp, exit_code, duration_ms 
             FROM entries ORDER BY id ASC"
        )?;
        let entry_iter = stmt.query_map([], |row| {
            Ok(Entry {
                id: row.get(0)?,
                command: row.get(1)?,
                output: row.get(2)?,
                cwd: row.get(3)?,
                timestamp: row.get::<_, String>(4)?.parse().unwrap(),
                exit_code: row.get(5)?,
                duration_ms: row.get(6)?,
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
        
        db.log_entry("echo hello", "hello", "/tmp", Some(0), 100)?;
        
        assert_eq!(db.count_entries()?, 1);
        Ok(())
    }
}
