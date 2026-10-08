<!-- SPDX-FileCopyrightText: 2026 BunnyCloud.IT -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial -->
<script lang="ts">
  import { onDestroy, onMount, untrack } from "svelte";
  import { Terminal } from "@xterm/xterm";
  import { FitAddon } from "@xterm/addon-fit";
  import "@xterm/xterm/css/xterm.css";
  import { app } from "../state.svelte";
  import { terminalLook } from "../terminal";

  /** A read-only terminal showing sample output in the current terminal settings. */
  let el: HTMLDivElement;
  let term: Terminal;
  let fit: FitAddon;
  let observer: ResizeObserver;

  const look = $derived(terminalLook(app.settings, app.shownTheme));

  const prompt = "\x1b[1;32mroot@bunny\x1b[0m:\x1b[1;34m~\x1b[0m# ";
  const swatches = (base: number) => Array.from({ length: 8 }, (_, i) => `\x1b[${base + i}m   `).join("") + "\x1b[0m";
  const SAMPLE = [
    `${prompt}ls --color`,
    "\x1b[1;34mbin\x1b[0m  \x1b[1;36mlib64\x1b[0m  \x1b[1;32mdeploy.sh\x1b[0m  notes.txt  \x1b[1;31mbackup.tar.zst\x1b[0m  \x1b[33mconfig.yml\x1b[0m",
    `${prompt}echo "The quick brown fox → ✓ 日本語"`,
    "The quick brown fox → ✓ 日本語",
    `${prompt}colors`,
    `${swatches(40)}  \x1b[2mnormal\x1b[0m`,
    `${swatches(100)}  \x1b[2mbright\x1b[0m`,
    "\x1b[31mred \x1b[32mgreen \x1b[33myellow \x1b[34mblue \x1b[35mmagenta \x1b[36mcyan \x1b[1mbold\x1b[0m \x1b[3mitalic\x1b[0m \x1b[4munderline\x1b[0m",
    prompt,
  ].join("\r\n");

  onMount(() => {
    term = new Terminal({ ...look.options, disableStdin: true, cursorInactiveStyle: app.settings.terminalCursor, scrollback: 0 });
    fit = new FitAddon();
    term.loadAddon(fit);
    term.open(el);
    fit.fit();
    // xterm draws no cursor until it is focused or typed into; switching to the alternate
    // screen, as full-screen programs do, shows it too.
    term.write(`\x1b[?1049h${SAMPLE}`);
    observer = new ResizeObserver(() => fit.fit());
    observer.observe(el);
  });

  $effect(() => {
    const { options } = look;
    const inactive = app.settings.terminalCursor;
    if (!term) return;
    untrack(() => {
      Object.assign(term.options, options, { cursorInactiveStyle: inactive });
      fit.fit();
    });
  });

  onDestroy(() => {
    observer?.disconnect();
    term?.dispose();
  });
</script>

<div class="preview" bind:this={el} style:background={look.scheme.background} aria-label="Terminal preview" role="img"></div>

<style>
  .preview { height: 196px; max-width: 760px; border: 1px solid var(--line); border-radius: 8px; overflow: hidden; display: flex; flex-direction: column; justify-content: flex-end; }
  .preview :global(.xterm) { padding: 6px 0 6px 10px; }
  .preview :global(.xterm-viewport) { background: transparent !important; overflow: hidden !important; }
</style>
