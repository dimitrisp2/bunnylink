// SPDX-FileCopyrightText: 2026 BunnyCloud.IT
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial
//! SSH connections: authentication, jump hosts, interactive terminals, one-shot
//! commands and port forwarding (local, remote and SOCKS5).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client::{self, Handle, Msg, Session};
use russh::keys::{decode_secret_key, HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::{Channel, ChannelMsg, Disconnect};
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use crate::error::{AppError, AppResult};
use crate::model::SecretString;
use crate::store::Store;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone)]
pub enum Auth {
    Password(SecretString),
    /// `ask` names the key when its passphrase is asked for on every connection.
    Key { pem: SecretString, passphrase: Option<SecretString>, ask: Option<String> },
    Agent,
}

/// Everything needed to reach one SSH server, including the hop in front of it.
#[derive(Clone)]
pub struct Target {
    pub label: String,
    pub address: String,
    pub port: u16,
    pub username: String,
    pub auth: Auth,
    pub jump: Option<Box<Target>>,
}

enum KeyCheck {
    Trusted,
    New(String),
    Changed { expected: String, got: String },
}

pub struct Client {
    store: Arc<Store>,
    address: String,
    port: u16,
    check: Arc<Mutex<Option<KeyCheck>>>,
    /// Remote forwards: server-side port -> local destination.
    forwards: Arc<Mutex<HashMap<u32, (String, u16)>>>,
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let fingerprint = match key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.fingerprint(HashAlg::Sha256).to_string(),
            PublicKeyOrCertificate::Certificate(c) => c.public_key().fingerprint(HashAlg::Sha256).to_string(),
        };
        let known = self.store.known_host(&self.address, self.port).ok().flatten();
        let (ok, outcome) = match known {
            Some(expected) if expected == fingerprint => (true, KeyCheck::Trusted),
            Some(expected) => (false, KeyCheck::Changed { expected, got: fingerprint }),
            None => {
                // Trust on first use; the UI shows the fingerprint so the user can verify it.
                let _ = self.store.save_known_host(&self.address, self.port, &fingerprint);
                (true, KeyCheck::New(fingerprint))
            }
        };
        *self.check.lock().unwrap() = Some(outcome);
        Ok(ok)
    }

    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: Channel<Msg>,
        _connected_address: &str,
        connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: client::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        let dest = self.forwards.lock().unwrap().get(&connected_port).cloned();
        reply.accept().await;
        if let Some((host, port)) = dest {
            tokio::spawn(async move {
                if let Ok(mut sock) = TcpStream::connect((host.as_str(), port)).await {
                    let mut stream = channel.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut sock, &mut stream).await;
                }
            });
        }
        Ok(())
    }
}

/// A live SSH connection. Holds the jump connection (if any) so it stays open.
pub struct Conn {
    pub handle: Handle<Client>,
    forwards: Arc<Mutex<HashMap<u32, (String, u16)>>>,
    _jump: Option<Box<Conn>>,
}

impl Conn {
    pub async fn close(&self) {
        let _ = self.handle.disconnect(Disconnect::ByApplication, "", "en").await;
    }
}

type DynAgent = russh::keys::agent::client::AgentClient<Box<dyn russh::keys::agent::client::AgentStream + Send + Unpin>>;

#[cfg(windows)]
const NO_AGENT: &str = "No SSH agent is running: the Windows OpenSSH agent isn't running and Pageant isn't open.";
#[cfg(unix)]
const NO_AGENT: &str = "No SSH agent is running (SSH_AUTH_SOCK is not set or not reachable).";

/// Connects to the user's SSH agent: `SSH_AUTH_SOCK` on Unix; on Windows the OpenSSH
/// agent's pipe, then Pageant. The flag says whether it is Pageant.
async fn open_agent() -> AppResult<(DynAgent, bool)> {
    use russh::keys::agent::client::AgentClient;
    #[cfg(unix)]
    {
        AgentClient::connect_env().await.map(|a| (a.dynamic(), false)).map_err(|_| AppError::Other(NO_AGENT.into()))
    }
    #[cfg(windows)]
    {
        if let Ok(a) = AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await {
            return Ok((a.dynamic(), false));
        }
        AgentClient::connect_pageant().await.map(|a| (a.dynamic(), true)).map_err(|_| AppError::Other(NO_AGENT.into()))
    }
}

/// Tries each key the agent holds until the server accepts one.
async fn agent_auth(handle: &mut Handle<Client>, user: &str, label: &str) -> AppResult<bool> {
    let (mut agent, pageant) = open_agent().await?;
    // Connecting to Pageant always succeeds; it only fails on the first request when
    // Pageant is not open.
    let identities = agent.request_identities().await.map_err(|e| {
        AppError::Other(if pageant { NO_AGENT.into() } else { format!("The SSH agent did not list its keys: {e}") })
    })?;
    if identities.is_empty() {
        return Err(AppError::Other("The SSH agent has no keys loaded. Add one with ssh-add.".into()));
    }
    let hash = handle.best_supported_rsa_hash().await?.flatten();
    for id in identities {
        let key = id.public_key().into_owned();
        match handle.authenticate_publickey_with(user, key, hash, &mut agent).await {
            Ok(r) if r.success() => return Ok(true),
            Ok(_) => continue,
            Err(e) => return Err(AppError::Other(format!("SSH agent signing failed for {label}: {e:?}"))),
        }
    }
    Ok(false)
}

type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// How a key passphrase is asked for when a credential doesn't store it.
pub trait Prompter: Send {
    /// `None` means the user cancelled. `retry` is set after a wrong passphrase.
    fn passphrase<'a>(&'a mut self, host: &'a str, key: &'a str, retry: bool) -> BoxFuture<'a, Option<SecretString>>;
}

/// Asks in a dialog in the main window.
pub struct DialogPrompter;

impl Prompter for DialogPrompter {
    fn passphrase<'a>(&'a mut self, host: &'a str, key: &'a str, retry: bool) -> BoxFuture<'a, Option<SecretString>> {
        Box::pin(crate::ask_passphrase(host, key, retry))
    }
}

/// Asks inside the terminal that is connecting, like OpenSSH: nothing is echoed,
/// Backspace and Ctrl+U edit, Enter confirms, Ctrl+C cancels.
pub struct TerminalPrompter<'a, F: Fn(TermEvent) + Send + Sync> {
    pub input: &'a mut mpsc::UnboundedReceiver<TermInput>,
    pub emit: &'a F,
    /// The latest size the terminal was resized to while waiting.
    pub size: (u32, u32),
}

impl<F: Fn(TermEvent) + Send + Sync> Prompter for TerminalPrompter<'_, F> {
    fn passphrase<'a>(&'a mut self, _host: &'a str, key: &'a str, retry: bool) -> BoxFuture<'a, Option<SecretString>> {
        Box::pin(async move {
            let say = |text: String| (self.emit)(TermEvent::Prompt { text });
            if retry {
                say("Wrong passphrase, try again.\r\n".into());
            }
            say(format!("Enter passphrase for key '{key}': "));
            // Wiped when the prompt returns, like the passphrase built from it.
            let mut line = zeroize::Zeroizing::new(Vec::<u8>::with_capacity(256));
            loop {
                match self.input.recv().await {
                    Some(TermInput::Data(d)) => {
                        // Escape sequences (arrow keys and the like) arrive whole; skip them.
                        if d.first() == Some(&0x1b) {
                            continue;
                        }
                        for b in d {
                            match b {
                                b'\r' | b'\n' => {
                                    say("\r\n".into());
                                    return Some(String::from_utf8_lossy(&line).into_owned().into());
                                }
                                0x03 => {
                                    say("^C\r\n".into());
                                    return None;
                                }
                                0x7f | 0x08 => {
                                    // Drop one whole UTF-8 character.
                                    while let Some(last) = line.pop() {
                                        if last & 0xC0 != 0x80 {
                                            break;
                                        }
                                    }
                                }
                                0x15 => line.clear(),
                                b if b < 0x20 => {}
                                b => line.push(b),
                            }
                        }
                    }
                    Some(TermInput::Resize { cols, rows }) => self.size = (cols, rows),
                    Some(TermInput::Close) | None => return None,
                }
            }
        })
    }
}

/// Runs one network step of a connection with a time limit. Waiting for the user (a
/// passphrase prompt) is not a network step, so it is never cut short.
async fn timed<T>(label: &str, step: impl std::future::Future<Output = AppResult<T>>) -> AppResult<T> {
    tokio::time::timeout(CONNECT_TIMEOUT, step)
        .await
        .unwrap_or_else(|_| Err(AppError::Other(format!("Timed out connecting to {label}."))))
}

/// Tries to read a key with passphrases from `prompter`, up to three times.
async fn ask_key(prompter: &mut dyn Prompter, pem: &str, host: &str, key_name: &str) -> AppResult<russh::keys::PrivateKey> {
    for attempt in 0..3 {
        let passphrase = prompter
            .passphrase(host, key_name, attempt > 0)
            .await
            .ok_or_else(|| AppError::Other(format!("Cancelled: no passphrase for {key_name}.")))?;
        if let Ok(key) = decode_secret_key(pem, Some(passphrase.as_str())) {
            return Ok(key);
        }
    }
    Err(AppError::Other(format!("Wrong passphrase for the key of {host}.")))
}

/// Connects (through any jump hosts) and authenticates, asking for key passphrases in
/// a dialog. Messages worth showing the user, such as a newly trusted host key, are
/// appended to `notices`.
pub async fn connect(target: &Target, store: Arc<Store>, notices: &mut Vec<String>) -> AppResult<Conn> {
    connect_with(target, store, notices, &mut DialogPrompter).await
}

/// Like [`connect`], asking for key passphrases through `prompter`.
pub async fn connect_with(
    target: &Target,
    store: Arc<Store>,
    notices: &mut Vec<String>,
    prompter: &mut dyn Prompter,
) -> AppResult<Conn> {
    connect_inner(target, store, notices, prompter).await
}

fn connect_inner<'a>(
    target: &'a Target,
    store: Arc<Store>,
    notices: &'a mut Vec<String>,
    prompter: &'a mut dyn Prompter,
) -> BoxFuture<'a, AppResult<Conn>> {
    Box::pin(async move {
        let config = Arc::new(client::Config {
            keepalive_interval: Some(Duration::from_secs(30)),
            keepalive_max: 3,
            nodelay: true,
            ..Default::default()
        });
        let check = Arc::new(Mutex::new(None));
        let forwards = Arc::new(Mutex::new(HashMap::new()));
        let handler = Client {
            store: store.clone(),
            address: target.address.clone(),
            port: target.port,
            check: check.clone(),
            forwards: forwards.clone(),
        };

        let label = target.label.as_str();
        let wrap = |r: Result<Handle<Client>, russh::Error>| Ok::<_, AppError>(r);
        let (result, jump) = match &target.jump {
            None => (timed(label, async { wrap(client::connect(config, (target.address.as_str(), target.port), handler).await) }).await?, None),
            Some(j) => {
                let jump = connect_inner(j, store.clone(), notices, prompter).await?;
                notices.push(format!("Connected to jump host {}.", j.label));
                let ch = timed(label, async {
                    jump.handle
                        .channel_open_direct_tcpip(target.address.clone(), target.port as u32, "127.0.0.1", 0)
                        .await
                        .map_err(|e| AppError::Other(format!("Jump host {} could not reach {}: {e}", j.label, target.label)))
                })
                .await?;
                (timed(label, async { wrap(client::connect_stream(config, ch.into_stream(), handler).await) }).await?, Some(Box::new(jump)))
            }
        };

        let outcome = check.lock().unwrap().take();
        let mut handle = match (result, outcome) {
            (_, Some(KeyCheck::Changed { expected, got })) => {
                return Err(AppError::Other(format!(
                    "The host key for {} has changed. Expected {expected}, got {got}. \
                     If the server was reinstalled, forget the old key and try again.",
                    target.label
                )))
            }
            (Err(e), _) => return Err(AppError::Other(format!("Could not connect to {}: {e}", target.label))),
            (Ok(h), outcome) => {
                if let Some(KeyCheck::New(fp)) = outcome {
                    notices.push(format!("Trusted new host key for {}: {fp}", target.label));
                }
                h
            }
        };

        let ok = match &target.auth {
            Auth::Password(pw) => {
                timed(label, async { Ok(handle.authenticate_password(&target.username, pw.as_str()).await?.success()) }).await?
            }
            Auth::Agent => timed(label, agent_auth(&mut handle, &target.username, label)).await?,
            Auth::Key { pem, passphrase, ask } => {
                let key = match ask {
                    Some(key_name) => ask_key(prompter, pem, label, key_name).await?,
                    None => decode_secret_key(pem, passphrase.as_ref().map(|p| p.as_str())).map_err(|e| match e {
                        russh::keys::Error::KeyIsEncrypted => {
                            AppError::Other(format!("The key for {label} needs a passphrase. Enter it in the credential."))
                        }
                        _ if passphrase.is_some() => AppError::Other(format!("Wrong passphrase for the key of {label}.")),
                        e => AppError::Key(e),
                    })?,
                };
                timed(label, async {
                    let hash = handle.best_supported_rsa_hash().await?.flatten();
                    Ok(handle
                        .authenticate_publickey(&target.username, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
                        .await?
                        .success())
                })
                .await?
            }
        };
        if !ok {
            return Err(AppError::Other(format!(
                "{} rejected the login for user \"{}\".",
                target.label, target.username
            )));
        }
        Ok(Conn { handle, forwards, _jump: jump })
    })
}

// ---------------------------------------------------------------- tunnelled streams

/// Opens a TCP stream to `host:port` as seen from the SSH server `jump`, for protocols
/// such as RDP and VNC that should go through a bastion. The SSH connection lives as
/// long as the returned stream.
pub async fn tunnel_stream(
    jump: &Target,
    store: Arc<Store>,
    host: &str,
    port: u16,
    notices: &mut Vec<String>,
) -> AppResult<tokio::io::DuplexStream> {
    let conn = connect(jump, store, notices).await?;
    let ch = conn
        .handle
        .channel_open_direct_tcpip(host.to_string(), port as u32, "127.0.0.1", 0)
        .await
        .map_err(|e| AppError::Other(format!("Jump host {} could not reach {host}:{port}: {e}", jump.label)))?;
    notices.push(format!("Connected through jump host {}.", jump.label));
    let (mut ours, theirs) = tokio::io::duplex(256 * 1024);
    tokio::spawn(async move {
        let mut stream = ch.into_stream();
        let _ = tokio::io::copy_bidirectional(&mut ours, &mut stream).await;
        conn.close().await;
    });
    Ok(theirs)
}

/// A local listener on 127.0.0.1 that forwards each connection to `host:port` through
/// an SSH connection. Stops when dropped.
pub struct Forward {
    pub addr: std::net::SocketAddr,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Forward {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub async fn forward(conn: Arc<Conn>, host: String, port: u16, once: bool) -> AppResult<Forward> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let addr = listener.local_addr()?;
    let task = tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            let conn = conn.clone();
            let host = host.clone();
            let pipe = async move {
                if let Ok(ch) = conn.handle.channel_open_direct_tcpip(host, port as u32, "127.0.0.1", 0).await {
                    let mut stream = ch.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut sock, &mut stream).await;
                }
            };
            if once {
                pipe.await;
                break;
            }
            tokio::spawn(pipe);
        }
    });
    Ok(Forward { addr, task })
}

// ---------------------------------------------------------------- terminal

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TermEvent {
    Notice { text: String },
    /// Text written as-is while the connection waits for the user (passphrase prompt).
    Prompt { text: String },
    Data { data: Vec<u8> },
    Exit { code: Option<u32> },
    Error { message: String },
}

pub enum TermInput {
    Data(Vec<u8>),
    Resize { cols: u32, rows: u32 },
    Close,
}

/// Runs an interactive shell until the server closes it or `Close` is received.
pub async fn run_terminal(
    conn: Conn,
    cols: u32,
    rows: u32,
    mut input: mpsc::UnboundedReceiver<TermInput>,
    emit: impl Fn(TermEvent) + Send + 'static,
) {
    let result: AppResult<Option<u32>> = async {
        let channel = conn.handle.channel_open_session().await?;
        channel.request_pty(false, "xterm-256color", cols, rows, 0, 0, &[]).await?;
        channel.request_shell(false).await?;
        let (mut read, write) = channel.split();
        let mut code = None;
        loop {
            tokio::select! {
                cmd = input.recv() => match cmd {
                    Some(TermInput::Data(d)) => write.data_bytes(d).await?,
                    Some(TermInput::Resize { cols, rows }) => write.window_change(cols, rows, 0, 0).await?,
                    Some(TermInput::Close) | None => break,
                },
                msg = read.wait() => match msg {
                    Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => {
                        emit(TermEvent::Data { data: data.to_vec() })
                    }
                    Some(ChannelMsg::ExitStatus { exit_status }) => code = Some(exit_status),
                    Some(ChannelMsg::Close) | None => break,
                    _ => {}
                },
            }
        }
        Ok(code)
    }
    .await;
    conn.close().await;
    match result {
        Ok(code) => emit(TermEvent::Exit { code }),
        Err(e) => emit(TermEvent::Error { message: e.to_string() }),
    }
}

// ---------------------------------------------------------------- run command

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecResult {
    pub output: String,
    pub exit_code: Option<u32>,
}

pub async fn exec(conn: Conn, command: &str) -> AppResult<ExecResult> {
    let mut channel = conn.handle.channel_open_session().await?;
    channel.exec(true, command).await?;
    let mut out = Vec::new();
    let mut exit_code = None;
    while let Some(msg) = channel.wait().await {
        match msg {
            ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => out.extend_from_slice(&data),
            ChannelMsg::ExitStatus { exit_status } => exit_code = Some(exit_status),
            ChannelMsg::Close => break,
            _ => {}
        }
    }
    conn.close().await;
    Ok(ExecResult { output: String::from_utf8_lossy(&out).into_owned(), exit_code })
}

// ---------------------------------------------------------------- tunnels

/// Forwards 127.0.0.1:`bind_port` to `target_host:target_port` as seen from the server.
pub async fn local_forward(conn: Conn, bind_port: u16, target_host: String, target_port: u16) -> AppResult<()> {
    let listener = TcpListener::bind(("127.0.0.1", bind_port)).await?;
    let conn = Arc::new(conn);
    loop {
        let (mut sock, peer) = listener.accept().await?;
        let conn = conn.clone();
        let host = target_host.clone();
        tokio::spawn(async move {
            if let Ok(ch) = conn
                .handle
                .channel_open_direct_tcpip(host, target_port as u32, peer.ip().to_string(), peer.port() as u32)
                .await
            {
                let mut stream = ch.into_stream();
                let _ = tokio::io::copy_bidirectional(&mut sock, &mut stream).await;
            }
        });
    }
}

/// Asks the server to listen on `bind_port` and forwards connections to a local destination.
pub async fn remote_forward(conn: Conn, bind_port: u16, target_host: String, target_port: u16) -> AppResult<()> {
    conn.forwards.lock().unwrap().insert(bind_port as u32, (target_host, target_port));
    conn.handle.tcpip_forward("127.0.0.1", bind_port as u32).await?;
    // Keep the connection alive until the task is aborted or the server goes away.
    while !conn.handle.is_closed() {
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err(AppError::Other("The SSH connection closed.".into()))
}

/// A SOCKS5 proxy (CONNECT, no auth) on 127.0.0.1:`bind_port` that dials out from the server.
pub async fn socks_proxy(conn: Conn, bind_port: u16) -> AppResult<()> {
    let listener = TcpListener::bind(("127.0.0.1", bind_port)).await?;
    let conn = Arc::new(conn);
    loop {
        let (sock, peer) = listener.accept().await?;
        let conn = conn.clone();
        tokio::spawn(async move {
            let _ = socks_session(conn, sock, peer).await;
        });
    }
}

async fn socks_session(conn: Arc<Conn>, mut sock: TcpStream, peer: std::net::SocketAddr) -> std::io::Result<()> {
    let mut head = [0u8; 2];
    sock.read_exact(&mut head).await?;
    if head[0] != 5 {
        return Ok(());
    }
    let mut methods = vec![0u8; head[1] as usize];
    sock.read_exact(&mut methods).await?;
    if !methods.contains(&0) {
        sock.write_all(&[5, 0xff]).await?;
        return Ok(());
    }
    sock.write_all(&[5, 0]).await?;

    let mut req = [0u8; 4];
    sock.read_exact(&mut req).await?;
    if req[1] != 1 {
        sock.write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0]).await?; // command not supported
        return Ok(());
    }
    let host = match req[3] {
        1 => {
            let mut a = [0u8; 4];
            sock.read_exact(&mut a).await?;
            std::net::Ipv4Addr::from(a).to_string()
        }
        3 => {
            let len = sock.read_u8().await? as usize;
            let mut name = vec![0u8; len];
            sock.read_exact(&mut name).await?;
            String::from_utf8_lossy(&name).into_owned()
        }
        4 => {
            let mut a = [0u8; 16];
            sock.read_exact(&mut a).await?;
            std::net::Ipv6Addr::from(a).to_string()
        }
        _ => {
            sock.write_all(&[5, 8, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
            return Ok(());
        }
    };
    let port = sock.read_u16().await?;

    match conn
        .handle
        .channel_open_direct_tcpip(host, port as u32, peer.ip().to_string(), peer.port() as u32)
        .await
    {
        Ok(ch) => {
            sock.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
            let mut stream = ch.into_stream();
            tokio::io::copy_bidirectional(&mut sock, &mut stream).await?;
        }
        Err(_) => sock.write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0]).await?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Live tests against a real SSH server. They are skipped unless
    //! `BUNNYLINK_TEST_SSH=host:port:user:password` is set.

    use super::*;

    fn target() -> Option<Target> {
        let spec = std::env::var("BUNNYLINK_TEST_SSH").ok()?;
        let mut p = spec.splitn(4, ':');
        Some(Target {
            label: "test".into(),
            address: p.next()?.into(),
            port: p.next()?.parse().ok()?,
            username: p.next()?.into(),
            auth: Auth::Password(p.next()?.to_string().into()),
            jump: None,
        })
    }

    async fn banner(port: u16) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let mut buf = [0u8; 7];
        s.read_exact(&mut buf).await.unwrap();
        String::from_utf8_lossy(&buf).into()
    }

    #[tokio::test]
    async fn live_exec_forward_socks_and_jump() {
        let Some(t) = target() else { return };
        let store = Arc::new(Store::in_memory().unwrap());

        // First connection trusts the key; exec works.
        let mut notices = Vec::new();
        let conn = connect(&t, store.clone(), &mut notices).await.unwrap();
        assert!(notices.iter().any(|n| n.contains("Trusted new host key")));
        let r = exec(conn, "echo bunny").await.unwrap();
        assert_eq!(r.output.trim(), "bunny");
        assert_eq!(r.exit_code, Some(0));

        // Changed host key is refused.
        store.save_known_host(&t.address, t.port, "SHA256:not-the-key").unwrap();
        assert!(connect(&t, store.clone(), &mut Vec::new()).await.is_err());
        store.forget_known_host(&t.address, t.port).unwrap();

        // Local forward to the SSH server itself returns its banner.
        let conn = connect(&t, store.clone(), &mut Vec::new()).await.unwrap();
        let (addr, port) = (t.address.clone(), t.port);
        let fwd = tokio::spawn(local_forward(conn, 47001, addr.clone(), port));
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(banner(47001).await, "SSH-2.0");
        fwd.abort();

        // SOCKS5 CONNECT to the SSH server.
        let conn = connect(&t, store.clone(), &mut Vec::new()).await.unwrap();
        let socks = tokio::spawn(socks_proxy(conn, 47002));
        tokio::time::sleep(Duration::from_millis(300)).await;
        let mut s = TcpStream::connect(("127.0.0.1", 47002)).await.unwrap();
        s.write_all(&[5, 1, 0]).await.unwrap();
        let mut r2 = [0u8; 2];
        s.read_exact(&mut r2).await.unwrap();
        assert_eq!(r2, [5, 0]);
        let ip: std::net::Ipv4Addr = addr.parse().unwrap();
        let mut req = vec![5, 1, 0, 1];
        req.extend_from_slice(&ip.octets());
        req.extend_from_slice(&port.to_be_bytes());
        s.write_all(&req).await.unwrap();
        let mut r10 = [0u8; 10];
        s.read_exact(&mut r10).await.unwrap();
        assert_eq!(r10[1], 0);
        let mut b = [0u8; 7];
        s.read_exact(&mut b).await.unwrap();
        assert_eq!(&b, b"SSH-2.0");
        socks.abort();

        // Remote forward: the server listens and connections come back to us.
        // (The test server is local, so its loopback is ours.)
        let conn = connect(&t, store.clone(), &mut Vec::new()).await.unwrap();
        let remote = tokio::spawn(remote_forward(conn, 47003, addr.clone(), port));
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(banner(47003).await, "SSH-2.0");
        remote.abort();

        // Jump host: reach the server through itself.
        let mut via = target().unwrap();
        via.jump = Some(Box::new(target().unwrap()));
        let conn = connect(&via, store.clone(), &mut Vec::new()).await.unwrap();
        assert_eq!(exec(conn, "echo hop").await.unwrap().output.trim(), "hop");

        // Interactive shell echoes input.
        let conn = connect(&t, store.clone(), &mut Vec::new()).await.unwrap();
        let (tx, rx) = mpsc::unbounded_channel();
        let (etx, mut erx) = mpsc::unbounded_channel();
        let term = tokio::spawn(run_terminal(conn, 80, 24, rx, move |e| {
            let _ = etx.send(e);
        }));
        tx.send(TermInput::Data(b"echo term-$((40+2))\n".to_vec())).unwrap();
        let mut seen = String::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !seen.contains("term-42") && tokio::time::Instant::now() < deadline {
            if let Ok(Some(TermEvent::Data { data })) = tokio::time::timeout(Duration::from_millis(500), erx.recv()).await {
                seen.push_str(&String::from_utf8_lossy(&data));
            }
        }
        assert!(seen.contains("term-42"), "terminal output: {seen}");
        tx.send(TermInput::Close).unwrap();
        term.await.unwrap();
    }

    /// Agent login, skipped unless `BUNNYLINK_TEST_SSH` and `SSH_AUTH_SOCK` are set and
    /// `BUNNYLINK_TEST_AGENT=1` (the agent's key must be authorised for the user).
    #[tokio::test]
    async fn live_agent_login() {
        if std::env::var("BUNNYLINK_TEST_AGENT").is_err() {
            return;
        }
        let Some(mut t) = target() else { return };
        t.auth = Auth::Agent;
        let store = Arc::new(Store::in_memory().unwrap());
        let conn = connect(&t, store, &mut Vec::new()).await.unwrap();
        assert_eq!(exec(conn, "echo agent-ok").await.unwrap().output.trim(), "agent-ok");
    }
}
