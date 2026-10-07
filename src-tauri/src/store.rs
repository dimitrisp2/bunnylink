// SPDX-FileCopyrightText: 2026 BunnyCloud.IT
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial
//! SQLite-backed storage. Records are stored as JSON documents so the schema can grow
//! without migrations; secrets are stored as encrypted blobs next to their credential.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Serialize};

use crate::error::{AppError, AppResult};
use crate::model::*;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS folders     (id TEXT PRIMARY KEY, doc TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS hosts       (id TEXT PRIMARY KEY, doc TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS credentials (id TEXT PRIMARY KEY, doc TEXT NOT NULL, secret BLOB);
CREATE TABLE IF NOT EXISTS tunnels     (id TEXT PRIMARY KEY, doc TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS snippets    (id TEXT PRIMARY KEY, doc TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS known_hosts (host TEXT NOT NULL, port INTEGER NOT NULL, fingerprint TEXT NOT NULL,
                                        PRIMARY KEY (host, port));
CREATE TABLE IF NOT EXISTS meta        (key TEXT PRIMARY KEY, value BLOB NOT NULL);
";

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &Path) -> AppResult<Self> {
        Ok(Self { conn: Mutex::new(Self::open_file(path)?) })
    }

    fn open_file(path: &Path) -> AppResult<Connection> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        conn.execute_batch(SCHEMA)?;
        Ok(conn)
    }

    /// A store that lives only in memory until `persist` gives it a file (first run).
    pub fn in_memory() -> AppResult<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    /// Writes the current contents to a new database file at `path` and continues there.
    pub fn persist(&self, path: &Path) -> AppResult<()> {
        let mut conn = self.conn();
        conn.execute("VACUUM INTO ?1", [path.to_string_lossy()])?;
        *conn = Self::open_file(path)?;
        Ok(())
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn list<T: DeserializeOwned>(&self, table: &str) -> AppResult<Vec<T>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!("SELECT doc FROM {table}"))?;
        let docs = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        docs.iter().map(|d| Ok(serde_json::from_str(d)?)).collect()
    }

    fn get<T: DeserializeOwned>(&self, table: &str, id: &str) -> AppResult<Option<T>> {
        let doc: Option<String> = self
            .conn()
            .query_row(&format!("SELECT doc FROM {table} WHERE id = ?1"), [id], |r| r.get(0))
            .optional()?;
        Ok(match doc {
            Some(d) => Some(serde_json::from_str(&d)?),
            None => None,
        })
    }

    fn put<T: Serialize>(&self, table: &str, id: &str, value: &T) -> AppResult<()> {
        let doc = serde_json::to_string(value)?;
        self.conn().execute(
            &format!("INSERT INTO {table} (id, doc) VALUES (?1, ?2) ON CONFLICT(id) DO UPDATE SET doc = excluded.doc"),
            params![id, doc],
        )?;
        Ok(())
    }

    fn delete(&self, table: &str, id: &str) -> AppResult<()> {
        self.conn().execute(&format!("DELETE FROM {table} WHERE id = ?1"), [id])?;
        Ok(())
    }

    // ---- folders ----
    pub fn folders(&self) -> AppResult<Vec<Folder>> { self.list("folders") }
    pub fn save_folder(&self, f: &Folder) -> AppResult<()> {
        if let Some(parent) = &f.parent_id {
            let mut cur = Some(parent.clone());
            while let Some(id) = cur {
                if id == f.id {
                    return Err(AppError::Invalid("A folder cannot be moved inside itself.".into()));
                }
                cur = self.get::<Folder>("folders", &id)?.and_then(|p| p.parent_id);
            }
        }
        self.put("folders", &f.id, f)
    }
    /// Deletes a folder; its hosts and subfolders move up to the deleted folder's parent.
    pub fn delete_folder(&self, id: &str) -> AppResult<()> {
        let folder: Folder = self.get("folders", id)?.ok_or(AppError::NotFound("Folder"))?;
        for mut sub in self.folders()?.into_iter().filter(|f| f.parent_id.as_deref() == Some(id)) {
            sub.parent_id = folder.parent_id.clone();
            self.put("folders", &sub.id, &sub)?;
        }
        for mut h in self.hosts()?.into_iter().filter(|h| h.folder_id.as_deref() == Some(id)) {
            h.folder_id = folder.parent_id.clone();
            self.put("hosts", &h.id, &h)?;
        }
        // Snippets follow the hosts to the parent folder. A top-level folder has none; the
        // snippet keeps the old id and matches no host until edited, rather than becoming global.
        if let Some(parent) = &folder.parent_id {
            for mut s in self.snippets()?.into_iter().filter(|s| s.folder_ids.iter().any(|f| f == id)) {
                s.folder_ids.retain(|f| f != id);
                if !s.folder_ids.contains(parent) {
                    s.folder_ids.push(parent.clone());
                }
                self.put("snippets", &s.id, &s)?;
            }
        }
        self.delete("folders", id)
    }

    // ---- hosts ----
    pub fn hosts(&self) -> AppResult<Vec<Host>> { self.list("hosts") }
    pub fn host(&self, id: &str) -> AppResult<Host> {
        self.get("hosts", id)?.ok_or(AppError::NotFound("Host"))
    }
    pub fn save_host(&self, h: &Host) -> AppResult<()> { self.put("hosts", &h.id, h) }
    pub fn delete_host(&self, id: &str) -> AppResult<()> { self.delete("hosts", id) }
    pub fn touch_host(&self, id: &str, now: i64) -> AppResult<()> {
        let mut h = self.host(id)?;
        h.last_used = Some(now);
        self.save_host(&h)
    }

    /// Applies folder inheritance: the host's own value wins, then the nearest folder's.
    pub fn effective(&self, host: &Host) -> AppResult<Effective> {
        let folders = self.folders()?;
        let mut eff = Effective {
            username: host.overrides.username.clone(),
            credential_id: host.overrides.credential_id.clone(),
            jump_host_id: host.overrides.jump_host_id.clone(),
            ..Default::default()
        };
        let mut cur = host.folder_id.clone();
        let mut guard = 0;
        while let Some(fid) = cur {
            guard += 1;
            let Some(f) = folders.iter().find(|f| f.id == fid) else { break };
            if guard > 64 { break; }
            if eff.username.is_none() && f.defaults.username.is_some() {
                eff.username = f.defaults.username.clone();
                eff.username_from = Some(f.name.clone());
            }
            if eff.credential_id.is_none() && f.defaults.credential_id.is_some() {
                eff.credential_id = f.defaults.credential_id.clone();
                eff.credential_from = Some(f.name.clone());
            }
            if eff.jump_host_id.is_none() && f.defaults.jump_host_id.is_some() {
                eff.jump_host_id = f.defaults.jump_host_id.clone();
                eff.jump_host_from = Some(f.name.clone());
            }
            cur = f.parent_id.clone();
        }
        if eff.jump_host_id.as_deref() == Some(host.id.as_str()) {
            eff.jump_host_id = None;
        }
        Ok(eff)
    }

    // ---- credentials ----
    pub fn credentials(&self) -> AppResult<Vec<Credential>> { self.list("credentials") }
    pub fn credential(&self, id: &str) -> AppResult<Credential> {
        self.get("credentials", id)?.ok_or(AppError::NotFound("Credential"))
    }
    pub fn save_credential(&self, c: &Credential, secret: Option<Vec<u8>>) -> AppResult<()> {
        self.put("credentials", &c.id, c)?;
        if let Some(blob) = secret {
            self.conn().execute("UPDATE credentials SET secret = ?2 WHERE id = ?1", params![c.id, blob])?;
        }
        Ok(())
    }
    pub fn credential_secret(&self, id: &str) -> AppResult<Option<Vec<u8>>> {
        Ok(self
            .conn()
            .query_row("SELECT secret FROM credentials WHERE id = ?1", [id], |r| r.get(0))
            .optional()?
            .flatten())
    }
    pub fn delete_credential(&self, id: &str) -> AppResult<()> { self.delete("credentials", id) }

    // ---- tunnels ----
    pub fn tunnels(&self) -> AppResult<Vec<Tunnel>> { self.list("tunnels") }
    pub fn tunnel(&self, id: &str) -> AppResult<Tunnel> {
        self.get("tunnels", id)?.ok_or(AppError::NotFound("Tunnel"))
    }
    pub fn save_tunnel(&self, t: &Tunnel) -> AppResult<()> { self.put("tunnels", &t.id, t) }
    pub fn delete_tunnel(&self, id: &str) -> AppResult<()> { self.delete("tunnels", id) }

    // ---- snippets ----
    pub fn snippets(&self) -> AppResult<Vec<Snippet>> { self.list("snippets") }
    pub fn save_snippet(&self, s: &Snippet) -> AppResult<()> { self.put("snippets", &s.id, s) }
    pub fn delete_snippet(&self, id: &str) -> AppResult<()> { self.delete("snippets", id) }

    // ---- known hosts ----
    pub fn known_host(&self, host: &str, port: u16) -> AppResult<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT fingerprint FROM known_hosts WHERE host = ?1 AND port = ?2",
                params![host, port],
                |r| r.get(0),
            )
            .optional()?)
    }
    pub fn save_known_host(&self, host: &str, port: u16, fingerprint: &str) -> AppResult<()> {
        self.conn().execute(
            "INSERT INTO known_hosts (host, port, fingerprint) VALUES (?1, ?2, ?3)
             ON CONFLICT(host, port) DO UPDATE SET fingerprint = excluded.fingerprint",
            params![host, port, fingerprint],
        )?;
        Ok(())
    }
    pub fn forget_known_host(&self, host: &str, port: u16) -> AppResult<()> {
        self.conn().execute("DELETE FROM known_hosts WHERE host = ?1 AND port = ?2", params![host, port])?;
        Ok(())
    }

    // ---- meta ----
    pub fn meta(&self, key: &str) -> AppResult<Option<Vec<u8>>> {
        Ok(self
            .conn()
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }
    pub fn set_meta(&self, key: &str, value: &[u8]) -> AppResult<()> {
        self.conn().execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn settings(&self) -> AppResult<Settings> {
        Ok(match self.meta("settings")? {
            Some(v) => serde_json::from_slice(&v).unwrap_or_default(),
            None => Settings::default(),
        })
    }
    pub fn save_settings(&self, s: &Settings) -> AppResult<()> {
        self.set_meta("settings", &serde_json::to_vec(s)?)
    }

    pub fn library(&self) -> AppResult<Library> {
        Ok(Library {
            folders: self.folders()?,
            hosts: self.hosts()?,
            credentials: self.credentials()?,
            tunnels: self.tunnels()?,
            snippets: self.snippets()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(id: &str, parent: Option<&str>, user: Option<&str>) -> Folder {
        Folder {
            id: id.into(),
            parent_id: parent.map(Into::into),
            name: id.to_uppercase(),
            defaults: Defaults { username: user.map(Into::into), ..Default::default() },
        }
    }

    fn host(folder: Option<&str>) -> Host {
        Host {
            id: "h1".into(),
            folder_id: folder.map(Into::into),
            name: "web01".into(),
            address: "10.0.0.1".into(),
            endpoints: vec![],
            tags: vec![],
            overrides: Defaults::default(),
            notes: String::new(),
            pinned: false,
            last_used: None,
        }
    }

    #[test]
    fn inherits_from_nearest_folder() {
        let s = Store::in_memory().unwrap();
        s.save_folder(&folder("prod", None, Some("root"))).unwrap();
        s.save_folder(&folder("web", Some("prod"), None)).unwrap();
        let mut h = host(Some("web"));
        let eff = s.effective(&h).unwrap();
        assert_eq!(eff.username.as_deref(), Some("root"));
        assert_eq!(eff.username_from.as_deref(), Some("PROD"));

        h.overrides.username = Some("deploy".into());
        let eff = s.effective(&h).unwrap();
        assert_eq!(eff.username.as_deref(), Some("deploy"));
        assert_eq!(eff.username_from, None);
    }

    #[test]
    fn rejects_folder_cycles() {
        let s = Store::in_memory().unwrap();
        s.save_folder(&folder("a", None, None)).unwrap();
        s.save_folder(&folder("b", Some("a"), None)).unwrap();
        assert!(s.save_folder(&folder("a", Some("b"), None)).is_err());
    }

    #[test]
    fn deleting_folder_moves_snippets_up() {
        let s = Store::in_memory().unwrap();
        s.save_folder(&folder("a", None, None)).unwrap();
        s.save_folder(&folder("b", Some("a"), None)).unwrap();
        let snippet = |id: &str, folders: &[&str]| Snippet {
            id: id.into(),
            name: id.into(),
            command: "uptime".into(),
            description: String::new(),
            send_enter: true,
            folder_ids: folders.iter().map(|f| f.to_string()).collect(),
            tags: vec![],
        };
        s.save_snippet(&snippet("in-b", &["b"])).unwrap();
        s.save_snippet(&snippet("in-a-and-b", &["a", "b"])).unwrap();
        s.save_snippet(&snippet("in-a", &["a"])).unwrap();
        s.delete_folder("b").unwrap();
        s.delete_folder("a").unwrap();
        let by_id = |id: &str| s.snippets().unwrap().into_iter().find(|x| x.id == id).unwrap().folder_ids;
        // "b" moved up into "a" without a duplicate; deleting top-level "a" keeps the id.
        assert_eq!(by_id("in-b"), ["a"]);
        assert_eq!(by_id("in-a-and-b"), ["a"]);
        assert_eq!(by_id("in-a"), ["a"]);
    }

    #[test]
    fn deleting_folder_moves_children_up() {
        let s = Store::in_memory().unwrap();
        s.save_folder(&folder("a", None, None)).unwrap();
        s.save_folder(&folder("b", Some("a"), None)).unwrap();
        s.save_host(&host(Some("b"))).unwrap();
        s.delete_folder("b").unwrap();
        assert_eq!(s.host("h1").unwrap().folder_id.as_deref(), Some("a"));
    }
}
