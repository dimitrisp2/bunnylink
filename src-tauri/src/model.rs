// SPDX-FileCopyrightText: 2026 BunnyCloud.IT
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Ssh,
    Sftp,
    Rdp,
    Vnc,
    Ftp,
    Telnet,
    Serial,
    Http,
    Https,
    Smb,
    Spice,
}

impl Protocol {
    pub fn default_port(self) -> u16 {
        match self {
            Protocol::Ssh | Protocol::Sftp => 22,
            Protocol::Rdp => 3389,
            Protocol::Vnc => 5900,
            Protocol::Ftp => 21,
            Protocol::Telnet => 23,
            Protocol::Serial => 0,
            Protocol::Http => 80,
            Protocol::Https => 443,
            Protocol::Smb => 445,
            Protocol::Spice => 5930,
        }
    }
}

/// One way of reaching a host. A host can have several endpoints (e.g. SSH + HTTPS).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Endpoint {
    pub protocol: Protocol,
    #[serde(default)]
    pub port: Option<u16>,
    /// Protocol specific: URL path for HTTP(S), share for SMB, device for Serial.
    #[serde(default)]
    pub path: Option<String>,
    /// Line settings for Serial endpoints.
    #[serde(default)]
    pub serial: Option<SerialSettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialSettings {
    pub baud: u32,
    pub data_bits: u8,
    /// "none", "odd" or "even".
    pub parity: String,
    pub stop_bits: u8,
    /// "none", "software" or "hardware".
    pub flow: String,
}

impl Default for SerialSettings {
    fn default() -> Self {
        Self { baud: 9600, data_bits: 8, parity: "none".into(), stop_bits: 1, flow: "none".into() }
    }
}

/// Values a folder passes down to the folders and hosts inside it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Defaults {
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub credential_id: Option<String>,
    #[serde(default)]
    pub jump_host_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub defaults: Defaults,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Host {
    pub id: String,
    #[serde(default)]
    pub folder_id: Option<String>,
    pub name: String,
    pub address: String,
    #[serde(default)]
    pub endpoints: Vec<Endpoint>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Overrides for the inherited folder defaults.
    #[serde(default)]
    pub overrides: Defaults,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub last_used: Option<i64>,
}

/// A host's settings after folder inheritance has been applied.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Effective {
    pub username: Option<String>,
    pub credential_id: Option<String>,
    pub jump_host_id: Option<String>,
    /// Name of the folder each value came from, `None` when set on the host itself.
    pub username_from: Option<String>,
    pub credential_from: Option<String>,
    pub jump_host_from: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CredentialKind {
    Password,
    Key,
    /// Keys held by the running SSH agent (ssh-agent, Windows OpenSSH agent, Pageant).
    Agent,
}

/// Credential metadata. The secret itself is stored encrypted and never sent to the UI
/// except through the explicit "copy" action.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Credential {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub username: Option<String>,
    pub kind: CredentialKind,
    /// Key credentials: ask for the key's passphrase on every connection instead of
    /// storing it.
    #[serde(default)]
    pub ask_passphrase: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialInput {
    pub credential: Credential,
    /// Password, or private key in OpenSSH/PEM format. `None` keeps the stored secret.
    #[serde(default)]
    pub secret: Option<String>,
    #[serde(default)]
    pub passphrase: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Secret {
    pub secret: String,
    #[serde(default)]
    pub passphrase: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TunnelKind {
    Local,
    Remote,
    Socks,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tunnel {
    pub id: String,
    pub name: String,
    pub kind: TunnelKind,
    /// SSH host the tunnel runs through.
    pub host_id: String,
    pub bind_port: u16,
    #[serde(default)]
    pub target_host: Option<String>,
    #[serde(default)]
    pub target_port: Option<u16>,
}

/// A saved command sent to a terminal. `{{name}}` and `{{name:default}}` are filled in
/// before sending. With no folders and no tags it applies to every host.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snippet {
    pub id: String,
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub description: String,
    /// Press Enter after the command is typed.
    #[serde(default)]
    pub send_enter: bool,
    /// Hosts in these folders (or their subfolders).
    #[serde(default)]
    pub folder_ids: Vec<String>,
    /// Hosts with any of these tags.
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Dark,
    Light,
    Auto,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub theme: Theme,
    /// Lock the vault after this many idle minutes. 0 disables auto-lock.
    pub auto_lock_minutes: u32,
    /// Clear copied credentials from the clipboard after this many seconds.
    pub clipboard_clear_seconds: u32,
    pub terminal_font_size: u16,
    /// Check for updates at startup and every 12 hours (official builds only).
    #[serde(default = "yes")]
    pub auto_update: bool,
}

fn yes() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::Dark,
            auto_lock_minutes: 15,
            clipboard_clear_seconds: 30,
            terminal_font_size: 14,
            auto_update: true,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Library {
    pub folders: Vec<Folder>,
    pub hosts: Vec<Host>,
    pub credentials: Vec<Credential>,
    pub tunnels: Vec<Tunnel>,
    pub snippets: Vec<Snippet>,
}
