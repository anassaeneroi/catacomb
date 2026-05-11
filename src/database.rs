use rusqlite::{Connection, Result};
use std::collections::{HashMap, HashSet};
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

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Database { conn };
        db.init_schema()?;
        Ok(db)
    }

    fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS watched (
                video_id TEXT PRIMARY KEY,
                watched_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );
            CREATE TABLE IF NOT EXISTS positions (
                video_id TEXT PRIMARY KEY,
                position_secs REAL NOT NULL,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );",
        )?;
        Ok(())
    }

    pub fn set_watched(&self, video_id: &str, watched: bool) -> Result<()> {
        if watched {
            self.conn.execute(
                "INSERT OR REPLACE INTO watched (video_id) VALUES (?1)",
                [video_id],
            )?;
        } else {
            self.conn.execute("DELETE FROM watched WHERE video_id = ?1", [video_id])?;
        }
        Ok(())
    }

    pub fn get_watched(&self) -> Result<HashSet<String>> {
        let mut stmt = self.conn.prepare("SELECT video_id FROM watched")?;
        let ids = stmt
            .query_map([], |row| row.get(0))?
            .filter_map(Result::ok)
            .collect();
        Ok(ids)
    }

    pub fn set_position(&self, video_id: &str, position_secs: f64) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO positions (video_id, position_secs, updated_at)
             VALUES (?1, ?2, CURRENT_TIMESTAMP)",
            rusqlite::params![video_id, position_secs],
        )?;
        Ok(())
    }

    pub fn clear_position(&self, video_id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM positions WHERE video_id = ?1", [video_id])?;
        Ok(())
    }

    pub fn get_positions(&self) -> Result<HashMap<String, f64>> {
        let mut stmt = self.conn.prepare("SELECT video_id, position_secs FROM positions")?;
        let map = stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?)))?
            .filter_map(Result::ok)
            .collect();
        Ok(map)
    }
}
