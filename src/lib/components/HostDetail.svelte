<!-- SPDX-FileCopyrightText: 2026 BunnyCloud.IT -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial -->
<script lang="ts">
  import Icon from "./Icon.svelte";
  import { api } from "../api";
  import type { Effective } from "../api";
  import {
    app, credentialById, deleteHost, folderPath, forgetHostKey, hasHostKey, hostActions, hostById, hostMenu, togglePin,
  } from "../state.svelte";

  const host = $derived(hostById(app.selectedHostId));
  let eff = $state<Effective | null>(null);

  $effect(() => {
    const h = host;
    // Re-resolve inheritance whenever the host or library changes.
    void app.library;
    eff = null;
    if (h) api.effective(h.id).then((e) => { if (app.selectedHostId === h.id) eff = e; }).catch(() => {});
  });

  const recent = $derived(
    app.library.hosts
      .filter((h) => h.lastUsed)
      .sort((a, b) => (b.lastUsed ?? 0) - (a.lastUsed ?? 0))
      .slice(0, 6),
  );

  const from = (src?: string | null) => (src ? `from ${src}` : "");
  const ago = (t: number) => {
    const s = Date.now() / 1000 - t;
    if (s < 60) return "just now";
    if (s < 3600) return `${Math.round(s / 60)} min ago`;
    if (s < 86400) return `${Math.round(s / 3600)} h ago`;
    return `${Math.round(s / 86400)} d ago`;
  };
</script>

<div class="detail">
  {#if host}
    <header>
      <div class="title">
        <h1>{host.name}</h1>
        <span class="mono muted">{host.address}</span>
        {#if host.folderId}<span class="path">{folderPath(host.folderId)}</span>{/if}
      </div>
      <div class="tools">
        <button class="icon-btn" class:pinned={host.pinned} title={host.pinned ? "Unpin" : "Pin"} onclick={() => togglePin(host)}><Icon name="pin" /></button>
        <button class="btn" onclick={() => (app.modal = { kind: "host", host })}><Icon name="edit" size={14} /> Edit</button>
      </div>
    </header>

    <dl class="meta">
      <div>
        <dt>Username</dt>
        <dd>{eff?.username ?? credentialById(eff?.credentialId)?.username ?? "—"} <small>{from(eff?.usernameFrom)}</small></dd>
      </div>
      <div>
        <dt>Credential</dt>
        <dd>{credentialById(eff?.credentialId)?.name ?? "None"} <small>{from(eff?.credentialFrom)}</small></dd>
      </div>
      <div>
        <dt>Jump host</dt>
        <dd>{hostById(eff?.jumpHostId)?.name ?? "Direct"} <small>{from(eff?.jumpHostFrom)}</small></dd>
      </div>
    </dl>

    <div class="actions">
      {#each hostActions(host) as a, i (a.key)}
        <button class="action" class:primary={i === 0 && a.ready} class:soon={!a.ready} onclick={a.run}>
          <span class="icon"><Icon name={a.icon} size={18} /></span>
          <span class="text">
            <b>{a.label}</b>
            <small class="mono">{a.detail}</small>
          </span>
          {#if !a.ready}<span class="badge">Soon</span>{/if}
        </button>
      {/each}
    </div>

    {#if host.tags.length}
      <div class="tags">{#each host.tags as t (t)}<button class="tag" onclick={() => { app.groupBy = "tags"; app.tagFilter = t; }}>#{t}</button>{/each}</div>
    {/if}
    {#if host.notes}<p class="notes">{host.notes}</p>{/if}

    <footer>
      {#if host.lastUsed}<span class="muted">Last connected {ago(host.lastUsed)}</span>{/if}
      <span class="spacer"></span>
      {#if hasHostKey(host)}<button class="btn" onclick={() => forgetHostKey(host)}>Forget host key</button>{/if}
      <button class="btn danger" onclick={() => deleteHost(host)}><Icon name="trash" size={14} /> Delete</button>
    </footer>
  {:else}
    <div class="welcome">
      <h1>Where to?</h1>
      <p class="muted">Pick a host on the left, or press <kbd>Ctrl</kbd> <kbd>K</kbd> and type a host name followed by what you want to do.</p>
      {#if recent.length}
        <h2>Recent</h2>
        <div class="cards">
          {#each recent as h (h.id)}
            <button class="card" onclick={() => (app.selectedHostId = h.id)} ondblclick={() => hostActions(h).find((a) => a.ready)?.run()} oncontextmenu={(e) => hostMenu(e, h)}>
              <b>{h.name}</b>
              <small class="mono muted">{h.address} · {ago(h.lastUsed!)}</small>
            </button>
          {/each}
        </div>
      {/if}
      <h2>Shortcuts</h2>
      <dl class="keys">
        <dt><kbd>Ctrl</kbd> <kbd>K</kbd></dt><dd>Search hosts and actions (<kbd>Ctrl</kbd> <kbd>Shift</kbd> <kbd>K</kbd> inside a terminal)</dd>
        <dt><kbd>Ctrl</kbd> <kbd>Tab</kbd></dt><dd>Next tab</dd>
        <dt><kbd>Ctrl</kbd> <kbd>Shift</kbd> <kbd>W</kbd></dt><dd>Close tab</dd>
        <dt><kbd>Ctrl</kbd> <kbd>\</kbd></dt><dd>Split or unsplit the view</dd>
        <dt>Double-click</dt><dd>Connect with the host's first action</dd>
      </dl>
    </div>
  {/if}
</div>

<style>
  .detail { padding: 22px 26px; display: grid; gap: 18px; align-content: start; overflow: auto; height: 100%; }
  header { display: flex; gap: 16px; align-items: flex-start; }
  .title { display: grid; gap: 2px; flex: 1; min-width: 0; }
  h1 { margin: 0; font-size: 22px; letter-spacing: -.01em; }
  .path { font-size: 12px; color: var(--faint); }
  .tools { display: flex; gap: 6px; align-items: center; }
  .pinned { color: var(--accent); }
  .meta { display: flex; flex-wrap: wrap; gap: 8px 36px; margin: 0; }
  .meta dt { font-size: 10.5px; color: var(--faint); text-transform: uppercase; letter-spacing: .06em; }
  .meta dd { margin: 2px 0 0; font-family: var(--mono); font-size: 12.5px; }
  .meta small { font-family: var(--ui); color: var(--faint); font-size: 11px; }
  .actions { display: grid; grid-template-columns: repeat(auto-fill, minmax(210px, 1fr)); gap: 8px; }
  .action {
    display: flex; gap: 10px; align-items: center; text-align: left; padding: 11px 12px;
    background: var(--panel); border: 1px solid var(--line); border-radius: 8px;
  }
  .action:hover { border-color: var(--line-strong); }
  .action.primary { border-color: var(--accent); box-shadow: inset 0 0 0 1px var(--accent); }
  .action .icon { color: var(--accent); display: grid; }
  .action.soon { opacity: .6; }
  .action.soon .icon { color: var(--muted); }
  .text { display: grid; flex: 1; min-width: 0; }
  .text small { color: var(--muted); font-size: 11px; }
  .badge { font-size: 10px; padding: 1px 6px; border-radius: 8px; border: 1px solid var(--line); color: var(--muted); }
  .tags { display: flex; gap: 6px; flex-wrap: wrap; }
  .tag { font: 11px var(--mono); color: var(--accent-2); }
  .notes { margin: 0; white-space: pre-wrap; color: var(--muted); max-width: 70ch; user-select: text; }
  footer { display: flex; gap: 8px; align-items: center; border-top: 1px solid var(--line); padding-top: 14px; }
  .spacer { flex: 1; }
  .welcome { display: grid; gap: 12px; max-width: 640px; }
  .welcome p { margin: 0; }
  h2 { margin: 10px 0 0; font-size: 11px; color: var(--faint); text-transform: uppercase; letter-spacing: .08em; }
  .cards { display: grid; grid-template-columns: repeat(auto-fill, minmax(170px, 1fr)); gap: 8px; }
  .card { display: grid; text-align: left; gap: 2px; padding: 10px 12px; background: var(--panel); border: 1px solid var(--line); border-radius: 8px; }
  .card:hover { border-color: var(--line-strong); }
  .keys { display: grid; grid-template-columns: max-content 1fr; gap: 8px 16px; margin: 0; align-items: center; }
  .keys dd { margin: 0; color: var(--muted); }
</style>
