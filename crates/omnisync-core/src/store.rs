use crate::model::*;
use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

pub struct Store {
    db: Connection,
}
impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let db = Connection::open(path)?;
        db.pragma_update(None, "journal_mode", "WAL")?;
        db.pragma_update(None, "synchronous", "FULL")?;
        let version: u32 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > 1 {
            bail!("This database belongs to a newer OmniSync version");
        }
        db.execute_batch("CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);CREATE TABLE IF NOT EXISTS configs(revision INTEGER PRIMARY KEY,json TEXT NOT NULL);CREATE TABLE IF NOT EXISTS items(id TEXT PRIMARY KEY,root_id TEXT NOT NULL,path_key TEXT NOT NULL,json TEXT NOT NULL,UNIQUE(root_id,path_key));CREATE TABLE IF NOT EXISTS jobs(item_id TEXT PRIMARY KEY,json TEXT NOT NULL);CREATE TABLE IF NOT EXISTS remote_paths(key TEXT PRIMARY KEY,remote_id TEXT NOT NULL);PRAGMA user_version=1;")?;
        let mut store = Self { db };
        if store.setting::<Config>("config")?.is_none() {
            store.save_config(&Config::default(), 0)?;
        }
        Ok(store)
    }
    pub fn setting<T: serde::de::DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let text: Option<String> = self
            .db
            .query_row("SELECT value FROM settings WHERE key=?", [key], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(text.map(|s| serde_json::from_str(&s)).transpose()?)
    }
    pub fn set<T: serde::Serialize>(&self, key: &str, value: &T) -> Result<()> {
        self.db.execute(
            "INSERT INTO settings VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, serde_json::to_string(value)?],
        )?;
        Ok(())
    }
    pub fn config(&self) -> Result<Config> {
        Ok(self.setting("config")?.expect("config initialized"))
    }
    pub fn save_config(&mut self, config: &Config, expected: u64) -> Result<()> {
        let current = self
            .setting::<Config>("config")?
            .map(|c| c.revision)
            .unwrap_or(0);
        if current != expected {
            bail!("Configuration changed. Preview again before saving.");
        }
        let text = serde_json::to_string(config)?;
        let tx = self.db.transaction()?;
        tx.execute(
            "INSERT INTO configs VALUES(?,?)",
            params![i64::try_from(config.revision)?, text],
        )?;
        tx.execute("INSERT INTO settings VALUES('config',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[&text])?;
        tx.commit()?;
        Ok(())
    }
    pub fn previous_config(&self, current: u64) -> Result<Option<Config>> {
        let json: Option<String> = self
            .db
            .query_row(
                "SELECT json FROM configs WHERE revision<? ORDER BY revision DESC LIMIT 1",
                [i64::try_from(current)?],
                |r| r.get(0),
            )
            .optional()?;
        Ok(json.map(|s| serde_json::from_str(&s)).transpose()?)
    }
    pub fn items(&self) -> Result<Vec<Item>> {
        let mut q = self
            .db
            .prepare("SELECT json FROM items ORDER BY root_id,path_key")?;
        let values = q
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        values
            .into_iter()
            .map(|s| Ok(serde_json::from_str(&s)?))
            .collect()
    }
    pub fn item(&self, id: &str) -> Result<Option<Item>> {
        let s: Option<String> = self
            .db
            .query_row("SELECT json FROM items WHERE id=?", [id], |r| r.get(0))
            .optional()?;
        Ok(s.map(|s| serde_json::from_str(&s)).transpose()?)
    }
    pub fn find(&self, root: &str, relative: &str) -> Result<Option<Item>> {
        let s: Option<String> = self
            .db
            .query_row(
                "SELECT json FROM items WHERE root_id=? AND path_key=?",
                params![root, relative.to_lowercase()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(s.map(|s| serde_json::from_str(&s)).transpose()?)
    }
    pub fn put_item(&self, item: &Item) -> Result<()> {
        self.db.execute("INSERT INTO items VALUES(?,?,?,?) ON CONFLICT(id) DO UPDATE SET json=excluded.json,path_key=excluded.path_key",params![item.id,item.root_id,item.relative_path.to_lowercase(),serde_json::to_string(item)?])?;
        Ok(())
    }
    pub fn jobs(&self) -> Result<Vec<Job>> {
        let mut q = self.db.prepare("SELECT json FROM jobs")?;
        let values = q
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        values
            .into_iter()
            .map(|s| Ok(serde_json::from_str(&s)?))
            .collect()
    }
    pub fn job(&self, item: &str) -> Result<Option<Job>> {
        let s: Option<String> = self
            .db
            .query_row("SELECT json FROM jobs WHERE item_id=?", [item], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(s.map(|s| serde_json::from_str(&s)).transpose()?)
    }
    pub fn put_job(&self, job: &Job) -> Result<()> {
        self.db.execute(
            "INSERT INTO jobs VALUES(?,?) ON CONFLICT(item_id) DO UPDATE SET json=excluded.json",
            params![job.item_id, serde_json::to_string(job)?],
        )?;
        Ok(())
    }
    pub fn delete_job(&self, item: &str) -> Result<()> {
        self.db
            .execute("DELETE FROM jobs WHERE item_id=?", [item])?;
        Ok(())
    }
    pub fn remote_path(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .db
            .query_row(
                "SELECT remote_id FROM remote_paths WHERE key=?",
                [key],
                |r| r.get(0),
            )
            .optional()?)
    }
    pub fn set_remote_path(&self, key: &str, id: &str) -> Result<()> {
        self.db.execute("INSERT INTO remote_paths VALUES(?,?) ON CONFLICT(key) DO UPDATE SET remote_id=excluded.remote_id",params![key,id])?;
        Ok(())
    }
    pub fn commit_verified(&mut self, item: &Item) -> Result<()> {
        let tx = self.db.transaction()?;
        tx.execute(
            "UPDATE items SET json=? WHERE id=?",
            params![serde_json::to_string(item)?, item.id],
        )?;
        tx.execute("DELETE FROM jobs WHERE item_id=?", [&item.id])?;
        tx.execute("INSERT INTO settings VALUES('catalogDirty','true') ON CONFLICT(key) DO UPDATE SET value='true'",[])?;
        tx.commit()?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_configuration_and_restart() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("state.db");
        {
            let mut s = Store::open(&p).unwrap();
            let mut c = s.config().unwrap();
            c.revision += 1;
            s.save_config(&c, 1).unwrap();
            assert!(s.save_config(&c, 1).is_err());
        }
        let s = Store::open(&p).unwrap();
        assert_eq!(s.config().unwrap().revision, 2);
        assert_eq!(s.previous_config(2).unwrap().unwrap().revision, 1);
    }
}
