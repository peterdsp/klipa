//! HistoryStore adapter — SQLite via `rusqlite`.

use async_trait::async_trait;
use klipa_core::{CoreError, HistoryItem, HistoryItemId, HistoryStore, ItemContent, ItemKind};
use rusqlite::{params, Connection};
use std::path::PathBuf;
use std::sync::Mutex;
use time::OffsetDateTime;
use uuid::Uuid;

pub struct SqliteStore {
    conn: Mutex<Connection>,
}

impl SqliteStore {
    pub async fn new() -> klipa_core::Result<Self> {
        let path = data_file_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CoreError::Storage(e.to_string()))?;
        }
        let conn = Connection::open(&path).map_err(|e| CoreError::Storage(e.to_string()))?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS history (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                application TEXT,
                pin TEXT,
                number_of_copies INTEGER NOT NULL,
                first_copied_at INTEGER NOT NULL,
                last_copied_at INTEGER NOT NULL,
                contents_json TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_last_copied ON history(last_copied_at DESC);
            "#,
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }
}

fn data_file_path() -> klipa_core::Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("dev", "peterdsp", "klipa")
        .ok_or_else(|| CoreError::Storage("no project dir".into()))?;
    Ok(dirs.data_dir().join("history.sqlite"))
}

fn row_to_item(row: &rusqlite::Row) -> rusqlite::Result<HistoryItem> {
    let id_s: String = row.get("id")?;
    let id = HistoryItemId(
        Uuid::parse_str(&id_s).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })?,
    );
    let title: String = row.get("title")?;
    let application: Option<String> = row.get("application")?;
    let pin: Option<String> = row.get("pin")?;
    let number_of_copies: u32 = row.get::<_, i64>("number_of_copies")? as u32;
    let first_ts: i64 = row.get("first_copied_at")?;
    let last_ts: i64 = row.get("last_copied_at")?;
    let contents_json: String = row.get("contents_json")?;
    let contents: Vec<ItemContent> = serde_json::from_str(&contents_json).unwrap_or_else(|_| {
        vec![ItemContent {
            kind: ItemKind::Text,
            value: title.clone(),
        }]
    });
    Ok(HistoryItem {
        id,
        contents,
        title,
        application,
        pin,
        number_of_copies,
        first_copied_at: OffsetDateTime::from_unix_timestamp(first_ts).unwrap_or(OffsetDateTime::UNIX_EPOCH),
        last_copied_at: OffsetDateTime::from_unix_timestamp(last_ts).unwrap_or(OffsetDateTime::UNIX_EPOCH),
    })
}

#[async_trait]
impl HistoryStore for SqliteStore {
    async fn load(&self) -> klipa_core::Result<Vec<HistoryItem>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT * FROM history ORDER BY last_copied_at DESC")
            .map_err(|e| CoreError::Storage(e.to_string()))?;
        let items = stmt
            .query_map([], row_to_item)
            .map_err(|e| CoreError::Storage(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| CoreError::Storage(e.to_string()))?;
        Ok(items)
    }

    async fn insert(&self, item: &HistoryItem) -> klipa_core::Result<()> {
        let conn = self.conn.lock().unwrap();
        let contents = serde_json::to_string(&item.contents)
            .map_err(|e| CoreError::Storage(e.to_string()))?;
        conn.execute(
            "INSERT INTO history (id, title, application, pin, number_of_copies, first_copied_at, last_copied_at, contents_json) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                item.id.0.to_string(),
                item.title,
                item.application,
                item.pin,
                item.number_of_copies as i64,
                item.first_copied_at.unix_timestamp(),
                item.last_copied_at.unix_timestamp(),
                contents,
            ],
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn update(&self, item: &HistoryItem) -> klipa_core::Result<()> {
        let conn = self.conn.lock().unwrap();
        let contents = serde_json::to_string(&item.contents)
            .map_err(|e| CoreError::Storage(e.to_string()))?;
        conn.execute(
            "UPDATE history SET title=?1, application=?2, pin=?3, number_of_copies=?4, \
             last_copied_at=?5, contents_json=?6 WHERE id=?7",
            params![
                item.title,
                item.application,
                item.pin,
                item.number_of_copies as i64,
                item.last_copied_at.unix_timestamp(),
                contents,
                item.id.0.to_string(),
            ],
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn delete(&self, id: HistoryItemId) -> klipa_core::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM history WHERE id=?1", params![id.0.to_string()])
            .map_err(|e| CoreError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn clear_unpinned(&self) -> klipa_core::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM history WHERE pin IS NULL", [])
            .map_err(|e| CoreError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn clear_all(&self) -> klipa_core::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM history", [])
            .map_err(|e| CoreError::Storage(e.to_string()))?;
        Ok(())
    }
}
