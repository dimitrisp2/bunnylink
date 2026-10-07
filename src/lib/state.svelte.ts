// SPDX-FileCopyrightText: 2026 BunnyCloud.IT
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial
import { listen } from "@tauri-apps/api/event";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, errorText, PROTOCOLS } from "./api";
import type { Folder, Host, Library, PassphraseRequest, Protocol, Settings, Snippet, UpdateInfo, Theme, Transfer, Tunnel, TunnelStatus, VaultStatus, Credential } from "./api";

export type TabKind = "terminal" | "command" | "files" | "desktop" | "settings";
export type TabStatus = "connecting" | "open" | "closed" | "error";
export interface Tab { id: string; kind: TabKind; hostId: string; title: string; status: TabStatus; protocol?: Protocol }
export type Pane = "left" | "right";

export type Modal =
  | { kind: "host"; host: Host | null; folderId?: string | null }
  | { kind: "folder"; folder: Folder | null; parentId?: string | null }
  | { kind: "credential"; credential: Credential | null; onSaved?: (c: Credential) => void }
  | { kind: "tunnel"; tunnel: Tunnel | null; hostId?: string; targetPort?: number }
  | { kind: "import" }
  | { kind: "snippet"; snippet: Snippet | null }
  | { kind: "snippetRun"; snippet: Snippet; tabId: string; fields: Placeholder[]; values: Record<string, string> }
  | { kind: "confirm"; title: string; body: string; confirm: string; run: () => Promise<void> | void };

export interface Toast { id: number; text: string; kind: "info" | "error" }

export type MenuItem = { label: string; run: () => void; danger?: boolean; disabled?: boolean } | "separator";
export interface ContextMenu { x: number; y: number; items: MenuItem[] }

export const HOME = "home";

export const app = $state({
  ready: false,
  /** The theme actually shown ("auto" resolved against the OS setting). */
  shownTheme: "dark" as "dark" | "light",
  vault: { initialized: false, unlocked: false, firstRun: false } as VaultStatus,
  library: { folders: [], hosts: [], credentials: [], tunnels: [], snippets: [] } as Library,
  settings: { theme: "dark", autoLockMinutes: 15, clipboardClearSeconds: 30, terminalFontSize: 14, autoUpdate: true } as Settings,
  selectedHostId: null as string | null,
  sidebar: "hosts" as "hosts" | "tunnels" | "credentials" | "snippets",
  groupBy: "folders" as "folders" | "tags",
  tagFilter: null as string | null,
  filter: "",
  collapsed: {} as Record<string, boolean>,
  tabs: [] as Tab[],
  panes: { left: HOME, right: null as string | null },
  focus: "left" as Pane,
  paletteOpen: false,
  modal: null as Modal | null,
  menu: null as ContextMenu | null,
  /** Set while asking whether to quit with active connections: what would be closed. */
  quitConfirm: null as string | null,
  update: { current: "", currentChangelog: "", status: { state: "unavailable" } } as UpdateInfo,
  /** Connections waiting for a key passphrase; the first one is shown. */
  passphrasePrompts: [] as PassphraseRequest[],
  toasts: [] as Toast[],
  tunnelState: {} as Record<string, { state: TunnelStatus["state"]; message?: string | null }>,
  transfers: {} as Record<string, Transfer>,
});

// ------------------------------------------------------------------ toasts

let toastSeq = 0;
export function toast(text: string, kind: Toast["kind"] = "info") {
  const id = ++toastSeq;
  app.toasts.push({ id, text, kind });
  setTimeout(() => (app.toasts = app.toasts.filter((t) => t.id !== id)), kind === "error" ? 7000 : 3500);
}

export async function attempt<T>(fn: () => Promise<T>): Promise<T | undefined> {
  try {
    return await fn();
  } catch (e) {
    toast(errorText(e), "error");
    return undefined;
  }
}

// ------------------------------------------------------------------ theme

let media: MediaQueryList | null = null;
export function applyTheme(theme: Theme) {
  const root = document.documentElement;
  media ??= window.matchMedia("(prefers-color-scheme: light)");
  const resolve = () => (theme === "auto" ? (media!.matches ? "light" : "dark") : theme);
  const set = () => {
    const t = resolve();
    root.dataset.theme = t;
    app.shownTheme = t;
  };
  set();
  media.onchange = set;
}

// ------------------------------------------------------------------ boot

export async function boot() {
  app.vault = await api.vaultStatus();
  app.settings = await api.settings();
  applyTheme(app.settings.theme);
  await listen<TunnelStatus>("tunnel-status", (e) => {
    const { id, state, message } = e.payload;
    app.tunnelState[id] = { state, message };
    if (state === "closed" && message) toast(message, "error");
  });
  await listen<Transfer>("transfer", (e) => {
    const t = e.payload;
    if (app.transfers[t.id]?.state === "cancelled") return;
    app.transfers[t.id] = t;
    // Finished transfers tidy themselves away.
    if (t.state === "done") setTimeout(() => delete app.transfers[t.id], 12000);
    if (t.state === "error") toast(`${t.upload ? "Upload" : "Download"} of ${t.label} failed: ${t.message}`, "error");
  });
  await guardWindowClose();
  app.update = await api.updateStatus();
  await listen<UpdateInfo>("update-status", (e) => (app.update = e.payload));
  await listen<PassphraseRequest>("passphrase-request", (e) => {
    app.passphrasePrompts.push(e.payload);
  });
  await listen("vault-locked", () => {
    app.vault.unlocked = false;
    for (const r of app.passphrasePrompts) api.passphraseReply(r.id, null);
    app.passphrasePrompts = [];
    app.modal = null;
    app.paletteOpen = false;
  });
  app.ready = true;
}

export async function afterUnlock() {
  app.vault = await api.vaultStatus();
  await reload();
  for (const id of await api.tunnelActive()) app.tunnelState[id] ??= { state: "open" };
}

export async function reload() {
  app.library = await api.library();
}

// ------------------------------------------------------------------ lookups

export const hostById = (id: string | null | undefined) => app.library.hosts.find((h) => h.id === id);
export const folderById = (id: string | null | undefined) => app.library.folders.find((f) => f.id === id);
export const credentialById = (id: string | null | undefined) => app.library.credentials.find((c) => c.id === id);
export const hasProtocol = (h: Host, ...p: Protocol[]) => h.endpoints.some((e) => p.includes(e.protocol));
export const portOf = (h: Host, p: Protocol) =>
  h.endpoints.find((e) => e.protocol === p)?.port ?? PROTOCOLS.find((x) => x.id === p)!.port;

export function folderPath(id: string | null | undefined): string {
  const parts: string[] = [];
  let f = folderById(id);
  while (f && parts.length < 16) {
    parts.unshift(f.name);
    f = folderById(f.parentId);
  }
  return parts.join(" / ");
}

export function allTags(): string[] {
  return [...new Set(app.library.hosts.flatMap((h) => h.tags))].sort((a, b) => a.localeCompare(b));
}

// ------------------------------------------------------------------ tabs & panes

let tabSeq = 0;

export function showTab(id: string, pane: Pane = app.focus) {
  if (app.panes.right === id && pane === "left") pane = "right";
  if (app.panes.left === id && pane === "right") pane = "left";
  app.panes[pane] = id;
  app.focus = pane;
}

const PROTOCOL_NAMES: Partial<Record<Protocol, string>> = { telnet: "Telnet", serial: "Serial", ftp: "FTP", smb: "SMB", vnc: "VNC" };

function tabTitle(kind: TabKind, name: string, protocol?: Protocol): string {
  const proto = protocol ? PROTOCOL_NAMES[protocol] : undefined;
  if (kind === "command") return `${name} · Run`;
  if (kind === "files") return `${name} · ${proto ?? "Files"}`;
  return proto ? `${name} · ${proto}` : name;
}

export function openTab(kind: TabKind, hostId: string, pane?: Pane, protocol?: Protocol): Tab {
  const host = hostById(hostId)!;
  const tab: Tab = {
    id: `t${++tabSeq}`,
    kind,
    hostId,
    title: tabTitle(kind, host.name, protocol),
    status: "connecting",
    protocol,
  };
  app.tabs.push(tab);
  showTab(tab.id, pane ?? (app.panes.right && app.focus === "right" ? "right" : "left"));
  return tab;
}

/** Opens the Settings tab, or switches to it when it is already open. */
export function openSettings() {
  let tab = app.tabs.find((t) => t.kind === "settings");
  if (!tab) {
    tab = { id: `t${++tabSeq}`, kind: "settings", hostId: "", title: "Settings", status: "closed" };
    app.tabs.push(tab);
  }
  showTab(tab.id, app.panes.right && app.focus === "right" ? "right" : "left");
}

export function closeTab(id: string) {
  const idx = app.tabs.findIndex((t) => t.id === id);
  if (idx < 0) return;
  app.tabs.splice(idx, 1);
  const fallback = app.tabs[Math.min(idx, app.tabs.length - 1)]?.id ?? HOME;
  if (app.panes.right === id) {
    app.panes.right = null;
    app.focus = "left";
  }
  if (app.panes.left === id) {
    app.panes.left = fallback === app.panes.right ? HOME : fallback;
  }
}

const isLive = (t: Tab) => t.status === "open" || t.status === "connecting";

/** What closing the app would interrupt, e.g. ["2 connections", "1 tunnel"]. */
function activeWork(): string[] {
  const count = (n: number, one: string) => (n ? [`${n} ${one}${n === 1 ? "" : "s"}`] : []);
  return [
    ...count(app.tabs.filter(isLive).length, "connection"),
    ...count(Object.values(app.tunnelState).filter((t) => t.state !== "closed").length, "tunnel"),
    ...count(Object.values(app.transfers).filter((t) => t.state === "running").length, "file transfer"),
  ];
}

/** Asks before the window closes while connections, tunnels or transfers are active. */
async function guardWindowClose() {
  const win = getCurrentWindow();
  await win.onCloseRequested((e) => {
    const active = activeWork();
    if (!active.length) return;
    e.preventDefault();
    app.menu = null;
    // Kept apart from app.modal so an open editor isn't lost when the user stays.
    app.quitConfirm = `${active.join(", ")} will be closed.`;
  });
}

/** Restarts into the downloaded update, asking first when work would be interrupted. */
export function restartToUpdate() {
  const install = () => attempt(() => api.updateInstall());
  const active = activeWork();
  if (!active.length) return void install();
  const version = app.update.status.state === "ready" ? app.update.status.version : "";
  app.modal = {
    kind: "confirm",
    title: `Restart to update to ${version}?`,
    body: `${active.join(", ")} will be closed.`,
    confirm: "Restart",
    run: install,
  };
}

export function quitNow() {
  getCurrentWindow().destroy();
}

/** Closes a tab, asking first when it is connected or connecting. */
export function requestCloseTab(id: string) {
  const t = app.tabs.find((x) => x.id === id);
  if (!t) return;
  if (!isLive(t)) return closeTab(id);
  app.modal = {
    kind: "confirm",
    title: `Close ${t.title}?`,
    body: "The connection in this tab will be closed.",
    confirm: "Close tab",
    run: () => closeTab(id),
  };
}

/** Opens the same session again in a new tab right after the original. */
export function duplicateTab(id: string) {
  const t = app.tabs.find((x) => x.id === id);
  if (!t) return;
  const copy = openTab(t.kind, t.hostId, undefined, t.protocol);
  app.tabs.splice(app.tabs.findIndex((x) => x.id === copy.id), 1);
  app.tabs.splice(app.tabs.findIndex((x) => x.id === id) + 1, 0, copy);
}

/** Drops the tab's connection and connects again in the same place. */
export function reconnectTab(id: string) {
  const t = app.tabs.find((x) => x.id === id);
  if (!t) return;
  const run = () => {
    const idx = app.tabs.findIndex((x) => x.id === id);
    if (idx < 0) return;
    // A new id remounts the view: the old one closes its session, the new one connects.
    const fresh: Tab = { ...app.tabs[idx], id: `t${++tabSeq}`, status: "connecting" };
    app.tabs[idx] = fresh;
    if (app.panes.left === id) app.panes.left = fresh.id;
    if (app.panes.right === id) app.panes.right = fresh.id;
  };
  if (!isLive(t)) return run();
  app.modal = {
    kind: "confirm",
    title: `Reconnect ${t.title}?`,
    body: "The current connection in this tab will be closed and a new one opened.",
    confirm: "Reconnect",
    run,
  };
}

export function tabMenu(e: MouseEvent, id: string) {
  if (app.tabs.find((t) => t.id === id)?.kind === "settings")
    return openMenu(e, [{ label: "Close", run: () => closeTab(id) }]);
  openMenu(e, [
    { label: "Duplicate", run: () => duplicateTab(id) },
    { label: "Reconnect", run: () => reconnectTab(id) },
    "separator",
    { label: "Close", run: () => requestCloseTab(id) },
  ]);
}

export function splitRight(id: string) {
  if (app.panes.left === id) app.panes.left = HOME;
  app.panes.right = id;
  app.focus = "right";
}

export function unsplit() {
  if (app.panes.right && app.focus === "right") app.panes.left = app.panes.right;
  app.panes.right = null;
  app.focus = "left";
}

export function cycleTab(dir: 1 | -1) {
  const ids = [HOME, ...app.tabs.map((t) => t.id)];
  const cur = ids.indexOf(app.panes[app.focus] ?? HOME);
  showTab(ids[(cur + dir + ids.length) % ids.length]);
}

// ------------------------------------------------------------------ actions

export interface HostAction {
  key: string;
  label: string;
  detail: string;
  icon: string;
  protocol?: Protocol;
  ready: boolean;
  run: () => void;
}

const soon = (what: string) => () => toast(`${what} is not available yet. It is on the roadmap.`);

export function hostActions(h: Host): HostAction[] {
  const out: HostAction[] = [];
  const add = (a: HostAction) => out.push(a);
  const p = (proto: Protocol) => `${PROTOCOLS.find((x) => x.id === proto)!.label} · ${portOf(h, proto)}`;

  if (hasProtocol(h, "ssh"))
    add({ key: "terminal", label: "Open terminal", detail: p("ssh"), icon: "terminal", protocol: "ssh", ready: true, run: () => openTab("terminal", h.id) });
  if (hasProtocol(h, "telnet"))
    add({ key: "telnet", label: "Open terminal", detail: p("telnet"), icon: "terminal", protocol: "telnet", ready: true, run: () => openTab("terminal", h.id, undefined, "telnet") });
  if (hasProtocol(h, "serial")) {
    const ep = h.endpoints.find((e) => e.protocol === "serial");
    add({ key: "serial", label: "Open console", detail: `Serial · ${ep?.path || "no device"} · ${ep?.serial?.baud ?? 9600}`, icon: "terminal", protocol: "serial", ready: true, run: () => openTab("terminal", h.id, undefined, "serial") });
  }
  if (hasProtocol(h, "rdp"))
    add({ key: "rdp", label: "Open desktop", detail: p("rdp"), icon: "monitor", protocol: "rdp", ready: true, run: () => openTab("desktop", h.id, undefined, "rdp") });
  if (hasProtocol(h, "vnc"))
    add({ key: "vnc", label: "Open desktop", detail: p("vnc"), icon: "monitor", protocol: "vnc", ready: true, run: () => openTab("desktop", h.id, undefined, "vnc") });
  for (const proto of ["spice"] as Protocol[])
    if (hasProtocol(h, proto))
      add({ key: proto, label: "Open desktop", detail: p(proto), icon: "monitor", protocol: proto, ready: false, run: soon(proto.toUpperCase()) });
  if (hasProtocol(h, "sftp", "ssh"))
    add({ key: "sftp", label: "Browse files", detail: `SFTP · ${portOf(h, hasProtocol(h, "sftp") ? "sftp" : "ssh")}`, icon: "folder", protocol: "sftp", ready: true, run: () => openTab("files", h.id) });
  for (const proto of ["smb", "ftp"] as Protocol[])
    if (hasProtocol(h, proto))
      add({ key: proto, label: "Browse files", detail: p(proto), icon: "folder", protocol: proto, ready: true, run: () => openTab("files", h.id, undefined, proto) });
  for (const proto of ["https", "http"] as Protocol[])
    if (hasProtocol(h, proto))
      add({ key: proto, label: "Open web console", detail: p(proto), icon: "globe", protocol: proto, ready: true, run: () => openWeb(h, proto) });
  if (hasProtocol(h, "ssh")) {
    add({ key: "run", label: "Run command", detail: "SSH · one-off", icon: "play", protocol: "ssh", ready: true, run: () => openTab("command", h.id) });
    add({ key: "tunnel", label: "Port forward", detail: "Local, remote or SOCKS", icon: "tunnel", protocol: "ssh", ready: true, run: () => (app.modal = { kind: "tunnel", tunnel: null, hostId: h.id }) });
  }
  add({ key: "copy-user", label: "Copy username", detail: "To clipboard", icon: "user", ready: true, run: () => copyCredential(h, "username") });
  add({ key: "copy-pass", label: "Copy password", detail: `Clears after ${app.settings.clipboardClearSeconds} s`, icon: "key", ready: true, run: () => copyCredential(h, "password") });
  return out;
}

// ------------------------------------------------------------------ context menus

export function openMenu(e: MouseEvent, items: MenuItem[]) {
  e.preventDefault();
  e.stopPropagation();
  app.menu = { x: e.clientX, y: e.clientY, items };
}

export async function togglePin(h: Host) {
  await attempt(() => api.saveHost({ ...h, pinned: !h.pinned }));
  await reload();
}

export function deleteHost(h: Host) {
  app.modal = {
    kind: "confirm",
    title: `Delete ${h.name}?`,
    body: "The host and its settings are removed. Credentials and tunnels are kept.",
    confirm: "Delete host",
    run: async () => {
      await attempt(() => api.deleteHost(h.id));
      if (app.selectedHostId === h.id) app.selectedHostId = null;
      await reload();
    },
  };
}

export async function forgetHostKey(h: Host) {
  await attempt(() => api.forgetHostKey(h.id));
  toast(`Forgot the saved host key for ${h.name}. The next connection trusts the new key.`);
}

/** Host keys only exist for SSH-based connections. */
export const hasHostKey = (h: Host) => hasProtocol(h, "ssh", "sftp");

/** Opens the editor on an unsaved copy of the host; it is created when saved. */
export function duplicateHost(h: Host) {
  const copy: Host = { ...$state.snapshot(h), id: "", name: `${h.name} (copy)`, lastUsed: null };
  app.modal = { kind: "host", host: copy };
}

export function hostMenu(e: MouseEvent, h: Host) {
  const proto = (p?: Protocol) => (p ? ` (${PROTOCOLS.find((x) => x.id === p)?.label ?? p})` : "");
  openMenu(e, [
    { label: "Edit", run: () => (app.modal = { kind: "host", host: h }) },
    { label: "Duplicate", run: () => duplicateHost(h) },
    "separator",
    ...hostActions(h).map((a): MenuItem => ({ label: a.label + proto(a.protocol), run: a.run, disabled: !a.ready })),
    "separator",
    { label: h.pinned ? "Unpin" : "Pin", run: () => togglePin(h) },
    ...(hasHostKey(h) ? [{ label: "Forget host key", run: () => forgetHostKey(h) }] : []),
    "separator",
    { label: "Delete", run: () => deleteHost(h), danger: true },
  ]);
}

export async function copyCredential(h: Host, field: "username" | "password") {
  try {
    await api.copyCredential(h.id, field);
    toast(field === "password" ? `Password for ${h.name} copied.` : `Username for ${h.name} copied.`);
  } catch (e) {
    toast(errorText(e), "error");
  }
}

let webSeq = 0;
export function openWeb(h: Host, proto: Protocol) {
  const ep = h.endpoints.find((e) => e.protocol === proto);
  const port = ep?.port ?? (proto === "https" ? 443 : 80);
  const defaultPort = proto === "https" ? 443 : 80;
  const path = ep?.path ? (ep.path.startsWith("/") ? ep.path : `/${ep.path}`) : "/";
  const url = `${proto}://${h.address}${port === defaultPort ? "" : `:${port}`}${path}`;
  const w = new WebviewWindow(`web-${++webSeq}`, { url, title: `${h.name} — BunnyLink`, width: 1200, height: 800 });
  w.once("tauri://error", (e) => toast(`Could not open ${url}: ${errorText(e.payload)}`, "error"));
}

// ------------------------------------------------------------------ tunnels

export async function toggleTunnel(t: Tunnel) {
  const st = app.tunnelState[t.id]?.state;
  if (st === "open" || st === "connecting") {
    await attempt(() => api.tunnelStop(t.id));
  } else {
    app.tunnelState[t.id] = { state: "connecting" };
    try {
      await api.tunnelStart(t.id);
    } catch (e) {
      app.tunnelState[t.id] = { state: "closed", message: errorText(e) };
      toast(errorText(e), "error");
    }
  }
}

export function deleteTunnel(t: Tunnel) {
  app.modal = {
    kind: "confirm", title: `Delete tunnel ${t.name}?`, body: "The tunnel is closed if it is open.", confirm: "Delete tunnel",
    run: async () => { await attempt(() => api.deleteTunnel(t.id)); await reload(); },
  };
}

export function tunnelMenu(e: MouseEvent, t: Tunnel) {
  openMenu(e, [
    { label: "Edit", run: () => (app.modal = { kind: "tunnel", tunnel: t }) },
    "separator",
    { label: "Delete", run: () => deleteTunnel(t), danger: true },
  ]);
}

// ------------------------------------------------------------------ credentials

export function deleteCredential(c: Credential) {
  app.modal = {
    kind: "confirm",
    title: `Delete credential ${c.name}?`,
    body: "Hosts and folders using it will have no credential until you pick another.",
    confirm: "Delete credential",
    run: async () => { await attempt(() => api.deleteCredential(c.id)); await reload(); },
  };
}

export function credentialMenu(e: MouseEvent, c: Credential) {
  openMenu(e, [
    { label: "Edit", run: () => (app.modal = { kind: "credential", credential: c }) },
    "separator",
    { label: "Delete", run: () => deleteCredential(c), danger: true },
  ]);
}

export function tunnelSummary(t: Tunnel): string {
  const via = hostById(t.hostId)?.name ?? "?";
  if (t.kind === "socks") return `SOCKS on localhost:${t.bindPort} via ${via}`;
  if (t.kind === "local") return `localhost:${t.bindPort} → ${t.targetHost}:${t.targetPort} via ${via}`;
  return `${via}:${t.bindPort} → ${t.targetHost}:${t.targetPort} (remote)`;
}

// ------------------------------------------------------------------ snippets

/** Where a snippet can be sent: an open terminal or a Run command tab. */
export interface SnippetTarget { send: (text: string, enter: boolean) => void; focus: () => void }
const snippetTargets = new Map<string, SnippetTarget>();

/** Called by terminal and command views; returns the unregister function. */
export function registerSnippetTarget(tabId: string, target: SnippetTarget) {
  snippetTargets.set(tabId, target);
  return () => snippetTargets.delete(tabId);
}

/** The tab in the focused pane, when it can take a snippet. */
export function snippetTab(): Tab | undefined {
  const t = app.tabs.find((x) => x.id === app.panes[app.focus]);
  return t && (t.kind === "terminal" || t.kind === "command") ? t : undefined;
}

/** True when the snippet is global, or the host is in one of its folders or has one of its tags. */
export function snippetApplies(s: Snippet, h: Host | undefined): boolean {
  if (!s.folderIds.length && !s.tags.length) return true;
  if (!h) return false;
  if (h.tags.some((t) => s.tags.includes(t))) return true;
  let f = folderById(h.folderId);
  for (let guard = 0; f && guard < 32; guard++, f = folderById(f.parentId)) if (s.folderIds.includes(f.id)) return true;
  return false;
}

export interface Placeholder { name: string; default: string }
const PLACEHOLDER = /\{\{\s*([\w.-]+)\s*(?::([^}]*))?\}\}/g;
/** Filled in from the host without asking. */
export const BUILTIN_PLACEHOLDERS = ["host", "address", "user"];

/** The placeholders in a command, in order, each once. */
export function placeholders(command: string): Placeholder[] {
  const out: Placeholder[] = [];
  for (const m of command.matchAll(PLACEHOLDER))
    if (!out.some((p) => p.name === m[1])) out.push({ name: m[1], default: m[2]?.trim() ?? "" });
  return out;
}

export const fillPlaceholders = (command: string, values: Record<string, string>) =>
  command.replace(PLACEHOLDER, (_, name: string, def?: string) => values[name] ?? def?.trim() ?? "");

/** Values typed for each snippet this session, offered again next time. */
const lastValues: Record<string, Record<string, string>> = {};

/** Sends a snippet to the focused terminal, asking first for any placeholders. */
export async function runSnippet(s: Snippet, tabId = snippetTab()?.id) {
  const tab = app.tabs.find((t) => t.id === tabId);
  if (!tab || !snippetTargets.has(tab.id)) return toast("Open a terminal to send a snippet to.");
  const h = hostById(tab.hostId);
  const values: Record<string, string> = { host: h?.name ?? "", address: h?.address ?? "" };
  const fields = placeholders(s.command);
  if (fields.some((f) => f.name === "user"))
    values.user = (await api.effective(tab.hostId).catch(() => null))?.username ?? "";
  const ask = fields.filter((f) => !BUILTIN_PLACEHOLDERS.includes(f.name));
  if (!ask.length) return sendSnippet(s, tab.id, values);
  const last = lastValues[s.id] ?? {};
  for (const f of ask) values[f.name] = last[f.name] ?? f.default;
  app.modal = { kind: "snippetRun", snippet: s, tabId: tab.id, fields: ask, values };
}

export function sendSnippet(s: Snippet, tabId: string, values: Record<string, string>) {
  const target = snippetTargets.get(tabId);
  if (!target) return toast("That tab is closed.");
  lastValues[s.id] = { ...values };
  target.send(fillPlaceholders(s.command, values), s.sendEnter);
  showTab(tabId);
  requestAnimationFrame(() => target.focus());
}

export function deleteSnippet(s: Snippet) {
  app.modal = {
    kind: "confirm", title: `Delete snippet ${s.name}?`, body: "The saved command is removed.", confirm: "Delete snippet",
    run: async () => { await attempt(() => api.deleteSnippet(s.id)); await reload(); },
  };
}

export function snippetMenu(e: MouseEvent, s: Snippet) {
  const tab = snippetTab();
  openMenu(e, [
    { label: tab ? `Send to ${tab.title}` : "Send to terminal", run: () => runSnippet(s), disabled: !tab },
    { label: "Edit", run: () => (app.modal = { kind: "snippet", snippet: s }) },
    "separator",
    { label: "Delete", run: () => deleteSnippet(s), danger: true },
  ]);
}

export function newSnippet(): Snippet {
  return { id: "", name: "", command: "", description: "", sendEnter: true, folderIds: [], tags: [] };
}

// ------------------------------------------------------------------ editing helpers

export function newHost(folderId: string | null = null): Host {
  return {
    id: "",
    folderId,
    name: "",
    address: "",
    endpoints: [{ protocol: "ssh", port: 22 }],
    tags: [],
    overrides: {},
    notes: "",
    pinned: false,
  };
}

export async function saveSettings(s: Settings) {
  app.settings = s;
  applyTheme(s.theme);
  await attempt(() => api.saveSettings(s));
}
