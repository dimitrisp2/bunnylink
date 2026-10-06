// SPDX-FileCopyrightText: 2026 BunnyCloud.IT
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial
//! Remote file systems (SFTP, FTP, SMB) behind one interface, with shared recursive
//! transfers, collision-free downloads and progress reporting.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::Serialize;

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_link: bool,
    pub size: u64,
    pub modified: Option<u32>,
    pub permissions: Option<u32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub path: String,
    pub entries: Vec<Entry>,
}

pub type Report<'a> = &'a mut (dyn FnMut(u64) + Send);

/// Operations every backend provides. Paths are `/`-separated and absolute.
#[async_trait]
pub trait RemoteFs: Send + Sync {
    async fn home(&self) -> AppResult<String>;
    /// Lists a folder; the returned path is the canonical form of `path`.
    async fn list(&self, path: &str) -> AppResult<Listing>;
    /// (is folder, size)
    async fn stat(&self, path: &str) -> AppResult<(bool, u64)>;
    async fn exists(&self, path: &str) -> AppResult<bool>;
    async fn mkdir(&self, path: &str) -> AppResult<()>;
    async fn rename(&self, from: &str, to: &str) -> AppResult<()>;
    async fn remove_file(&self, path: &str) -> AppResult<()>;
    /// Removes an empty folder.
    async fn remove_dir(&self, path: &str) -> AppResult<()>;
    /// Copies a remote file to `local`, reporting bytes as they arrive.
    async fn get(&self, remote: &str, local: &Path, report: Report<'_>) -> AppResult<()>;
    /// Copies `local` to a remote file, replacing it.
    async fn put(&self, local: &Path, remote: &str, report: Report<'_>) -> AppResult<()>;
    /// A handle to use for a long transfer. Protocols with a single control channel
    /// (FTP) open a second connection so browsing stays responsive.
    async fn for_transfer(self: Arc<Self>) -> AppResult<Arc<dyn RemoteFs>>;
}

pub fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') { format!("{dir}{name}") } else { format!("{dir}/{name}") }
}

pub fn base_name(path: &str) -> String {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or(path).to_string()
}

pub fn sort_entries(entries: &mut [Entry]) {
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
}

/// Deletes files, and folders with everything inside them.
pub async fn remove(fs: &dyn RemoteFs, path: &str) -> AppResult<()> {
    let (is_dir, _) = fs.stat(path).await?;
    if is_dir {
        for e in fs.list(path).await?.entries {
            if e.is_dir && !e.is_link {
                Box::pin(remove(fs, &e.path)).await?;
            } else {
                fs.remove_file(&e.path).await?;
            }
        }
        fs.remove_dir(path).await
    } else {
        fs.remove_file(path).await
    }
}

pub async fn rename(fs: &dyn RemoteFs, from: &str, to: &str) -> AppResult<()> {
    if fs.exists(to).await? {
        return Err(AppError::Invalid(format!("{} already exists.", base_name(to))));
    }
    fs.rename(from, to).await
}

/// One file or folder in a transfer.
struct Step {
    remote: String,
    local: PathBuf,
    dir: bool,
    size: u64,
}

async fn plan_download(fs: &dyn RemoteFs, remote: &str, local: PathBuf, out: &mut Vec<Step>) -> AppResult<()> {
    let (is_dir, size) = fs.stat(remote).await?;
    if is_dir {
        out.push(Step { remote: remote.into(), local: local.clone(), dir: true, size: 0 });
        for e in fs.list(remote).await?.entries {
            Box::pin(plan_download(fs, &e.path, local.join(&e.name), out)).await?;
        }
    } else {
        out.push(Step { remote: remote.into(), local, dir: false, size });
    }
    Ok(())
}

fn plan_upload(local: &Path, remote: String, out: &mut Vec<Step>) -> AppResult<()> {
    let meta = std::fs::metadata(local)?;
    if meta.is_dir() {
        out.push(Step { remote: remote.clone(), local: local.into(), dir: true, size: 0 });
        let mut children: Vec<_> = std::fs::read_dir(local)?.collect::<Result<_, _>>()?;
        children.sort_by_key(|e| e.file_name());
        for c in children {
            let name = c.file_name().to_string_lossy().into_owned();
            plan_upload(&c.path(), join(&remote, &name), out)?;
        }
    } else {
        out.push(Step { remote, local: local.into(), dir: false, size: meta.len() });
    }
    Ok(())
}

pub async fn download(
    fs: &dyn RemoteFs,
    remotes: &[String],
    local_dir: &Path,
    mut progress: impl FnMut(Progress) + Send,
) -> AppResult<Vec<PathBuf>> {
    let mut steps = Vec::new();
    let mut roots = Vec::new();
    for r in remotes {
        let target = unique_local(&local_dir.join(base_name(r)));
        roots.push(target.clone());
        plan_download(fs, r, target, &mut steps).await?;
    }
    let mut meter = Meter::new(steps.iter().map(|s| s.size).sum());
    for s in &steps {
        if s.dir {
            tokio::fs::create_dir_all(&s.local).await?;
            continue;
        }
        let name = base_name(&s.remote);
        fs.get(&s.remote, &s.local, &mut |n| meter.add(n, &name, &mut progress)).await?;
    }
    meter.finish(&mut progress);
    Ok(roots)
}

pub async fn upload(
    fs: &dyn RemoteFs,
    locals: &[PathBuf],
    remote_dir: &str,
    mut progress: impl FnMut(Progress) + Send,
) -> AppResult<()> {
    let mut steps = Vec::new();
    for l in locals {
        let name = l.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        plan_upload(l, join(remote_dir, &name), &mut steps)?;
    }
    let mut meter = Meter::new(steps.iter().map(|s| s.size).sum());
    for s in &steps {
        if s.dir {
            if !fs.exists(&s.remote).await? {
                fs.mkdir(&s.remote).await?;
            }
            continue;
        }
        let name = base_name(&s.remote);
        fs.put(&s.local, &s.remote, &mut |n| meter.add(n, &name, &mut progress)).await?;
    }
    meter.finish(&mut progress);
    Ok(())
}

/// `report (1).pdf` style names so downloads never overwrite local files.
pub fn unique_local(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.into();
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (1..)
        .map(|i| path.with_file_name(format!("{stem} ({i}){ext}")))
        .find(|p| !p.exists())
        .unwrap()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub done: u64,
    pub total: u64,
    pub current: String,
    pub finished: bool,
}

struct Meter {
    done: u64,
    total: u64,
    last: Instant,
}

impl Meter {
    fn new(total: u64) -> Self {
        Self { done: 0, total, last: Instant::now() - Duration::from_secs(1) }
    }
    fn add(&mut self, n: u64, current: &str, report: &mut impl FnMut(Progress)) {
        self.done += n;
        if self.last.elapsed() >= Duration::from_millis(150) {
            self.last = Instant::now();
            report(Progress { done: self.done, total: self.total, current: current.into(), finished: false });
        }
    }
    fn finish(&mut self, report: &mut impl FnMut(Progress)) {
        report(Progress { done: self.done, total: self.total, current: String::new(), finished: true });
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    fn remote_paths() {
        assert_eq!(join("/", "etc"), "/etc");
        assert_eq!(join("/home/u", "a.txt"), "/home/u/a.txt");
        assert_eq!(base_name("/home/u/a.txt"), "a.txt");
        assert_eq!(base_name("/home/u/"), "u");
    }

    #[test]
    fn unique_names() {
        let dir = std::env::temp_dir().join(format!("bl-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.txt");
        assert_eq!(unique_local(&p), p);
        std::fs::write(&p, "x").unwrap();
        assert_eq!(unique_local(&p), dir.join("a (1).txt"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Shared live test for any backend: upload a folder, list, rename, download,
    /// compare, delete. `base` is a writable folder on the server.
    pub async fn exercise(fs: Arc<dyn RemoteFs>, base: &str) {
        let work = join(base, &format!("bl-test-{}", uuid::Uuid::new_v4()));
        fs.mkdir(&work).await.unwrap();

        let local = std::env::temp_dir().join(format!("bl-src-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(local.join("sub")).unwrap();
        let big: Vec<u8> = (0..700_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(local.join("sub/big.bin"), &big).unwrap();
        std::fs::write(local.join("hello.txt"), "hi").unwrap();
        let t = fs.clone().for_transfer().await.unwrap();
        let mut reports = 0;
        upload(&*t, &[local.clone()], &work, |_| reports += 1).await.unwrap();
        assert!(reports >= 1);

        let folder = join(&work, local.file_name().unwrap().to_str().unwrap());
        let listing = fs.list(&folder).await.unwrap();
        let names: Vec<_> = listing.entries.iter().map(|e| (e.name.as_str(), e.is_dir)).collect();
        assert_eq!(names, vec![("sub", true), ("hello.txt", false)]);
        let hello = listing.entries.iter().find(|e| e.name == "hello.txt").unwrap();
        assert_eq!(hello.size, 2);

        let renamed = join(&work, "renamed");
        rename(&*fs, &folder, &renamed).await.unwrap();
        let out = std::env::temp_dir().join(format!("bl-dst-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&out).unwrap();
        let saved = download(&*t, &[renamed.clone()], &out, |_| {}).await.unwrap();
        assert_eq!(saved, vec![out.join("renamed")]);
        assert_eq!(std::fs::read(out.join("renamed/sub/big.bin")).unwrap(), big);
        let saved = download(&*t, &[renamed], &out, |_| {}).await.unwrap();
        assert_eq!(saved, vec![out.join("renamed (1)")]);

        remove(&*fs, &work).await.unwrap();
        assert!(!fs.exists(&work).await.unwrap());
        std::fs::remove_dir_all(local).unwrap();
        std::fs::remove_dir_all(out).unwrap();
    }
}
