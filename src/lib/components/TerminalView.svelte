<!-- SPDX-FileCopyrightText: 2026 BunnyCloud.IT -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial -->
<script lang="ts">
  import { onDestroy, onMount, untrack } from "svelte";
  import { Terminal } from "@xterm/xterm";
  import { FitAddon } from "@xterm/addon-fit";
  import { WebglAddon } from "@xterm/addon-webgl";
  import { SearchAddon } from "@xterm/addon-search";
  import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";
  import "@xterm/xterm/css/xterm.css";
  import Icon from "./Icon.svelte";
  import { api, errorText } from "../api";
  import type { TermEvent } from "../api";
  import { app, registerSnippetTarget, toast, type Tab } from "../state.svelte";
  import { isLight, playBell, terminalLook } from "../terminal";

  let { tab, visible }: { tab: Tab; visible: boolean } = $props();
  /** Replays a session log, read-only, instead of connecting. */
  // svelte-ignore state_referenced_locally
  const viewer = tab.kind === "log";

  let el: HTMLDivElement;
  let term: Terminal;
  let fit: FitAddon;
  let webgl: WebglAddon | undefined;
  let search: SearchAddon;
  let findInput = $state<HTMLInputElement>();
  let find = $state({ open: false, text: "", caseSensitive: false, wholeWord: false, regex: false });
  let found = $state({ index: -1, count: 0 });
  let sessionId: string | null = null;
  let observer: ResizeObserver;
  let resizeTimer: ReturnType<typeof setTimeout>;
  let unregister = () => {};

  const live = () => app.tabs.find((t) => t.id === tab.id);
  const setStatus = (s: Tab["status"]) => {
    const t = live();
    if (t) t.status = s;
  };

  const look = $derived(terminalLook(app.settings, app.shownTheme));
  let flashing = $state(false);

  // The WebGL glyph atlas is shared between terminals and can go stale while this one is
  // hidden, so rebuild it and redraw every row.
  function redraw() {
    term.clearTextureAtlas();
    term.refresh(0, term.rows - 1);
  }

  // Search highlights; the add-on needs #rrggbb colours.
  const MATCH = {
    dark: { match: "#3a4a63", active: "#c9a227" },
    light: { match: "#cdd8ea", active: "#f2c94c" },
  };

  function findOptions(incremental = false) {
    const c = MATCH[isLight(look.scheme.background) ? "light" : "dark"];
    const { caseSensitive, wholeWord, regex } = find;
    return {
      caseSensitive, wholeWord, regex, incremental,
      decorations: { matchBackground: c.match, matchOverviewRuler: c.match, activeMatchBackground: c.active, activeMatchColorOverviewRuler: c.active },
    };
  }

  /** Jumps to the next (or previous) match; typing searches from the current match. */
  function findStep(dir: 1 | -1 = 1, incremental = false) {
    if (!find.text) {
      search.clearDecorations();
      found = { index: -1, count: 0 };
      return;
    }
    try {
      if (dir === 1) search.findNext(find.text, findOptions(incremental));
      else search.findPrevious(find.text, findOptions());
    } catch {
      // An unfinished regex; wait for more input.
      found = { index: -1, count: 0 };
    }
  }

  function openFind() {
    find.open = true;
    const sel = term.getSelection();
    // A one-line selection becomes the search text.
    if (sel && !sel.includes("\n")) find.text = sel;
    requestAnimationFrame(() => {
      findInput?.focus();
      findInput?.select();
      findStep(1, true);
    });
  }

  function closeFind() {
    find.open = false;
    search.clearDecorations();
    found = { index: -1, count: 0 };
    term.focus();
  }

  function onFindKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      closeFind();
    } else if (e.key === "Enter") {
      e.preventDefault();
      findStep(e.shiftKey ? -1 : 1);
    } else if (e.ctrlKey && e.shiftKey && e.key.toLowerCase() === "f") {
      e.preventDefault();
      findInput?.select();
    }
  }

  function toggleFind(opt: "caseSensitive" | "wholeWord" | "regex") {
    find[opt] = !find[opt];
    findStep(1, true);
    findInput?.focus();
  }

  function onBell() {
    const mode = app.settings.terminalBell;
    if (mode === "sound") playBell();
    else if (mode === "flash") {
      flashing = false;
      requestAnimationFrame(() => (flashing = true));
    }
    const t = live();
    if (t && !visible && app.settings.terminalBellMarkTab) t.bell = true;
  }

  async function copySelection() {
    const sel = term.getSelection();
    if (sel) await writeText(sel);
  }

  async function paste() {
    const t = await readText().catch(() => "");
    if (t) term.paste(t);
  }

  async function onContextMenu(e: MouseEvent) {
    e.preventDefault();
    const sel = term.getSelection();
    switch (app.settings.terminalRightClick) {
      case "copyPaste":
        if (sel) {
          await writeText(sel);
          term.clearSelection();
        } else if (!viewer) await paste();
        break;
      case "paste":
        if (!viewer) await paste();
        break;
      case "menu":
        app.menu = {
          x: e.clientX,
          y: e.clientY,
          items: [
            { label: "Copy", disabled: !sel, run: () => copySelection().then(() => term.focus()) },
            ...(viewer ? [] : [{ label: "Paste", run: () => paste().then(() => term.focus()) }]),
            "separator",
            { label: "Select all", run: () => (term.selectAll(), term.focus()) },
            { label: "Find…", run: openFind },
            ...(viewer ? [] : ["separator" as const, { label: "Clear scrollback", run: () => (term.clear(), term.focus()) }]),
          ],
        };
        break;
    }
  }

  function onEvent(e: TermEvent) {
    switch (e.type) {
      case "data":
        if (live()?.status !== "open") setStatus("open");
        term.write(e.data);
        break;
      case "notice":
        term.write(`\x1b[2m${e.text}\x1b[0m\r\n`);
        break;
      case "prompt":
        term.write(e.text);
        break;
      case "exit":
        setStatus("closed");
        sessionId = null;
        term.write(`\r\n\x1b[2mSession ended${e.code != null ? ` (exit ${e.code})` : ""}. Press Enter to reconnect.\x1b[0m\r\n`);
        break;
      case "error":
        setStatus("error");
        sessionId = null;
        term.write(`\r\n\x1b[31m${e.message}\x1b[0m\r\n\x1b[2mPress Enter to try again.\x1b[0m\r\n`);
        break;
    }
  }

  const LOG_LIMIT = 16 * 1024 * 1024;
  async function loadLog() {
    try {
      const bytes = new Uint8Array(await api.readSessionLog(tab.logName!));
      if (bytes.length >= LOG_LIMIT) term.write("\x1b[2mThis log is large; only its last 16 MB are shown.\x1b[0m\r\n");
      term.write(bytes, () => term.scrollToTop());
    } catch (e) {
      term.write(`\x1b[31m${errorText(e)}\x1b[0m\r\n`);
    }
  }

  async function connect() {
    setStatus("connecting");
    try {
      sessionId = await api.terminalOpen(tab.hostId, tab.protocol, term.cols, term.rows, onEvent);
    } catch (e) {
      onEvent({ type: "error", message: errorText(e) });
    }
  }

  function refit() {
    if (!visible || !el?.offsetWidth) return;
    fit.fit();
  }

  onMount(() => {
    term = new Terminal({
      ...look.options,
      cursorBlink: !viewer && app.settings.terminalCursorBlink,
      // A log is shown whole, as far as memory allows.
      scrollback: viewer ? 200000 : app.settings.terminalScrollback,
      disableStdin: viewer,
      allowProposedApi: true,
    });
    fit = new FitAddon();
    term.loadAddon(fit);
    search = new SearchAddon();
    term.loadAddon(search);
    search.onDidChangeResults(({ resultIndex, resultCount }) => (found = { index: resultIndex, count: resultCount }));
    term.open(el);
    try {
      webgl = new WebglAddon();
      webgl.onContextLoss(() => {
        webgl?.dispose();
        webgl = undefined;
      });
      term.loadAddon(webgl);
    } catch {
      webgl = undefined;
      /* falls back to the DOM renderer */
    }
    fit.fit();

    // Let app shortcuts through instead of sending them to the shell.
    term.attachCustomKeyEventHandler((e) => {
      if (e.type !== "keydown") return true;
      const k = e.key.toLowerCase();
      if (e.ctrlKey && e.shiftKey && (k === "k" || k === "w")) return false;
      if (e.ctrlKey && e.shiftKey && k === "f") {
        openFind();
        return false;
      }
      if (e.ctrlKey && (e.key === "Tab" || e.key === "\\")) return false;
      if (e.ctrlKey && e.shiftKey && k === "c") {
        const sel = term.getSelection();
        if (sel) writeText(sel);
        return false;
      }
      if (e.ctrlKey && e.shiftKey && k === "v" && !viewer) {
        readText().then((t) => t && term.paste(t));
        return false;
      }
      return true;
    });
    el.addEventListener("contextmenu", onContextMenu);
    term.onBell(onBell);
    // Copy on select: wait until the selection stops changing, as it does while dragging.
    let selTimer: ReturnType<typeof setTimeout>;
    term.onSelectionChange(() => {
      if (!app.settings.terminalCopyOnSelect) return;
      clearTimeout(selTimer);
      selTimer = setTimeout(() => term.hasSelection() && copySelection(), 200);
    });

    term.onData((d) => {
      if (viewer) return;
      if (sessionId) api.terminalInput(sessionId, d);
      else if (d === "\r" && live()?.status !== "connecting") {
        term.reset();
        connect();
      }
    });
    term.onResize(({ cols, rows }) => {
      clearTimeout(resizeTimer);
      resizeTimer = setTimeout(() => sessionId && api.terminalResize(sessionId, cols, rows), 80);
    });

    observer = new ResizeObserver(() => refit());
    observer.observe(el);
    if (viewer) {
      loadLog();
      return;
    }

    // Snippets are typed in as if from the keyboard: each line ends with Enter.
    unregister = registerSnippetTarget(tab.id, {
      send: (text, enter) => {
        if (!sessionId) return toast("This terminal is not connected.");
        api.terminalInput(sessionId, text.replace(/\r?\n/g, "\r") + (enter ? "\r" : ""));
      },
      focus: () => term.focus(),
    });

    connect();
  });

  $effect(() => {
    if (visible && term) {
      untrack(() => {
        const t = live();
        if (t?.bell) t.bell = false;
      });
      requestAnimationFrame(() => {
        refit();
        redraw();
        term.focus();
      });
    }
  });

  // Settings apply to open terminals straight away, except scrollback: xterm does not resize
  // an open terminal's buffer to a new length until the next resize, which upsets scrolling.
  $effect(() => {
    const { options } = look;
    const blink = !viewer && app.settings.terminalCursorBlink;
    if (!term) return;
    untrack(() => {
      Object.assign(term.options, options, { cursorBlink: blink });
      refit();
      // The WebGL renderer caches glyphs per font and colour; redraw everything on the next frame.
      requestAnimationFrame(redraw);
      // Re-colour the search highlights for the new background.
      if (find.open) findStep(1, true);
    });
  });

  onDestroy(() => {
    unregister();
    observer?.disconnect();
    if (sessionId) api.terminalClose(sessionId);
    term?.dispose();
  });
</script>

<div class="wrap">
  <div class="term" bind:this={el} style:background={look.scheme.background}></div>
  {#if flashing}<div class="flash" onanimationend={() => (flashing = false)}></div>{/if}
  {#if find.open}
    <div class="find" role="search">
      <Icon name="search" size={13} />
      <input
        bind:this={findInput}
        bind:value={find.text}
        oninput={() => findStep(1, true)}
        onkeydown={onFindKey}
        placeholder="Find"
        aria-label="Find in terminal"
        spellcheck="false"
      />
      <span class="count" class:none={find.text && !found.count}>
        {#if !find.text}{:else if !found.count}No results{:else if found.index < 0}{found.count}+ matches{:else}{found.index + 1} of {found.count}{/if}
      </span>
      <button class="opt" class:act={find.caseSensitive} title="Match case" aria-pressed={find.caseSensitive} onclick={() => toggleFind("caseSensitive")}>Aa</button>
      <button class="opt" class:act={find.wholeWord} title="Whole word" aria-pressed={find.wholeWord} onclick={() => toggleFind("wholeWord")}><u>ab</u></button>
      <button class="opt mono" class:act={find.regex} title="Regular expression" aria-pressed={find.regex} onclick={() => toggleFind("regex")}>.*</button>
      <button class="icon-btn mini" title="Previous (Shift+Enter)" onclick={() => findStep(-1)}><Icon name="up" size={13} /></button>
      <button class="icon-btn mini down" title="Next (Enter)" onclick={() => findStep(1)}><Icon name="up" size={13} /></button>
      <button class="icon-btn mini" title="Close (Esc)" onclick={closeFind}><Icon name="x" size={13} /></button>
    </div>
  {/if}
</div>

<style>
  .wrap { position: relative; height: 100%; width: 100%; }
  /* Rows sit at the bottom, so the leftover space under a whole row shows above the first line, not below the prompt. */
  .term { height: 100%; width: 100%; display: flex; flex-direction: column; justify-content: flex-end; background: var(--term-bg); overflow: hidden; }
  /* Padding goes on .xterm, not .term, so the fit addon subtracts it when counting rows. */
  .term :global(.xterm) { padding: 6px 0 4px 8px; }
  .term :global(.xterm-viewport) { background: transparent !important; }
  .flash { position: absolute; inset: 0; pointer-events: none; background: var(--fg); opacity: 0; animation: flash 0.2s ease-out; }
  @keyframes flash { from { opacity: 0.12; } to { opacity: 0; } }
  .find { position: absolute; top: 8px; right: 18px; display: flex; align-items: center; gap: 4px; padding: 4px 6px 4px 10px; background: var(--panel); border: 1px solid var(--line-strong); border-radius: 7px; box-shadow: var(--shadow); color: var(--muted); z-index: 5; }
  .find input { width: 200px; padding: 3px 6px; }
  .find .count { min-width: 64px; font-size: 11px; text-align: right; white-space: nowrap; }
  .find .count.none { color: var(--bad); }
  .find .opt { min-width: 24px; height: 22px; padding: 0 4px; border-radius: 4px; font-size: 11px; font-weight: 600; color: var(--muted); }
  .find .opt:hover { background: var(--panel-2); color: var(--fg); }
  .find .opt.act { background: var(--accent-soft); color: var(--accent); box-shadow: inset 0 0 0 1px var(--accent); }
  .find .mini { width: 22px; height: 22px; }
  .find .down :global(svg) { transform: rotate(180deg); }
</style>
