// SPDX-FileCopyrightText: 2026 BunnyCloud.IT
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial
//! Self-update from the BunnyCloud update server.
//!
//! Official builds get the server URL and the release public key at build time
//! (`BUNNYLINK_UPDATE_URL`, `BUNNYLINK_UPDATE_KEY`); other builds have no updater.
//! A package is installed only when its size, SHA-256 and Ed25519 signature all match,
//! so the server alone can't push code. Installing renames the running executable to
//! `<name>.old.exe`, moves the new one into its place and starts it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use base64::Engine;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};
use tokio::io::AsyncWriteExt;

use crate::error::{AppError, AppResult};

const PRODUCT: &str = "bunnylink";
/// Must match the update server's signing message.
const CONTEXT: &str = "bunnysite-release-v1";
pub const CHECK_EVERY: Duration = Duration::from_secs(12 * 60 * 60);
const MAX_PACKAGE: u64 = 200 * 1024 * 1024;

/// The update server and its release key, when this build has them.
fn config() -> Option<(&'static str, &'static str)> {
    let url = option_env!("BUNNYLINK_UPDATE_URL")?.trim().trim_end_matches('/');
    let key = option_env!("BUNNYLINK_UPDATE_KEY")?.trim();
    (!url.is_empty() && !key.is_empty()).then_some((url, key))
}

pub fn available() -> bool {
    config().is_some()
}

/// This release's notes, embedded at build time; empty when the build has none.
const CURRENT_CHANGELOG: &str = include_str!(concat!(env!("OUT_DIR"), "/changelog.md"));

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[derive(Clone, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Status {
    /// This build has no update server.
    Unavailable,
    Idle,
    Checking,
    #[serde(rename_all = "camelCase")]
    UpToDate { checked_at: u64 },
    #[serde(rename_all = "camelCase")]
    Downloading { version: String, done: u64, total: u64 },
    /// Downloaded and verified; installs on restart.
    Ready { version: String, changelog: String },
    Error { message: String },
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    pub current: &'static str,
    pub current_changelog: &'static str,
    pub status: Status,
}

struct Staged {
    path: PathBuf,
}

pub struct Updater {
    status: Mutex<Status>,
    staged: Mutex<Option<Staged>>,
    busy: AtomicBool,
}

impl Default for Updater {
    fn default() -> Self {
        Self {
            status: Mutex::new(if available() { Status::Idle } else { Status::Unavailable }),
            staged: Mutex::new(None),
            busy: AtomicBool::new(false),
        }
    }
}

#[derive(Deserialize)]
struct CheckResponse {
    update: Option<Offer>,
    #[serde(default)]
    changelogs: Vec<Changelog>,
}

#[derive(Deserialize)]
struct Offer {
    version: String,
    download_url: String,
    size: u64,
    sha256: String,
    signature: String,
}

#[derive(Deserialize)]
struct Changelog {
    version: String,
    #[serde(default)]
    changelog: String,
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Updater {
    pub fn info(&self) -> Info {
        Info { current: current_version(), current_changelog: CURRENT_CHANGELOG, status: self.status.lock().unwrap().clone() }
    }

    fn set(&self, app: &AppHandle, status: Status) {
        *self.status.lock().unwrap() = status.clone();
        let _ = app.emit("update-status", Info { current: current_version(), current_changelog: CURRENT_CHANGELOG, status });
    }

    /// Checks the server and, when there is a newer release, downloads and verifies it.
    /// Does nothing while another run is in progress or an update is already waiting.
    pub async fn run(&self, app: &AppHandle) {
        if !available() || self.staged.lock().unwrap().is_some() || self.busy.swap(true, Ordering::SeqCst) {
            return;
        }
        self.set(app, Status::Checking);
        let status = match self.check_and_download(app).await {
            Ok(status) => status,
            Err(e) => Status::Error { message: e.to_string() },
        };
        self.set(app, status);
        self.busy.store(false, Ordering::SeqCst);
    }

    async fn check_and_download(&self, app: &AppHandle) -> AppResult<Status> {
        let (server, key) = config().ok_or_else(|| AppError::Other("Updates are not available in this build.".into()))?;
        let client = reqwest::Client::builder()
            .user_agent(format!("BunnyLink/{}", current_version()))
            .connect_timeout(Duration::from_secs(20))
            .build()
            .map_err(|e| AppError::Other(format!("Could not start the update check: {e}")))?;
        let unreachable = |e: reqwest::Error| AppError::Other(format!("Could not reach the update server: {e}"));

        let check: CheckResponse = client
            .get(format!("{server}/api/v1/check"))
            .query(&[("product", PRODUCT), ("version", current_version()), ("channel", "stable")])
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(unreachable)?
            .json()
            .await
            .map_err(|e| AppError::Other(format!("The update server sent an unexpected answer: {e}")))?;

        let Some(offer) = check.update else {
            return Ok(Status::UpToDate { checked_at: now() });
        };
        // The signature covers the version, but a server could still offer an older
        // signed release; never go backwards.
        let newer = match (semver::Version::parse(&offer.version), semver::Version::parse(current_version())) {
            (Ok(offered), Ok(current)) => offered > current,
            _ => false,
        };
        if !newer {
            return Ok(Status::UpToDate { checked_at: now() });
        }
        if offer.size == 0 || offer.size > MAX_PACKAGE {
            return Err(AppError::Other("The offered update has an invalid size.".into()));
        }

        // Download.
        let zip_path = std::env::temp_dir().join(format!("bunnylink-update-{}.zip", offer.version));
        let mut file = tokio::fs::File::create(&zip_path).await?;
        let response = client.get(&offer.download_url).send().await.and_then(|r| r.error_for_status()).map_err(unreachable)?;
        let mut stream = response.bytes_stream();
        let mut hasher = Sha256::new();
        let (mut done, mut reported) = (0u64, 0u64);
        self.set(app, Status::Downloading { version: offer.version.clone(), done, total: offer.size });
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(unreachable)?;
            done += chunk.len() as u64;
            if done > offer.size {
                drop(file);
                let _ = tokio::fs::remove_file(&zip_path).await;
                return Err(AppError::Other("The update download is larger than announced.".into()));
            }
            hasher.update(&chunk);
            file.write_all(&chunk).await?;
            if done - reported >= offer.size / 50 {
                reported = done;
                self.set(app, Status::Downloading { version: offer.version.clone(), done, total: offer.size });
            }
        }
        file.flush().await?;
        drop(file);

        // Verify size, hash and signature before anything is unpacked.
        let sha256 = hex(&hasher.finalize());
        let verified = verify(key, &offer, done, &sha256);
        if let Err(e) = verified {
            let _ = tokio::fs::remove_file(&zip_path).await;
            return Err(e);
        }

        // Unpack next to the running executable, so the swap is a rename on one drive.
        let staged = tauri::async_runtime::spawn_blocking({
            let zip_path = zip_path.clone();
            move || unpack(&zip_path)
        })
        .await
        .map_err(|e| AppError::Other(e.to_string()))?;
        let _ = tokio::fs::remove_file(&zip_path).await;
        let path = staged?;
        *self.staged.lock().unwrap() = Some(Staged { path });

        let changelog = check
            .changelogs
            .iter()
            .map(|c| format!("## {}\n\n{}", c.version, c.changelog.trim()))
            .collect::<Vec<_>>()
            .join("\n\n");
        Ok(Status::Ready { version: offer.version, changelog })
    }

    /// Swaps in the downloaded executable, starts it and quits this one.
    pub fn install(&self, app: &AppHandle) -> AppResult<()> {
        let staged = self.staged.lock().unwrap().take().ok_or_else(|| AppError::Other("No update is ready to install.".into()))?;
        let exe = std::env::current_exe()?;
        let old = sibling(&exe, "old");
        let _ = std::fs::remove_file(&old);
        std::fs::rename(&exe, &old).map_err(|e| AppError::Other(format!("Could not replace BunnyLink: {e}")))?;
        if let Err(e) = std::fs::rename(&staged.path, &exe) {
            let _ = std::fs::rename(&old, &exe);
            return Err(AppError::Other(format!("Could not install the update: {e}")));
        }
        if let Err(e) = std::process::Command::new(&exe).spawn() {
            // Put the running version back so the next start still works.
            let _ = std::fs::rename(&exe, &staged.path);
            let _ = std::fs::rename(&old, &exe);
            return Err(AppError::Other(format!("Could not start the new version: {e}")));
        }
        app.exit(0);
        Ok(())
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Checks the downloaded size, hash and the Ed25519 signature over the server's message.
fn verify(public_key: &str, offer: &Offer, size: u64, sha256: &str) -> AppResult<()> {
    let bad = |what: &str| AppError::Other(format!("The update failed verification ({what}) and was discarded."));
    if size != offer.size {
        return Err(bad("size"));
    }
    if !sha256.eq_ignore_ascii_case(offer.sha256.trim()) {
        return Err(bad("checksum"));
    }
    let b64 = base64::engine::general_purpose::STANDARD;
    let key: [u8; 32] = b64.decode(public_key).ok().and_then(|k| k.try_into().ok()).ok_or_else(|| bad("release key"))?;
    let sig: [u8; 64] = b64.decode(offer.signature.trim()).ok().and_then(|s| s.try_into().ok()).ok_or_else(|| bad("signature"))?;
    let key = ed25519_dalek::VerifyingKey::from_bytes(&key).map_err(|_| bad("release key"))?;
    let message = format!("{CONTEXT}\n{PRODUCT}\n{}\n{sha256}\n{size}", offer.version);
    key.verify_strict(message.as_bytes(), &ed25519_dalek::Signature::from_bytes(&sig)).map_err(|_| bad("signature"))
}

/// `C:\…\bunnylink.exe` -> `C:\…\bunnylink.<tag>.exe`
fn sibling(exe: &Path, tag: &str) -> PathBuf {
    let stem = exe.file_stem().and_then(|s| s.to_str()).unwrap_or("bunnylink");
    exe.with_file_name(format!("{stem}.{tag}.exe"))
}

/// Extracts the package's single `.exe` to `<name>.new.exe` beside the running one.
fn unpack(zip_path: &Path) -> AppResult<PathBuf> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(zip_path)?)
        .map_err(|e| AppError::Other(format!("The update package can't be opened: {e}")))?;
    let names: Vec<String> = archive.file_names().filter(|n| n.to_ascii_lowercase().ends_with(".exe")).map(String::from).collect();
    let [name] = names.as_slice() else {
        return Err(AppError::Other("The update package must contain exactly one program.".into()));
    };
    let mut entry = archive.by_name(name).map_err(|e| AppError::Other(e.to_string()))?;
    let target = sibling(&std::env::current_exe()?, "new");
    let mut out = std::fs::File::create(&target).map_err(|e| {
        AppError::Other(format!(
            "BunnyLink can't update itself because its folder isn't writable ({e}). Download the new version by hand."
        ))
    })?;
    std::io::copy(&mut entry, &mut out)?;
    Ok(target)
}

/// Removes what an earlier update left behind: the previous executable (once it has
/// exited) and an unfinished download.
pub async fn clean_up() {
    let Ok(exe) = std::env::current_exe() else { return };
    let _ = tokio::fs::remove_file(sibling(&exe, "new")).await;
    let old = sibling(&exe, "old");
    for _ in 0..20 {
        match tokio::fs::remove_file(&old).await {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => tokio::time::sleep(Duration::from_millis(500)).await,
            _ => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::Signer;

    #[test]
    fn verifies_the_server_signature_format() {
        let signing = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let b64 = base64::engine::general_purpose::STANDARD;
        let public = b64.encode(signing.verifying_key().to_bytes());
        let sha = "ab".repeat(32);
        let message = format!("bunnysite-release-v1\nbunnylink\n1.2.0\n{sha}\n1234");
        let offer = Offer {
            version: "1.2.0".into(),
            download_url: String::new(),
            size: 1234,
            sha256: sha.clone(),
            signature: b64.encode(signing.sign(message.as_bytes()).to_bytes()),
        };
        assert!(verify(&public, &offer, 1234, &sha).is_ok());
        assert!(verify(&public, &offer, 1235, &sha).is_err(), "size mismatch");
        assert!(verify(&public, &offer, 1234, &"cd".repeat(32)).is_err(), "hash mismatch");
        let other = Offer { version: "1.3.0".into(), ..offer };
        assert!(verify(&public, &other, 1234, &sha).is_err(), "signature covers the version");
    }
}
