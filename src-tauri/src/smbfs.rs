// SPDX-FileCopyrightText: 2026 BunnyCloud.IT
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial
//! SMB (Windows file sharing) backend. With a share configured on the host, `/` is the
//! share's root; without one, `/` lists the server's shares.

use std::path::Path;
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use async_trait::async_trait;
use futures_util::StreamExt;
use smb::{
    Client, ClientConfig, CreateOptions, DirAccessMask, Directory, FileAccessMask, FileAttributes, FileCreateArgs,
    FileDispositionInformation, FileFullDirectoryInformation, FileRenameInformation, FileStandardInformation,
    ReadAt, UncPath, WriteAt,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::error::{AppError, AppResult};
use crate::files::{join, sort_entries, Entry, Listing, RemoteFs, Report};
use crate::ssh;
use crate::store::Store;

#[derive(Clone)]
pub struct SmbTarget {
    pub label: String,
    pub address: String,
    pub port: u16,
    pub share: Option<String>,
    pub username: String,
    pub password: crate::model::SecretString,
    /// SSH server to tunnel the connection through.
    pub jump: Option<ssh::Target>,
}

pub struct Smb {
    target: SmbTarget,
    client: Client,
    _tunnel: Option<(Arc<ssh::Conn>, ssh::Forward)>,
}

impl From<smb::Error> for AppError {
    fn from(e: smb::Error) -> Self {
        AppError::Other(format!("SMB: {e}"))
    }
}

const CHUNK: usize = 1024 * 1024;

impl Smb {
    pub async fn open(target: SmbTarget, store: Arc<Store>, notices: &mut Vec<String>) -> AppResult<Self> {
        let mut config = ClientConfig::default();
        config.connection.port = Some(target.port);
        config.connection.timeout = Some(std::time::Duration::from_secs(20));
        let client = Client::new(config);
        // Through a jump host: connect the server name to a local forward first, so the
        // later share connections reuse that transport.
        let tunnel = match &target.jump {
            Some(j) => {
                let conn = Arc::new(ssh::connect(j, store, notices).await?);
                let fwd = ssh::forward(conn.clone(), target.address.clone(), target.port, false).await?;
                client
                    .connect_to_address(&target.address, fwd.addr)
                    .await
                    .map_err(|e| AppError::Other(format!("Could not reach {} through {}: {e}", target.label, j.label)))?;
                notices.push(format!("Connected through jump host {}.", j.label));
                Some((conn, fwd))
            }
            None => None,
        };
        let me = Self { target, client, _tunnel: tunnel };
        // Connect once up front so bad credentials are reported immediately.
        let root = me.unc(me.target.share.as_deref().unwrap_or("IPC$"), "")?;
        // The smb crate takes plain Strings; those copies are out of our hands.
        let connect = if me.target.share.is_some() {
            me.client.share_connect(&root, &me.target.username, me.target.password.to_string()).await
        } else {
            me.client.ipc_connect(&me.target.address, &me.target.username, me.target.password.to_string()).await
        };
        connect.map_err(|e| {
            let text = e.to_string();
            if text.contains("LogonFailure") || text.to_lowercase().contains("logon") {
                AppError::Other(format!("{} rejected the login for \"{}\".", me.target.label, me.target.username))
            } else {
                AppError::Other(format!("Could not connect to {}: {text}", me.target.label))
            }
        })?;
        Ok(me)
    }

    fn unc(&self, share: &str, rel: &str) -> AppResult<UncPath> {
        let p = UncPath::new(&self.target.address)?.with_share(share)?;
        Ok(if rel.is_empty() { p } else { p.with_path(rel) })
    }

    /// Splits a UI path into (share, share-relative path with backslashes).
    fn split(&self, path: &str) -> AppResult<(String, String)> {
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        match &self.target.share {
            Some(share) => Ok((share.clone(), parts.join("\\"))),
            None => match parts.split_first() {
                Some((share, rest)) => Ok((share.to_string(), rest.join("\\"))),
                None => Err(AppError::Invalid("Pick a share first.".into())),
            },
        }
    }

    async fn ensure_share(&self, share: &str) -> AppResult<()> {
        if self.target.share.as_deref() != Some(share) {
            let root = self.unc(share, "")?;
            self.client.share_connect(&root, &self.target.username, self.target.password.to_string()).await?;
        }
        Ok(())
    }

    async fn open_path(&self, path: &str, args: &FileCreateArgs) -> AppResult<smb::Resource> {
        let (share, rel) = self.split(path)?;
        self.ensure_share(&share).await?;
        Ok(self.client.create_file(&self.unc(&share, &rel)?, args).await?)
    }

    fn is_share_list(&self, path: &str) -> bool {
        self.target.share.is_none() && path.split('/').all(|s| s.is_empty())
    }
}

#[async_trait]
impl RemoteFs for Smb {
    async fn home(&self) -> AppResult<String> {
        Ok("/".into())
    }

    async fn list(&self, path: &str) -> AppResult<Listing> {
        let path = if path.is_empty() { "/".to_string() } else { path.to_string() };
        if self.is_share_list(&path) {
            let mut entries: Vec<Entry> = self
                .client
                .list_shares(&self.target.address)
                .await
                .map_err(|_| {
                    AppError::Invalid(format!(
                        "{} did not list its shares. Enter the share name in the host settings (SMB path).",
                        self.target.label
                    ))
                })?
                .into_iter()
                .filter_map(|s| s.netname.as_ref().map(|n| n.to_string()))
                .filter(|n| !n.ends_with('$'))
                .map(|name| Entry {
                    path: join("/", &name),
                    name,
                    is_dir: true,
                    is_link: false,
                    size: 0,
                    modified: None,
                    permissions: None,
                })
                .collect();
            sort_entries(&mut entries);
            return Ok(Listing { path, entries });
        }
        let dir = self
            .open_path(
                &path,
                &FileCreateArgs::make_open_existing(
                    DirAccessMask::new().with_list_directory(true).with_synchronize(true).into(),
                ),
            )
            .await?
            .unwrap_dir();
        let dir = Arc::new(dir);
        let mut stream = Directory::query::<FileFullDirectoryInformation>(&dir, "*").await?;
        let mut entries = Vec::new();
        while let Some(item) = stream.next().await {
            let info = item?;
            let name = info.file_name.to_string();
            if name == "." || name == ".." {
                continue;
            }
            let modified = std::time::SystemTime::from(info.last_write_time)
                .duration_since(UNIX_EPOCH)
                .ok()
                .map(|d| d.as_secs() as u32);
            entries.push(Entry {
                path: join(&path, &name),
                name,
                is_dir: info.file_attributes.directory(),
                is_link: info.file_attributes.reparse_point(),
                size: info.end_of_file,
                modified,
                permissions: None,
            });
        }
        drop(stream);
        dir.close().await?;
        sort_entries(&mut entries);
        Ok(Listing { path, entries })
    }

    async fn stat(&self, path: &str) -> AppResult<(bool, u64)> {
        if self.is_share_list(path) || self.split(path)?.1.is_empty() {
            return Ok((true, 0));
        }
        let r = self
            .open_path(path, &FileCreateArgs::make_open_existing(FileAccessMask::new().with_file_read_attributes(true)))
            .await?;
        let info: FileStandardInformation = match &r {
            smb::Resource::File(f) => f.query_info().await?,
            smb::Resource::Directory(d) => d.query_info().await?,
            _ => return Err(AppError::Invalid("Not a file or folder.".into())),
        };
        close(r).await;
        Ok((bool::from(info.directory), info.end_of_file))
    }

    async fn exists(&self, path: &str) -> AppResult<bool> {
        match self.stat(path).await {
            Ok(_) => Ok(true),
            // STATUS_OBJECT_NAME_NOT_FOUND / STATUS_OBJECT_PATH_NOT_FOUND
            Err(e) if ["0xc0000034", "0xc000003a"].iter().any(|c| e.to_string().contains(c)) => Ok(false),
            Err(e) => Err(e),
        }
    }

    async fn mkdir(&self, path: &str) -> AppResult<()> {
        let r = self
            .open_path(
                path,
                &FileCreateArgs::make_create_new(
                    FileAttributes::new().with_directory(true),
                    CreateOptions::new().with_directory_file(true),
                ),
            )
            .await?;
        close(r).await;
        Ok(())
    }

    async fn rename(&self, from: &str, to: &str) -> AppResult<()> {
        let (_, new_rel) = self.split(to)?;
        let r = self
            .open_path(from, &FileCreateArgs::make_open_existing(FileAccessMask::new().with_delete(true)))
            .await?;
        let info = FileRenameInformation {
            replace_if_exists: false.into(),
            root_directory: 0,
            file_name: new_rel.as_str().into(),
        };
        let result = match &r {
            smb::Resource::File(f) => f.set_info(info).await,
            smb::Resource::Directory(d) => d.set_info(info).await,
            _ => Ok(()),
        };
        close(r).await;
        Ok(result?)
    }

    async fn remove_file(&self, path: &str) -> AppResult<()> {
        delete(self, path).await
    }

    async fn remove_dir(&self, path: &str) -> AppResult<()> {
        delete(self, path).await
    }

    async fn get(&self, remote: &str, local: &Path, report: Report<'_>) -> AppResult<()> {
        let file = self
            .open_path(remote, &FileCreateArgs::make_open_existing(FileAccessMask::new().with_generic_read(true)))
            .await?
            .unwrap_file();
        let mut dst = tokio::fs::File::create(local).await?;
        let mut buf = vec![0u8; CHUNK];
        let mut offset = 0u64;
        let result: AppResult<()> = async {
            loop {
                let n = file.read_at(&mut buf, offset).await?;
                if n == 0 {
                    break;
                }
                dst.write_all(&buf[..n]).await?;
                offset += n as u64;
                report(n as u64);
            }
            dst.flush().await?;
            Ok(())
        }
        .await;
        let _ = file.close().await;
        result
    }

    async fn put(&self, local: &Path, remote: &str, report: Report<'_>) -> AppResult<()> {
        let file = self
            .open_path(remote, &FileCreateArgs::make_overwrite(FileAttributes::new(), CreateOptions::new()))
            .await?
            .unwrap_file();
        let mut src = tokio::fs::File::open(local).await?;
        let mut buf = vec![0u8; CHUNK];
        let mut offset = 0u64;
        let result: AppResult<()> = async {
            loop {
                let n = src.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                let mut written = 0;
                while written < n {
                    written += file.write_at(&buf[written..n], offset + written as u64).await?;
                }
                offset += n as u64;
                report(n as u64);
            }
            Ok(())
        }
        .await;
        let _ = file.close().await;
        result
    }

    async fn for_transfer(self: Arc<Self>) -> AppResult<Arc<dyn RemoteFs>> {
        Ok(self)
    }
}

async fn close(r: smb::Resource) {
    let _ = match r {
        smb::Resource::File(f) => f.close().await,
        smb::Resource::Directory(d) => d.close().await,
        _ => Ok(()),
    };
}

async fn delete(fs: &Smb, path: &str) -> AppResult<()> {
    let r = fs
        .open_path(path, &FileCreateArgs::make_open_existing(FileAccessMask::new().with_delete(true)))
        .await?;
    let info = FileDispositionInformation { delete_pending: true.into() };
    let result = match &r {
        smb::Resource::File(f) => f.set_info(info).await,
        smb::Resource::Directory(d) => d.set_info(info).await,
        _ => Ok(()),
    };
    close(r).await;
    Ok(result?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live test, skipped unless `BUNNYLINK_TEST_SMB=host:port:share:user:password` is set.
    #[tokio::test(flavor = "multi_thread")]
    async fn live_browse_and_transfer() {
        let Ok(spec) = std::env::var("BUNNYLINK_TEST_SMB") else { return };
        let p: Vec<&str> = spec.splitn(5, ':').collect();
        let target = SmbTarget {
            label: "test".into(),
            address: p[0].into(),
            port: p[1].parse().unwrap(),
            share: Some(p[2].into()),
            username: p[3].into(),
            password: p[4].to_string().into(),
            jump: None,
        };
        let store = Arc::new(Store::in_memory().unwrap());
        let fs = Arc::new(Smb::open(target.clone(), store.clone(), &mut Vec::new()).await.unwrap());
        crate::files::tests::exercise(fs, "/").await;

        // Through a jump host as well, when an SSH test server is configured.
        if let Ok(ssh_spec) = std::env::var("BUNNYLINK_TEST_SSH") {
            let s: Vec<&str> = ssh_spec.splitn(4, ':').collect();
            let jump = ssh::Target {
                label: "jump".into(),
                address: s[0].into(),
                port: s[1].parse().unwrap(),
                username: s[2].into(),
                auth: ssh::Auth::Password(s[3].to_string().into()),
                jump: None,
            };
            let mut notices = Vec::new();
            let via = Smb::open(SmbTarget { jump: Some(jump), ..target.clone() }, store.clone(), &mut notices).await.unwrap();
            assert!(notices.iter().any(|n| n.contains("jump host")), "{notices:?}");
            crate::files::tests::exercise(Arc::new(via), "/").await;
        }

        // Without a configured share, the root lists shares. Samba rejects the RPC
        // syntax used for that; the user then gets a clear instruction instead.
        let browse = Smb::open(SmbTarget { share: None, ..target }, store, &mut Vec::new()).await.unwrap();
        match browse.list("/").await {
            Ok(shares) => assert!(shares.entries.iter().any(|e| e.name == p[2] && e.is_dir), "{:?}", shares.entries),
            Err(e) => assert!(e.to_string().contains("Enter the share name"), "{e}"),
        }
        // Shares can still be browsed by path.
        assert!(browse.list(&format!("/{}", p[2])).await.is_ok());
    }
}
