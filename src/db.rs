use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};

pub struct Database {
    conn: Connection,
}

#[allow(dead_code)]
pub struct Session {
    pub id: i64,
    pub start_time: DateTime<Utc>,
    pub name: Option<String>,
}

pub struct Entry {
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
            "CREATE TABLE IF NOT EXISTS sessions (
                id INTEGER PRIMARY KEY,
                start_time TEXT NOT NULL,
                name TEXT
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS entries (
                id INTEGER PRIMARY KEY,
                session_id INTEGER NOT NULL,
                command TEXT NOT NULL,
                output TEXT NOT NULL,
                cwd TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                exit_code INTEGER,
                duration_ms INTEGER,
                FOREIGN KEY(session_id) REFERENCES sessions(id)
            )",
            [],
        )?;
        Ok(())
    }

    pub fn create_session(&self, name: Option<String>) -> Result<i64> {
        let start_time = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO sessions (start_time, name) VALUES (?1, ?2)",
            params![start_time, name],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn log_entry(
        &self,
        session_id: i64,
        command: &str,
        output: &str,
        cwd: &str,
        exit_code: Option<i32>,
        duration_ms: u128,
    ) -> Result<()> {
        let timestamp = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO entries (session_id, command, output, cwd, timestamp, exit_code, duration_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                session_id,
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

    pub fn list_sessions(&self) -> Result<Vec<Session>> {
        let mut stmt = self.conn.prepare("SELECT id, start_time, name FROM sessions ORDER BY id DESC")?;
        let session_iter = stmt.query_map([], |row| {
            Ok(Session {
                id: row.get(0)?,
                start_time: row.get::<_, String>(1)?.parse().unwrap(), // Assuming valid date
                name: row.get(2)?,
            })
        })?;

        let mut sessions = Vec::new();
        for session in session_iter {
            sessions.push(session?);
        }
        Ok(sessions)
    }

    pub fn get_last_session_id(&self) -> Result<Option<i64>> {
        let mut stmt = self.conn.prepare("SELECT id FROM sessions ORDER BY id DESC LIMIT 1")?;
        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    pub fn get_session(&self, id: i64) -> Result<Session> {
        let mut stmt = self.conn.prepare("SELECT id, start_time, name FROM sessions WHERE id = ?1")?;
        let session = stmt.query_row(params![id], |row| {
            Ok(Session {
                id: row.get(0)?,
                start_time: row.get::<_, String>(1)?.parse().unwrap(),
                name: row.get(2)?,
            })
        })?;
        Ok(session)
    }

    pub fn get_entries(&self, session_id: i64) -> Result<Vec<Entry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, command, output, cwd, timestamp, exit_code, duration_ms 
             FROM entries WHERE session_id = ?1 ORDER BY id ASC"
        )?;
        let entry_iter = stmt.query_map(params![session_id], |row| {
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
        let session_id = db.create_session(Some("test".to_string()))?;
        
        db.log_entry(session_id, "echo hello", "hello", "/tmp", Some(0), 100)?;
        
        assert_eq!(db.count_entries()?, 1);
        Ok(())
    }
}
