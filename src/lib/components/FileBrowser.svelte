<!-- SPDX-FileCopyrightText: 2026 BunnyCloud.IT -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial -->
<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { writeText } from "@tauri-apps/plugin-clipboard-manager";
  import Icon from "./Icon.svelte";
  import { api, errorText } from "../api";
  import type { FileEntry } from "../api";
  import { app, hostById, toast, type Tab } from "../state.svelte";

  let { tab, visible }: { tab: Tab; visible: boolean } = $props();
  const host = $derived(hostById(tab.hostId));

  let sessionId = $state<string | null>(null);
  let path = $state("");
  let pathInput = $state("");
  let entries = $state<FileEntry[]>([]);
  let selected = $state<Set<string>>(new Set());
  let anchor = $state<string | null>(null);
  let loading = $state(false);
  let error = $state("");
  let filter = $state("");
  let history: string[] = [];
  let renaming = $state<string | null>(null);
  let renameValue = $state("");
  let creating = $state(false);
  let newName = $state("");
  let dragOver = $state(false);
  let menu = $state<{ x: number; y: number } | null>(null);
  let root: HTMLDivElement;
  let unlistenDrop: (() => void) | undefined;

  const setStatus = (s: Tab["status"]) => {
    const t = app.tabs.find((x) => x.id === tab.id);
    if (t) t.status = s;
  };

  const shown = $derived(
    filter.trim() ? entries.filter((e) => e.name.toLowerCase().includes(filter.trim().toLowerCase())) : entries,
  );
  const selectedEntries = $derived(entries.filter((e) => selected.has(e.path)));
  const transfers = $derived(Object.values(app.transfers).filter((t) => t.sessionId === sessionId));
  const crumbs = $derived.by(() => {
    const parts = path.split("/").filter(Boolean);
    return [{ name: "/", path: "/" }, ...parts.map((p, i) => ({ name: p, path: "/" + parts.slice(0, i + 1).join("/") }))];
  });

  async function connect() {
    setStatus("connecting");
    error = "";
    loading = true;
    try {
      const r = await api.filesOpen(tab.hostId, tab.protocol);
      sessionId = r.sessionId;
      for (const n of r.notices) toast(n);
      setStatus("open");
      await go(r.home, false);
    } catch (e) {
      setStatus("error");
      error = errorText(e);
      loading = false;
    }
  }

  async function go(p: string, remember = true) {
    if (!sessionId) return;
    loading = true;
    try {
      const l = await api.filesList(sessionId, p);
      if (remember && path && l.path !== path) history.push(path);
      path = l.path;
      pathInput = l.path;
      entries = l.entries;
      selected = new Set();
      filter = "";
      error = "";
    } catch (e) {
      toast(errorText(e), "error");
      pathInput = path;
    } finally {
      loading = false;
    }
  }

  const parentOf = (p: string) => (p.replace(/\/+$/, "").split("/").slice(0, -1).join("/") || "/");
  const joinPath = (dir: string, name: string) => (dir.endsWith("/") ? dir + name : `${dir}/${name}`);
  const refresh = () => go(path, false);
  const back = () => {
    const p = history.pop();
    if (p) go(p, false);
  };

  function click(e: MouseEvent, entry: FileEntry) {
    const next = new Set(e.ctrlKey || e.metaKey ? selected : []);
    if (e.shiftKey && anchor) {
      const a = shown.findIndex((x) => x.path === anchor);
      const b = shown.findIndex((x) => x.path === entry.path);
      for (const x of shown.slice(Math.min(a, b), Math.max(a, b) + 1)) next.add(x.path);
    } else if ((e.ctrlKey || e.metaKey) && selected.has(entry.path)) {
      next.delete(entry.path);
    } else {
      next.add(entry.path);
      anchor = entry.path;
    }
    selected = next;
  }

  function activate(entry: FileEntry) {
    if (entry.isDir) go(entry.path);
    else download([entry]);
  }

  async function download(list: FileEntry[] = selectedEntries, pick = false) {
    if (!sessionId || !list.length) return;
    let dir: string | undefined;
    if (pick) {
      const chosen = await openDialog({ directory: true, title: "Download to folder" });
      if (!chosen || Array.isArray(chosen)) return;
      dir = chosen;
    }
    await api.filesDownload(sessionId, list.map((e) => e.path), dir).catch((e) => toast(errorText(e), "error"));
  }

  async function upload(paths?: string[]) {
    if (!sessionId) return;
    if (!paths) {
      const chosen = await openDialog({ multiple: true, title: `Upload to ${path}` });
      if (!chosen) return;
      paths = Array.isArray(chosen) ? chosen : [chosen];
    }
    if (paths.length) await api.filesUpload(sessionId, paths, path).catch((e) => toast(errorText(e), "error"));
  }

  async function mkdir() {
    const name = newName.trim();
    creating = false;
    newName = "";
    if (!name || !sessionId) return;
    try {
      await api.filesMkdir(sessionId, joinPath(path, name));
      await refresh();
      selected = new Set([joinPath(path, name)]);
    } catch (e) {
      toast(errorText(e), "error");
    }
  }

  function startRename(entry: FileEntry | undefined = selectedEntries[0]) {
    if (!entry) return;
    renaming = entry.path;
    renameValue = entry.name;
  }

  async function finishRename(entry: FileEntry) {
    const name = renameValue.trim();
    renaming = null;
    if (!name || name === entry.name || !sessionId) return;
    try {
      await api.filesRename(sessionId, entry.path, joinPath(path, name));
      await refresh();
    } catch (e) {
      toast(errorText(e), "error");
    }
  }

  function remove(list: FileEntry[] = selectedEntries) {
    if (!list.length || !sessionId) return;
    const sid = sessionId;
    const what = list.length === 1 ? list[0].name : `${list.length} items`;
    app.modal = {
      kind: "confirm",
      title: `Delete ${what}?`,
      body: list.some((e) => e.isDir)
        ? "Folders are deleted with everything inside them. This cannot be undone."
        : "This cannot be undone.",
      confirm: "Delete",
      run: async () => {
        try {
          await api.filesDelete(sid, list.map((e) => e.path));
        } catch (e) {
          toast(errorText(e), "error");
        }
        await refresh();
      },
    };
  }

  function onKey(e: KeyboardEvent) {
    if (renaming || creating || (e.target as HTMLElement).tagName === "INPUT") return;
    if (e.key === "Delete") remove();
    else if (e.key === "F2") startRename();
    else if (e.key === "F5") { e.preventDefault(); refresh(); }
    else if (e.key === "Backspace" || (e.altKey && e.key === "ArrowUp")) go(parentOf(path));
    else if (e.key === "Enter" && selectedEntries.length === 1) activate(selectedEntries[0]);
    else if (e.ctrlKey && e.key.toLowerCase() === "a") { e.preventDefault(); selected = new Set(shown.map((x) => x.path)); }
    else return;
    e.preventDefault();
  }

  function openMenu(e: MouseEvent, entry?: FileEntry) {
    e.preventDefault();
    if (entry && !selected.has(entry.path)) selected = new Set([entry.path]);
    if (!entry) selected = new Set();
    const r = root.getBoundingClientRect();
    menu = { x: Math.min(e.clientX - r.left, r.width - 200), y: Math.min(e.clientY - r.top, r.height - 220) };
  }
  const runMenu = (fn: () => void) => () => {
    menu = null;
    fn();
  };

  const size = (n: number) => {
    if (n < 1024) return `${n} B`;
    const u = ["KB", "MB", "GB", "TB"];
    let i = -1;
    do { n /= 1024; i++; } while (n >= 1024 && i < u.length - 1);
    return `${n.toFixed(n < 10 ? 1 : 0)} ${u[i]}`;
  };
  const date = (t: number | null) =>
    t ? new Date(t * 1000).toLocaleString(undefined, { year: "numeric", month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" }) : "";
  const perms = (m: number | null) => {
    if (m == null) return "";
    const r = (b: number, c: string) => (m & b ? c : "-");
    return [0o400, 0o200, 0o100, 0o40, 0o20, 0o10, 0o4, 0o2, 0o1].map((b, i) => r(b, "rwx"[i % 3])).join("");
  };
  const pct = (d: number, t: number) => (t ? Math.round((d / t) * 100) : 100);

  // Refresh after an upload into the folder being shown.
  let seenDone = new Set<string>();
  $effect(() => {
    for (const t of transfers) {
      if (t.state === "done" && !seenDone.has(t.id)) {
        seenDone.add(t.id);
        if (t.upload) refresh();
        else toast(`Downloaded ${t.label}.`);
      }
    }
  });

  onMount(async () => {
    connect();
    unlistenDrop = await getCurrentWebview().onDragDropEvent((e) => {
      // Drops reach every webview listener; while locked this browser may be hidden behind the lock screen.
      if (!visible || !sessionId || !app.vault.unlocked) return;
      const p = e.payload;
      if (p.type === "leave") { dragOver = false; return; }
      const r = root.getBoundingClientRect();
      const x = p.position.x / devicePixelRatio, y = p.position.y / devicePixelRatio;
      const inside = x >= r.left && x <= r.right && y >= r.top && y <= r.bottom;
      if (p.type === "drop") {
        dragOver = false;
        if (inside) upload(p.paths);
      } else {
        dragOver = inside;
      }
    });
  });

  onDestroy(() => {
    unlistenDrop?.();
    if (sessionId) api.filesClose(sessionId);
  });
</script>

<svelte:window onclick={() => (menu = null)} />

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div class="fb" class:drag={dragOver} bind:this={root} tabindex="0" role="application" aria-label="Files on {host?.name}" onkeydown={onKey}>
  <div class="bar">
    <button class="icon-btn" title="Back" disabled={!history.length} onclick={back}><Icon name="back" size={15} /></button>
    <button class="icon-btn" title="Up one folder (Backspace)" onclick={() => go(parentOf(path))}><Icon name="up" size={15} /></button>
    <button class="icon-btn" title="Refresh (F5)" onclick={refresh}><Icon name="refresh" size={15} /></button>
    <form class="path" onsubmit={(e) => { e.preventDefault(); go(pathInput.trim() || "/"); }}>
      <div class="crumbs">
        {#each crumbs as c, i (c.path)}
          <button type="button" onclick={() => go(c.path)}>{c.name}</button>{#if i > 0 && i < crumbs.length - 1}<span>/</span>{/if}
        {/each}
      </div>
      <input id="path-{tab.id}" class="mono" bind:value={pathInput} aria-label="Folder path" />
    </form>
    <input id="filter-{tab.id}" class="filter" placeholder="Filter" bind:value={filter} />
    <button class="btn" onclick={() => { creating = true; newName = ""; }}><Icon name="folder-plus" size={14} /> New folder</button>
    <button class="btn primary" onclick={() => upload()} disabled={!sessionId}><Icon name="upload" size={14} /> Upload</button>
  </div>

  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div class="table" role="grid" tabindex="-1" oncontextmenu={(e) => openMenu(e)}>
    <div class="row head" role="row">
      <span>Name</span><span class="num">Size</span><span>Modified</span><span>Permissions</span>
    </div>
    {#if creating}
      <div class="row editing" role="row">
        <span class="name"><Icon name="folder" size={15} />
          <!-- svelte-ignore a11y_autofocus -->
          <input id="newdir-{tab.id}" bind:value={newName} autofocus placeholder="Folder name"
            onkeydown={(e) => { if (e.key === "Enter") mkdir(); if (e.key === "Escape") creating = false; }}
            onblur={mkdir} />
        </span>
      </div>
    {/if}
    {#each shown as e (e.path)}
      <!-- Keyboard navigation is handled on the browser root (Enter, Delete, F2…). -->
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div
        class="row"
        class:sel={selected.has(e.path)}
        role="row"
        tabindex="-1"
        onclick={(ev) => click(ev, e)}
        ondblclick={() => activate(e)}
        oncontextmenu={(ev) => { ev.stopPropagation(); openMenu(ev, e); }}
      >
        <span class="name" class:dir={e.isDir}>
          <Icon name={e.isDir ? "folder" : "file"} size={15} />
          {#if renaming === e.path}
            <!-- svelte-ignore a11y_autofocus -->
            <input id="rename-{tab.id}" bind:value={renameValue} autofocus
              onclick={(ev) => ev.stopPropagation()}
              onkeydown={(ev) => { if (ev.key === "Enter") finishRename(e); if (ev.key === "Escape") renaming = null; }}
              onblur={() => finishRename(e)} />
          {:else}
            <span class="label">{e.name}</span>
            {#if e.isLink}<span class="link" title="Symbolic link"><Icon name="link" size={11} /></span>{/if}
          {/if}
        </span>
        <span class="num">{e.isDir ? "" : size(e.size)}</span>
        <span class="muted">{date(e.modified)}</span>
        <span class="mono muted">{perms(e.permissions)}</span>
      </div>
    {:else}
      {#if error}
        <div class="state">
          <p class="err">{error}</p>
          <button class="btn" onclick={connect}>Try again</button>
        </div>
      {:else if loading}
        <p class="state muted">Loading…</p>
      {:else}
        <p class="state muted">{filter ? "Nothing matches the filter." : "This folder is empty. Drop files here to upload."}</p>
      {/if}
    {/each}
  </div>

  {#if transfers.length}
    <div class="transfers">
      {#each transfers as t (t.id)}
        <div class="transfer">
          <Icon name={t.upload ? "upload" : "download"} size={13} />
          <span class="tl">{t.label}{#if t.state === "running" && t.current && t.current !== t.label}<small class="muted"> · {t.current}</small>{/if}</span>
          {#if t.state === "running"}
            <div class="meter"><div style:width="{pct(t.done, t.total)}%"></div></div>
            <small class="mono muted">{size(t.done)} / {size(t.total)}</small>
            <button class="icon-btn" title="Cancel" onclick={() => { api.transferCancel(t.id); app.transfers[t.id] = { ...t, state: "cancelled" }; }}><Icon name="x" size={12} /></button>
          {:else if t.state === "done"}
            <small class="ok">{t.upload ? "Uploaded" : "Downloaded"}</small>
            {#if t.saved.length}<button class="linkbtn" onclick={() => revealItemInDir(t.saved)}>Show in folder</button>{/if}
            <button class="icon-btn" title="Dismiss" onclick={() => delete app.transfers[t.id]}><Icon name="x" size={12} /></button>
          {:else}
            <small class={t.state === "error" ? "err" : "muted"} title={t.message ?? ""}>{t.state === "error" ? "Failed" : "Cancelled"}</small>
            <button class="icon-btn" title="Dismiss" onclick={() => delete app.transfers[t.id]}><Icon name="x" size={12} /></button>
          {/if}
        </div>
      {/each}
    </div>
  {/if}

  <div class="status">
    <span>{entries.length} {entries.length === 1 ? "item" : "items"}{#if selectedEntries.length} · {selectedEntries.length} selected{/if}</span>
    <span class="hint">Drop files from your computer to upload · Double-click a file to download it</span>
  </div>

  {#if menu}
    <div class="menu" style:left="{menu.x}px" style:top="{menu.y}px" role="menu">
      {#if selectedEntries.length}
        {#if selectedEntries.length === 1 && selectedEntries[0].isDir}
          <button role="menuitem" onclick={runMenu(() => go(selectedEntries[0].path))}>Open</button>
        {/if}
        <button role="menuitem" onclick={runMenu(() => download())}>Download</button>
        <button role="menuitem" onclick={runMenu(() => download(selectedEntries, true))}>Download to…</button>
        {#if selectedEntries.length === 1}
          <button role="menuitem" onclick={runMenu(() => startRename())}>Rename<kbd>F2</kbd></button>
          <button role="menuitem" onclick={runMenu(() => writeText(selectedEntries[0].path).then(() => toast("Path copied.")))}>Copy path</button>
        {/if}
        <hr />
        <button role="menuitem" class="danger" onclick={runMenu(() => remove())}>Delete<kbd>Del</kbd></button>
      {:else}
        <button role="menuitem" onclick={runMenu(() => upload())}>Upload files…</button>
        <button role="menuitem" onclick={runMenu(() => (creating = true))}>New folder</button>
        <button role="menuitem" onclick={runMenu(refresh)}>Refresh<kbd>F5</kbd></button>
        <button role="menuitem" onclick={runMenu(() => writeText(path).then(() => toast("Path copied.")))}>Copy folder path</button>
      {/if}
    </div>
  {/if}
</div>

<style>
  .fb { height: 100%; display: grid; grid-template-rows: auto 1fr auto auto; background: var(--panel); position: relative; outline: none; min-height: 0; }
  .fb.drag::after { content: "Drop to upload here"; position: absolute; inset: 8px; border: 2px dashed var(--accent); border-radius: 10px; display: grid; place-items: center; background: color-mix(in srgb, var(--accent-soft) 85%, transparent); color: var(--fg); font-weight: 600; pointer-events: none; }
  .bar { display: flex; gap: 6px; align-items: center; padding: 8px 10px; border-bottom: 1px solid var(--line); }
  .icon-btn:disabled { opacity: .35; }
  .path { flex: 1; min-width: 0; position: relative; }
  .path input { padding: 5px 8px; opacity: 0; position: absolute; inset: 0; }
  .path:focus-within input { opacity: 1; position: static; }
  .path:focus-within .crumbs { display: none; }
  .crumbs { display: flex; align-items: center; gap: 2px; padding: 4px 6px; border: 1px solid var(--line); border-radius: var(--radius); background: var(--panel-2); overflow: hidden; white-space: nowrap; font-family: var(--mono); font-size: 12px; pointer-events: none; }
  .crumbs button { pointer-events: auto; padding: 1px 4px; border-radius: 4px; color: var(--muted); position: relative; z-index: 1; }
  .crumbs button:hover { color: var(--fg); background: var(--sel); }
  .crumbs span { color: var(--faint); }
  .filter { width: 130px; padding: 5px 8px; }
  .table { overflow: auto; min-height: 0; padding-bottom: 8px; }
  .row { display: grid; grid-template-columns: minmax(200px, 1fr) 90px 170px 100px; gap: 12px; align-items: center; padding: 4px 14px; white-space: nowrap; }
  .row.head { position: sticky; top: 0; background: var(--panel); color: var(--faint); font-size: 11px; text-transform: uppercase; letter-spacing: .06em; border-bottom: 1px solid var(--line); z-index: 1; }
  .row:not(.head):hover { background: var(--panel-2); }
  .row.sel, .row.sel:hover { background: var(--sel); }
  .name { display: flex; align-items: center; gap: 8px; min-width: 0; color: var(--muted); }
  .name.dir { color: var(--accent); }
  .label { color: var(--fg); overflow: hidden; text-overflow: ellipsis; }
  .link { color: var(--faint); display: inline-grid; }
  .name input { padding: 2px 6px; }
  .num { text-align: right; font-variant-numeric: tabular-nums; color: var(--muted); }
  .state { padding: 24px 14px; display: grid; gap: 10px; justify-items: start; margin: 0; }
  .err { color: var(--bad); margin: 0; }
  .ok { color: var(--good); }
  .transfers { border-top: 1px solid var(--line); max-height: 140px; overflow: auto; padding: 4px 0; }
  .transfer { display: flex; align-items: center; gap: 8px; padding: 3px 14px; }
  .tl { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .meter { width: 140px; height: 5px; border-radius: 3px; background: var(--line); overflow: hidden; }
  .meter div { height: 100%; background: var(--accent); transition: width .15s; }
  .linkbtn { color: var(--accent); font-size: 12px; }
  .status { display: flex; justify-content: space-between; gap: 12px; padding: 4px 14px; border-top: 1px solid var(--line); color: var(--faint); font-size: 11.5px; white-space: nowrap; overflow: hidden; }
  .hint { overflow: hidden; text-overflow: ellipsis; }
  .menu { position: absolute; z-index: 20; min-width: 190px; padding: 4px; background: var(--panel); border: 1px solid var(--line-strong); border-radius: 8px; box-shadow: var(--shadow); display: grid; }
  .menu button { display: flex; justify-content: space-between; align-items: center; gap: 12px; text-align: left; padding: 6px 10px; border-radius: 5px; }
  .menu button:hover { background: var(--sel); }
  .menu .danger { color: var(--bad); }
  .menu hr { border: 0; border-top: 1px solid var(--line); margin: 4px 0; width: 100%; }
</style>
