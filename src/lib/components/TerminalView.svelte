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

  let { tab, visible }: { tab: Tab; visible: boolean } = $props();

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

  // ANSI palettes tuned for contrast on each background.
  const ANSI = {
    dark: {
      black: "#1b232b", red: "#ef7a6c", green: "#7fd39b", yellow: "#e8c06a", blue: "#6fa8f0", magenta: "#c79bf0", cyan: "#5fd0d2", white: "#d3dde6",
      brightBlack: "#5c6b79", brightRed: "#ff9a8c", brightGreen: "#9be8b4", brightYellow: "#f5d58c", brightBlue: "#93c0ff", brightMagenta: "#dcb8ff", brightCyan: "#86e6e7", brightWhite: "#f4f7fa",
    },
    light: {
      black: "#1d2833", red: "#b8322a", green: "#1d7a44", yellow: "#8a5d00", blue: "#1f5fbf", magenta: "#8a3fb3", cyan: "#0d7f86", white: "#c5ccd3",
      brightBlack: "#5b6876", brightRed: "#d4483e", brightGreen: "#25924f", brightYellow: "#a87200", brightBlue: "#2f72d6", brightMagenta: "#a052cc", brightCyan: "#11959d", brightWhite: "#eef1f4",
    },
  };

  function themeFromCss() {
    const css = getComputedStyle(document.documentElement);
    const v = (n: string) => css.getPropertyValue(n).trim();
    const mode = document.documentElement.dataset.theme === "light" ? "light" : "dark";
    return {
      ...ANSI[mode],
      background: v("--term-bg"),
      foreground: v("--term-fg"),
      cursor: v("--accent"),
      cursorAccent: v("--term-bg"),
      selectionBackground: v("--sel"),
    };
  }

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
    const c = MATCH[app.shownTheme];
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

  function applyTermTheme() {
    term.options.theme = themeFromCss();
    // The WebGL renderer caches glyphs per colour; redraw everything on the next frame.
    requestAnimationFrame(redraw);
  }

  function onEvent(e: TermEvent) {
    switch (e.type) {
      case "data":
        if (live()?.status !== "open") setStatus("open");
        term.write(new Uint8Array(e.data));
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
      fontFamily: getComputedStyle(document.documentElement).getPropertyValue("--mono"),
      fontSize: app.settings.terminalFontSize,
      cursorBlink: true,
      scrollback: 10000,
      allowProposedApi: true,
      theme: themeFromCss(),
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
      if (e.ctrlKey && e.shiftKey && k === "v") {
        readText().then((t) => t && term.paste(t));
        return false;
      }
      return true;
    });
    // Right-click copies the selection, or pastes when nothing is selected.
    el.addEventListener("contextmenu", async (e) => {
      e.preventDefault();
      const sel = term.getSelection();
      if (sel) {
        await writeText(sel);
        term.clearSelection();
      } else {
        const t = await readText().catch(() => "");
        if (t) term.paste(t);
      }
    });

    term.onData((d) => {
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

    // Snippets are typed in as if from the keyboard: each line ends with Enter.
    unregister = registerSnippetTarget(tab.id, {
      send: (text, enter) => {
        if (!sessionId) return toast("This terminal is not connected.");
        api.terminalInput(sessionId, text.replace(/\r?\n/g, "\r") + (enter ? "\r" : ""));
      },
      focus: () => term.focus(),
    });

    observer = new ResizeObserver(() => refit());
    observer.observe(el);

    connect();
  });

  $effect(() => {
    if (visible && term) {
      requestAnimationFrame(() => {
        refit();
        redraw();
        term.focus();
      });
    }
  });

  $effect(() => {
    if (term) term.options.fontSize = app.settings.terminalFontSize;
  });

  $effect(() => {
    void app.shownTheme;
    if (term) applyTermTheme();
    // Re-colour the highlights for the new theme.
    untrack(() => term && find.open && findStep(1, true));
  });

  onDestroy(() => {
    unregister();
    observer?.disconnect();
    if (sessionId) api.terminalClose(sessionId);
    term?.dispose();
  });
</script>

<div class="wrap">
  <div class="term" bind:this={el}></div>
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
  .term { height: 100%; width: 100%; padding: 6px 0 0 8px; background: var(--term-bg); overflow: hidden; }
  .term :global(.xterm) { height: 100%; }
  .term :global(.xterm-viewport) { background: transparent !important; }
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
