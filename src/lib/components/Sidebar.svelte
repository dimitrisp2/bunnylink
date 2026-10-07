<!-- SPDX-FileCopyrightText: 2026 BunnyCloud.IT -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial -->
<script lang="ts">
  import Icon from "./Icon.svelte";
  import type { Credential, Folder, Host } from "../api";
  import { PROTOCOLS } from "../api";
  import {
    app, allTags, credentialMenu, deleteCredential, hostActions, hostMenu, toggleTunnel, tunnelMenu, tunnelSummary, showTab, HOME,
  } from "../state.svelte";

  const q = $derived(app.filter.trim().toLowerCase());
  const matches = (h: Host) =>
    (!app.tagFilter || h.tags.includes(app.tagFilter)) &&
    (!q || [h.name, h.address, ...h.tags].some((s) => s.toLowerCase().includes(q)));
  const visible = $derived(
    app.library.hosts.filter(matches).sort((a, b) => a.name.localeCompare(b.name, undefined, { numeric: true })),
  );
  const pinned = $derived(visible.filter((h) => h.pinned));
  const children = (parent: string | null) =>
    app.library.folders
      .filter((f) => (f.parentId ?? null) === parent)
      .sort((a, b) => a.name.localeCompare(b.name));
  const hostsIn = (folder: string | null) => visible.filter((h) => (h.folderId ?? null) === folder);
  const countIn = (f: Folder): number =>
    hostsIn(f.id).length + children(f.id).reduce((n, c) => n + countIn(c), 0);

  function select(e: MouseEvent, h: Host) {
    // Clicking the host already on show goes back to the welcome page. Only a single click:
    // the second click of a double-click (detail 2) keeps it selected for the connect.
    const shown = app.panes[app.focus] === HOME;
    app.selectedHostId = shown && e.detail === 1 && app.selectedHostId === h.id ? null : h.id;
    showTab(HOME, "left");
  }
  function primary(h: Host) {
    hostActions(h).find((a) => a.ready)?.run();
  }
  const protoLabel = (h: Host) =>
    h.endpoints.length === 1 ? PROTOCOLS.find((p) => p.id === h.endpoints[0].protocol)?.label : `${h.endpoints.length}`;
  const sessionOpen = (h: Host) => app.tabs.some((t) => t.hostId === h.id && t.status === "open");

  const credentialKind = (c: Credential) =>
    c.kind === "key" ? (c.askPassphrase ? "Private key, asks passphrase" : "Private key") : c.kind === "agent" ? "SSH agent" : "Password";
  const credentialUsage = (id: string) =>
    app.library.hosts.filter((h) => h.overrides.credentialId === id).length +
    app.library.folders.filter((f) => f.defaults.credentialId === id).length;
</script>

{#snippet hostRow(h: Host, depth: number)}
  <button
    class="row host"
    class:sel={app.selectedHostId === h.id}
    style:padding-left="{12 + depth * 14}px"
    onclick={(e) => select(e, h)}
    ondblclick={() => primary(h)}
    oncontextmenu={(e) => hostMenu(e, h)}
    title="{h.address} — double-click to connect"
  >
    <span class="dot" class:on={sessionOpen(h)}></span>
    <span class="name">{h.name}</span>
    <span class="proto">{protoLabel(h)}</span>
  </button>
{/snippet}

{#snippet folderNode(f: Folder, depth: number)}
  {#if !q || countIn(f) > 0}
    <div class="row folder" style:padding-left="{6 + depth * 14}px">
      <button class="twisty" class:open={!app.collapsed[f.id]} onclick={() => (app.collapsed[f.id] = !app.collapsed[f.id])} aria-label="Toggle {f.name}">
        <Icon name="chevron" size={12} />
      </button>
      <button class="fname" onclick={() => (app.collapsed[f.id] = !app.collapsed[f.id])} ondblclick={() => (app.modal = { kind: "folder", folder: f })}>
        {f.name}
      </button>
      <span class="count">{countIn(f)}</span>
      <button class="icon-btn mini" title="Add host to {f.name}" onclick={() => (app.modal = { kind: "host", host: null, folderId: f.id })}><Icon name="plus" size={13} /></button>
      <button class="icon-btn mini" title="Edit folder" onclick={() => (app.modal = { kind: "folder", folder: f })}><Icon name="edit" size={13} /></button>
    </div>
    {#if !app.collapsed[f.id] || q}
      {#each children(f.id) as c (c.id)}{@render folderNode(c, depth + 1)}{/each}
      {#each hostsIn(f.id) as h (h.id)}{@render hostRow(h, depth + 1)}{/each}
    {/if}
  {/if}
{/snippet}

<aside>
  <div class="seg">
    <button class:act={app.sidebar === "hosts"} onclick={() => (app.sidebar = "hosts")}>Hosts <span>{app.library.hosts.length}</span></button>
    <button class:act={app.sidebar === "tunnels"} onclick={() => (app.sidebar = "tunnels")}>Tunnels <span>{app.library.tunnels.length}</span></button>
    <button class:act={app.sidebar === "credentials"} onclick={() => (app.sidebar = "credentials")}>Credentials <span>{app.library.credentials.length}</span></button>
  </div>

  {#if app.sidebar === "hosts"}
    <div class="tools">
      <input id="host-filter" placeholder="Filter by name, IP or tag" bind:value={app.filter} />
      <button class="icon-btn" title="New host" onclick={() => (app.modal = { kind: "host", host: null })}><Icon name="plus" /></button>
    </div>
    <div class="chips">
      <button class="chip" class:act={app.groupBy === "folders"} onclick={() => (app.groupBy = "folders")}>Folders</button>
      <button class="chip" class:act={app.groupBy === "tags"} onclick={() => (app.groupBy = "tags")}>Tags</button>
      {#if app.groupBy === "tags"}
        {#each allTags() as t (t)}
          <button class="chip tag" class:act={app.tagFilter === t} onclick={() => (app.tagFilter = app.tagFilter === t ? null : t)}>#{t}</button>
        {/each}
      {/if}
    </div>

    <div class="list">
      {#if app.library.hosts.length === 0}
        <div class="empty">
          <p>No hosts yet.</p>
          <button class="btn primary" onclick={() => (app.modal = { kind: "host", host: null })}><Icon name="plus" size={14} /> Add your first host</button>
          <button class="btn" onclick={() => (app.modal = { kind: "folder", folder: null })}>Create a folder</button>
          <button class="btn" onclick={() => (app.modal = { kind: "import" })}><Icon name="download" size={14} /> Import connections</button>
        </div>
      {:else}
        {#if pinned.length}
          <div class="group">Pinned</div>
          {#each pinned as h (h.id)}{@render hostRow(h, 0)}{/each}
          <div class="group">All hosts</div>
        {/if}
        {#if app.groupBy === "folders"}
          {#each children(null) as f (f.id)}{@render folderNode(f, 0)}{/each}
          {#each hostsIn(null) as h (h.id)}{@render hostRow(h, 0)}{/each}
          <button class="add-folder" onclick={() => (app.modal = { kind: "folder", folder: null })}><Icon name="plus" size={12} /> New folder</button>
        {:else}
          {#each visible as h (h.id)}{@render hostRow(h, 0)}{/each}
        {/if}
        {#if visible.length === 0}<p class="none muted">No hosts match.</p>{/if}
      {/if}
    </div>
  {:else if app.sidebar === "credentials"}
    <div class="tools">
      <span class="muted grow">Saved passwords and keys</span>
      <button class="icon-btn" title="New credential" onclick={() => (app.modal = { kind: "credential", credential: null })}><Icon name="plus" /></button>
    </div>
    <div class="list">
      {#each app.library.credentials as c (c.id)}
        <div class="cred" role="listitem" oncontextmenu={(e) => credentialMenu(e, c)}>
          <Icon name={c.kind === "password" ? "lock" : "key"} size={14} />
          <button class="tinfo" title="Edit {c.name}" onclick={() => (app.modal = { kind: "credential", credential: c })}>
            <span class="name">{c.name}</span>
            <small class="mono">{credentialKind(c)}{c.username ? ` · ${c.username}` : ""} · used by {credentialUsage(c.id)}</small>
          </button>
          <button class="icon-btn mini" title="Delete" onclick={() => deleteCredential(c)}><Icon name="trash" size={13} /></button>
        </div>
      {:else}
        <div class="empty">
          <p>No credentials yet. Passwords and keys are encrypted with your master password.</p>
          <button class="btn primary" onclick={() => (app.modal = { kind: "credential", credential: null })}><Icon name="plus" size={14} /> New credential</button>
        </div>
      {/each}
    </div>
  {:else}
    <div class="tools">
      <span class="muted grow">Saved tunnels</span>
      <button class="icon-btn" title="New tunnel" onclick={() => (app.modal = { kind: "tunnel", tunnel: null, hostId: app.selectedHostId ?? undefined })}><Icon name="plus" /></button>
    </div>
    <div class="list">
      {#each app.library.tunnels as t (t.id)}
        {@const st = app.tunnelState[t.id]?.state ?? "closed"}
        <div class="tunnel" role="listitem" oncontextmenu={(e) => tunnelMenu(e, t)}>
          <span class="dot" class:on={st === "open"} class:busy={st === "connecting"}></span>
          <button class="tinfo" onclick={() => (app.modal = { kind: "tunnel", tunnel: t })}>
            <span class="name">{t.name}</span>
            <small class="mono">{tunnelSummary(t)}</small>
          </button>
          <button class="icon-btn" title={st === "closed" ? "Open tunnel" : "Close tunnel"} onclick={() => toggleTunnel(t)}>
            <Icon name={st === "closed" ? "play" : "stop"} size={14} />
          </button>
        </div>
      {:else}
        <div class="empty">
          <p>No tunnels yet. Forward a port or start a SOCKS proxy through any SSH host.</p>
          <button class="btn primary" disabled={!app.library.hosts.length} onclick={() => (app.modal = { kind: "tunnel", tunnel: null })}><Icon name="plus" size={14} /> New tunnel</button>
          {#if !app.library.hosts.length}<small class="muted">Add an SSH host first.</small>{/if}
        </div>
      {/each}
    </div>
  {/if}
</aside>

<style>
  aside { display: grid; grid-template-rows: auto auto auto 1fr; min-height: 0; background: var(--panel); border-right: 1px solid var(--line); }
  .seg { display: flex; gap: 2px; padding: 8px; }
  .seg button { flex: 1 1 auto; padding: 5px 6px; border-radius: 5px; color: var(--muted); font-weight: 600; white-space: nowrap; }
  .seg button span { font-weight: 400; color: var(--faint); margin-left: 3px; }
  .seg button.act { background: var(--panel-2); color: var(--fg); box-shadow: inset 0 0 0 1px var(--line); }
  .tools { display: flex; gap: 6px; padding: 0 8px 8px; align-items: center; }
  .tools input { padding: 5px 8px; }
  .grow { flex: 1; padding-left: 4px; }
  .chips { display: flex; flex-wrap: wrap; gap: 4px; padding: 0 8px 8px; border-bottom: 1px solid var(--line); }
  .chip { font-size: 11px; padding: 2px 8px; border-radius: 10px; border: 1px solid var(--line); color: var(--muted); }
  .chip.act { background: var(--accent); border-color: var(--accent); color: var(--accent-fg); }
  .chip.tag { font-family: var(--mono); font-size: 10.5px; }
  .list { overflow: auto; padding: 4px 0 12px; min-height: 0; }
  .row { display: flex; align-items: center; gap: 7px; width: 100%; padding: 4px 10px 4px 12px; text-align: left; white-space: nowrap; }
  .row.host:hover { background: var(--panel-2); }
  .row.host.sel { background: var(--sel); }
  .name { overflow: hidden; text-overflow: ellipsis; flex: 1; min-width: 0; }
  .folder { color: var(--muted); font-weight: 600; font-size: 12px; gap: 4px; }
  .folder .fname { flex: 1; text-align: left; overflow: hidden; text-overflow: ellipsis; }
  .folder .count { font-weight: 400; color: var(--faint); font-size: 11px; }
  .folder .mini { width: 20px; height: 20px; visibility: hidden; }
  .folder:hover .mini { visibility: visible; }
  .twisty { display: grid; place-items: center; width: 16px; color: var(--faint); transition: transform .12s; }
  .twisty.open { transform: rotate(90deg); }
  .group { padding: 10px 12px 4px; font-size: 10.5px; letter-spacing: .08em; text-transform: uppercase; color: var(--faint); font-weight: 600; }
  .add-folder { display: flex; gap: 6px; align-items: center; color: var(--faint); padding: 8px 12px; font-size: 12px; }
  .add-folder:hover { color: var(--fg); }
  .empty { display: grid; gap: 8px; padding: 16px 12px; }
  .empty p { margin: 0; color: var(--muted); }
  .none { padding: 8px 12px; margin: 0; }
  .tunnel { display: flex; align-items: center; gap: 8px; padding: 6px 8px 6px 12px; }
  .tunnel:hover { background: var(--panel-2); }
  .cred { display: flex; align-items: center; gap: 8px; padding: 6px 8px 6px 12px; color: var(--muted); }
  .cred:hover { background: var(--panel-2); }
  .cred .tinfo { color: var(--fg); }
  .cred .mini { width: 22px; height: 22px; visibility: hidden; }
  .cred:hover .mini { visibility: visible; }
  .tinfo { display: grid; text-align: left; flex: 1; min-width: 0; }
  .tinfo small { color: var(--muted); font-size: 10.5px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
