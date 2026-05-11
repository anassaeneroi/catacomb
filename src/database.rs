use rusqlite::{Connection, Result};
use std::path::Path;

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        let db = Database { conn };
        db.init_schema()?;
        Ok(db)
    }

    fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS videos (
                id TEXT PRIMARY KEY,
                channel TEXT NOT NULL,
                title TEXT NOT NULL,
                downloaded_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );
            CREATE TABLE IF NOT EXISTS downloads (
                id INTEGER PRIMARY KEY,
                url TEXT NOT NULL,
                directory TEXT NOT NULL,
                status TEXT NOT NULL,
                started_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                completed_at DATETIME
            );",
        )?;
        Ok(())
    }

    pub fn record_download(&self, url: &str, dir: &str, status: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO downloads (url, directory, status) VALUES (?1, ?2, ?3)",
            [url, dir, status],
        )?;
        Ok(())
    }

    pub fn update_download_status(&self, id: i64, status: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE downloads SET status = ?1, completed_at = CURRENT_TIMESTAMP WHERE id = ?2",
            [status, &id.to_string()],
        )?;
        Ok(())
    }
}
