<!-- SPDX-FileCopyrightText: 2026 BunnyCloud.IT -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial -->
<script lang="ts">
  import Icon from "./Icon.svelte";
  import HostDetail from "./HostDetail.svelte";
  import TerminalView from "./TerminalView.svelte";
  import CommandView from "./CommandView.svelte";
  import FileBrowser from "./FileBrowser.svelte";
  import RemoteDesktop from "./RemoteDesktop.svelte";
  import SettingsView from "./SettingsView.svelte";
  import { app, requestCloseTab, showTab, splitRight, tabMenu, unsplit, HOME, type Pane } from "../state.svelte";

  const split = $derived(app.panes.right !== null);
  const paneOf = (id: string): Pane | null =>
    app.panes.left === id ? "left" : app.panes.right === id ? "right" : null;
  const column = (id: string) => (paneOf(id) === "right" ? 2 : 1);
</script>

<section class="ws">
  <nav class="tabs" aria-label="Open sessions">
    <button class="tab" class:act={paneOf(HOME)} class:focus={app.panes[app.focus] === HOME} onclick={() => showTab(HOME)}>
      <Icon name="hosts" size={13} /> Hosts
    </button>
    {#each app.tabs as t (t.id)}
      <div class="tab" class:act={paneOf(t.id)} class:focus={app.panes[app.focus] === t.id} role="presentation"
        onmousedown={(e) => e.button === 1 && e.preventDefault()}
        onauxclick={(e) => e.button === 1 && requestCloseTab(t.id)}
        oncontextmenu={(e) => tabMenu(e, t.id)}>
        <button class="label" onclick={() => showTab(t.id)} title={t.title}>
          {#if t.kind === "settings"}<Icon name="settings" size={12} />{:else}
            <span class="dot" class:on={t.status === "open"} class:busy={t.status === "connecting"} class:err={t.status === "error"}></span>
          {/if}
          {#if t.kind === "command"}<Icon name="play" size={11} />{:else if t.kind === "files"}<Icon name="folder" size={11} />{:else if t.kind === "desktop"}<Icon name="monitor" size={11} />{/if}
          {t.title}
          {#if paneOf(t.id) === "right"}<span class="side">R</span>{/if}
        </button>
        <button class="icon-btn small" title="Open on the right" onclick={() => splitRight(t.id)}><Icon name="split" size={12} /></button>
        <button class="icon-btn small" title="Close (Ctrl+Shift+W)" onclick={() => requestCloseTab(t.id)}><Icon name="x" size={12} /></button>
      </div>
    {/each}
    <span class="spacer"></span>
    {#if split}
      <button class="icon-btn" title="Back to one pane (Ctrl+\)" onclick={unsplit}><Icon name="split" size={14} /></button>
    {/if}
  </nav>

  <div class="panes" class:split>
    <div
      class="pane"
      class:focused={split && app.focus === paneOf(HOME)}
      style:grid-column={column(HOME)}
      hidden={!paneOf(HOME)}
      role="presentation"
      onmousedown={() => (app.focus = paneOf(HOME) ?? app.focus)}
    >
      <HostDetail />
    </div>
    {#each app.tabs as t (t.id)}
      <div
        class="pane"
        class:focused={split && app.focus === paneOf(t.id)}
        style:grid-column={column(t.id)}
        hidden={!paneOf(t.id)}
        role="presentation"
        onmousedown={() => (app.focus = paneOf(t.id) ?? app.focus)}
      >
        {#if t.kind === "terminal"}
          <TerminalView tab={t} visible={!!paneOf(t.id)} />
        {:else if t.kind === "desktop"}
          <RemoteDesktop tab={t} visible={!!paneOf(t.id)} />
        {:else if t.kind === "files"}
          <FileBrowser tab={t} visible={!!paneOf(t.id)} />
        {:else if t.kind === "settings"}
          <SettingsView />
        {:else}
          <CommandView tab={t} />
        {/if}
      </div>
    {/each}
  </div>
</section>

<style>
  .ws { display: grid; grid-template-rows: auto 1fr; min-width: 0; min-height: 0; }
  .tabs { display: flex; gap: 2px; padding: 6px 8px 0; background: var(--bg); border-bottom: 1px solid var(--line); overflow-x: auto; align-items: flex-end; }
  .tab {
    display: flex; align-items: center; gap: 2px; padding: 0 4px 0 0; border-radius: 6px 6px 0 0;
    color: var(--muted); white-space: nowrap; border: 1px solid transparent; border-bottom: 0; margin-bottom: -1px;
  }
  button.tab { padding: 6px 12px; gap: 6px; }
  .tab .label { display: flex; align-items: center; gap: 6px; padding: 6px 4px 6px 12px; }
  .tab:hover { color: var(--fg); }
  .tab.act { background: var(--panel); color: var(--fg); border-color: var(--line); }
  .tab.focus { box-shadow: inset 0 2px 0 var(--accent); }
  .tab .small { width: 20px; height: 20px; visibility: hidden; }
  .tab:hover .small, .tab.act .small { visibility: visible; }
  .side { font: 600 9px var(--mono); color: var(--accent); }
  .spacer { flex: 1; }
  .panes { display: grid; grid-template-columns: 1fr; min-height: 0; background: var(--panel); }
  .panes.split { grid-template-columns: 1fr 1fr; }
  .pane { grid-row: 1; min-width: 0; min-height: 0; overflow: hidden; position: relative; }
  .panes.split .pane { border-top: 2px solid transparent; }
  .panes.split .pane:nth-child(n) { border-left: 1px solid var(--line); }
  .pane.focused { border-top-color: var(--accent) !important; }
</style>
