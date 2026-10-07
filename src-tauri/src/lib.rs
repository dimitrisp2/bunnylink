// SPDX-FileCopyrightText: 2026 BunnyCloud.IT
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial
mod display;
mod error;
mod files;
mod ftp;
mod import;
mod smbfs;
mod model;
mod net;
mod serial;
mod telnet;
mod rdp;
mod sftp;
mod ssh;
mod vnc;
mod store;
mod updater;
mod vault;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tokio::sync::mpsc;
use tauri::async_runtime::JoinHandle;

use error::{AppError, AppResult};
use model::*;
use ssh::{Auth, Target, TermEvent, TermInput};
use store::Store;
use vault::Vault;

struct AppState {
    store: Arc<Store>,
    /// First run only: the store is in memory until the vault is created, and these are
    /// the folders it can be saved to (app data, next to the executable).
    first_run: Mutex<Option<DbLocations>>,
    vault: Vault,
    updater: Arc<updater::Updater>,
    terminals: Mutex<HashMap<String, mpsc::UnboundedSender<TermInput>>>,
    files: Mutex<HashMap<String, Arc<dyn files::RemoteFs>>>,
    desktops: Mutex<HashMap<String, mpsc::UnboundedSender<display::DesktopInput>>>,
    transfers: Mutex<HashMap<String, JoinHandle<()>>>,
    tunnels: Mutex<HashMap<String, JoinHandle<()>>>,
    last_activity: Mutex<Instant>,
}

impl AppState {
    fn remote(&self, session_id: &str) -> AppResult<Arc<dyn files::RemoteFs>> {
        self.files
            .lock()
            .unwrap()
            .get(session_id)
            .cloned()
            .ok_or(AppError::Invalid("This file session has closed. Reopen the tab.".into()))
    }

    fn touch(&self) {
        *self.last_activity.lock().unwrap() = Instant::now();
    }

    fn secret(&self, credential_id: &str) -> AppResult<Secret> {
        let blob = self
            .store
            .credential_secret(credential_id)?
            .ok_or(AppError::Invalid("This credential has no secret saved.".into()))?;
        Ok(serde_json::from_slice(&self.vault.decrypt(&blob)?)?)
    }

    /// Builds the SSH target for a host, resolving inheritance, credentials and jump hosts.
    fn ssh_target(&self, host_id: &str, depth: u8) -> AppResult<Target> {
        if depth > 4 {
            return Err(AppError::Invalid("Jump hosts are nested too deeply (or form a loop).".into()));
        }
        let host = self.store.host(host_id)?;
        let eff = self.store.effective(&host)?;
        let cred = match &eff.credential_id {
            Some(id) => Some(self.store.credential(id)?),
            None => None,
        };
        let username = eff
            .username
            .or(cred.as_ref().and_then(|c| c.username.clone()))
            .ok_or_else(|| AppError::Invalid(format!("{} has no username. Set one, or pick a credential.", host.name)))?;
        // With no credential, fall back to the SSH agent.
        let auth = match cred {
            None => Auth::Agent,
            Some(c) => match c.kind {
                CredentialKind::Agent => Auth::Agent,
                CredentialKind::Password => Auth::Password(self.secret(&c.id)?.secret),
                CredentialKind::Key => {
                    let s = self.secret(&c.id)?;
                    Auth::Key { pem: s.secret, passphrase: s.passphrase, ask: c.ask_passphrase.then(|| c.name.clone()) }
                }
            },
        };
        let port = host
            .endpoints
            .iter()
            .find(|e| matches!(e.protocol, Protocol::Ssh | Protocol::Sftp))
            .and_then(|e| e.port)
            .unwrap_or(Protocol::Ssh.default_port());
        let jump = match eff.jump_host_id {
            Some(j) => Some(Box::new(self.ssh_target(&j, depth + 1)?)),
            None => None,
        };
        Ok(Target { label: host.name, address: host.address, port, username, auth, jump })
    }
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

// ---------------------------------------------------------------- vault

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VaultStatus {
    initialized: bool,
    unlocked: bool,
    /// The database has not been saved yet, so the user can choose portable mode.
    first_run: bool,
}

#[tauri::command]
fn vault_status(state: State<AppState>) -> AppResult<VaultStatus> {
    Ok(VaultStatus {
        initialized: Vault::is_initialized(&state.store)?,
        unlocked: state.vault.is_unlocked(),
        first_run: state.first_run.lock().unwrap().is_some(),
    })
}

#[tauri::command]
async fn vault_create(state: State<'_, AppState>, password: String, portable: bool) -> AppResult<()> {
    state.touch();
    {
        let mut first_run = state.first_run.lock().unwrap();
        if let Some(locations) = first_run.as_ref() {
            let dir = if portable { &locations.portable } else { &locations.app_data };
            std::fs::create_dir_all(dir)
                .and_then(|_| state.store.persist(&dir.join(DB_FILE)).map_err(std::io::Error::other))
                .map_err(|e| AppError::Invalid(format!("Can't create the database in {}: {e}", dir.display())))?;
            *first_run = None;
        }
    }
    state.vault.create(&state.store, &password)
}

#[tauri::command]
async fn vault_unlock(state: State<'_, AppState>, password: String) -> AppResult<()> {
    state.touch();
    state.vault.unlock(&state.store, &password)
}

#[tauri::command]
fn vault_lock(state: State<AppState>) {
    state.vault.lock();
}

// ---------------------------------------------------------------- import

fn import_plan(app: &AppHandle, source: import::Source, path: Option<String>, password: Option<String>) -> AppResult<import::Plan> {
    let home = app.path().home_dir().map_err(|e| AppError::Invalid(format!("No home folder: {e}")))?;
    let path = path.map(|p| p.trim().to_string()).filter(|p| !p.is_empty());
    match source {
        import::Source::Openssh => {
            let path = path.map(std::path::PathBuf::from).unwrap_or_else(|| home.join(".ssh").join("config"));
            import::parse_openssh(&path, &home)
        }
        import::Source::Putty => Ok(import::putty_plan(import::read_putty(&home)?)),
        import::Source::Mremoteng => {
            let path = path.ok_or_else(|| AppError::Invalid("Choose an mRemoteNG connections file.".into()))?;
            let xml = std::fs::read_to_string(&path).map_err(|e| AppError::Invalid(format!("Could not read {path}: {e}")))?;
            import::parse_mremoteng(&xml, password.as_deref().filter(|p| !p.is_empty()))
        }
    }
}

#[tauri::command]
async fn import_preview(
    app: AppHandle,
    source: import::Source,
    path: Option<String>,
    password: Option<String>,
) -> AppResult<import::Preview> {
    tauri::async_runtime::spawn_blocking(move || Ok(import::preview(&import_plan(&app, source, path, password)?)))
        .await
        .map_err(|e| AppError::Invalid(e.to_string()))?
}

#[tauri::command]
async fn import_apply(
    app: AppHandle,
    source: import::Source,
    path: Option<String>,
    password: Option<String>,
    keys: Option<import::KeyChoices>,
) -> AppResult<import::Applied> {
    tauri::async_runtime::spawn_blocking(move || {
        let plan = import_plan(&app, source, path, password)?;
        let state = app.state::<AppState>();
        state.touch();
        import::apply(&state.store, &state.vault, source, plan, &keys.unwrap_or_default())
    })
    .await
    .map_err(|e| AppError::Invalid(e.to_string()))?
}

// ---------------------------------------------------------------- library

#[tauri::command]
fn get_library(state: State<AppState>) -> AppResult<Library> {
    state.touch();
    state.store.library()
}

#[tauri::command]
fn get_effective(state: State<AppState>, host_id: String) -> AppResult<Effective> {
    let host = state.store.host(&host_id)?;
    state.store.effective(&host)
}

#[tauri::command]
fn save_folder(state: State<AppState>, mut folder: Folder) -> AppResult<Folder> {
    state.touch();
    if folder.id.is_empty() {
        folder.id = new_id();
    }
    state.store.save_folder(&folder)?;
    Ok(folder)
}

#[tauri::command]
fn delete_folder(state: State<AppState>, id: String) -> AppResult<()> {
    state.store.delete_folder(&id)
}

#[tauri::command]
fn save_host(state: State<AppState>, mut host: Host) -> AppResult<Host> {
    state.touch();
    if host.id.is_empty() {
        host.id = new_id();
    }
    let serial_only = !host.endpoints.is_empty() && host.endpoints.iter().all(|e| e.protocol == Protocol::Serial);
    if host.name.trim().is_empty() || (host.address.trim().is_empty() && !serial_only) {
        return Err(AppError::Invalid("A host needs a name and an address.".into()));
    }
    state.store.save_host(&host)?;
    Ok(host)
}

#[tauri::command]
fn delete_host(state: State<AppState>, id: String) -> AppResult<()> {
    state.store.delete_host(&id)
}

#[tauri::command]
fn save_credential(state: State<AppState>, input: CredentialInput) -> AppResult<Credential> {
    state.touch();
    let mut cred = input.credential;
    let is_new = cred.id.is_empty();
    if is_new {
        cred.id = new_id();
    }
    cred.ask_passphrase &= cred.kind == CredentialKind::Key;
    // A passphrase asked for on every connection is never stored.
    let passphrase = input.passphrase.filter(|p| !p.is_empty() && !cred.ask_passphrase);
    let secret = match input.secret {
        Some(secret) => Some(Secret { secret, passphrase }),
        None if is_new && cred.kind != CredentialKind::Agent => {
            return Err(AppError::Invalid("Enter a password or private key.".into()))
        }
        // Same key: update or drop the stored passphrase.
        None if cred.kind == CredentialKind::Key && (passphrase.is_some() || cred.ask_passphrase) => {
            let stored = state.secret(&cred.id)?;
            Some(Secret { secret: stored.secret, passphrase })
        }
        None => None,
    };
    let blob = match secret {
        Some(s) => Some(state.vault.encrypt(&serde_json::to_vec(&s)?)?),
        None => None,
    };
    state.store.save_credential(&cred, blob)?;
    Ok(cred)
}

#[tauri::command]
fn delete_credential(state: State<AppState>, id: String) -> AppResult<()> {
    state.store.delete_credential(&id)
}

#[tauri::command]
fn save_tunnel(state: State<AppState>, mut tunnel: Tunnel) -> AppResult<Tunnel> {
    if tunnel.id.is_empty() {
        tunnel.id = new_id();
    }
    if tunnel.kind != TunnelKind::Socks && (tunnel.target_host.is_none() || tunnel.target_port.is_none()) {
        return Err(AppError::Invalid("Local and remote tunnels need a destination host and port.".into()));
    }
    state.store.save_tunnel(&tunnel)?;
    Ok(tunnel)
}

#[tauri::command]
fn delete_tunnel(state: State<AppState>, id: String) -> AppResult<()> {
    if let Some(task) = state.tunnels.lock().unwrap().remove(&id) {
        task.abort();
    }
    state.store.delete_tunnel(&id)
}

#[tauri::command]
fn save_snippet(state: State<AppState>, mut snippet: Snippet) -> AppResult<Snippet> {
    state.touch();
    if snippet.id.is_empty() {
        snippet.id = new_id();
    }
    if snippet.name.trim().is_empty() || snippet.command.trim().is_empty() {
        return Err(AppError::Invalid("A snippet needs a name and a command.".into()));
    }
    state.store.save_snippet(&snippet)?;
    Ok(snippet)
}

#[tauri::command]
fn delete_snippet(state: State<AppState>, id: String) -> AppResult<()> {
    state.store.delete_snippet(&id)
}

#[tauri::command]
fn get_settings(state: State<AppState>) -> AppResult<Settings> {
    state.store.settings()
}

#[tauri::command]
fn save_settings(state: State<AppState>, settings: Settings) -> AppResult<()> {
    state.store.save_settings(&settings)
}

#[tauri::command]
fn forget_host_key(state: State<AppState>, host_id: String) -> AppResult<()> {
    let host = state.store.host(&host_id)?;
    let port = host
        .endpoints
        .iter()
        .find(|e| matches!(e.protocol, Protocol::Ssh | Protocol::Sftp))
        .and_then(|e| e.port)
        .unwrap_or(Protocol::Ssh.default_port());
    state.store.forget_known_host(&host.address, port)
}

// ---------------------------------------------------------------- copy credentials

#[tauri::command]
fn copy_credential(app: AppHandle, state: State<AppState>, host_id: String, field: String) -> AppResult<()> {
    state.touch();
    let host = state.store.host(&host_id)?;
    let eff = state.store.effective(&host)?;
    let cred = match &eff.credential_id {
        Some(id) => Some(state.store.credential(id)?),
        None => None,
    };
    let text = match field.as_str() {
        "username" => eff
            .username
            .or(cred.and_then(|c| c.username))
            .ok_or(AppError::Invalid(format!("{} has no username.", host.name)))?,
        "password" => {
            let cred = cred.ok_or(AppError::Invalid(format!("{} has no credential.", host.name)))?;
            if cred.kind != CredentialKind::Password {
                return Err(AppError::Invalid("This host uses a key, not a password.".into()));
            }
            state.secret(&cred.id)?.secret
        }
        _ => return Err(AppError::Invalid("Unknown field.".into())),
    };
    app.clipboard().write_text(text.clone()).map_err(|e| AppError::Other(e.to_string()))?;
    let seconds = state.store.settings()?.clipboard_clear_seconds;
    if seconds > 0 && field == "password" {
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_secs(seconds as u64)).await;
            // Only clear if the user hasn't copied something else meanwhile.
            if app.clipboard().read_text().ok().as_deref() == Some(text.as_str()) {
                let _ = app.clipboard().write_text(String::new());
            }
        });
    }
    Ok(())
}

// ---------------------------------------------------------------- terminals

#[tauri::command]
fn terminal_open(
    state: State<AppState>,
    host_id: String,
    protocol: Option<Protocol>,
    cols: u32,
    rows: u32,
    on_event: Channel<TermEvent>,
) -> AppResult<String> {
    state.touch();
    let protocol = protocol.unwrap_or(Protocol::Ssh);
    let host = state.store.host(&host_id)?;
    let session_id = new_id();
    let (tx, rx) = mpsc::unbounded_channel();
    let store = state.store.clone();
    let send = move |e: TermEvent| {
        let _ = on_event.send(e);
    };
    match protocol {
        Protocol::Ssh => {
            let target = state.ssh_target(&host_id, 0)?;
            tauri::async_runtime::spawn(async move {
                let mut rx = rx;
                send(TermEvent::Notice { text: format!("Connecting to {}…", target.label) });
                let mut notices = Vec::new();
                // Key passphrases are typed into this terminal.
                let mut prompter = ssh::TerminalPrompter { input: &mut rx, emit: &send, size: (cols, rows) };
                let conn = ssh::connect_with(&target, store, &mut notices, &mut prompter).await;
                let (cols, rows) = prompter.size;
                for text in notices {
                    send(TermEvent::Notice { text });
                }
                match conn {
                    Ok(conn) => ssh::run_terminal(conn, cols, rows, rx, send).await,
                    Err(e) => send(TermEvent::Error { message: e.to_string() }),
                }
            });
        }
        Protocol::Telnet => {
            let eff = state.store.effective(&host)?;
            let cred = match &eff.credential_id {
                Some(id) => Some(state.store.credential(id)?),
                None => None,
            };
            let password = match &cred {
                Some(c) if c.kind == CredentialKind::Password => Some(state.secret(&c.id)?.secret),
                _ => None,
            };
            let jump = match &eff.jump_host_id {
                Some(j) => Some(state.ssh_target(j, 1)?),
                None => None,
            };
            let target = telnet::TelnetTarget {
                label: host.name.clone(),
                address: host.address.clone(),
                port: endpoint_port(&host, Protocol::Telnet),
                username: eff.username.clone().or(cred.and_then(|c| c.username)),
                password,
                jump,
            };
            tauri::async_runtime::spawn(telnet::run(target, store, cols, rows, rx, send));
        }
        Protocol::Serial => {
            let ep = host.endpoints.iter().find(|e| e.protocol == Protocol::Serial);
            let device = ep
                .and_then(|e| e.path.clone())
                .filter(|p| !p.trim().is_empty())
                .ok_or_else(|| AppError::Invalid(format!("{} has no serial device. Set one in the host settings.", host.name)))?;
            let target = serial::SerialTarget {
                device,
                settings: ep.and_then(|e| e.serial.clone()).unwrap_or_default(),
            };
            tauri::async_runtime::spawn(serial::run(target, rx, send));
        }
        _ => return Err(AppError::Invalid("This protocol has no terminal.".into())),
    }
    let _ = state.store.touch_host(&host_id, now());
    state.terminals.lock().unwrap().insert(session_id.clone(), tx);
    Ok(session_id)
}

#[tauri::command]
fn serial_ports() -> Vec<String> {
    serial::ports()
}

fn endpoint_port(host: &Host, protocol: Protocol) -> u16 {
    host.endpoints
        .iter()
        .find(|e| e.protocol == protocol)
        .and_then(|e| e.port)
        .unwrap_or(protocol.default_port())
}

#[tauri::command]
fn terminal_input(state: State<AppState>, session_id: String, data: String) {
    state.touch();
    if let Some(tx) = state.terminals.lock().unwrap().get(&session_id) {
        let _ = tx.send(TermInput::Data(data.into_bytes()));
    }
}

#[tauri::command]
fn terminal_resize(state: State<AppState>, session_id: String, cols: u32, rows: u32) {
    if let Some(tx) = state.terminals.lock().unwrap().get(&session_id) {
        let _ = tx.send(TermInput::Resize { cols, rows });
    }
}

#[tauri::command]
fn terminal_close(state: State<AppState>, session_id: String) {
    if let Some(tx) = state.terminals.lock().unwrap().remove(&session_id) {
        let _ = tx.send(TermInput::Close);
    }
}

#[tauri::command]
async fn run_command(state: State<'_, AppState>, host_id: String, command: String) -> AppResult<ssh::ExecResult> {
    state.touch();
    let target = state.ssh_target(&host_id, 0)?;
    let conn = ssh::connect(&target, state.store.clone(), &mut Vec::new()).await?;
    ssh::exec(conn, &command).await
}

// ---------------------------------------------------------------- files (SFTP)

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FilesOpened {
    session_id: String,
    home: String,
    notices: Vec<String>,
}

#[tauri::command]
async fn files_open(state: State<'_, AppState>, host_id: String, protocol: Option<Protocol>) -> AppResult<FilesOpened> {
    state.touch();
    let host = state.store.host(&host_id)?;
    let protocol = protocol.unwrap_or(Protocol::Sftp);
    let mut notices = Vec::new();
    let fs: Arc<dyn files::RemoteFs> = match protocol {
        Protocol::Sftp | Protocol::Ssh => {
            let target = state.ssh_target(&host_id, 0)?;
            let conn = ssh::connect(&target, state.store.clone(), &mut notices).await?;
            Arc::new(sftp::Sftp::open(conn).await?)
        }
        Protocol::Ftp | Protocol::Smb => {
            let eff = state.store.effective(&host)?;
            let cred = match &eff.credential_id {
                Some(id) => Some(state.store.credential(id)?),
                None => None,
            };
            let password = match &cred {
                Some(c) if c.kind == CredentialKind::Password => Some(state.secret(&c.id)?.secret),
                _ => None,
            };
            let username = eff.username.clone().or(cred.and_then(|c| c.username));
            let port = endpoint_port(&host, protocol);
            let jump = match &eff.jump_host_id {
                Some(j) => Some(state.ssh_target(j, 1)?),
                None => None,
            };
            if protocol == Protocol::Ftp {
                let target =
                    ftp::FtpTarget { label: host.name.clone(), address: host.address.clone(), port, username, password, jump };
                Arc::new(ftp::Ftp::open(target, state.store.clone(), &mut notices).await?)
            } else {
                let share = host
                    .endpoints
                    .iter()
                    .find(|e| e.protocol == Protocol::Smb)
                    .and_then(|e| e.path.clone())
                    .map(|p| p.trim_matches(['/', '\\']).to_string())
                    .filter(|p| !p.is_empty());
                let target = smbfs::SmbTarget {
                    label: host.name.clone(),
                    address: host.address.clone(),
                    port,
                    share,
                    username: username.unwrap_or_else(|| "Guest".into()),
                    password: password.unwrap_or_default(),
                    jump,
                };
                Arc::new(smbfs::Smb::open(target, state.store.clone(), &mut notices).await?)
            }
        }
        _ => return Err(AppError::Invalid("This protocol has no file browser.".into())),
    };
    let home = fs.home().await?;
    let _ = state.store.touch_host(&host_id, now());
    let session_id = new_id();
    state.files.lock().unwrap().insert(session_id.clone(), fs);
    Ok(FilesOpened { session_id, home, notices })
}

#[tauri::command]
async fn files_list(state: State<'_, AppState>, session_id: String, path: String) -> AppResult<files::Listing> {
    state.touch();
    state.remote(&session_id)?.list(&path).await
}

#[tauri::command]
async fn files_mkdir(state: State<'_, AppState>, session_id: String, path: String) -> AppResult<()> {
    state.remote(&session_id)?.mkdir(&path).await
}

#[tauri::command]
async fn files_rename(state: State<'_, AppState>, session_id: String, from: String, to: String) -> AppResult<()> {
    let fs = state.remote(&session_id)?;
    files::rename(&*fs, &from, &to).await
}

#[tauri::command]
async fn files_delete(state: State<'_, AppState>, session_id: String, paths: Vec<String>) -> AppResult<()> {
    let fs = state.remote(&session_id)?;
    for p in paths {
        files::remove(&*fs, &p).await?;
    }
    Ok(())
}

#[tauri::command]
fn files_close(state: State<AppState>, session_id: String) {
    state.files.lock().unwrap().remove(&session_id);
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TransferEvent {
    id: String,
    session_id: String,
    upload: bool,
    label: String,
    state: &'static str,
    done: u64,
    total: u64,
    current: String,
    message: Option<String>,
    /// Where downloads were saved.
    saved: Vec<String>,
}

fn start_transfer(
    app: AppHandle,
    state: &AppState,
    session_id: String,
    upload: bool,
    label: String,
    job: impl FnOnce(Arc<dyn files::RemoteFs>, Box<dyn FnMut(files::Progress) + Send>) -> std::pin::Pin<Box<dyn std::future::Future<Output = AppResult<Vec<String>>> + Send>>
        + Send
        + 'static,
) -> AppResult<String> {
    let s = state.remote(&session_id)?;
    let id = new_id();
    let base = TransferEvent {
        id: id.clone(),
        session_id,
        upload,
        label,
        state: "running",
        done: 0,
        total: 0,
        current: String::new(),
        message: None,
        saved: vec![],
    };
    let _ = app.emit("transfer", base.clone());
    let progress_app = app.clone();
    let progress_base = base.clone();
    let report = Box::new(move |p: files::Progress| {
        if !p.finished {
            let _ = progress_app.emit(
                "transfer",
                TransferEvent { done: p.done, total: p.total, current: p.current, ..progress_base.clone() },
            );
        }
    });
    let task_id = id.clone();
    let task = tauri::async_runtime::spawn(async move {
        let result = job(s, report).await;
        let ev = match result {
            Ok(saved) => TransferEvent { state: "done", saved, ..base },
            Err(e) => TransferEvent { state: "error", message: Some(e.to_string()), ..base },
        };
        let _ = app.emit("transfer", ev);
        app.state::<AppState>().transfers.lock().unwrap().remove(&task_id);
    });
    state.transfers.lock().unwrap().insert(id.clone(), task);
    Ok(id)
}

#[tauri::command]
fn files_download(
    app: AppHandle,
    state: State<AppState>,
    session_id: String,
    paths: Vec<String>,
    local_dir: Option<String>,
) -> AppResult<String> {
    state.touch();
    let dir = match local_dir {
        Some(d) => std::path::PathBuf::from(d),
        // Fall back to ~/Downloads when the OS has no Downloads folder configured.
        None => match app.path().download_dir() {
            Ok(d) => d,
            Err(_) => {
                let d = app
                    .path()
                    .home_dir()
                    .map_err(|_| AppError::Other("Could not find a folder to download into. Use \"Download to…\".".into()))?
                    .join("Downloads");
                std::fs::create_dir_all(&d)?;
                d
            }
        },
    };
    let label = match paths.as_slice() {
        [one] => one.rsplit('/').next().unwrap_or(one).to_string(),
        many => format!("{} items", many.len()),
    };
    start_transfer(app, &state, session_id, false, label, move |s, report| {
        Box::pin(async move {
            let t = s.for_transfer().await?;
            let saved = files::download(&*t, &paths, &dir, report).await?;
            Ok(saved.into_iter().map(|p| p.to_string_lossy().into_owned()).collect())
        })
    })
}

#[tauri::command]
fn files_upload(
    app: AppHandle,
    state: State<AppState>,
    session_id: String,
    local_paths: Vec<String>,
    remote_dir: String,
) -> AppResult<String> {
    state.touch();
    let locals: Vec<std::path::PathBuf> = local_paths.iter().map(Into::into).collect();
    let label = match locals.as_slice() {
        [one] => one.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        many => format!("{} items", many.len()),
    };
    start_transfer(app, &state, session_id, true, label, move |s, report| {
        Box::pin(async move {
            let t = s.for_transfer().await?;
            files::upload(&*t, &locals, &remote_dir, report).await?;
            Ok(vec![])
        })
    })
}

#[tauri::command]
fn transfer_cancel(state: State<AppState>, id: String) {
    if let Some(t) = state.transfers.lock().unwrap().remove(&id) {
        t.abort();
    }
}

// ---------------------------------------------------------------- remote desktop (RDP)

#[tauri::command]
fn desktop_open(
    state: State<AppState>,
    host_id: String,
    protocol: Protocol,
    width: u16,
    height: u16,
    on_event: Channel<InvokeResponseBody>,
) -> AppResult<String> {
    state.touch();
    let host = state.store.host(&host_id)?;
    let eff = state.store.effective(&host)?;
    let port = host
        .endpoints
        .iter()
        .find(|e| e.protocol == protocol)
        .and_then(|e| e.port)
        .unwrap_or(protocol.default_port());
    let jump = match &eff.jump_host_id {
        Some(j) => Some(state.ssh_target(j, 1)?),
        None => None,
    };
    let password_credential = |required: bool| -> AppResult<Option<(Credential, String)>> {
        let Some(cred_id) = eff.credential_id.clone() else {
            return if required {
                Err(AppError::Invalid(format!("{} has no credential. Pick one in the host settings.", host.name)))
            } else {
                Ok(None)
            };
        };
        let cred = state.store.credential(&cred_id)?;
        if cred.kind != CredentialKind::Password {
            return Err(AppError::Invalid("Remote desktop needs a password credential, not a key.".into()));
        }
        let secret = state.secret(&cred_id)?.secret;
        Ok(Some((cred, secret)))
    };

    enum Job {
        Rdp(rdp::RdpTarget),
        Vnc(vnc::VncTarget),
    }
    let job = match protocol {
        Protocol::Rdp => {
            let (cred, password) = password_credential(true)?.expect("required");
            let username = eff
                .username
                .clone()
                .or(cred.username)
                .ok_or_else(|| AppError::Invalid(format!("{} has no username.", host.name)))?;
            Job::Rdp(rdp::RdpTarget {
                label: host.name.clone(),
                address: host.address.clone(),
                port,
                username,
                password,
                width: width.clamp(640, 8192),
                height: height.clamp(480, 8192),
                jump,
            })
        }
        Protocol::Vnc => Job::Vnc(vnc::VncTarget {
            label: host.name.clone(),
            address: host.address.clone(),
            port,
            password: password_credential(false)?.map(|(_, p)| p),
            jump,
        }),
        _ => return Err(AppError::Invalid("This protocol has no desktop view.".into())),
    };

    let _ = state.store.touch_host(&host_id, now());
    let session_id = new_id();
    let (tx, rx) = mpsc::unbounded_channel();
    state.desktops.lock().unwrap().insert(session_id.clone(), tx);
    let store = state.store.clone();
    let emit = move |out| {
        let body = match out {
            display::Out::Binary(bytes) => InvokeResponseBody::Raw(bytes),
            display::Out::Event(e) => match serde_json::to_string(&e) {
                Ok(json) => InvokeResponseBody::Json(json),
                Err(_) => return,
            },
        };
        let _ = on_event.send(body);
    };
    // IronRDP's handshake is not `Send`, and decoding is CPU heavy: give each session its own thread.
    std::thread::Builder::new()
        .name(format!("desktop-{}", host.name))
        .spawn(move || match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(rt) => match job {
                Job::Rdp(t) => rt.block_on(rdp::run(t, store, rx, emit)),
                Job::Vnc(t) => rt.block_on(vnc::run(t, store, rx, emit)),
            },
            Err(e) => emit(display::Out::Event(display::DesktopEvent::Error { message: e.to_string() })),
        })?;
    Ok(session_id)
}

#[tauri::command]
fn desktop_input(state: State<AppState>, session_id: String, events: Vec<display::DesktopInput>) {
    state.touch();
    if let Some(tx) = state.desktops.lock().unwrap().get(&session_id) {
        for e in events {
            let _ = tx.send(e);
        }
    }
}

#[tauri::command]
fn desktop_close(state: State<AppState>, session_id: String) {
    if let Some(tx) = state.desktops.lock().unwrap().remove(&session_id) {
        let _ = tx.send(display::DesktopInput::Close);
    }
}

// ---------------------------------------------------------------- tunnels

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TunnelStatus {
    id: String,
    state: &'static str,
    message: Option<String>,
}

#[tauri::command]
fn tunnel_start(app: AppHandle, state: State<AppState>, id: String) -> AppResult<()> {
    state.touch();
    if state.tunnels.lock().unwrap().contains_key(&id) {
        return Ok(());
    }
    let tunnel = state.store.tunnel(&id)?;
    let target = state.ssh_target(&tunnel.host_id, 0)?;
    let store = state.store.clone();
    let tid = id.clone();
    let emit = move |state: &'static str, message: Option<String>| {
        let _ = app.emit("tunnel-status", TunnelStatus { id: tid.clone(), state, message });
    };
    emit("connecting", None);
    let task = tauri::async_runtime::spawn(async move {
        let result = async {
            let conn = ssh::connect(&target, store, &mut Vec::new()).await?;
            emit("open", None);
            let host = tunnel.target_host.clone().unwrap_or_default();
            let port = tunnel.target_port.unwrap_or_default();
            match tunnel.kind {
                TunnelKind::Local => ssh::local_forward(conn, tunnel.bind_port, host, port).await,
                TunnelKind::Remote => ssh::remote_forward(conn, tunnel.bind_port, host, port).await,
                TunnelKind::Socks => ssh::socks_proxy(conn, tunnel.bind_port).await,
            }
        }
        .await;
        emit("closed", result.err().map(|e| e.to_string()));
    });
    state.tunnels.lock().unwrap().insert(id, task);
    Ok(())
}

#[tauri::command]
fn tunnel_stop(app: AppHandle, state: State<AppState>, id: String) {
    if let Some(task) = state.tunnels.lock().unwrap().remove(&id) {
        task.abort();
    }
    let _ = app.emit("tunnel-status", TunnelStatus { id, state: "closed", message: None });
}

#[tauri::command]
fn tunnel_active(state: State<AppState>) -> Vec<String> {
    let mut map = state.tunnels.lock().unwrap();
    map.retain(|_, t| !t.inner().is_finished());
    map.keys().cloned().collect()
}

// ---------------------------------------------------------------- app

// ---------------------------------------------------------------- updates

#[tauri::command]
fn update_status(state: State<AppState>) -> updater::Info {
    state.updater.info()
}

/// Starts a check (and download) in the background; progress arrives as "update-status".
#[tauri::command]
fn update_check(app: AppHandle, state: State<AppState>) {
    let updater = state.updater.clone();
    tauri::async_runtime::spawn(async move { updater.run(&app).await });
}

#[tauri::command]
fn update_install(app: AppHandle, state: State<AppState>) -> AppResult<()> {
    state.updater.install(&app)
}

/// Cleans up after a previous update, then checks shortly after startup and every 12
/// hours while automatic updates are on.
fn spawn_update_checks(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        updater::clean_up().await;
        if !updater::available() {
            return;
        }
        tokio::time::sleep(Duration::from_secs(15)).await;
        loop {
            let state = app.state::<AppState>();
            if state.store.settings().map(|s| s.auto_update).unwrap_or(true) {
                state.updater.run(&app).await;
            }
            tokio::time::sleep(updater::CHECK_EVERY).await;
        }
    });
}

fn spawn_auto_lock(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(20)).await;
            let state = app.state::<AppState>();
            let minutes = state.store.settings().map(|s| s.auto_lock_minutes).unwrap_or(15);
            let idle = state.last_activity.lock().unwrap().elapsed();
            if minutes > 0 && state.vault.is_unlocked() && idle > Duration::from_secs(minutes as u64 * 60) {
                state.vault.lock();
                let _ = app.emit("vault-locked", ());
            }
        }
    });
}

// ---------------------------------------------------------------- passphrase prompts

static APP: std::sync::OnceLock<AppHandle> = std::sync::OnceLock::new();
static PROMPTS: Mutex<Option<HashMap<String, tokio::sync::oneshot::Sender<Option<String>>>>> = Mutex::new(None);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PassphraseRequest {
    id: String,
    host: String,
    key: String,
    /// Set after a wrong passphrase.
    retry: bool,
}

/// Asks the UI for a key passphrase and waits for the answer. `None` means cancelled.
pub(crate) async fn ask_passphrase(host: &str, key: &str, retry: bool) -> Option<String> {
    let app = APP.get()?;
    let id = new_id();
    let (tx, rx) = tokio::sync::oneshot::channel();
    PROMPTS.lock().unwrap().get_or_insert_with(HashMap::new).insert(id.clone(), tx);
    let request = PassphraseRequest { id: id.clone(), host: host.into(), key: key.into(), retry };
    if app.emit("passphrase-request", request).is_err() {
        PROMPTS.lock().unwrap().as_mut()?.remove(&id);
        return None;
    }
    rx.await.ok().flatten()
}

#[tauri::command]
fn passphrase_reply(id: String, passphrase: Option<String>) {
    if let Some(tx) = PROMPTS.lock().unwrap().as_mut().and_then(|p| p.remove(&id)) {
        let _ = tx.send(passphrase);
    }
}

const DB_FILE: &str = "bunnylink.db";

/// Where the database can live: the OS app-data folder, or next to the executable
/// (portable mode, chosen on first run).
struct DbLocations {
    app_data: PathBuf,
    portable: PathBuf,
}

/// Opens the existing database (next to the executable first), or on first run an
/// in-memory store that `vault_create` saves once the user has chosen a location.
fn open_store(app: &tauri::App) -> Result<(Store, Option<DbLocations>), Box<dyn std::error::Error>> {
    let exe = std::env::current_exe()?;
    let locations = DbLocations {
        app_data: app.path().app_data_dir()?,
        portable: exe.parent().ok_or("The executable has no folder")?.to_path_buf(),
    };
    for dir in [&locations.portable, &locations.app_data] {
        let path = dir.join(DB_FILE);
        if path.exists() {
            return Ok((Store::open(&path)?, None));
        }
    }
    Ok((Store::in_memory()?, Some(locations)))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // sspi's network client turns on rustls' aws-lc-rs next to ring, so rustls can no
    // longer pick a provider by itself.
    let _ = rustls::crypto::ring::default_provider().install_default();
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let (store, first_run) = open_store(app)?;
            app.manage(AppState {
                store: Arc::new(store),
                first_run: Mutex::new(first_run),
                vault: Vault::default(),
                updater: Arc::new(updater::Updater::default()),
                terminals: Mutex::new(HashMap::new()),
                files: Mutex::new(HashMap::new()),
                desktops: Mutex::new(HashMap::new()),
                transfers: Mutex::new(HashMap::new()),
                tunnels: Mutex::new(HashMap::new()),
                last_activity: Mutex::new(Instant::now()),
            });
            spawn_auto_lock(app.handle().clone());
            spawn_update_checks(app.handle().clone());
            let _ = APP.set(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            vault_status,
            vault_create,
            vault_unlock,
            vault_lock,
            passphrase_reply,
            update_status,
            update_check,
            update_install,
            get_library,
            get_effective,
            save_folder,
            delete_folder,
            save_host,
            delete_host,
            save_credential,
            delete_credential,
            save_tunnel,
            delete_tunnel,
            save_snippet,
            delete_snippet,
            get_settings,
            save_settings,
            forget_host_key,
            copy_credential,
            import_preview,
            import_apply,
            terminal_open,
            serial_ports,
            terminal_input,
            terminal_resize,
            terminal_close,
            run_command,
            files_open,
            files_list,
            files_mkdir,
            files_rename,
            files_delete,
            files_close,
            files_download,
            files_upload,
            transfer_cancel,
            desktop_open,
            desktop_input,
            desktop_close,
            tunnel_start,
            tunnel_stop,
            tunnel_active,
        ])
        .run(tauri::generate_context!())
        .expect("error while running BunnyLink");
}
