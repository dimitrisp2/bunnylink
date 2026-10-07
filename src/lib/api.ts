// SPDX-FileCopyrightText: 2026 BunnyCloud.IT
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial
import { invoke, Channel } from "@tauri-apps/api/core";

export type Protocol =
  | "ssh" | "sftp" | "rdp" | "vnc" | "ftp" | "telnet" | "serial" | "http" | "https" | "smb" | "spice";

/** `soon`: not supported yet, so it is not offered for new endpoints. */
export const PROTOCOLS: { id: Protocol; label: string; port: number; soon?: boolean }[] = [
  { id: "ssh", label: "SSH", port: 22 },
  { id: "sftp", label: "SFTP", port: 22 },
  { id: "rdp", label: "RDP", port: 3389 },
  { id: "vnc", label: "VNC", port: 5900 },
  { id: "https", label: "HTTPS", port: 443 },
  { id: "http", label: "HTTP", port: 80 },
  { id: "smb", label: "SMB", port: 445 },
  { id: "ftp", label: "FTP", port: 21 },
  { id: "telnet", label: "Telnet", port: 23 },
  { id: "serial", label: "Serial", port: 0 },
  { id: "spice", label: "SPICE", port: 5930, soon: true },
];

export interface SerialSettings { baud: number; dataBits: number; parity: "none" | "odd" | "even"; stopBits: number; flow: "none" | "software" | "hardware" }
export const DEFAULT_SERIAL: SerialSettings = { baud: 9600, dataBits: 8, parity: "none", stopBits: 1, flow: "none" };
export interface Endpoint { protocol: Protocol; port?: number | null; path?: string | null; serial?: SerialSettings | null }
export interface Defaults { username?: string | null; credentialId?: string | null; jumpHostId?: string | null }
export interface Folder { id: string; parentId?: string | null; name: string; defaults: Defaults }
export interface Host {
  id: string;
  folderId?: string | null;
  name: string;
  address: string;
  endpoints: Endpoint[];
  tags: string[];
  overrides: Defaults;
  notes: string;
  pinned: boolean;
  lastUsed?: number | null;
}
export interface Effective {
  username?: string | null;
  credentialId?: string | null;
  jumpHostId?: string | null;
  usernameFrom?: string | null;
  credentialFrom?: string | null;
  jumpHostFrom?: string | null;
}
export type CredentialKind = "password" | "key" | "agent";
export interface Credential { id: string; name: string; username?: string | null; kind: CredentialKind; askPassphrase?: boolean }
export type TunnelKind = "local" | "remote" | "socks";
export interface Tunnel {
  id: string;
  name: string;
  kind: TunnelKind;
  hostId: string;
  bindPort: number;
  targetHost?: string | null;
  targetPort?: number | null;
}
export type Theme = "dark" | "light" | "auto";
export type UpdateStatus =
  | { state: "unavailable" }
  | { state: "idle" }
  | { state: "checking" }
  | { state: "upToDate"; checkedAt: number }
  | { state: "downloading"; version: string; done: number; total: number }
  | { state: "ready"; version: string; changelog: string }
  | { state: "error"; message: string };
export interface UpdateInfo { current: string; currentChangelog: string; status: UpdateStatus }
export interface Settings {
  theme: Theme;
  autoLockMinutes: number;
  stayUnlockedWebConsole: boolean;
  stayUnlockedDesktop: boolean;
  stayUnlockedTransfer: boolean;
  keepSessionsWhenLocked: boolean;
  clipboardClearSeconds: number;
  terminalFontSize: number;
  autoUpdate: boolean;
}
/** A saved command. With no folders and no tags it applies to every host. */
export interface Snippet {
  id: string;
  name: string;
  command: string;
  description: string;
  sendEnter: boolean;
  folderIds: string[];
  tags: string[];
}
export interface Library { folders: Folder[]; hosts: Host[]; credentials: Credential[]; tunnels: Tunnel[]; snippets: Snippet[] }
export interface VaultStatus { initialized: boolean; unlocked: boolean; firstRun: boolean }
export type TermEvent =
  | { type: "notice"; text: string }
  | { type: "prompt"; text: string }
  | { type: "data"; data: number[] }
  | { type: "exit"; code: number | null }
  | { type: "error"; message: string };
export interface ExecResult { output: string; exitCode: number | null }
export interface TunnelStatus { id: string; state: "connecting" | "open" | "closed"; message?: string | null }

export interface FileEntry {
  name: string;
  path: string;
  isDir: boolean;
  isLink: boolean;
  size: number;
  modified: number | null;
  permissions: number | null;
}
export interface Listing { path: string; entries: FileEntry[] }
export interface Transfer {
  id: string;
  sessionId: string;
  upload: boolean;
  label: string;
  state: "running" | "done" | "error" | "cancelled";
  done: number;
  total: number;
  current: string;
  message?: string | null;
  saved: string[];
}

export type DesktopInput =
  | { type: "mouseMove"; x: number; y: number }
  | { type: "mouseButton"; button: number; down: boolean }
  | { type: "wheel"; vertical: boolean; units: number }
  | { type: "key"; scancode?: number; keysym?: number; down: boolean }
  | { type: "unicode"; ch: string; down: boolean }
  | { type: "releaseAll" }
  | { type: "resize"; width: number; height: number };
export type DesktopEvent =
  | { type: "notice"; text: string }
  | { type: "connected"; width: number; height: number }
  | { type: "resized"; width: number; height: number }
  | { type: "closed"; reason: string }
  | { type: "error"; message: string };

export type ImportSource = "openssh" | "putty" | "mremoteng";
export interface ImportPreviewHost {
  name: string; address: string; protocols: Protocol[]; folder?: string | null; username?: string | null;
  auth: "password" | "key" | "agent" | "none";
}
export interface ImportPreview { hosts: ImportPreviewHost[]; folders: number; warnings: string[]; encryptedKeys: { path: string; hosts: number }[] }
/** Per encrypted key file: the passphrase to save, or null to ask on every connection. */
export type KeyChoices = Record<string, string | null>;
export interface PassphraseRequest { id: string; host: string; key: string; retry: boolean }
export interface ImportApplied { hosts: number; folders: number; credentials: number; rootFolderId: string }

export const api = {
  vaultStatus: () => invoke<VaultStatus>("vault_status"),
  vaultCreate: (password: string, portable: boolean) => invoke<void>("vault_create", { password, portable }),
  vaultUnlock: (password: string) => invoke<void>("vault_unlock", { password }),
  vaultLock: () => invoke<void>("vault_lock"),
  userActivity: () => invoke<void>("user_activity"),
  updateStatus: () => invoke<UpdateInfo>("update_status"),
  updateCheck: () => invoke<void>("update_check"),
  updateInstall: () => invoke<void>("update_install"),
  passphraseReply: (id: string, passphrase: string | null) => invoke<void>("passphrase_reply", { id, passphrase }),

  library: () => invoke<Library>("get_library"),
  importPreview: (source: ImportSource, path: string | null, password: string | null) =>
    invoke<ImportPreview>("import_preview", { source, path, password }),
  importApply: (source: ImportSource, path: string | null, password: string | null, keys: KeyChoices) =>
    invoke<ImportApplied>("import_apply", { source, path, password, keys }),
  effective: (hostId: string) => invoke<Effective>("get_effective", { hostId }),
  saveFolder: (folder: Folder) => invoke<Folder>("save_folder", { folder }),
  deleteFolder: (id: string) => invoke<void>("delete_folder", { id }),
  saveHost: (host: Host) => invoke<Host>("save_host", { host }),
  deleteHost: (id: string) => invoke<void>("delete_host", { id }),
  saveCredential: (credential: Credential, secret?: string, passphrase?: string) =>
    invoke<Credential>("save_credential", { input: { credential, secret, passphrase } }),
  deleteCredential: (id: string) => invoke<void>("delete_credential", { id }),
  saveTunnel: (tunnel: Tunnel) => invoke<Tunnel>("save_tunnel", { tunnel }),
  deleteTunnel: (id: string) => invoke<void>("delete_tunnel", { id }),
  saveSnippet: (snippet: Snippet) => invoke<Snippet>("save_snippet", { snippet }),
  deleteSnippet: (id: string) => invoke<void>("delete_snippet", { id }),
  settings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<void>("save_settings", { settings }),
  forgetHostKey: (hostId: string) => invoke<void>("forget_host_key", { hostId }),
  copyCredential: (hostId: string, field: "username" | "password") =>
    invoke<void>("copy_credential", { hostId, field }),

  terminalOpen: (hostId: string, protocol: Protocol | undefined, cols: number, rows: number, onEvent: (e: TermEvent) => void) => {
    const ch = new Channel<TermEvent>();
    ch.onmessage = onEvent;
    return invoke<string>("terminal_open", { hostId, protocol, cols, rows, onEvent: ch });
  },
  serialPorts: () => invoke<string[]>("serial_ports"),
  terminalInput: (sessionId: string, data: string) => invoke<void>("terminal_input", { sessionId, data }),
  terminalResize: (sessionId: string, cols: number, rows: number) =>
    invoke<void>("terminal_resize", { sessionId, cols, rows }),
  terminalClose: (sessionId: string) => invoke<void>("terminal_close", { sessionId }),
  runCommand: (hostId: string, command: string) => invoke<ExecResult>("run_command", { hostId, command }),

  filesOpen: (hostId: string, protocol?: Protocol) =>
    invoke<{ sessionId: string; home: string; notices: string[] }>("files_open", { hostId, protocol }),
  filesList: (sessionId: string, path: string) => invoke<Listing>("files_list", { sessionId, path }),
  filesMkdir: (sessionId: string, path: string) => invoke<void>("files_mkdir", { sessionId, path }),
  filesRename: (sessionId: string, from: string, to: string) => invoke<void>("files_rename", { sessionId, from, to }),
  filesDelete: (sessionId: string, paths: string[]) => invoke<void>("files_delete", { sessionId, paths }),
  filesClose: (sessionId: string) => invoke<void>("files_close", { sessionId }),
  filesDownload: (sessionId: string, paths: string[], localDir?: string) =>
    invoke<string>("files_download", { sessionId, paths, localDir }),
  filesUpload: (sessionId: string, localPaths: string[], remoteDir: string) =>
    invoke<string>("files_upload", { sessionId, localPaths, remoteDir }),
  transferCancel: (id: string) => invoke<void>("transfer_cancel", { id }),

  desktopOpen: (
    hostId: string,
    protocol: "rdp" | "vnc",
    width: number,
    height: number,
    onEvent: (e: DesktopEvent | ArrayBuffer) => void,
  ) => {
    const ch = new Channel<DesktopEvent | ArrayBuffer>();
    ch.onmessage = onEvent;
    return invoke<string>("desktop_open", { hostId, protocol, width, height, onEvent: ch });
  },
  desktopInput: (sessionId: string, events: DesktopInput[]) => invoke<void>("desktop_input", { sessionId, events }),
  desktopClose: (sessionId: string) => invoke<void>("desktop_close", { sessionId }),

  tunnelStart: (id: string) => invoke<void>("tunnel_start", { id }),
  tunnelStop: (id: string) => invoke<void>("tunnel_stop", { id }),
  tunnelActive: () => invoke<string[]>("tunnel_active"),
};

export function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
}
