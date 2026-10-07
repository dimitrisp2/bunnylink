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
  import SnippetEditor from "$lib/components/SnippetEditor.svelte";
  import SnippetRun from "$lib/components/SnippetRun.svelte";
  import ImportModal from "$lib/components/ImportModal.svelte";
  import Confirm from "$lib/components/Confirm.svelte";
  import ContextMenu from "$lib/components/ContextMenu.svelte";
  import PassphrasePrompt from "$lib/components/PassphrasePrompt.svelte";
  import Modal from "$lib/components/Modal.svelte";
  import CredentialForm from "$lib/components/CredentialForm.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import { app, toast, boot, afterUnlock, openSettings, quitNow, requestCloseTab, cycleTab, splitRight, unsplit, HOME } from "$lib/state.svelte";
  import { api, errorText } from "$lib/api";

  let bootError = $state("");
  const locked = $derived(!app.vault.unlocked);
  // With the option on, a lock hides open tabs behind the lock screen instead of closing them.
  const keepWhileLocked = $derived(app.settings.keepSessionsWhenLocked && app.tabs.some((t) => t.kind !== "settings"));
  let appEl = $state<HTMLDivElement>();
  // Move focus off a hidden terminal or desktop, so nothing typed at the lock screen reaches it.
  $effect(() => {
    if (locked && appEl?.contains(document.activeElement)) (document.activeElement as HTMLElement).blur();
  });

  onMount(async () => {
    window.addEventListener("error", (e) => toast(`Unexpected error: ${e.message}`, "error"));
    window.addEventListener("unhandledrejection", (e) => toast(`Unexpected error: ${errorText(e.reason)}`, "error"));
    // Any interaction postpones the auto-lock, not only the actions that reach the backend.
    // Captured, so events a component stops (menus, terminals) still count; sent at most every 15 s.
    let lastActivity = 0;
    const activity = () => {
      const now = Date.now();
      if (!app.vault.unlocked || now - lastActivity < 15_000) return;
      lastActivity = now;
      api.userActivity().catch(() => {});
    };
    for (const type of ["pointerdown", "pointermove", "keydown", "wheel"])
      window.addEventListener(type, activity, { capture: true, passive: true });
    try {
      await boot();
      if (app.vault.unlocked) await afterUnlock();
    } catch (e) {
      bootError = errorText(e);
    }
  });

  const TEXT_TYPES = ["text", "password", "search", "email", "url", "tel", "number"];
  /** Text inputs and textareas, where the webview's own menu and autofill would apply. */
  function textField(t: EventTarget | null): HTMLInputElement | HTMLTextAreaElement | null {
    if (t instanceof HTMLTextAreaElement) return t;
    if (t instanceof HTMLInputElement && TEXT_TYPES.includes(t.type)) return t;
    return null;
  }

  // The webview's right-click menu only shows in text fields; our own menus cover the rest.
  function onContextMenu(e: MouseEvent) {
    if (!textField(e.target)) e.preventDefault();
  }

  // No browser autofill suggestions in any field, including ones added later.
  function onFocusIn(e: FocusEvent) {
    textField(e.target)?.setAttribute("autocomplete", "off");
  }

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
      // Don't let the close confirmation replace an open editor or prompt.
      if (app.modal || app.passphrasePrompts.length) return;
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

<svelte:window onkeydown={onKey} oncontextmenu={onContextMenu} />
<svelte:document onfocusin={onFocusIn} />

{#if bootError}
  <div class="fatal"><h1>BunnyLink could not start</h1><p>{bootError}</p></div>
{:else if !app.ready}
  <div></div>
{:else if locked && !keepWhileLocked}
  <Unlock />
{:else}
  <!-- While locked it is hidden and inert: no clicks, keys or focus reach it. -->
  <div class="app" class:locked inert={locked} bind:this={appEl}>
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

  {#if locked}
    <div class="lock-over"><Unlock /></div>
  {:else}
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
  {:else if app.modal?.kind === "snippet"}
    <SnippetEditor snippet={app.modal.snippet} />
  {:else if app.modal?.kind === "snippetRun"}
    <SnippetRun snippet={app.modal.snippet} tabId={app.modal.tabId} fields={app.modal.fields} values={app.modal.values} />
  {:else if app.modal?.kind === "confirm"}
    <Confirm title={app.modal.title} body={app.modal.body} confirm={app.modal.confirm} run={app.modal.run} />
  {/if}
  {/key}
  {/if}
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
  /* Hidden but laid out, so terminals and desktops don't see a resize. */
  .app.locked { visibility: hidden; }
  .lock-over { position: fixed; inset: 0; z-index: 1000; }
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
