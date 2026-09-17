//! SQLite-backed `.worldos` project files.
//!
//! A project is one SQLite database (WAL mode, full-snapshot writes inside
//! a single SQLite transaction → atomic saves, crash-safe by construction).
//! Schema version is tracked via `PRAGMA user_version`; migrations live in
//! [`migrate`].

use crate::error::StoreError;
use crate::snapshot::{Snapshot, FORMAT_VERSION};
use crate::store::ProjectStore;
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use worldos_commands::History;
use worldos_kernel::project::{Project, PROJECT_SCHEMA_VERSION};

const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS objects (
    id         TEXT PRIMARY KEY,
    type_id    TEXT NOT NULL,
    name       TEXT NOT NULL,
    tags       TEXT NOT NULL,
    components TEXT NOT NULL,
    meta       TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS relations (
    id         TEXT PRIMARY KEY,
    type_id    TEXT NOT NULL,
    from_id    TEXT NOT NULL,
    to_id      TEXT NOT NULL,
    properties TEXT NOT NULL,
    meta       TEXT
);
CREATE TABLE IF NOT EXISTS transactions (
    idx          INTEGER PRIMARY KEY,
    id           TEXT NOT NULL,
    actor        TEXT NOT NULL,
    label        TEXT NOT NULL,
    started_at   INTEGER NOT NULL,
    committed_at INTEGER NOT NULL,
    undone       INTEGER NOT NULL DEFAULT 0,
    commands     TEXT NOT NULL,
    ops          TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_objects_type ON objects(type_id);
CREATE INDEX IF NOT EXISTS idx_relations_from ON relations(from_id);
CREATE INDEX IF NOT EXISTS idx_relations_to ON relations(to_id);
"#;

/// Ordered schema migrations; index = target user_version - 1.
const MIGRATIONS: &[&str] = &[SCHEMA_V1];

pub struct SqliteStore {
    path: PathBuf,
    conn: Connection,
}

impl SqliteStore {
    /// Open (or initialize) a `.worldos` file.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        let conn = Connection::open(&path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let mut store = Self { path, conn };
        store.migrate()?;
        Ok(store)
    }

    /// In-memory store (tests).
    pub fn in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory()?;
        let mut store = Self { path: PathBuf::from(":memory:"), conn };
        store.migrate()?;
        Ok(store)
    }

    fn user_version(&self) -> Result<u32, StoreError> {
        Ok(self.conn.pragma_query_value(None, "user_version", |r| r.get(0))?)
    }

    fn migrate(&mut self) -> Result<(), StoreError> {
        let mut v = self.user_version()?;
        if v as usize > MIGRATIONS.len() {
            return Err(StoreError::UnsupportedVersion {
                found: v,
                supported: MIGRATIONS.len() as u32,
            });
        }
        while (v as usize) < MIGRATIONS.len() {
            self.conn.execute_batch(MIGRATIONS[v as usize])?;
            v += 1;
            self.conn.pragma_update(None, "user_version", v)?;
        }
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl ProjectStore for SqliteStore {
    fn exists(&self) -> bool {
        self.conn
            .query_row(
                "SELECT value FROM meta WHERE key='project_id'",
                [],
                |r| r.get::<_, String>(0),
            )
            .is_ok()
    }

    fn load(&self) -> Result<Snapshot, StoreError> {
        if !self.exists() {
            return Err(StoreError::NotFound(self.path.display().to_string()));
        }
        let meta = |k: &str| -> Result<String, StoreError> {
            Ok(self.conn.query_row(
                "SELECT value FROM meta WHERE key=?1",
                params![k],
                |r| r.get(0),
            )?)
        };
        let mut project = Project {
            id: meta("project_id")?.parse().map_err(|_| StoreError::Corrupt("project_id".into()))?,
            name: meta("name")?,
            schema_version: meta("schema_version")?
                .parse()
                .map_err(|_| StoreError::Corrupt("schema_version".into()))?,
            created_at: meta("created_at")?.parse().unwrap_or(0),
            objects: Default::default(),
            relations: Default::default(),
            settings: serde_json::from_str(&meta("settings").unwrap_or_else(|_| "{}".into()))?,
        };

        let mut stmt = self.conn.prepare(
            "SELECT id, type_id, name, tags, components, meta FROM objects",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?;
        for row in rows {
            let (id, type_id, name, tags, components, meta) = row?;
            let obj = worldos_kernel::Object {
                id: id.parse().map_err(|_| StoreError::Corrupt("object id".into()))?,
                type_id: worldos_kernel::TypeId::new(type_id),
                name,
                tags: serde_json::from_str(&tags)?,
                components: serde_json::from_str(&components)?,
                meta: serde_json::from_str(&meta)?,
            };
            project.objects.insert(obj.id, obj);
        }

        let mut stmt = self.conn.prepare(
            "SELECT id, type_id, from_id, to_id, properties, meta FROM relations",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Option<String>>(5)?,
            ))
        })?;
        for row in rows {
            let (id, type_id, from, to, properties, meta) = row?;
            let rel = worldos_kernel::Relation {
                id: id.parse().map_err(|_| StoreError::Corrupt("relation id".into()))?,
                type_id,
                from: from.parse().map_err(|_| StoreError::Corrupt("from id".into()))?,
                to: to.parse().map_err(|_| StoreError::Corrupt("to id".into()))?,
                properties: serde_json::from_str(&properties)?,
                meta: meta.and_then(|m| serde_json::from_str(&m).ok()).flatten(),
            };
            project.relations.insert(rel.id, rel);
        }

        let mut history = History::new();
        let mut stmt = self.conn.prepare(
            "SELECT id, actor, label, started_at, committed_at, undone, commands, ops
             FROM transactions ORDER BY idx",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
            ))
        })?;
        for row in rows {
            let (id, actor, label, started, committed, undone, commands, ops) = row?;
            history.records.push(worldos_commands::TransactionRecord {
                id: id.parse().map_err(|_| StoreError::Corrupt("txn id".into()))?,
                index: history.records.len() as u64,
                actor: worldos_kernel::ActorId::new(actor),
                label,
                started_at: started,
                committed_at: committed,
                commands: serde_json::from_str(&commands)?,
                ops: serde_json::from_str(&ops)?,
                undone: undone != 0,
            });
        }
        // Linear undo: undone records form a contiguous suffix.
        let undone_tail = history.records.iter().rev().take_while(|r| r.undone).count();
        history.cursor = history.records.len() - undone_tail;

        let mut snap = Snapshot::new(project, history);
        snap.format_version = meta("format_version")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(FORMAT_VERSION);
        Ok(snap)
    }

    fn save(&mut self, snapshot: &Snapshot) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        let p = &snapshot.project;
        tx.execute_batch("DELETE FROM objects; DELETE FROM relations; DELETE FROM transactions; DELETE FROM meta;")?;
        {
            let mut m = tx.prepare("INSERT INTO meta(key, value) VALUES(?1, ?2)")?;
            for (k, v) in [
                ("project_id", p.id.to_string()),
                ("name", p.name.clone()),
                ("schema_version", PROJECT_SCHEMA_VERSION.to_string()),
                ("format_version", FORMAT_VERSION.to_string()),
                ("created_at", p.created_at.to_string()),
                ("settings", serde_json::to_string(&p.settings)?),
            ] {
                m.execute(params![k, v])?;
            }
        }
        {
            let mut s = tx.prepare(
                "INSERT INTO objects(id, type_id, name, tags, components, meta)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for o in p.objects.values() {
                s.execute(params![
                    o.id.to_string(),
                    o.type_id.0,
                    o.name,
                    serde_json::to_string(&o.tags)?,
                    serde_json::to_string(&o.components)?,
                    serde_json::to_string(&o.meta)?,
                ])?;
            }
        }
        {
            let mut s = tx.prepare(
                "INSERT INTO relations(id, type_id, from_id, to_id, properties, meta)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for r in p.relations.values() {
                s.execute(params![
                    r.id.to_string(),
                    r.type_id,
                    r.from.to_string(),
                    r.to.to_string(),
                    serde_json::to_string(&r.properties)?,
                    r.meta.as_ref().map(serde_json::to_string).transpose()?,
                ])?;
            }
        }
        {
            let mut s = tx.prepare(
                "INSERT INTO transactions(idx, id, actor, label, started_at, committed_at, undone, commands, ops)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;
            for (i, rec) in snapshot.history.records.iter().enumerate() {
                s.execute(params![
                    i as i64,
                    rec.id.to_string(),
                    rec.actor.0,
                    rec.label,
                    rec.started_at,
                    rec.committed_at,
                    if rec.undone { 1 } else { 0 },
                    serde_json::to_string(&rec.commands)?,
                    serde_json::to_string(&rec.ops)?,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}
