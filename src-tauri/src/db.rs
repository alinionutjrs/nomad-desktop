use crate::models::{ChatMessage, Project, Proposal};
use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::Mutex;

pub struct Database {
    connection: Mutex<Connection>,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        let mut connection = Connection::open(path)
            .with_context(|| format!("Nu pot deschide baza de date {}", path.display()))?;
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS projects (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               name TEXT NOT NULL,
               path TEXT NOT NULL UNIQUE,
               created_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS messages (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
               session_id INTEGER REFERENCES chat_sessions(id) ON DELETE CASCADE,
               role TEXT NOT NULL,
               content TEXT NOT NULL,
               created_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS chat_sessions (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
               title TEXT NOT NULL,
               created_at TEXT NOT NULL,
               updated_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS proposals (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
               path TEXT NOT NULL,
               original_content TEXT NOT NULL,
               proposed_content TEXT NOT NULL,
               summary TEXT NOT NULL,
               status TEXT NOT NULL DEFAULT 'pending',
               created_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS settings (
               project_id INTEGER NOT NULL DEFAULT 0,
               key TEXT NOT NULL,
               value TEXT NOT NULL,
               PRIMARY KEY (project_id, key)
             );",
        )?;

        // Older local databases stored messages by project only. Keep them intact and
        // attach them to one imported conversation per project before isolating sessions.
        let has_session_id = {
            let mut statement = connection.prepare("PRAGMA table_info(messages)")?;
            let columns = statement.query_map([], |row| row.get::<_, String>(1))?;
            columns
                .collect::<rusqlite::Result<Vec<_>>>()?
                .iter()
                .any(|name| name == "session_id")
        };
        if !has_session_id {
            connection.execute(
                "ALTER TABLE messages ADD COLUMN session_id INTEGER REFERENCES chat_sessions(id) ON DELETE CASCADE",
                [],
            )?;
            let tx = connection.transaction()?;
            tx.execute(
                "INSERT INTO chat_sessions (project_id, title, created_at, updated_at)
                 SELECT p.id, 'Conversație importată', MIN(m.created_at), MAX(m.created_at)
                 FROM projects p JOIN messages m ON m.project_id = p.id
                 WHERE NOT EXISTS (SELECT 1 FROM chat_sessions s WHERE s.project_id = p.id)
                 GROUP BY p.id",
                [],
            )?;
            tx.execute(
                "UPDATE messages SET session_id = (
                   SELECT s.id FROM chat_sessions s
                   WHERE s.project_id = messages.project_id ORDER BY s.id ASC LIMIT 1
                 ) WHERE session_id IS NULL",
                [],
            )?;
            tx.commit()?;
        }
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| anyhow::anyhow!("Baza de date este blocată"))
    }

    pub fn upsert_project(&self, name: &str, path: &str) -> Result<Project> {
        let connection = self.lock()?;
        connection.execute(
            "INSERT INTO projects (name, path, created_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(path) DO UPDATE SET name = excluded.name",
            params![name, path, Utc::now().to_rfc3339()],
        )?;
        Self::get_project_with(&connection, path)
    }

    fn get_project_with(connection: &Connection, path: &str) -> Result<Project> {
        connection
            .query_row(
                "SELECT id, name, path, created_at FROM projects WHERE path = ?1",
                [path],
                |row| {
                    Ok(Project {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        path: row.get(2)?,
                        created_at: row.get(3)?,
                    })
                },
            )
            .map_err(Into::into)
    }

    pub fn get_project(&self, id: i64) -> Result<Project> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT id, name, path, created_at FROM projects WHERE id = ?1",
                [id],
                |row| {
                    Ok(Project {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        path: row.get(2)?,
                        created_at: row.get(3)?,
                    })
                },
            )
            .with_context(|| format!("Proiectul {id} nu există"))
    }

    pub fn list_projects(&self) -> Result<Vec<Project>> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare("SELECT id, name, path, created_at FROM projects ORDER BY id DESC")?;
        let rows = statement.query_map([], |row| {
            Ok(Project {
                id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn list_messages(&self, project_id: i64, limit: usize) -> Result<Vec<ChatMessage>> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT id, role, content, created_at FROM (
               SELECT id, role, content, created_at FROM messages WHERE project_id = ?1 ORDER BY id DESC LIMIT ?2
             ) ORDER BY id ASC"
        )?;
        let rows = statement.query_map(params![project_id, limit as i64], |row| {
            Ok(ChatMessage {
                id: row.get(0)?,
                role: row.get(1)?,
                content: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn create_chat_session(&self, project_id: i64, title: &str) -> Result<i64> {
        let now = Utc::now().to_rfc3339();
        let connection = self.lock()?;
        connection.execute(
            "INSERT INTO chat_sessions (project_id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
            params![project_id, title.chars().take(100).collect::<String>(), now],
        )?;
        Ok(connection.last_insert_rowid())
    }

    pub fn get_chat_session(&self, session_id: i64) -> Result<(i64, String)> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT project_id, title FROM chat_sessions WHERE id = ?1",
                [session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(Into::into)
    }

    pub fn insert_session_message(
        &self,
        session_id: i64,
        role: &str,
        content: &str,
    ) -> Result<ChatMessage> {
        let created_at = Utc::now().to_rfc3339();
        let connection = self.lock()?;
        let project_id: i64 = connection
            .query_row(
                "SELECT project_id FROM chat_sessions WHERE id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .context("Conversația nu există")?;
        connection.execute(
            "INSERT INTO messages (project_id, session_id, role, content, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![project_id, session_id, role, content, created_at],
        )?;
        connection.execute(
            "UPDATE chat_sessions SET title = CASE WHEN title = '' OR title = 'Conversație nouă' THEN ?1 ELSE title END, updated_at = ?2 WHERE id = ?3",
            params![content.chars().take(80).collect::<String>(), created_at, session_id],
        )?;
        Ok(ChatMessage {
            id: connection.last_insert_rowid(),
            role: role.into(),
            content: content.into(),
            created_at,
        })
    }

    pub fn list_session_messages(&self, session_id: i64, limit: usize) -> Result<Vec<ChatMessage>> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT id, role, content, created_at FROM (
               SELECT id, role, content, created_at FROM messages WHERE session_id = ?1 ORDER BY id DESC LIMIT ?2
             ) ORDER BY id ASC"
        )?;
        let rows = statement.query_map(params![session_id, limit as i64], |row| {
            Ok(ChatMessage {
                id: row.get(0)?,
                role: row.get(1)?,
                content: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn list_chat_sessions(&self, project_id: i64) -> Result<Vec<serde_json::Value>> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT id, title, created_at, updated_at FROM chat_sessions WHERE project_id = ?1 ORDER BY updated_at DESC"
        )?;
        let rows = statement.query_map([project_id], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, i64>(0)?, "title": row.get::<_, String>(1)?,
                "created_at": row.get::<_, String>(2)?, "updated_at": row.get::<_, String>(3)?
            }))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn latest_chat_session_id(&self, project_id: i64) -> Result<Option<i64>> {
        Ok(self.lock()?.query_row(
            "SELECT id FROM chat_sessions WHERE project_id = ?1 ORDER BY updated_at DESC LIMIT 1",
            [project_id], |row| row.get(0),
        ).optional()?)
    }

    pub fn create_proposal(
        &self,
        project_id: i64,
        path: &str,
        original: &str,
        proposed: &str,
        summary: &str,
    ) -> Result<Proposal> {
        let created_at = Utc::now().to_rfc3339();
        let connection = self.lock()?;
        connection.execute(
            "INSERT INTO proposals (project_id, path, original_content, proposed_content, summary, status, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6)",
            params![project_id, path, original, proposed, summary, created_at],
        )?;
        Ok(Proposal {
            id: connection.last_insert_rowid(),
            project_id,
            path: path.into(),
            original_content: original.into(),
            proposed_content: proposed.into(),
            summary: summary.into(),
            status: "pending".into(),
            created_at,
        })
    }

    pub fn get_proposal(&self, id: i64) -> Result<Proposal> {
        let connection = self.lock()?;
        connection.query_row(
            "SELECT id, project_id, path, original_content, proposed_content, summary, status, created_at FROM proposals WHERE id = ?1",
            [id],
            |row| Ok(Proposal { id: row.get(0)?, project_id: row.get(1)?, path: row.get(2)?, original_content: row.get(3)?, proposed_content: row.get(4)?, summary: row.get(5)?, status: row.get(6)?, created_at: row.get(7)? }),
        ).map_err(Into::into)
    }

    pub fn list_proposals(&self, project_id: i64) -> Result<Vec<Proposal>> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT id, project_id, path, original_content, proposed_content, summary, status, created_at
             FROM proposals WHERE project_id = ?1 ORDER BY id DESC LIMIT 50"
        )?;
        let rows = statement.query_map([project_id], |row| {
            Ok(Proposal {
                id: row.get(0)?,
                project_id: row.get(1)?,
                path: row.get(2)?,
                original_content: row.get(3)?,
                proposed_content: row.get(4)?,
                summary: row.get(5)?,
                status: row.get(6)?,
                created_at: row.get(7)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn update_proposal_status(&self, id: i64, status: &str) -> Result<Proposal> {
        if !matches!(status, "pending" | "accepted" | "rejected") {
            anyhow::bail!("Starea propunerii nu este validă");
        }
        let connection = self.lock()?;
        connection.execute(
            "UPDATE proposals SET status = ?1 WHERE id = ?2 AND status = 'pending'",
            params![status, id],
        )?;
        drop(connection);
        self.get_proposal(id)
    }

    pub fn set_setting(&self, project_id: i64, key: &str, value: &str) -> Result<()> {
        self.lock()?.execute(
            "INSERT INTO settings (project_id, key, value) VALUES (?1, ?2, ?3)
             ON CONFLICT(project_id, key) DO UPDATE SET value = excluded.value",
            params![project_id, key, value],
        )?;
        Ok(())
    }

    pub fn get_setting(&self, project_id: i64, key: &str) -> Result<Option<String>> {
        Ok(self
            .lock()?
            .query_row(
                "SELECT value FROM settings WHERE project_id = ?1 AND key = ?2",
                params![project_id, key],
                |row| row.get(0),
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn sessions_keep_messages_isolated_within_a_project() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let db = Database::open(&directory.path().join("nomad.sqlite3"))?;
        let project = db.upsert_project("demo", "C:/demo")?;
        let first = db.create_chat_session(project.id, "Prima")?;
        let second = db.create_chat_session(project.id, "A doua")?;

        db.insert_session_message(first, "user", "Întrebare unu")?;
        db.insert_session_message(second, "user", "Întrebare doi")?;

        assert_eq!(
            db.list_session_messages(first, 100)?[0].content,
            "Întrebare unu"
        );
        assert_eq!(
            db.list_session_messages(second, 100)?[0].content,
            "Întrebare doi"
        );
        Ok(())
    }

    #[test]
    fn upgrades_legacy_project_messages_without_losing_history() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("legacy.sqlite3");
        let legacy = Connection::open(&path)?;
        legacy.execute_batch(
            "CREATE TABLE projects (id INTEGER PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL UNIQUE, created_at TEXT NOT NULL);
             CREATE TABLE messages (id INTEGER PRIMARY KEY, project_id INTEGER NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL, created_at TEXT NOT NULL);
             CREATE TABLE chat_sessions (id INTEGER PRIMARY KEY, project_id INTEGER NOT NULL, title TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
             INSERT INTO projects VALUES (1, 'demo', 'C:/demo', '2026-11-10T00:00:00Z');
             INSERT INTO messages VALUES (1, 1, 'user', 'Istoric păstrat', '2026-11-10T00:00:00Z');",
        )?;
        drop(legacy);

        let db = Database::open(&path)?;
        let session_id = db
            .latest_chat_session_id(1)?
            .expect("legacy conversation created");
        let messages = db.list_session_messages(session_id, 100)?;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "Istoric păstrat");
        Ok(())
    }
}
