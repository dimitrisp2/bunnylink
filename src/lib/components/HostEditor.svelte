<!-- SPDX-FileCopyrightText: 2026 BunnyCloud.IT -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial -->
<script lang="ts">
  import Modal from "./Modal.svelte";
  import CredentialForm from "./CredentialForm.svelte";
  import { onMount } from "svelte";
  import { api, errorText, PROTOCOLS, DEFAULT_SERIAL } from "../api";
  import type { Credential, Host, Protocol } from "../api";
  import { app, credentialById, folderById, folderPath, hostById, newHost, reload } from "../state.svelte";

  let { host, folderId = null }: { host: Host | null; folderId?: string | null } = $props();

  // svelte-ignore state_referenced_locally
  let draft = $state<Host>(host ? $state.snapshot(host) as Host : newHost(folderId));
  let tags = $state(draft.tags.join(", "));
  let error = $state("");
  /** Inline credential form: a new credential, or the one shown in the dropdown. */
  let credForm = $state<Credential | "new" | null>(null);

  // Inherited values for the draft's folder, so placeholders show what applies.
  const inherited = $derived.by(() => {
    const out: { username?: string; credentialId?: string; jumpHostId?: string; src: Record<string, string> } = { src: {} };
    let f = folderById(draft.folderId);
    let guard = 0;
    while (f && guard++ < 32) {
      const d = f.defaults;
      if (!out.username && d.username) { out.username = d.username; out.src.username = f.name; }
      if (!out.credentialId && d.credentialId) { out.credentialId = d.credentialId; out.src.credential = f.name; }
      if (!out.jumpHostId && d.jumpHostId) { out.jumpHostId = d.jumpHostId; out.src.jump = f.name; }
      f = folderById(f.parentId);
    }
    return out;
  });
  // The selected credential, or the inherited one when none is selected.
  const shownCred = $derived(credentialById(draft.overrides.credentialId ?? inherited.credentialId));

  const has =(p: Protocol) => draft.endpoints.some((e) => e.protocol === p);
  function toggle(p: Protocol) {
    if (has(p)) draft.endpoints = draft.endpoints.filter((e) => e.protocol !== p);
    else if (p === "serial") draft.endpoints.push({ protocol: p, port: null, path: "", serial: { ...DEFAULT_SERIAL } });
    else draft.endpoints.push({ protocol: p, port: PROTOCOLS.find((x) => x.id === p)!.port || null });
  }
  const serialOnly = $derived(draft.endpoints.length > 0 && draft.endpoints.every((e) => e.protocol === "serial"));
  const BAUDS = [300, 1200, 2400, 4800, 9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600];
  let serialPorts = $state<string[]>([]);
  onMount(async () => {
    serialPorts = await api.serialPorts().catch(() => []);
  });
  const sshHosts = $derived(app.library.hosts.filter((h) => h.id !== draft.id && h.endpoints.some((e) => e.protocol === "ssh")));
  const folders = $derived(
    app.library.folders.map((f) => ({ id: f.id, path: folderPath(f.id) })).sort((a, b) => a.path.localeCompare(b.path)),
  );

  /** "user@host" in the address moves the user into the Username field. */
  function splitAddress() {
    const at = draft.address.lastIndexOf("@");
    if (at < 0) return;
    const user = draft.address.slice(0, at).trim();
    draft.address = draft.address.slice(at + 1).trim();
    if (user) draft.overrides.username = user;
  }

  async function save() {
    error = "";
    splitAddress();
    draft.tags = tags.split(/[,\s]+/).map((t) => t.replace(/^#/, "").trim()).filter(Boolean);
    for (const k of ["username", "credentialId", "jumpHostId"] as const) if (!draft.overrides[k]) draft.overrides[k] = null;
    try {
      const saved = await api.saveHost($state.snapshot(draft) as Host);
      await reload();
      app.selectedHostId = saved.id;
      app.modal = null;
    } catch (e) {
      error = errorText(e);
    }
  }
</script>

<Modal title={host?.id ? `Edit ${host.name}` : host ? "Duplicate host" : "New host"} width={600}>
  <div class="grid">
    <!-- svelte-ignore a11y_autofocus -->
    <label class="field"><span>Name</span><input id="host-name" bind:value={draft.name} placeholder="web01" autofocus /></label>
    <label class="field"><span>Address{serialOnly ? " (not needed for serial)" : ""}</span><input id="host-address" class="mono" bind:value={draft.address} onblur={splitAddress} placeholder={serialOnly ? "Optional" : "10.0.1.11 or web01.example.com"} /></label>
  </div>

  <div class="field">
    <span class="lbl">Protocols</span>
    <div class="protos">
      {#each PROTOCOLS.filter((p) => !p.soon || has(p.id)) as p (p.id)}
        <button type="button" class="chip" class:act={has(p.id)} onclick={() => toggle(p.id)}>{p.label}</button>
      {/each}
    </div>
    {#if draft.endpoints.length}
      <div class="ports">
        {#each draft.endpoints as ep (ep.protocol)}
          <label class="port">
            <span class="proto">{ep.protocol}</span>
            {#if ep.protocol === "serial"}
              <input id="ep-serial" class="mono" list="serial-ports" bind:value={ep.path} placeholder="COM3 or /dev/ttyUSB0" />
              <datalist id="serial-ports">{#each serialPorts as sp (sp)}<option value={sp}></option>{/each}</datalist>
            {:else}
              <input id="ep-{ep.protocol}" class="mono" type="number" min="1" max="65535" bind:value={ep.port} />
              {#if ep.protocol === "http" || ep.protocol === "https" || ep.protocol === "smb"}
                <input id="ep-{ep.protocol}-path" class="mono" bind:value={ep.path} placeholder={ep.protocol === "smb" ? "share (optional)" : "/path"} title={ep.protocol === "smb" ? "Share to open. Leave empty to choose from the server's shares." : "Path to open"} />
              {/if}
            {/if}
          </label>
        {/each}
      </div>
      {#each draft.endpoints.filter((e) => e.protocol === "serial") as ep (ep.protocol)}
        {#if ep.serial}
          <div class="serial">
            <label class="field"><span>Baud rate</span>
              <select id="serial-baud" bind:value={ep.serial.baud}>{#each BAUDS as b (b)}<option value={b}>{b}</option>{/each}</select>
            </label>
            <label class="field"><span>Data bits</span>
              <select id="serial-bits" bind:value={ep.serial.dataBits}>{#each [8, 7, 6, 5] as b (b)}<option value={b}>{b}</option>{/each}</select>
            </label>
            <label class="field"><span>Parity</span>
              <select id="serial-parity" bind:value={ep.serial.parity}><option value="none">None</option><option value="even">Even</option><option value="odd">Odd</option></select>
            </label>
            <label class="field"><span>Stop bits</span>
              <select id="serial-stop" bind:value={ep.serial.stopBits}><option value={1}>1</option><option value={2}>2</option></select>
            </label>
            <label class="field"><span>Flow control</span>
              <select id="serial-flow" bind:value={ep.serial.flow}><option value="none">None</option><option value="software">XON/XOFF</option><option value="hardware">RTS/CTS</option></select>
            </label>
          </div>
        {/if}
      {/each}
    {/if}
  </div>

  <div class="grid">
    <label class="field">
      <span>Folder</span>
      <select id="host-folder" bind:value={draft.folderId}>
        <option value={null}>No folder</option>
        {#each folders as f (f.id)}<option value={f.id}>{f.path}</option>{/each}
      </select>
    </label>
    <label class="field"><span>Tags</span><input id="host-tags" class="mono" bind:value={tags} placeholder="prod, web" /></label>
  </div>

  <div class="grid">
    <label class="field">
      <span>Username</span>
      <input id="host-user" bind:value={draft.overrides.username} placeholder={inherited.username ? `${inherited.username} (from ${inherited.src.username})` : "From credential"} autocomplete="off" />
    </label>
    <label class="field">
      <span>Jump host</span>
      <select id="host-jump" bind:value={draft.overrides.jumpHostId}>
        <option value={null}>{inherited.jumpHostId ? `Inherit: ${hostById(inherited.jumpHostId)?.name} (from ${inherited.src.jump})` : "Direct connection"}</option>
        {#each sshHosts as h (h.id)}<option value={h.id}>{h.name}</option>{/each}
      </select>
    </label>
  </div>

  <div class="field">
    <span class="lbl">Credential</span>
    <div class="row">
      <select id="host-cred" bind:value={draft.overrides.credentialId}>
        <option value={null}>{inherited.credentialId ? `Inherit: ${credentialById(inherited.credentialId)?.name} (from ${inherited.src.credential})` : "None"}</option>
        {#each app.library.credentials as c (c.id)}<option value={c.id}>{c.name}{c.username ? ` · ${c.username}` : ""}</option>{/each}
      </select>
      {#if !credForm}
        {#if shownCred}<button type="button" class="btn" onclick={() => (credForm = shownCred)}>Edit…</button>{/if}
        <button type="button" class="btn" onclick={() => (credForm = "new")}>New…</button>
      {/if}
    </div>
    {#if credForm}
      {#key credForm}
        <CredentialForm
          compact
          credential={credForm === "new" ? null : credForm}
          suggestedName={draft.name}
          onSaved={(c) => { if (credForm === "new") draft.overrides.credentialId = c.id; credForm = null; }}
          onCancel={() => (credForm = null)}
        />
      {/key}
    {/if}
  </div>

  <label class="field"><span>Notes</span><textarea id="host-notes" rows="3" bind:value={draft.notes} style="font-family: var(--ui); font-size: 13px"></textarea></label>
  {#if error}<p class="error">{error}</p>{/if}

  {#snippet footer()}
    <button class="btn" onclick={() => (app.modal = null)}>Cancel</button>
    <button class="btn primary" onclick={save}>{host ? "Save" : "Add host"}</button>
  {/snippet}
</Modal>

<style>
  .grid { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
  .field { display: grid; gap: 4px; }
  .lbl, .field > span { font-size: 11.5px; color: var(--muted); }
  .protos { display: flex; flex-wrap: wrap; gap: 5px; }
  .chip { font-size: 11.5px; padding: 3px 10px; border-radius: 12px; border: 1px solid var(--line); color: var(--muted); }
  .chip.act { background: var(--accent-soft); border-color: var(--accent); color: var(--fg); }
  .ports { display: grid; grid-template-columns: repeat(auto-fill, minmax(170px, 1fr)); gap: 6px; margin-top: 6px; }
  .port { display: flex; gap: 6px; align-items: center; }
  .port .proto { width: 46px; text-align: center; flex: none; }
  .serial { display: grid; grid-template-columns: repeat(5, 1fr); gap: 8px; margin-top: 8px; }
  .serial select { padding: 4px 24px 4px 6px; }
  .port input { padding: 4px 6px; }
  .row { display: flex; gap: 8px; }
  .error { margin: 0; color: var(--bad); }
</style>
