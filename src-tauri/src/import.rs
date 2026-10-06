// SPDX-FileCopyrightText: 2026 BunnyCloud.IT
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial
//! Importing connections from OpenSSH config, PuTTY sessions and mRemoteNG.
//! Each parser produces a [`Plan`]; [`apply`] turns a plan into folders, credentials
//! and hosts.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::model::*;
use crate::store::Store;
use crate::vault::Vault;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Openssh,
    Putty,
    Mremoteng,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Openssh => "OpenSSH config",
            Source::Putty => "PuTTY",
            Source::Mremoteng => "mRemoteNG",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PSecret {
    Password(String),
    /// `encrypted` keys need a passphrase; `path` identifies the key file for the
    /// user's choice of how to handle it.
    Key { pem: String, label: String, path: String, encrypted: bool },
    Agent,
}

/// What to do with a passphrase-protected key: save this passphrase, or (`None`) ask
/// for it on every connection.
pub type KeyChoices = HashMap<String, Option<String>>;

#[derive(Debug, Clone)]
pub struct PFolder {
    pub key: String,
    pub parent: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct PHost {
    pub key: String,
    pub folder: Option<String>,
    pub name: String,
    pub address: String,
    pub endpoints: Vec<Endpoint>,
    pub username: Option<String>,
    pub secret: Option<PSecret>,
    /// Key of the host to jump through.
    pub jump: Option<String>,
    pub notes: String,
}

#[derive(Debug, Default)]
pub struct Plan {
    pub folders: Vec<PFolder>,
    pub hosts: Vec<PHost>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewHost {
    pub name: String,
    pub address: String,
    pub protocols: Vec<Protocol>,
    pub folder: Option<String>,
    pub username: Option<String>,
    pub auth: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub hosts: Vec<PreviewHost>,
    pub folders: usize,
    pub warnings: Vec<String>,
    /// Passphrase-protected key files, so the user can choose how to handle each.
    pub encrypted_keys: Vec<PreviewKey>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewKey {
    pub path: String,
    pub hosts: usize,
}

pub fn preview(plan: &Plan) -> Preview {
    let folder_name = |k: &Option<String>| {
        k.as_ref().and_then(|k| plan.folders.iter().find(|f| &f.key == k)).map(|f| f.name.clone())
    };
    Preview {
        hosts: plan
            .hosts
            .iter()
            .map(|h| PreviewHost {
                name: h.name.clone(),
                address: h.address.clone(),
                protocols: h.endpoints.iter().map(|e| e.protocol).collect(),
                folder: folder_name(&h.folder),
                username: h.username.clone(),
                auth: match h.secret {
                    Some(PSecret::Password(_)) => "password",
                    Some(PSecret::Key { .. }) => "key",
                    Some(PSecret::Agent) => "agent",
                    None => "none",
                },
            })
            .collect(),
        folders: plan.folders.len(),
        warnings: plan.warnings.clone(),
        encrypted_keys: {
            let mut keys: Vec<PreviewKey> = vec![];
            for h in &plan.hosts {
                if let Some(PSecret::Key { path, encrypted: true, .. }) = &h.secret {
                    match keys.iter_mut().find(|k| &k.path == path) {
                        Some(k) => k.hosts += 1,
                        None => keys.push(PreviewKey { path: path.clone(), hosts: 1 }),
                    }
                }
            }
            keys
        },
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Applied {
    pub hosts: usize,
    pub folders: usize,
    pub credentials: usize,
    pub root_folder_id: String,
}

fn ep(protocol: Protocol, port: Option<u16>) -> Endpoint {
    Endpoint { protocol, port: port.or(Some(protocol.default_port())).filter(|p| *p != 0), path: None, serial: None }
}

/// Writes a plan into the library under a new top-level folder.
pub fn apply(store: &Store, vault: &Vault, source: Source, plan: Plan, keys: &KeyChoices) -> AppResult<Applied> {
    // Check saved passphrases before writing anything.
    for h in &plan.hosts {
        if let Some(PSecret::Key { pem, path, encrypted: true, .. }) = &h.secret {
            if let Some(Some(pass)) = keys.get(path) {
                russh::keys::decode_secret_key(pem, Some(pass))
                    .map_err(|_| AppError::Invalid(format!("Wrong passphrase for {path}.")))?;
            }
        }
    }
    let new_id = || uuid::Uuid::new_v4().to_string();
    let root = Folder { id: new_id(), parent_id: None, name: format!("{} import", source.label()), defaults: Defaults::default() };
    store.save_folder(&root)?;

    let mut folder_ids: HashMap<String, String> = HashMap::new();
    // Parents come before children in every parser's output.
    for f in &plan.folders {
        let parent = f.parent.as_ref().and_then(|p| folder_ids.get(p).cloned()).unwrap_or(root.id.clone());
        let folder = Folder { id: new_id(), parent_id: Some(parent), name: f.name.clone(), defaults: Defaults::default() };
        store.save_folder(&folder)?;
        folder_ids.insert(f.key.clone(), folder.id);
    }

    // Credentials are shared between hosts with the same secret.
    let mut creds: HashMap<String, String> = HashMap::new();
    let mut created = 0;
    let existing_agent = store.credentials()?.into_iter().find(|c| c.kind == CredentialKind::Agent).map(|c| c.id);
    let mut credential_for = |h: &PHost| -> AppResult<Option<String>> {
        let Some(secret) = &h.secret else { return Ok(None) };
        let mut ask_passphrase = false;
        let (dedupe, name, kind, blob) = match secret {
            PSecret::Agent => {
                if let Some(id) = &existing_agent {
                    return Ok(Some(id.clone()));
                }
                ("agent".to_string(), "SSH agent".to_string(), CredentialKind::Agent, None)
            }
            PSecret::Password(pw) => {
                let user = h.username.clone().unwrap_or_default();
                (
                    format!("pw:{user}:{pw}"),
                    if user.is_empty() { format!("{} password", source.label()) } else { format!("{user} ({})", source.label()) },
                    CredentialKind::Password,
                    Some(Secret { secret: pw.clone(), passphrase: None }),
                )
            }
            PSecret::Key { pem, label, path, encrypted } => {
                let passphrase = if *encrypted { keys.get(path).cloned().flatten() } else { None };
                ask_passphrase = *encrypted && passphrase.is_none();
                (format!("key:{pem}"), label.clone(), CredentialKind::Key, Some(Secret { secret: pem.clone(), passphrase }))
            }
        };
        if let Some(id) = creds.get(&dedupe) {
            return Ok(Some(id.clone()));
        }
        let c = Credential { id: new_id(), name, username: None, kind, ask_passphrase };
        let blob = match blob {
            Some(s) => Some(vault.encrypt(&serde_json::to_vec(&s)?)?),
            None => None,
        };
        store.save_credential(&c, blob)?;
        created += 1;
        creds.insert(dedupe, c.id.clone());
        Ok(Some(c.id))
    };

    let mut host_ids: HashMap<String, String> = HashMap::new();
    let mut saved = Vec::new();
    for h in &plan.hosts {
        let host = Host {
            id: new_id(),
            folder_id: Some(h.folder.as_ref().and_then(|k| folder_ids.get(k).cloned()).unwrap_or(root.id.clone())),
            name: h.name.clone(),
            address: h.address.clone(),
            endpoints: h.endpoints.clone(),
            tags: vec![],
            overrides: Defaults { username: h.username.clone(), credential_id: credential_for(h)?, jump_host_id: None },
            notes: h.notes.clone(),
            pinned: false,
            last_used: None,
        };
        host_ids.insert(h.key.clone(), host.id.clone());
        saved.push((host, h.jump.clone()));
    }
    for (mut host, jump) in saved {
        host.overrides.jump_host_id = jump.and_then(|k| host_ids.get(&k).cloned()).filter(|j| *j != host.id);
        store.save_host(&host)?;
    }
    Ok(Applied { hosts: plan.hosts.len(), folders: plan.folders.len() + 1, credentials: created, root_folder_id: root.id })
}

// ------------------------------------------------------------------ OpenSSH

fn expand_home(path: &str, home: &Path) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(path)
    }
}

/// Reads a key file. Passphrase-protected keys are imported too; the user chooses
/// whether to save the passphrase. Keys that can't be read are left to the SSH agent.
fn key_secret(path: &Path, warnings: &mut Vec<String>) -> Option<PSecret> {
    let pem = std::fs::read_to_string(path).ok()?;
    let encrypted = match russh::keys::decode_secret_key(&pem, None) {
        Ok(_) => false,
        Err(e) if is_encrypted(&pem, &e) => true,
        Err(_) => {
            warnings.push(format!("{} could not be read; those hosts will use the SSH agent.", path.display()));
            return Some(PSecret::Agent);
        }
    };
    Some(PSecret::Key {
        pem,
        label: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "key".into()),
        path: path.display().to_string(),
        encrypted,
    })
}

fn is_encrypted(text: &str, err: &russh::keys::Error) -> bool {
    matches!(err, russh::keys::Error::KeyIsEncrypted)
        || text.contains("ENCRYPTED PRIVATE KEY")
        // PuTTY keys name their cipher in an "Encryption:" header.
        || text.lines().any(|l| l.strip_prefix("Encryption:").is_some_and(|c| c.trim() != "none"))
}

#[derive(Default, Clone)]
struct SshBlock {
    hostname: Option<String>,
    user: Option<String>,
    port: Option<u16>,
    identity: Option<String>,
    proxy_jump: Option<String>,
}

fn read_ssh_config(path: &Path, home: &Path, depth: u8, out: &mut Vec<(Vec<String>, SshBlock)>) {
    let Ok(text) = std::fs::read_to_string(path) else { return };
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = match line.split_once(|c: char| c.is_whitespace() || c == '=') {
            Some((k, v)) => (k.to_lowercase(), v.trim().trim_start_matches('=').trim().trim_matches('"').to_string()),
            None => continue,
        };
        match key.as_str() {
            "host" => out.push((value.split_whitespace().map(String::from).collect(), SshBlock::default())),
            "match" => out.push((vec![], SshBlock::default())), // not supported: ignore its settings
            "include" if depth < 4 => {
                let inc = expand_home(&value, home);
                let inc = if inc.is_relative() { home.join(".ssh").join(inc) } else { inc };
                read_ssh_config(&inc, home, depth + 1, out);
            }
            _ => {
                if out.is_empty() {
                    out.push((vec!["*".into()], SshBlock::default()));
                }
                let b = &mut out.last_mut().unwrap().1;
                match key.as_str() {
                    "hostname" => b.hostname.get_or_insert(value),
                    "user" => b.user.get_or_insert(value),
                    "identityfile" => b.identity.get_or_insert(value),
                    "proxyjump" => b.proxy_jump.get_or_insert(value),
                    "port" => {
                        if b.port.is_none() {
                            b.port = value.parse().ok();
                        }
                        continue;
                    }
                    _ => continue,
                };
            }
        }
    }
}

pub fn parse_openssh(path: &Path, home: &Path) -> AppResult<Plan> {
    if !path.exists() {
        return Err(AppError::Invalid(format!("{} does not exist.", path.display())));
    }
    let mut blocks = Vec::new();
    read_ssh_config(path, home, 0, &mut blocks);
    let wildcard = |p: &str| p.contains(['*', '?', '!']);
    // Settings from wildcard blocks apply to every host that doesn't set them.
    let mut defaults = SshBlock::default();
    for (patterns, b) in &blocks {
        if patterns.iter().any(|p| p == "*") {
            defaults.user = defaults.user.clone().or(b.user.clone());
            defaults.port = defaults.port.or(b.port);
            defaults.identity = defaults.identity.clone().or(b.identity.clone());
        }
    }
    let mut plan = Plan::default();
    let mut keys_cache: HashMap<String, Option<PSecret>> = HashMap::new();
    let mut aliases: Vec<String> = vec![];
    for (patterns, b) in &blocks {
        for alias in patterns.iter().filter(|p| !wildcard(p)) {
            if aliases.contains(alias) {
                continue;
            }
            aliases.push(alias.clone());
            let identity = b.identity.clone().or(defaults.identity.clone());
            let secret = match identity {
                Some(id) => keys_cache
                    .entry(id.clone())
                    .or_insert_with(|| key_secret(&expand_home(&id, home), &mut plan.warnings))
                    .clone()
                    .or(Some(PSecret::Agent)),
                None => Some(PSecret::Agent),
            };
            plan.hosts.push(PHost {
                key: alias.clone(),
                folder: None,
                name: alias.clone(),
                address: b.hostname.clone().unwrap_or_else(|| alias.clone()),
                endpoints: vec![ep(Protocol::Ssh, b.port.or(defaults.port))],
                username: b.user.clone().or(defaults.user.clone()),
                secret,
                jump: None,
                notes: String::new(),
            });
        }
    }
    // ProxyJump: link to an imported alias, or add the jump host itself.
    let jumps: Vec<(String, String)> = blocks
        .iter()
        .flat_map(|(pats, b)| pats.iter().filter_map(move |p| b.proxy_jump.clone().map(|j| (p.clone(), j))))
        .collect();
    for (alias, spec) in jumps {
        let first = spec.split(',').next().unwrap_or("").trim().to_string();
        if first.eq_ignore_ascii_case("none") || first.is_empty() {
            continue;
        }
        let (user, rest) = match first.split_once('@') {
            Some((u, r)) => (Some(u.to_string()), r.to_string()),
            None => (None, first.clone()),
        };
        let (host, port) = match rest.rsplit_once(':') {
            Some((h, p)) if p.parse::<u16>().is_ok() => (h.to_string(), p.parse().ok()),
            _ => (rest.clone(), None),
        };
        let key = if plan.hosts.iter().any(|h| h.key == host) {
            host.clone()
        } else {
            let key = format!("jump:{first}");
            if !plan.hosts.iter().any(|h| h.key == key) {
                plan.hosts.push(PHost {
                    key: key.clone(),
                    folder: None,
                    name: host.clone(),
                    address: host.clone(),
                    endpoints: vec![ep(Protocol::Ssh, port)],
                    username: user.or(defaults.user.clone()),
                    secret: Some(PSecret::Agent),
                    jump: None,
                    notes: "Jump host added by the OpenSSH import.".into(),
                });
            }
            key
        };
        if spec.contains(',') {
            plan.warnings.push(format!("{alias}: only the first ProxyJump hop ({first}) was imported."));
        }
        if let Some(h) = plan.hosts.iter_mut().find(|h| h.key == alias) {
            h.jump = Some(key);
        }
    }
    if plan.hosts.is_empty() {
        plan.warnings.push("No hosts found (wildcard patterns are skipped).".into());
    }
    Ok(plan)
}

// ------------------------------------------------------------------ PuTTY

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 3 <= b.len() && s.is_char_boundary(i + 3) {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// One PuTTY session's settings, as strings (registry DWORDs are stringified).
pub type PuttySession = (String, HashMap<String, String>);

pub fn putty_plan(sessions: Vec<PuttySession>) -> Plan {
    let mut plan = Plan::default();
    for (name, v) in sessions {
        let get = |k: &str| v.get(k).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let num = |k: &str| get(k).and_then(|s| s.parse::<u32>().ok());
        let protocol = get("Protocol").unwrap_or_else(|| "ssh".into());
        let mut host = get("HostName").unwrap_or_default();
        // PuTTY accepts "user@host" in HostName; that user wins over UserName.
        let mut username = get("UserName");
        if let Some((user, h)) = host.rsplit_once('@') {
            username = Some(user.trim().to_string()).filter(|u| !u.is_empty()).or(username);
            host = h.trim().to_string();
        }
        if name == "Default Settings" && host.is_empty() {
            continue;
        }
        let port = num("PortNumber").and_then(|p| u16::try_from(p).ok());
        let mut secret = None;
        let endpoint = match protocol.as_str() {
            "ssh" => {
                if let Some(ppk) = get("PublicKeyFile") {
                    secret = key_secret(Path::new(&ppk), &mut plan.warnings);
                }
                ep(Protocol::Ssh, port)
            }
            "telnet" => ep(Protocol::Telnet, port),
            "serial" => {
                let half = num("SerialStopHalfbits").unwrap_or(2);
                Endpoint {
                    protocol: Protocol::Serial,
                    port: None,
                    path: get("SerialLine"),
                    serial: Some(SerialSettings {
                        baud: num("SerialSpeed").unwrap_or(9600),
                        data_bits: num("SerialDataBits").unwrap_or(8) as u8,
                        parity: match num("SerialParity").unwrap_or(0) {
                            1 => "odd",
                            2 => "even",
                            _ => "none",
                        }
                        .into(),
                        stop_bits: if half >= 4 { 2 } else { 1 },
                        flow: match num("SerialFlowControl").unwrap_or(1) {
                            1 => "software",
                            2 => "hardware",
                            _ => "none",
                        }
                        .into(),
                    }),
                }
            }
            other => {
                plan.warnings.push(format!("{name}: {other} sessions are not supported and were skipped."));
                continue;
            }
        };
        if endpoint.protocol != Protocol::Serial && host.is_empty() {
            plan.warnings.push(format!("{name}: no host name, skipped."));
            continue;
        }
        plan.hosts.push(PHost {
            key: name.clone(),
            folder: None,
            name: name.clone(),
            address: host,
            endpoints: vec![endpoint],
            username,
            secret,
            jump: None,
            notes: String::new(),
        });
    }
    if plan.hosts.is_empty() {
        plan.warnings.push("No PuTTY sessions found.".into());
    }
    plan
}

/// Reads PuTTY sessions: the registry on Windows, `~/.putty/sessions` elsewhere.
pub fn read_putty(home: &Path) -> AppResult<Vec<PuttySession>> {
    #[cfg(windows)]
    {
        let _ = home;
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::types::FromRegValue;
        let root = winreg::RegKey::predef(HKEY_CURRENT_USER);
        let Ok(sessions) = root.open_subkey(r"Software\SimonTatham\PuTTY\Sessions") else {
            return Ok(vec![]);
        };
        let mut out = vec![];
        for name in sessions.enum_keys().flatten() {
            let Ok(key) = sessions.open_subkey(&name) else { continue };
            let mut values = HashMap::new();
            for (k, v) in key.enum_values().flatten() {
                let s = String::from_reg_value(&v).ok().or_else(|| u32::from_reg_value(&v).ok().map(|n| n.to_string()));
                if let Some(s) = s {
                    values.insert(k, s);
                }
            }
            out.push((percent_decode(&name), values));
        }
        Ok(out)
    }
    #[cfg(not(windows))]
    {
        let dir = home.join(".putty").join("sessions");
        let Ok(entries) = std::fs::read_dir(&dir) else { return Ok(vec![]) };
        let mut out = vec![];
        for e in entries.flatten() {
            let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
            let values = text
                .lines()
                .filter_map(|l| l.split_once('='))
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            out.push((percent_decode(&e.file_name().to_string_lossy()), values));
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }
}

// ------------------------------------------------------------------ mRemoteNG

mod mremoteng_crypto {
    use aes_gcm::aead::{Aead, KeyInit, Payload};
    use aes_gcm::{aes::Aes256, AesGcm};
    use base64::Engine;

    type Gcm = AesGcm<Aes256, aes_gcm::aead::consts::U16>;

    /// Decrypts a mRemoteNG secret. Newer files use AES-256-GCM with a PBKDF2-SHA1 key
    /// (`salt | nonce | ciphertext+tag`, salt as associated data); older ones use
    /// AES-CBC with an MD5 key (`iv | ciphertext`).
    pub fn decrypt(data: &str, password: &str, gcm: bool, iterations: u32) -> Option<String> {
        let raw = base64::engine::general_purpose::STANDARD.decode(data.trim()).ok()?;
        if gcm {
            if raw.len() < 48 {
                return None;
            }
            let (salt, rest) = raw.split_at(16);
            let (nonce, ct) = rest.split_at(16);
            let mut key = [0u8; 32];
            pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password.as_bytes(), salt, iterations, &mut key);
            let cipher = Gcm::new((&key).into());
            let plain = cipher.decrypt(nonce.into(), Payload { msg: ct, aad: salt }).ok()?;
            String::from_utf8(plain).ok()
        } else {
            use cbc::cipher::{BlockDecryptMut, KeyIvInit};
            use md5::Digest;
            if raw.len() < 32 {
                return None;
            }
            let key = md5::Md5::digest(password.as_bytes());
            let (iv, ct) = raw.split_at(16);
            let dec = cbc::Decryptor::<aes::Aes128>::new_from_slices(&key, iv).ok()?;
            let plain = dec.decrypt_padded_vec_mut::<cbc::cipher::block_padding::Pkcs7>(ct).ok()?;
            String::from_utf8(plain).ok()
        }
    }
}

pub fn parse_mremoteng(xml: &str, password: Option<&str>) -> AppResult<Plan> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    // Root element attributes decide how secrets are protected.
    let mut reader = Reader::from_str(xml);
    let mut root_attrs: HashMap<String, String> = HashMap::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                for a in e.attributes().flatten() {
                    let k = String::from_utf8_lossy(a.key.local_name().as_ref()).into_owned();
                    let v = a.unescape_value().map(|v| v.into_owned()).unwrap_or_default();
                    root_attrs.insert(k, v);
                }
                break;
            }
            Ok(Event::Eof) => return Err(AppError::Invalid("This is not a mRemoteNG connections file.".into())),
            Err(e) => return Err(AppError::Invalid(format!("Could not read the file: {e}"))),
            _ => {}
        }
    }
    let gcm = root_attrs.get("BlockCipherMode").map(|m| m.eq_ignore_ascii_case("GCM")).unwrap_or(false);
    let iterations = root_attrs.get("KdfIterations").and_then(|v| v.parse().ok()).unwrap_or(1000);
    let password = password.filter(|p| !p.is_empty()).unwrap_or("mR3m");
    let decrypt = |s: &str| mremoteng_crypto::decrypt(s, password, gcm, iterations);

    if let Some(check) = root_attrs.get("Protected") {
        if !check.is_empty() && decrypt(check).is_none() {
            return Err(AppError::Invalid(
                "This file is protected with a password. Enter the mRemoteNG master password.".into(),
            ));
        }
    }
    // Fully encrypted files carry the node tree as encrypted text in the root element.
    let full = root_attrs.get("FullFileEncryption").map(|v| v == "true").unwrap_or(false);
    let body;
    let xml = if full {
        let start = xml.find('>').map(|i| i + 1).unwrap_or(0);
        let end = xml.rfind("</").unwrap_or(xml.len());
        body = decrypt(xml[start..end].trim())
            .ok_or_else(|| AppError::Invalid("Could not decrypt the file. Check the master password.".into()))?;
        format!("<Root>{body}</Root>")
    } else {
        xml.to_string()
    };

    let mut plan = Plan::default();
    let mut reader = Reader::from_str(&xml);
    // Stack of open containers: (folder key, attributes for inheritance).
    let mut stack: Vec<(Option<String>, HashMap<String, String>)> = vec![];
    let mut counter = 0;
    loop {
        let event = reader.read_event().map_err(|e| AppError::Invalid(format!("Could not read the file: {e}")))?;
        let (e, empty) = match &event {
            Event::Start(e) => (e, false),
            Event::Empty(e) => (e, true),
            Event::End(e) => {
                if e.local_name().as_ref() == b"Node" {
                    stack.pop();
                }
                continue;
            }
            Event::Eof => break,
            _ => continue,
        };
        if e.local_name().as_ref() != b"Node" {
            continue;
        }
        let attrs: HashMap<String, String> = e
            .attributes()
            .flatten()
            .map(|a| {
                (
                    String::from_utf8_lossy(a.key.local_name().as_ref()).into_owned(),
                    a.unescape_value().map(|v| v.into_owned()).unwrap_or_default(),
                )
            })
            .collect();
        counter += 1;
        let parent_key = stack.iter().rev().find_map(|(k, _)| k.clone());
        let get = |k: &str| attrs.get(k).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        // Inherited values come from the nearest container that has them.
        let inherited = |field: &str| -> Option<String> {
            if attrs.get(&format!("Inherit{field}")).map(|v| v == "true").unwrap_or(false) {
                stack.iter().rev().find_map(|(_, a)| a.get(field).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()))
            } else {
                get(field)
            }
        };
        let name = get("Name").unwrap_or_else(|| format!("Imported {counter}"));
        let username = inherited("Username");
        let domain = inherited("Domain");
        let password = inherited("Password");
        if get("Type").as_deref() == Some("Container") {
            let key = format!("f{counter}");
            plan.folders.push(PFolder { key: key.clone(), parent: parent_key, name });
            if !empty {
                stack.push((Some(key), attrs.clone()));
            }
            continue;
        }
        if !empty {
            stack.push((None, attrs.clone()));
        }
        let protocol = match get("Protocol").unwrap_or_default().to_uppercase().as_str() {
            "RDP" => Protocol::Rdp,
            "VNC" => Protocol::Vnc,
            "SSH1" | "SSH2" => Protocol::Ssh,
            "TELNET" => Protocol::Telnet,
            "HTTP" => Protocol::Http,
            "HTTPS" => Protocol::Https,
            other => {
                plan.warnings.push(format!("{name}: {} connections are not supported and were skipped.", other.to_lowercase()));
                continue;
            }
        };
        let Some(address) = get("Hostname") else {
            plan.warnings.push(format!("{name}: no host name, skipped."));
            continue;
        };
        let port = get("Port").and_then(|p| p.parse().ok());
        let username = match (username, domain) {
            (Some(u), Some(d)) if protocol == Protocol::Rdp && !u.contains('\\') && !u.contains('@') => Some(format!("{d}\\{u}")),
            (u, _) => u,
        };
        let secret = match password {
            Some(enc) => match decrypt(&enc) {
                Some(pw) if !pw.is_empty() => Some(PSecret::Password(pw)),
                Some(_) => None,
                None => {
                    plan.warnings.push(format!("{name}: the saved password could not be decrypted."));
                    None
                }
            },
            None => None,
        };
        plan.hosts.push(PHost {
            key: format!("h{counter}"),
            folder: parent_key,
            name,
            address,
            endpoints: vec![ep(protocol, port)],
            username,
            secret,
            jump: None,
            notes: get("Descr").unwrap_or_default(),
        });
    }
    if plan.hosts.is_empty() {
        plan.warnings.push("No connections found.".into());
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("bl-imp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(d.join(".ssh")).unwrap();
        d
    }

    /// Throwaway unencrypted test key.
    const TEST_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW\nQyNTUxOQAAACDnMnHqq+tBzmnHyKphucuxfmXMP3YBWgWsL73WR2UyXgAAAIjZHSac2R0m\nnAAAAAtzc2gtZWQyNTUxOQAAACDnMnHqq+tBzmnHyKphucuxfmXMP3YBWgWsL73WR2UyXg\nAAAECEz70Uzh07sfo4GwufdpT+8WkiXH6LQhh6pgEiPTCofOcyceqr60HOacfIqmG5y7F+\nZcw/dgFaBawvvdZHZTJeAAAABHRlc3QB\n-----END OPENSSH PRIVATE KEY-----\n";

    #[test]
    fn openssh_config() {
        let home = tmp();
        std::fs::write(home.join(".ssh/id_test"), TEST_KEY).unwrap();
        std::fs::write(home.join(".ssh/extra"), "Host db\n  HostName 10.0.0.5\n  ProxyJump bastion\n").unwrap();
        std::fs::write(
            home.join(".ssh/config"),
            "# comment\nHost *\n  User deploy\n\nHost bastion bast\n  HostName bastion.example.com\n  Port 2222\n  IdentityFile ~/.ssh/id_test\n\nHost web-*\n  User www\n\nHost web1\n  HostName=10.0.0.10\n  User root\n  ProxyJump admin@gw.example.com:2200\n\nInclude extra\n",
        )
        .unwrap();
        let plan = parse_openssh(&home.join(".ssh/config"), &home).unwrap();
        let names: Vec<_> = plan.hosts.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(names, vec!["bastion", "bast", "web1", "db", "gw.example.com"]);
        let bastion = &plan.hosts[0];
        assert_eq!(bastion.address, "bastion.example.com");
        assert_eq!(bastion.endpoints[0].port, Some(2222));
        assert_eq!(bastion.username.as_deref(), Some("deploy"));
        assert!(matches!(bastion.secret, Some(PSecret::Key { .. })));
        let web1 = &plan.hosts[2];
        assert_eq!(web1.username.as_deref(), Some("root"));
        assert_eq!(web1.secret, Some(PSecret::Agent));
        assert_eq!(web1.jump.as_deref(), Some("jump:admin@gw.example.com:2200"));
        let gw = &plan.hosts[4];
        assert_eq!((gw.username.as_deref(), gw.endpoints[0].port), (Some("admin"), Some(2200)));
        assert_eq!(plan.hosts[3].jump.as_deref(), Some("bastion"));
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn putty_sessions() {
        let s = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let plan = putty_plan(vec![
            ("Default Settings".into(), s(&[("HostName", "")])),
            ("my%20router".into(), s(&[("HostName", "10.1.1.1"), ("Protocol", "telnet"), ("PortNumber", "23")])),
            (
                "console".into(),
                s(&[
                    ("Protocol", "serial"),
                    ("SerialLine", "COM3"),
                    ("SerialSpeed", "115200"),
                    ("SerialParity", "2"),
                    ("SerialFlowControl", "0"),
                ]),
            ),
            ("web".into(), s(&[("HostName", "web.example.com"), ("UserName", "me"), ("PortNumber", "22")])),
            ("old".into(), s(&[("HostName", "x"), ("Protocol", "rlogin")])),
            ("vm".into(), s(&[("HostName", "root@203.0.113.10"), ("UserName", "me")])),
        ]);
        assert_eq!(plan.hosts.len(), 4);
        assert_eq!(plan.hosts[0].endpoints[0].protocol, Protocol::Telnet);
        let serial = plan.hosts[1].endpoints[0].serial.as_ref().unwrap();
        assert_eq!((serial.baud, serial.parity.as_str(), serial.flow.as_str()), (115200, "even", "none"));
        assert_eq!(plan.hosts[2].username.as_deref(), Some("me"));
        assert_eq!((plan.hosts[3].address.as_str(), plan.hosts[3].username.as_deref()), ("203.0.113.10", Some("root")));
        assert!(plan.warnings.iter().any(|w| w.contains("rlogin")));
        assert_eq!(percent_decode("my%20router"), "my router");
    }

    fn encrypt_gcm(plain: &str, password: &str) -> String {
        use aes_gcm::aead::{Aead, KeyInit, Payload};
        use base64::Engine;
        let salt = [7u8; 16];
        let nonce = [9u8; 16];
        let mut key = [0u8; 32];
        pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password.as_bytes(), &salt, 1000, &mut key);
        let cipher = aes_gcm::AesGcm::<aes_gcm::aes::Aes256, aes_gcm::aead::consts::U16>::new((&key).into());
        let ct = cipher.encrypt((&nonce).into(), Payload { msg: plain.as_bytes(), aad: &salt }).unwrap();
        base64::engine::general_purpose::STANDARD.encode([&salt[..], &nonce[..], &ct].concat())
    }

    #[test]
    fn mremoteng_file() {
        let pw = encrypt_gcm("S3cret!", "mR3m");
        let folder_pw = encrypt_gcm("Inherited1", "mR3m");
        let protected = encrypt_gcm("ThisIsNotProtected", "mR3m");
        let xml = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<mrng:Connections xmlns:mrng="http://mremoteng.org" Name="Connections" EncryptionEngine="AES" BlockCipherMode="GCM" KdfIterations="1000" FullFileEncryption="false" Protected="{protected}" ConfVersion="2.6">
  <Node Name="Production" Type="Container" Username="admin" Domain="CORP" Password="{folder_pw}">
    <Node Name="dc01" Type="Connection" Hostname="10.0.2.5" Protocol="RDP" Port="3389" Username="" Password="" InheritUsername="true" InheritDomain="true" InheritPassword="true" />
    <Node Name="Linux" Type="Container">
      <Node Name="web01" Type="Connection" Hostname="10.0.1.11" Protocol="SSH2" Port="2222" Username="deploy" Password="{pw}" Descr="frontend" />
    </Node>
  </Node>
  <Node Name="printer" Type="Connection" Hostname="10.9.9.9" Protocol="HTTPS" Port="443" />
  <Node Name="legacy" Type="Connection" Hostname="10.9.9.8" Protocol="Rlogin" />
</mrng:Connections>"#
        );
        let plan = parse_mremoteng(&xml, None).unwrap();
        assert_eq!(plan.folders.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), vec!["Production", "Linux"]);
        assert_eq!(plan.folders[1].parent.as_deref(), Some(plan.folders[0].key.as_str()));
        let dc = &plan.hosts[0];
        assert_eq!(dc.username.as_deref(), Some("CORP\\admin"));
        assert_eq!(dc.secret, Some(PSecret::Password("Inherited1".into())));
        let web = &plan.hosts[1];
        assert_eq!(web.secret, Some(PSecret::Password("S3cret!".into())));
        assert_eq!(web.endpoints[0].port, Some(2222));
        assert_eq!(web.folder.as_deref(), Some(plan.folders[1].key.as_str()));
        assert_eq!(web.notes, "frontend");
        assert_eq!(plan.hosts[2].folder, None);
        assert_eq!(plan.hosts.len(), 3);
        assert!(plan.warnings.iter().any(|w| w.contains("legacy")));

        // A file protected with a custom master password needs it.
        let custom = xml.replace(&protected, &encrypt_gcm("ThisIsProtected", "hunter2"));
        assert!(parse_mremoteng(&custom, None).is_err());
    }

    #[test]
    fn apply_plan() {
        let store = Store::in_memory().unwrap();
        let vault = Vault::default();
        vault.create(&store, "master-password").unwrap();
        let plan = parse_mremoteng(
            &format!(
                r#"<Connections Name="Connections" BlockCipherMode="GCM" KdfIterations="1000">
  <Node Name="A" Type="Container">
    <Node Name="s1" Type="Connection" Hostname="h1" Protocol="SSH2" Username="u" Password="{p}" />
    <Node Name="s2" Type="Connection" Hostname="h2" Protocol="SSH2" Username="u" Password="{p}" />
  </Node>
</Connections>"#,
                p = encrypt_gcm("pw", "mR3m")
            ),
            None,
        )
        .unwrap();
        let applied = apply(&store, &vault, Source::Mremoteng, plan, &KeyChoices::new()).unwrap();
        assert_eq!((applied.hosts, applied.folders, applied.credentials), (2, 2, 1));
        let hosts = store.hosts().unwrap();
        assert_eq!(hosts[0].overrides.credential_id, hosts[1].overrides.credential_id);
        let folders = store.folders().unwrap();
        assert!(folders.iter().any(|f| f.name == "mRemoteNG import" && f.parent_id.is_none()));
    }
}
