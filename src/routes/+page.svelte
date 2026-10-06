<!-- SPDX-FileCopyrightText: 2026 BunnyCloud.IT -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial -->
<script lang="ts">
  import { onMount } from "svelte";
  import Unlock from "$lib/components/Unlock.svelte";
  import Sidebar from "$lib/components/Sidebar.svelte";
  import Workspace from "$lib/components/Workspace.svelte";
  import Palette from "$lib/components/Palette.svelte";
  import StatusBar from "$lib/components/StatusBar.svelte";
  import Toasts from "$lib/components/Toasts.svelte";
  import HostEditor from "$lib/components/HostEditor.svelte";
  import FolderEditor from "$lib/components/FolderEditor.svelte";
  import TunnelEditor from "$lib/components/TunnelEditor.svelte";
  import ImportModal from "$lib/components/ImportModal.svelte";
  import Confirm from "$lib/components/Confirm.svelte";
  import ContextMenu from "$lib/components/ContextMenu.svelte";
  import PassphrasePrompt from "$lib/components/PassphrasePrompt.svelte";
  import Modal from "$lib/components/Modal.svelte";
  import CredentialForm from "$lib/components/CredentialForm.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import { app, toast, boot, afterUnlock, openSettings, quitNow, requestCloseTab, cycleTab, splitRight, unsplit, HOME } from "$lib/state.svelte";
  import { errorText } from "$lib/api";

  let bootError = $state("");

  onMount(async () => {
    window.addEventListener("error", (e) => toast(`Unexpected error: ${e.message}`, "error"));
    window.addEventListener("unhandledrejection", (e) => toast(`Unexpected error: ${errorText(e.reason)}`, "error"));
    try {
      await boot();
      if (app.vault.unlocked) await afterUnlock();
    } catch (e) {
      bootError = errorText(e);
    }
  });

  function onKey(e: KeyboardEvent) {
    if (!app.vault.unlocked) return;
    const k = e.key.toLowerCase();
    const inTerminal = (e.target as HTMLElement)?.closest?.(".xterm");
    if (e.ctrlKey && k === "k" && (e.shiftKey || !inTerminal)) {
      e.preventDefault();
      app.paletteOpen = !app.paletteOpen;
    } else if (e.ctrlKey && e.key === "Tab") {
      e.preventDefault();
      cycleTab(e.shiftKey ? -1 : 1);
    } else if (e.ctrlKey && e.shiftKey && k === "w") {
      e.preventDefault();
      const cur = app.panes[app.focus];
      if (cur && cur !== HOME) requestCloseTab(cur);
    } else if (e.ctrlKey && e.key === "\\") {
      e.preventDefault();
      if (app.panes.right) unsplit();
      else {
        const cur = app.panes.left;
        const other = app.tabs.find((t) => t.id !== cur);
        if (cur !== HOME) splitRight(cur);
        else if (other) splitRight(other.id);
      }
    }
  }
</script>

<svelte:window onkeydown={onKey} />

{#if bootError}
  <div class="fatal"><h1>BunnyLink could not start</h1><p>{bootError}</p></div>
{:else if !app.ready}
  <div></div>
{:else if !app.vault.unlocked}
  <Unlock />
{:else}
  <div class="app">
    <header class="bar">
      <span class="brand"><img src="/logo-small.png" alt="" /><span>Bunny<b>Link</b></span></span>
      <button class="search" onclick={() => (app.paletteOpen = true)}>
        <Icon name="search" size={14} />
        <span>Search hosts or run an action…</span>
        <kbd>Ctrl K</kbd>
      </button>
      <button class="icon-btn" title="Settings" onclick={openSettings}><Icon name="settings" /></button>
    </header>
    <div class="main">
      <Sidebar />
      <Workspace />
    </div>
    <StatusBar />
  </div>

  {#if app.paletteOpen}<Palette />{/if}
  {#if app.menu}<ContextMenu menu={app.menu} />{/if}
  {#each app.passphrasePrompts.slice(0, 1) as request (request.id)}
    <PassphrasePrompt {request} />
  {/each}

  {#key app.modal}
  {#if app.modal?.kind === "host"}
    <HostEditor host={app.modal.host} folderId={app.modal.folderId} />
  {:else if app.modal?.kind === "folder"}
    <FolderEditor folder={app.modal.folder} parentId={app.modal.parentId} />
  {:else if app.modal?.kind === "tunnel"}
    <TunnelEditor tunnel={app.modal.tunnel} hostId={app.modal.hostId} targetPort={app.modal.targetPort} />
  {:else if app.modal?.kind === "credential"}
    {@const m = app.modal}
    <Modal title={m.credential ? `Edit ${m.credential.name}` : "New credential"}>
      <CredentialForm credential={m.credential} onSaved={(c) => { app.modal = null; m.onSaved?.(c); }} onCancel={() => (app.modal = null)} />
    </Modal>
  {:else if app.modal?.kind === "import"}
    <ImportModal />
  {:else if app.modal?.kind === "confirm"}
    <Confirm title={app.modal.title} body={app.modal.body} confirm={app.modal.confirm} run={app.modal.run} />
  {/if}
  {/key}
{/if}

{#if app.quitConfirm}
  <Modal title="Quit BunnyLink?" width={420} dismissable onclose={() => (app.quitConfirm = null)}>
    <p class="quit">{app.quitConfirm}</p>
    {#snippet footer()}
      <button class="btn" onclick={() => (app.quitConfirm = null)}>Cancel</button>
      <!-- svelte-ignore a11y_autofocus -->
      <button class="btn primary quit-btn" autofocus onclick={quitNow}>Quit</button>
    {/snippet}
  </Modal>
{/if}

<Toasts />

<style>
  .app { height: 100%; display: grid; grid-template-rows: 40px 1fr auto; }
  .bar { display: flex; align-items: center; gap: 12px; padding: 0 10px 0 14px; border-bottom: 1px solid var(--line); background: var(--panel); }
  .brand { font-weight: 700; font-size: 14px; width: 220px; display: flex; align-items: center; gap: 8px; }
  .brand img { width: 24px; height: 24px; border-radius: 5px; }
  .brand b { color: var(--accent); }
  .search {
    margin: 0 auto; width: min(440px, 50%); display: flex; align-items: center; gap: 8px;
    padding: 5px 10px; border: 1px solid var(--line); border-radius: 6px; color: var(--faint); background: var(--bg);
  }
  .search:hover { border-color: var(--line-strong); color: var(--muted); }
  .search span { flex: 1; text-align: left; }
  .main { display: grid; grid-template-columns: 260px 1fr; min-height: 0; }
  .fatal { padding: 40px; }
  .quit { margin: 0; color: var(--muted); }
  .quit-btn { background: var(--bad); border-color: var(--bad); color: #fff; }
</style>
