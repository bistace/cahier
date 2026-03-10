use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

pub struct Database {
    conn: Connection,
}

#[derive(Debug, Clone)]
pub struct Entry {
    #[allow(dead_code)]
    pub id: i64,
    pub command: String,
    pub output: String,
    pub exit_code: Option<i32>,
    pub duration_ms: i64,
    pub output_file: Option<String>,
    pub annotation: Option<String>,
    pub rank: i64,
    pub is_separator: bool,
}

#[derive(Debug, Clone)]
pub struct EntrySummary {
    pub id: i64,
    pub command: String,
    pub exit_code: Option<i32>,
    pub duration_ms: i64,
    pub annotation: Option<String>,
    pub rank: i64,
    pub is_separator: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum Direction {
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnippetScope {
    Project,
    Global,
}

impl SnippetScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Global => "global",
        }
    }
}

impl std::fmt::Display for SnippetScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct Snippet {
    pub id: i64,
    pub name: String,
    pub command: String,
    pub description: Option<String>,
    pub scope: SnippetScope,
    pub tags: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl Database {
    pub fn init<P: AsRef<Path>>(path: P) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent).context("Failed to create database directory")?;
            }
        }
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
                output_file TEXT,
                annotation TEXT,
                rank INTEGER
            )",
            [],
        )
        .context("Failed to create entries table")?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS snippets (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                command TEXT NOT NULL,
                description TEXT,
                scope TEXT NOT NULL CHECK(scope IN ('project', 'global')),
                tags TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
            [],
        )
        .context("Failed to create snippets table")?;

        // Migrate existing table if output_file column doesn't exist
        let column_exists: Result<i32, _> = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('entries') WHERE name='output_file'",
            [],
            |row| row.get(0),
        );

        if let Ok(0) = column_exists {
            conn.execute("ALTER TABLE entries ADD COLUMN output_file TEXT", [])
                .context("Failed to add output_file column")?;
        }

        // Migrate for annotation and rank
        let rank_exists: Result<i32, _> = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('entries') WHERE name='rank'",
            [],
            |row| row.get(0),
        );

        if let Ok(0) = rank_exists {
            conn.execute("ALTER TABLE entries ADD COLUMN annotation TEXT", [])
                .context("Failed to add annotation column")?;
            conn.execute("ALTER TABLE entries ADD COLUMN rank INTEGER", [])
                .context("Failed to add rank column")?;
            // Initialize rank with id for existing entries
            conn.execute("UPDATE entries SET rank = id", [])
                .context("Failed to initialize rank")?;
        }

        // Migrate for is_separator
        let separator_exists: Result<i32, _> = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('entries') WHERE name='is_separator'",
            [],
            |row| row.get(0),
        );

        if let Ok(0) = separator_exists {
            conn.execute(
                "ALTER TABLE entries ADD COLUMN is_separator INTEGER DEFAULT 0",
                [],
            )
            .context("Failed to add is_separator column")?;
        }

        // Attempt to drop legacy columns if they exist (SQLite 3.35.0+)
        // We ignore errors because they might not exist or SQLite version might be old
        let _ = conn.execute("ALTER TABLE entries DROP COLUMN cwd", []);
        let _ = conn.execute("ALTER TABLE entries DROP COLUMN timestamp", []);

        Ok(())
    }

    pub fn create_snippet(
        &self,
        name: &str,
        command: &str,
        description: Option<&str>,
        scope: SnippetScope,
        tags: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn
            .execute(
                "INSERT INTO snippets (name, command, description, scope, tags, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                params![name, command, description, scope.as_str(), tags, now],
            )
            .context("Failed to create snippet")?;
        Ok(())
    }

    pub fn update_snippet(
        &self,
        id: i64,
        name: &str,
        description: Option<&str>,
        tags: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn
            .execute(
                "UPDATE snippets
                 SET name = ?1, description = ?2, tags = ?3, updated_at = ?4
                 WHERE id = ?5",
                params![name, description, tags, now, id],
            )
            .context("Failed to update snippet")?;
        Ok(())
    }

    pub fn delete_snippet(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM snippets WHERE id = ?1", params![id])
            .context("Failed to delete snippet")?;
        Ok(())
    }

    pub fn get_all_snippets(&self) -> Result<Vec<Snippet>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, command, description, scope, tags, created_at, updated_at
             FROM snippets ORDER BY updated_at DESC, id DESC",
        )?;

        let rows = stmt.query_map([], |row| {
            let scope_str: String = row.get(4)?;
            let scope = match scope_str.as_str() {
                "project" => SnippetScope::Project,
                "global" => SnippetScope::Global,
                _ => return Err(rusqlite::Error::InvalidColumnType(4, "scope".into(), rusqlite::types::Type::Text)),
            };

            Ok(Snippet {
                id: row.get(0)?,
                name: row.get(1)?,
                command: row.get(2)?,
                description: row.get(3)?,
                scope,
                tags: row.get(5)?,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
            })
        })?;

        let mut snippets = Vec::new();
        for row in rows {
            snippets.push(row?);
        }
        Ok(snippets)
    }

    pub fn get_snippet(&self, id: i64) -> Result<Snippet> {
        let snippet = self
            .conn
            .query_row(
                "SELECT id, name, command, description, scope, tags, created_at, updated_at
                 FROM snippets WHERE id = ?1",
                params![id],
                |row| {
                    let scope_str: String = row.get(4)?;
                    let scope = match scope_str.as_str() {
                        "project" => SnippetScope::Project,
                        "global" => SnippetScope::Global,
                        _ => {
                            return Err(rusqlite::Error::InvalidColumnType(
                                4,
                                "scope".into(),
                                rusqlite::types::Type::Text,
                            ))
                        }
                    };

                    Ok(Snippet {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        command: row.get(2)?,
                        description: row.get(3)?,
                        scope,
                        tags: row.get(5)?,
                        created_at: row.get(6)?,
                        updated_at: row.get(7)?,
                    })
                },
            )
            .context("Failed to fetch snippet")?;
        Ok(snippet)
    }

    pub fn log_entry(
        &self,
        command: &str,
        output: &str,
        exit_code: Option<i32>,
        duration_ms: u128,
        output_file: Option<&str>,
    ) -> Result<()> {
        // We want rank to be next available.
        // However, for simplicity and since ID is auto-increment,
        // we can let SQLite handle ID and then update rank or just insert max rank + 1.
        // Using a transaction to ensure consistency would be better but for this app,
        // simple query is likely fine.

        // Let's just use max(rank) + 1.
        let next_rank: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(rank), 0) + 1 FROM entries",
                [],
                |row| row.get(0),
            )
            .unwrap_or(1);

        self.conn.execute(
            "INSERT INTO entries (command, output, exit_code, duration_ms, output_file, rank, is_separator)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
            params![
                command,
                output,
                exit_code,
                duration_ms as i64,
                output_file,
                next_rank
            ],
        ).context("Failed to insert log entry")?;
        Ok(())
    }

    pub fn insert_separator(&self, target_rank: i64) -> Result<()> {
        // Shift existing ranks
        self.conn
            .execute(
                "UPDATE entries SET rank = rank + 1 WHERE rank >= ?1",
                params![target_rank],
            )
            .context("Failed to shift ranks for separator")?;

        // Insert separator
        self.conn
            .execute(
                "INSERT INTO entries (command, output, rank, is_separator, duration_ms)
             VALUES ('', '', ?1, 1, 0)",
                params![target_rank],
            )
            .context("Failed to insert separator")?;

        Ok(())
    }

    pub fn delete_entry(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM entries WHERE id = ?1", params![id])
            .context("Failed to delete entry")?;
        Ok(())
    }

    pub fn update_annotation(&self, id: i64, annotation: String) -> Result<()> {
        self.conn
            .execute(
                "UPDATE entries SET annotation = ?1 WHERE id = ?2",
                params![annotation, id],
            )
            .context("Failed to update annotation")?;
        Ok(())
    }

    pub fn move_entry(&self, id: i64, direction: Direction) -> Result<()> {
        // Find the current entry's rank
        let current_rank: i64 = self
            .conn
            .query_row(
                "SELECT rank FROM entries WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .context("Entry not found")?;

        // Find the target entry to swap with
        let swap_target =
            match direction {
                Direction::Up => {
                    // Find entry with rank immediately less than current
                    self.conn.query_row(
                    "SELECT id, rank FROM entries WHERE rank < ?1 ORDER BY rank DESC LIMIT 1",
                    params![current_rank],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
                ).optional()?
                }
                Direction::Down => {
                    // Find entry with rank immediately greater than current
                    self.conn.query_row(
                    "SELECT id, rank FROM entries WHERE rank > ?1 ORDER BY rank ASC LIMIT 1",
                    params![current_rank],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
                ).optional()?
                }
            };

        if let Some((target_id, target_rank)) = swap_target {
            // Swap ranks
            self.conn.execute(
                "UPDATE entries SET rank = ?1 WHERE id = ?2",
                params![target_rank, id],
            )?;
            self.conn.execute(
                "UPDATE entries SET rank = ?1 WHERE id = ?2",
                params![current_rank, target_id],
            )?;
        }

        Ok(())
    }

    pub fn get_all_entries_ordered(&self) -> Result<Vec<Entry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, command, output, exit_code, duration_ms, output_file, annotation, rank, is_separator 
             FROM entries ORDER BY rank ASC"
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(Entry {
                id: row.get(0)?,
                command: row.get(1)?,
                output: row.get(2)?,
                exit_code: row.get(3)?,
                duration_ms: row.get(4)?,
                output_file: row.get(5)?,
                annotation: row.get(6)?,
                rank: row.get(7)?,
                is_separator: row.get::<_, i32>(8)? != 0,
            })
        })?;

        let mut entries = Vec::new();
        for row in rows {
            entries.push(row?);
        }
        Ok(entries)
    }

    pub fn get_all_entry_summaries(&self) -> Result<Vec<EntrySummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, command, exit_code, duration_ms, annotation, rank, is_separator 
             FROM entries ORDER BY rank ASC",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(EntrySummary {
                id: row.get(0)?,
                command: row.get(1)?,
                exit_code: row.get(2)?,
                duration_ms: row.get(3)?,
                annotation: row.get(4)?,
                rank: row.get(5)?,
                is_separator: row.get::<_, i32>(6)? != 0,
            })
        })?;

        let mut entries = Vec::new();
        for row in rows {
            entries.push(row?);
        }
        Ok(entries)
    }

    pub fn get_entry_output(&self, id: i64) -> Result<String> {
        let output: String = self
            .conn
            .query_row(
                "SELECT output FROM entries WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .context("Failed to fetch output")?;
        Ok(output)
    }

    #[cfg(test)]
    pub fn count_entries(&self) -> Result<i64> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM entries", [], |row| row.get(0))
            .context("Failed to count entries")?;
        Ok(count)
    }

    pub fn iterate_entries<F>(&self, mut callback: F) -> Result<()>
    where
        F: FnMut(Entry) -> Result<()>,
    {
        let mut stmt = self.conn.prepare(
            "SELECT id, command, output, exit_code, duration_ms, output_file, annotation, rank, is_separator 
             FROM entries ORDER BY rank ASC"
        ).context("Failed to prepare iteration statement")?;
        let entry_iter = stmt
            .query_map([], |row| {
                Ok(Entry {
                    id: row.get(0)?,
                    command: row.get(1)?,
                    output: row.get(2)?,
                    exit_code: row.get(3)?,
                    duration_ms: row.get(4)?,
                    output_file: row.get(5)?,
                    annotation: row.get(6)?,
                    rank: row.get(7)?,
                    is_separator: row.get::<_, i32>(8)? != 0,
                })
            })
            .context("Failed to query entries")?;

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
        db.log_entry(
            "failing_command",
            "error output",
            Some(1),
            50,
            Some(".cahier/outputs/output_123.txt"),
        )?;

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
        assert_eq!(
            entries[2].output_file,
            Some(".cahier/outputs/output_123.txt".to_string())
        );

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
        let count: i32 = conn.query_row("SELECT COUNT(*) FROM entries", [], |row| row.get(0))?;
        assert_eq!(count, 1);

        // Verify we can query the old data with new schema
        let command: String =
            conn.query_row("SELECT command FROM entries WHERE id=1", [], |row| {
                row.get(0)
            })?;
        assert_eq!(command, "test command");

        Ok(())
    }

    #[test]
    fn test_reordering() -> Result<()> {
        let db = Database::init_memory()?;

        db.log_entry("1", "1", None, 0, None)?; // rank 1
        db.log_entry("2", "2", None, 0, None)?; // rank 2
        db.log_entry("3", "3", None, 0, None)?; // rank 3

        let entries = db.get_all_entries_ordered()?;
        assert_eq!(entries[0].command, "1");
        assert_eq!(entries[1].command, "2");
        assert_eq!(entries[2].command, "3");

        // Move 2 up -> should swap with 1
        let id_2 = entries[1].id;
        db.move_entry(id_2, Direction::Up)?;

        let entries = db.get_all_entries_ordered()?;
        assert_eq!(entries[0].command, "2");
        assert_eq!(entries[1].command, "1");
        assert_eq!(entries[2].command, "3");

        // Move 2 down -> should swap with 1
        db.move_entry(id_2, Direction::Down)?;
        let entries = db.get_all_entries_ordered()?;
        assert_eq!(entries[0].command, "1");
        assert_eq!(entries[1].command, "2");
        assert_eq!(entries[2].command, "3");

        Ok(())
    }

    #[test]
    fn test_iterate_entries_ordering() -> Result<()> {
        let db = Database::init_memory()?;

        // Insert 1, 2, 3
        db.log_entry("1", "", None, 0, None)?;
        db.log_entry("2", "", None, 0, None)?;
        db.log_entry("3", "", None, 0, None)?;

        // Move 3 to top (rank 1)
        // Current ranks: 1(1), 2(2), 3(3)
        // Move 3 up twice
        let entries = db.get_all_entries_ordered()?;
        let id_3 = entries[2].id;

        db.move_entry(id_3, Direction::Up)?; // swap with 2
        db.move_entry(id_3, Direction::Up)?; // swap with 1

        // Expected order: 3, 1, 2
        let mut commands = Vec::new();
        db.iterate_entries(|e| {
            commands.push(e.command);
            Ok(())
        })?;

        assert_eq!(commands[0], "3");
        assert_eq!(commands[1], "1");
        assert_eq!(commands[2], "2");

        Ok(())
    }

    #[test]
    fn test_snippet_workflow() -> Result<()> {
        let db = Database::init_memory()?;

        db.create_snippet(
            "Run tests",
            "cargo test",
            Some("Workspace test suite"),
            SnippetScope::Project,
            Some("rust,test"),
        )?;

        let snippets = db.get_all_snippets()?;
        assert_eq!(snippets.len(), 1);
        assert_eq!(snippets[0].name, "Run tests");
        assert_eq!(snippets[0].command, "cargo test");
        assert_eq!(snippets[0].scope, SnippetScope::Project);
        assert_eq!(snippets[0].tags.as_deref(), Some("rust,test"));

        let id = snippets[0].id;
        db.update_snippet(
            id,
            "Run full tests",
            Some("Expanded test suite"),
            Some("rust,ci"),
        )?;

        let snippet = db.get_snippet(id)?;
        assert_eq!(snippet.name, "Run full tests");
        assert_eq!(snippet.description.as_deref(), Some("Expanded test suite"));
        assert_eq!(snippet.tags.as_deref(), Some("rust,ci"));

        db.delete_snippet(id)?;
        assert!(db.get_all_snippets()?.is_empty());

        Ok(())
    }

    #[test]
    fn test_snippet_scope_roundtrip() -> Result<()> {
        let db = Database::init_memory()?;

        db.create_snippet("Deploy", "bin/deploy", None, SnippetScope::Global, None)?;

        let snippet = db.get_all_snippets()?.remove(0);
        assert_eq!(snippet.scope, SnippetScope::Global);

        Ok(())
    }
}
