// SPDX-FileCopyrightText: 2026 BunnyCloud.IT
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial
//! SFTP backend.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::OpenFlags;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::error::{AppError, AppResult};
use crate::files::{join, sort_entries, Entry, Listing, RemoteFs, Report};
use crate::ssh::Conn;

impl From<russh_sftp::client::error::Error> for AppError {
    fn from(e: russh_sftp::client::error::Error) -> Self {
        AppError::Other(format!("SFTP: {e}"))
    }
}

pub struct Sftp {
    pub sftp: SftpSession,
    _conn: Conn,
}

impl Sftp {
    pub async fn open(conn: Conn) -> AppResult<Self> {
        let mut channel = conn.handle.channel_open_session().await?;
        channel.request_subsystem(true, "sftp").await?;
        loop {
            match channel.wait().await {
                Some(russh::ChannelMsg::Success) => break,
                Some(russh::ChannelMsg::Failure) | Some(russh::ChannelMsg::Close) | None => {
                    return Err(AppError::Other("This server does not offer SFTP.".into()))
                }
                _ => {}
            }
        }
        let sftp = SftpSession::new(channel.into_stream()).await?;
        sftp.set_timeout(30);
        Ok(Self { sftp, _conn: conn })
    }
}

#[async_trait]
impl RemoteFs for Sftp {
    async fn home(&self) -> AppResult<String> {
        Ok(self.sftp.canonicalize(".").await?)
    }

    async fn list(&self, path: &str) -> AppResult<Listing> {
        let path = self.sftp.canonicalize(path).await?;
        let mut entries = Vec::new();
        for e in self.sftp.read_dir(&path).await? {
            let name = e.file_name();
            if name == "." || name == ".." {
                continue;
            }
            let meta = e.metadata();
            let full = join(&path, &name);
            let is_link = e.file_type().is_symlink();
            // Follow symlinks so links to folders can be opened.
            let is_dir = if is_link {
                self.sftp.metadata(&full).await.map(|m| m.is_dir()).unwrap_or(false)
            } else {
                e.file_type().is_dir()
            };
            entries.push(Entry {
                name,
                path: full,
                is_dir,
                is_link,
                size: meta.size.unwrap_or(0),
                modified: meta.mtime,
                permissions: meta.permissions,
            });
        }
        sort_entries(&mut entries);
        Ok(Listing { path, entries })
    }

    async fn stat(&self, path: &str) -> AppResult<(bool, u64)> {
        let m = self.sftp.symlink_metadata(path).await?;
        Ok((m.is_dir(), m.size.unwrap_or(0)))
    }

    async fn exists(&self, path: &str) -> AppResult<bool> {
        Ok(self.sftp.try_exists(path).await?)
    }

    async fn mkdir(&self, path: &str) -> AppResult<()> {
        Ok(self.sftp.create_dir(path).await?)
    }

    async fn rename(&self, from: &str, to: &str) -> AppResult<()> {
        Ok(self.sftp.rename(from, to).await?)
    }

    async fn remove_file(&self, path: &str) -> AppResult<()> {
        Ok(self.sftp.remove_file(path).await?)
    }

    async fn remove_dir(&self, path: &str) -> AppResult<()> {
        Ok(self.sftp.remove_dir(path).await?)
    }

    async fn get(&self, remote: &str, local: &Path, report: Report<'_>) -> AppResult<()> {
        let mut src = self.sftp.open(remote).await?;
        let mut dst = tokio::fs::File::create(local).await?;
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            let n = src.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            dst.write_all(&buf[..n]).await?;
            report(n as u64);
        }
        dst.flush().await?;
        Ok(())
    }

    async fn put(&self, local: &Path, remote: &str, report: Report<'_>) -> AppResult<()> {
        let mut src = tokio::fs::File::open(local).await?;
        let mut dst = self
            .sftp
            .open_with_flags(remote, OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE)
            .await?;
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            let n = src.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            dst.write_all(&buf[..n]).await?;
            report(n as u64);
        }
        dst.shutdown().await?;
        Ok(())
    }

    async fn for_transfer(self: Arc<Self>) -> AppResult<Arc<dyn RemoteFs>> {
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live test, skipped unless `BUNNYLINK_TEST_SSH=host:port:user:password` is set.
    #[tokio::test]
    async fn live_browse_and_transfer() {
        let Ok(spec) = std::env::var("BUNNYLINK_TEST_SSH") else { return };
        let p: Vec<&str> = spec.splitn(4, ':').collect();
        let target = crate::ssh::Target {
            label: "test".into(),
            address: p[0].into(),
            port: p[1].parse().unwrap(),
            username: p[2].into(),
            auth: crate::ssh::Auth::Password(p[3].into()),
            jump: None,
        };
        let store = Arc::new(crate::store::Store::in_memory().unwrap());
        let conn = crate::ssh::connect(&target, store, &mut Vec::new()).await.unwrap();
        let fs = Arc::new(Sftp::open(conn).await.unwrap());
        let home = fs.home().await.unwrap();
        crate::files::tests::exercise(fs, &home).await;
    }
}
