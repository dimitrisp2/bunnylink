<!-- SPDX-FileCopyrightText: 2026 BunnyCloud.IT -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial -->
<script lang="ts">
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import Modal from "./Modal.svelte";
  import Icon from "./Icon.svelte";
  import { api, errorText, PROTOCOLS } from "../api";
  import { hasSecret, readSecret, takeSecret } from "../secret";
  import type { ImportPreview, ImportSource } from "../api";
  import { app, reload, toast } from "../state.svelte";

  const sources: { id: ImportSource; label: string; hint: string }[] = [
    { id: "mremoteng", label: "mRemoteNG", hint: "A confCons.xml connections file. Saved passwords are imported into the vault." },
    { id: "putty", label: "PuTTY", hint: "Saved sessions (from the registry on Windows, ~/.putty elsewhere), including their key files." },
    { id: "openssh", label: "SSH config", hint: "Hosts from an OpenSSH config file, including ProxyJump and identity files." },
  ];

  let source = $state<ImportSource>("mremoteng");
  let path = $state("");
  // Not bound to state: read when sent (see ../secret). The password is needed for both
  // the preview and the import, so it stays in its field until the dialog closes.
  let passwordInput = $state<HTMLInputElement>();
  let preview = $state<ImportPreview | null>(null);
  let error = $state("");
  let busy = $state(false);
  /** Per passphrase-protected key file: save the passphrase, or ask on every connection. */
  let keyChoice = $state<Record<string, { save: boolean }>>({});
  const keyInputs: Record<string, HTMLInputElement> = {};

  const hint = $derived(sources.find((s) => s.id === source)!.hint);
  const needsFile = $derived(source === "mremoteng");
  const label = (p: string) => PROTOCOLS.find((x) => x.id === p)?.label ?? p;
  const rootName: Record<ImportSource, string> = { mremoteng: "mRemoteNG", putty: "PuTTY", openssh: "OpenSSH config" };
  const authLabel = { password: "Password", key: "Key", agent: "Agent", none: "" };

  function pick(s: ImportSource) {
    source = s;
    path = "";
    takeSecret(passwordInput);
    preview = null;
    error = "";
  }

  async function browse() {
    const chosen = await openDialog({
      title: "Choose a connections file",
      filters: needsFile ? [{ name: "mRemoteNG connections", extensions: ["xml"] }] : undefined,
    });
    if (chosen && !Array.isArray(chosen)) {
      path = chosen;
      preview = null;
      if (needsFile) await load();
    }
  }

  const args = () => [source, path.trim() || null, readSecret(passwordInput) || null] as const;

  async function load() {
    if (needsFile && !path.trim()) return void (error = "Choose an mRemoteNG connections file.");
    error = "";
    busy = true;
    try {
      preview = await api.importPreview(...args());
      keyChoice = Object.fromEntries(preview.encryptedKeys.map((k) => [k.path, { save: false }]));
    } catch (e) {
      preview = null;
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

  async function run() {
    const missing = Object.entries(keyChoice).find(([p, c]) => c.save && !hasSecret(keyInputs[p]));
    if (missing) return void (error = `Enter the passphrase for ${missing[0]}, or choose to be asked when connecting.`);
    busy = true;
    error = "";
    try {
      const keys = Object.fromEntries(Object.entries(keyChoice).map(([p, c]) => [p, c.save ? readSecret(keyInputs[p]) : null]));
      const r = await api.importApply(...args(), keys);
      await reload();
      app.collapsed[r.rootFolderId] = false;
      app.modal = null;
      toast(`Imported ${r.hosts} host${r.hosts === 1 ? "" : "s"}${r.credentials ? ` and ${r.credentials} credential${r.credentials === 1 ? "" : "s"}` : ""}.`);
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }
</script>

<Modal title="Import connections" width={640}>
  <div class="form">
    <div class="seg" role="radiogroup" aria-label="Import from">
      {#each sources as s (s.id)}
        <button type="button" class:act={source === s.id} onclick={() => pick(s.id)}>{s.label}</button>
      {/each}
    </div>
    <p class="muted hint">{hint}</p>

    {#if source !== "putty"}
      <label class="field">
        <span>{needsFile ? "Connections file" : "Config file"}</span>
        <div class="row">
          <input id="imp-path" class="grow mono" bind:value={path} placeholder={needsFile ? "confCons.xml" : "~/.ssh/config (default)"}
            onchange={() => (preview = null)} />
          <button type="button" class="btn" onclick={browse}>Browse…</button>
        </div>
      </label>
    {/if}
    {#if needsFile}
      <label class="field">
        <span>mRemoteNG master password</span>
        <input id="imp-pass" type="password" bind:this={passwordInput} placeholder="Only if you set one in mRemoteNG" autocomplete="off"
          onchange={() => (preview = null)} />
      </label>
    {/if}

    {#if preview}
      <div class="summary">
        <b>{preview.hosts.length}</b> host{preview.hosts.length === 1 ? "" : "s"}{#if preview.folders}, <b>{preview.folders}</b> folder{preview.folders === 1 ? "" : "s"}{/if}
        <span class="muted">· they go into a new folder, “{rootName[source]} import”</span>
      </div>
      {#if preview.hosts.length}
        <div class="list">
          {#each preview.hosts as h, i (i)}
            <div class="item">
              <Icon name="server" size={13} />
              <div class="info">
                <b>{h.name}</b>
                <small class="muted mono">{h.username ? `${h.username}@` : ""}{h.address || "—"}{h.folder ? ` · ${h.folder}` : ""}</small>
              </div>
              <span class="tags">
                {#each h.protocols as p (p)}<span class="tag">{label(p)}</span>{/each}
                {#if h.auth !== "none"}<span class="tag auth">{authLabel[h.auth]}</span>{/if}
              </span>
            </div>
          {/each}
        </div>
      {/if}
      {#if preview.encryptedKeys.length}
        <div class="keys">
          <span class="lbl">Passphrase-protected keys</span>
          {#each preview.encryptedKeys as k (k.path)}
            {@const c = keyChoice[k.path]}
            <div class="key">
              <div class="info">
                <b class="mono" title={k.path}>{k.path}</b>
                <small class="muted">Used by {k.hosts} host{k.hosts === 1 ? "" : "s"}</small>
              </div>
              <div class="seg small" role="radiogroup" aria-label="Passphrase for {k.path}">
                <button type="button" class:act={c.save} onclick={() => (c.save = true)}>Save passphrase</button>
                <button type="button" class:act={!c.save} onclick={() => (c.save = false)}>Ask every time</button>
              </div>
              {#if c.save}
                <input type="password" bind:this={keyInputs[k.path]} required placeholder="Passphrase, stored encrypted in the vault" autocomplete="off" />
              {/if}
            </div>
          {/each}
        </div>
      {/if}
      {#if preview.warnings.length}
        <ul class="warnings">
          {#each preview.warnings as w, i (i)}<li>{w}</li>{/each}
        </ul>
      {/if}
    {/if}

    {#if error}<p class="error">{error}</p>{/if}
    <div class="actions">
      <button type="button" class="btn" onclick={() => (app.modal = null)}>Cancel</button>
      {#if preview && preview.hosts.length}
        <button type="button" class="btn primary" disabled={busy} onclick={run}>Import {preview.hosts.length} host{preview.hosts.length === 1 ? "" : "s"}</button>
      {:else}
        <button type="button" class="btn primary" disabled={busy} onclick={load}>Preview</button>
      {/if}
    </div>
  </div>
</Modal>

<style>
  .form { display: grid; gap: 10px; }
  .seg { display: inline-flex; gap: 2px; padding: 2px; border: 1px solid var(--line); border-radius: 7px; justify-self: start; }
  .seg button { padding: 5px 16px; border-radius: 5px; color: var(--muted); }
  .seg button.act { background: var(--accent); color: var(--accent-fg); font-weight: 600; }
  .hint { margin: 0; font-size: 12px; }
  .field { display: grid; gap: 5px; }
  .field > span { font-size: 11.5px; color: var(--muted); }
  .row { display: flex; gap: 8px; }
  .grow { flex: 1; min-width: 0; }
  .summary { font-size: 13px; }
  .list { max-height: 260px; overflow: auto; border: 1px solid var(--line); border-radius: 8px; }
  .item { display: flex; align-items: center; gap: 10px; padding: 6px 10px; border-bottom: 1px solid var(--line); }
  .item:last-child { border-bottom: 0; }
  .info { flex: 1; min-width: 0; display: grid; }
  .info b, .info small { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tags { display: flex; gap: 4px; }
  .tag { font-size: 11px; padding: 1px 6px; border-radius: 4px; background: var(--panel-2); color: var(--muted); }
  .tag.auth { color: var(--accent); }
  .keys { display: grid; gap: 6px; }
  .lbl { font-size: 11.5px; color: var(--muted); }
  .key { display: grid; grid-template-columns: 1fr auto; gap: 6px 10px; align-items: center; padding: 8px 10px; border: 1px solid var(--line); border-radius: 8px; }
  .key input { grid-column: 1 / -1; }
  .seg.small button { padding: 3px 10px; font-size: 12px; }
  .warnings { margin: 0; padding: 8px 10px 8px 26px; border-radius: 8px; background: var(--panel-2); color: var(--muted); font-size: 12px; max-height: 120px; overflow: auto; }
  .error { margin: 0; color: var(--bad); overflow-wrap: anywhere; }
  .actions { display: flex; gap: 8px; justify-content: flex-end; }
</style>
