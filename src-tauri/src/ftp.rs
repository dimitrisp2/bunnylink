// SPDX-FileCopyrightText: 2026 BunnyCloud.IT
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial
//! FTP backend. Uses explicit TLS (FTPS) when the server offers it, trusting the
//! certificate on first use like SSH host keys; falls back to plain FTP otherwise.

use std::path::Path;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::UNIX_EPOCH;

use async_trait::async_trait;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};
use suppaftp::list::{File as ListFile, PosixPexQuery};
use suppaftp::tokio::{AsyncRustlsConnector, AsyncRustlsFtpStream};
use suppaftp::types::FileType;
use suppaftp::FtpError;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;

use crate::error::{AppError, AppResult};
use crate::files::{base_name, join, sort_entries, Entry, Listing, RemoteFs, Report};
use crate::ssh;
use crate::store::Store;

#[derive(Clone)]
pub struct FtpTarget {
    pub label: String,
    pub address: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    /// SSH server to tunnel the control and data connections through.
    pub jump: Option<ssh::Target>,
}

/// Keeps a jump-host tunnel open for as long as the FTP session lives.
type Tunnel = Option<(Arc<ssh::Conn>, ssh::Forward)>;

pub struct Ftp {
    target: FtpTarget,
    store: Arc<Store>,
    _tunnel: Tunnel,
    ctrl: Mutex<AsyncRustlsFtpStream>,
    /// Whether the server supports machine-readable listings (MLSD).
    mlsd: bool,
}

impl From<FtpError> for AppError {
    fn from(e: FtpError) -> Self {
        AppError::Other(format!("FTP: {e}"))
    }
}

/// Accepts any certificate but records its fingerprint, which is then checked against
/// the trusted one (trust on first use). Signatures are still verified.
#[derive(Debug)]
struct Tofu {
    provider: Arc<rustls::crypto::CryptoProvider>,
    seen: StdMutex<Option<String>>,
}

impl ServerCertVerifier for Tofu {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let fp = Sha256::digest(end_entity.as_ref()).iter().map(|b| format!("{b:02x}")).collect::<String>();
        *self.seen.lock().unwrap() = Some(format!("SHA256:{fp}"));
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

fn mode_bits(f: &ListFile) -> Option<u32> {
    let mut m = 0;
    for (i, who) in [PosixPexQuery::Owner, PosixPexQuery::Group, PosixPexQuery::Others].into_iter().enumerate() {
        let shift = 6 - i as u32 * 3;
        if f.can_read(who) {
            m |= 4 << shift;
        }
        if f.can_write(who) {
            m |= 2 << shift;
        }
        if f.can_execute(who) {
            m |= 1 << shift;
        }
    }
    (m != 0).then_some(m)
}

impl Ftp {
    async fn open_stream(
        t: &FtpTarget,
        store: &Arc<Store>,
        notices: &mut Vec<String>,
    ) -> AppResult<(AsyncRustlsFtpStream, bool, bool, Tunnel)> {
        // Through a jump host, the control connection goes via a local forward and every
        // passive data connection gets its own one-shot forward to the same server.
        let tunnel: Tunnel = match &t.jump {
            Some(j) => {
                let conn = Arc::new(ssh::connect(j, store.clone(), notices).await?);
                let fwd = ssh::forward(conn.clone(), t.address.clone(), t.port, false).await?;
                notices.push(format!("Connected through jump host {}.", j.label));
                Some((conn, fwd))
            }
            None => None,
        };
        let addr: std::net::SocketAddr = match &tunnel {
            Some((_, fwd)) => fwd.addr,
            None => tokio::net::lookup_host((t.address.as_str(), t.port))
                .await
                .map_err(|e| AppError::Other(format!("Could not resolve {}: {e}", t.address)))?
                .next()
                .ok_or_else(|| AppError::Other(format!("Could not resolve {}.", t.address)))?,
        };
        let data_conn = tunnel.as_ref().map(|(c, _)| c.clone());
        let address = t.address.clone();
        let with_tunnel = move |s: AsyncRustlsFtpStream| match data_conn.clone() {
            None => s,
            Some(conn) => {
                let address = address.clone();
                s.passive_stream_builder(move |data: std::net::SocketAddr| {
                    let conn = conn.clone();
                    let address = address.clone();
                    Box::pin(async move {
                        let fwd = ssh::forward(conn, address, data.port(), true)
                            .await
                            .map_err(|e| FtpError::ConnectionError(std::io::Error::other(e.to_string())))?;
                        let stream = tokio::net::TcpStream::connect(fwd.addr).await.map_err(FtpError::ConnectionError)?;
                        // The one-shot forward has accepted; let it run on its own.
                        std::mem::forget(fwd);
                        Ok(stream)
                    })
                })
            }
        };
        let connect = || async {
            tokio::time::timeout(std::time::Duration::from_secs(20), AsyncRustlsFtpStream::connect(addr))
                .await
                .map_err(|_| AppError::Other(format!("Timed out connecting to {}.", t.label)))?
                .map(&with_tunnel)
                .map_err(|e| AppError::Other(format!("Could not connect to {}: {e}", t.label)))
        };

        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let tofu = Arc::new(Tofu { provider: provider.clone(), seen: StdMutex::new(None) });
        let config = rustls::ClientConfig::builder_with_provider(provider)
            // FTPS data connections are separate TLS sessions that resume the control one;
            // several common servers (vsftpd among them) fail uploads over TLS 1.3.
            .with_protocol_versions(&[&rustls::version::TLS12])
            .map_err(|e| AppError::Other(e.to_string()))?
            .dangerous()
            .with_custom_certificate_verifier(tofu.clone())
            .with_no_client_auth();
        let connector = AsyncRustlsConnector::from(tokio_rustls::TlsConnector::from(Arc::new(config)));

        let (mut stream, secure) = match connect().await?.into_secure(connector, &t.address).await {
            Ok(s) => {
                let fp = tofu.seen.lock().unwrap().clone().unwrap_or_default();
                let key = format!("ftps:{}", t.address);
                match store.known_host(&key, t.port)? {
                    Some(known) if known == fp => {}
                    Some(known) => {
                        return Err(AppError::Other(format!(
                            "The certificate for {} has changed. Expected {known}, got {fp}. \
                             If the server was reinstalled, forget the old key and try again.",
                            t.label
                        )))
                    }
                    None => {
                        store.save_known_host(&key, t.port, &fp)?;
                        notices.push(format!("Trusted new certificate for {}: {fp}", t.label));
                    }
                }
                (s, true)
            }
            // The server does not do TLS: reconnect without it.
            Err(FtpError::UnexpectedResponse(_)) => {
                notices.push(format!("{} does not support FTPS. This connection is not encrypted.", t.label));
                (connect().await?, false)
            }
            Err(e) => return Err(AppError::Other(format!("FTPS handshake with {} failed: {e}", t.label))),
        };
        stream.set_passive_nat_workaround(true);
        let user = t.username.clone().unwrap_or_else(|| "anonymous".into());
        let pass = t.password.clone().unwrap_or_else(|| "anonymous@".into());
        stream.login(&user, &pass).await.map_err(|e| match e {
            FtpError::UnexpectedResponse(_) => AppError::Other(format!("{} rejected the login for \"{user}\".", t.label)),
            other => other.into(),
        })?;
        stream.transfer_type(FileType::Binary).await?;
        // Only use MLSD when advertised: some servers open the data connection and then
        // reject the command, leaving the client waiting.
        let mlsd = stream.feat().await.map(|f| f.contains_key("MLSD") || f.contains_key("MLST")).unwrap_or(false);
        Ok((stream, secure, mlsd, tunnel))
    }

    pub async fn open(target: FtpTarget, store: Arc<Store>, notices: &mut Vec<String>) -> AppResult<Self> {
        let (stream, _secure, mlsd, tunnel) = Self::open_stream(&target, &store, notices).await?;
        Ok(Self { target, store, _tunnel: tunnel, ctrl: Mutex::new(stream), mlsd })
    }

    async fn raw_list(&self, path: &str) -> AppResult<Vec<ListFile>> {
        let mut c = self.ctrl.lock().await;
        if self.mlsd {
            let lines = c.mlsd(Some(path)).await?;
            return Ok(lines.iter().filter_map(|l| suppaftp::list::ListParser::parse_mlsd(l).ok()).collect());
        }
        let lines = c.list(Some(path)).await?;
        Ok(lines.iter().filter_map(|l| l.parse::<ListFile>().ok()).collect())
    }
}

#[async_trait]
impl RemoteFs for Ftp {
    async fn home(&self) -> AppResult<String> {
        Ok(self.ctrl.lock().await.pwd().await?)
    }

    async fn list(&self, path: &str) -> AppResult<Listing> {
        let path = if path.is_empty() { "/".to_string() } else { path.to_string() };
        let mut entries: Vec<Entry> = self
            .raw_list(&path)
            .await?
            .into_iter()
            .filter(|f| f.name() != "." && f.name() != "..")
            .map(|f| Entry {
                name: f.name().to_string(),
                path: join(&path, f.name()),
                is_dir: f.is_directory(),
                is_link: f.is_symlink(),
                size: f.size() as u64,
                modified: f.modified().duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs() as u32),
                permissions: mode_bits(&f),
            })
            .collect();
        sort_entries(&mut entries);
        Ok(Listing { path, entries })
    }

    async fn stat(&self, path: &str) -> AppResult<(bool, u64)> {
        let trimmed = path.trim_end_matches('/');
        if trimmed.is_empty() {
            return Ok((true, 0));
        }
        let parent = match trimmed.rsplit_once('/') {
            Some(("", _)) | None => "/".to_string(),
            Some((p, _)) => p.to_string(),
        };
        let name = base_name(trimmed);
        self.list(&parent)
            .await?
            .entries
            .into_iter()
            .find(|e| e.name == name)
            .map(|e| (e.is_dir, e.size))
            .ok_or_else(|| AppError::NotFound("File"))
    }

    async fn exists(&self, path: &str) -> AppResult<bool> {
        match self.stat(path).await {
            Ok(_) => Ok(true),
            Err(AppError::NotFound(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }

    async fn mkdir(&self, path: &str) -> AppResult<()> {
        Ok(self.ctrl.lock().await.mkdir(path).await?)
    }

    async fn rename(&self, from: &str, to: &str) -> AppResult<()> {
        Ok(self.ctrl.lock().await.rename(from, to).await?)
    }

    async fn remove_file(&self, path: &str) -> AppResult<()> {
        Ok(self.ctrl.lock().await.rm(path).await?)
    }

    async fn remove_dir(&self, path: &str) -> AppResult<()> {
        Ok(self.ctrl.lock().await.rmdir(path).await?)
    }

    async fn get(&self, remote: &str, local: &Path, report: Report<'_>) -> AppResult<()> {
        let mut c = self.ctrl.lock().await;
        let mut src = c.retr_as_stream(remote).await?;
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
        src.finish().await?;
        Ok(())
    }

    async fn put(&self, local: &Path, remote: &str, report: Report<'_>) -> AppResult<()> {
        let mut c = self.ctrl.lock().await;
        let mut src = tokio::fs::File::open(local).await?;
        let mut dst = c.put_with_stream(remote).await?;
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            let n = src.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            dst.write_all(&buf[..n]).await?;
            report(n as u64);
        }
        dst.finish().await?;
        Ok(())
    }

    async fn for_transfer(self: Arc<Self>) -> AppResult<Arc<dyn RemoteFs>> {
        Ok(Arc::new(Ftp::open(self.target.clone(), self.store.clone(), &mut Vec::new()).await?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live test, skipped unless `BUNNYLINK_TEST_FTP=host:port:user:password` is set.
    #[tokio::test]
    async fn live_browse_and_transfer() {
        let Ok(spec) = std::env::var("BUNNYLINK_TEST_FTP") else { return };
        let p: Vec<&str> = spec.splitn(4, ':').collect();
        let target = FtpTarget {
            label: "test".into(),
            address: p[0].into(),
            port: p[1].parse().unwrap(),
            username: Some(p[2].into()),
            password: Some(p[3].into()),
            jump: None,
        };
        // Through a jump host as well, when an SSH test server is configured.
        if let Ok(ssh_spec) = std::env::var("BUNNYLINK_TEST_SSH") {
            let s: Vec<&str> = ssh_spec.splitn(4, ':').collect();
            let jump = ssh::Target {
                label: "jump".into(),
                address: s[0].into(),
                port: s[1].parse().unwrap(),
                username: s[2].into(),
                auth: ssh::Auth::Password(s[3].into()),
                jump: None,
            };
            let via = FtpTarget { jump: Some(jump), ..target.clone() };
            let mut notices = Vec::new();
            let fs = Arc::new(Ftp::open(via, Arc::new(Store::in_memory().unwrap()), &mut notices).await.unwrap());
            assert!(notices.iter().any(|n| n.contains("jump host")), "{notices:?}");
            let home = fs.home().await.unwrap();
            crate::files::tests::exercise(fs, &home).await;
        }
        let store = Arc::new(Store::in_memory().unwrap());
        let mut notices = Vec::new();
        let fs = Arc::new(Ftp::open(target, store, &mut notices).await.unwrap());
        eprintln!("ftp notices: {notices:?}");
        let home = fs.home().await.unwrap();
        crate::files::tests::exercise(fs, &home).await;
    }
}

