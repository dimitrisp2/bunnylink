<!-- SPDX-FileCopyrightText: 2026 BunnyCloud.IT -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial -->
<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { Terminal } from "@xterm/xterm";
  import { FitAddon } from "@xterm/addon-fit";
  import { WebglAddon } from "@xterm/addon-webgl";
  import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";
  import "@xterm/xterm/css/xterm.css";
  import { api, errorText } from "../api";
  import type { TermEvent } from "../api";
  import { app, type Tab } from "../state.svelte";

  let { tab, visible }: { tab: Tab; visible: boolean } = $props();

  let el: HTMLDivElement;
  let term: Terminal;
  let fit: FitAddon;
  let webgl: WebglAddon | undefined;
  let sessionId: string | null = null;
  let observer: ResizeObserver;
  let resizeTimer: ReturnType<typeof setTimeout>;

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

  function applyTermTheme() {
    term.options.theme = themeFromCss();
    // The WebGL renderer caches glyphs per colour; redraw everything on the next frame.
    requestAnimationFrame(() => {
      term.clearTextureAtlas();
      term.refresh(0, term.rows - 1);
    });
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

    observer = new ResizeObserver(() => refit());
    observer.observe(el);

    connect();
  });

  $effect(() => {
    if (visible && term) {
      requestAnimationFrame(() => {
        refit();
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
  });

  onDestroy(() => {
    observer?.disconnect();
    if (sessionId) api.terminalClose(sessionId);
    term?.dispose();
  });
</script>

<div class="term" bind:this={el}></div>

<style>
  .term { height: 100%; width: 100%; padding: 6px 0 0 8px; background: var(--term-bg); overflow: hidden; }
  .term :global(.xterm) { height: 100%; }
  .term :global(.xterm-viewport) { background: transparent !important; }
</style>
