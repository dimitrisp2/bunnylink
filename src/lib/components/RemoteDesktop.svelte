<!-- SPDX-FileCopyrightText: 2026 BunnyCloud.IT -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial -->
<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import Icon from "./Icon.svelte";
  import { api, errorText } from "../api";
  import type { DesktopEvent, DesktopInput } from "../api";
  import { CTRL_ALT_DEL, CTRL_ALT_DEL_KEYSYMS, SCANCODES, keysym } from "../scancodes";
  import { app, hostById, type Tab } from "../state.svelte";

  let { tab, visible }: { tab: Tab; visible: boolean } = $props();
  const host = $derived(hostById(tab.hostId));
  const protocol = $derived(tab.protocol === "vnc" ? "vnc" : "rdp");

  let wrap: HTMLDivElement;
  let canvas: HTMLCanvasElement;
  let ctx: CanvasRenderingContext2D;
  let sessionId: string | null = null;
  let phase = $state<"connecting" | "open" | "closed" | "error">("connecting");
  let message = $state("");
  let notices = $state<string[]>([]);
  let size = $state({ width: 0, height: 0 });
  let resizeTimer: ReturnType<typeof setTimeout>;
  let observer: ResizeObserver;
  let pending: DesktopInput[] = [];
  let lastMove: DesktopInput | null = null;
  let flushQueued = false;

  const setStatus = (s: Tab["status"]) => {
    const t = app.tabs.find((x) => x.id === tab.id);
    if (t) t.status = s;
  };

  // ------------------------------------------------------------ output

  function drawRects(buf: ArrayBuffer) {
    const v = new DataView(buf);
    const count = v.getUint16(1, true);
    let at = 3;
    for (let i = 0; i < count; i++) {
      const x = v.getUint16(at, true), y = v.getUint16(at + 2, true);
      const w = v.getUint16(at + 4, true), h = v.getUint16(at + 6, true);
      at += 8;
      const bytes = new Uint8ClampedArray(buf, at, w * h * 4);
      ctx.putImageData(new ImageData(bytes, w, h), x, y);
      at += w * h * 4;
    }
  }

  function setCursor(buf: ArrayBuffer) {
    const v = new DataView(buf);
    const w = v.getUint16(1, true), h = v.getUint16(3, true);
    const hx = v.getUint16(5, true), hy = v.getUint16(7, true);
    if (!w || !h) return;
    const c = document.createElement("canvas");
    c.width = w;
    c.height = h;
    c.getContext("2d")!.putImageData(new ImageData(new Uint8ClampedArray(buf, 9, w * h * 4), w, h), 0, 0);
    canvas.style.cursor = `url(${c.toDataURL()}) ${hx} ${hy}, default`;
  }

  function onMessage(m: DesktopEvent | ArrayBuffer) {
    if (m instanceof ArrayBuffer) {
      const kind = new Uint8Array(m, 0, 1)[0];
      if (kind === 1) drawRects(m);
      else if (kind === 2) setCursor(m);
      else if (kind === 3) canvas.style.cursor = "default";
      else if (kind === 4) canvas.style.cursor = "none";
      return;
    }
    switch (m.type) {
      case "notice":
        notices = [...notices, m.text];
        break;
      case "connected":
      case "resized":
        size = { width: m.width, height: m.height };
        canvas.width = m.width;
        canvas.height = m.height;
        if (m.type === "connected") {
          phase = "open";
          setStatus("open");
          wrap.focus();
        }
        break;
      case "closed":
        phase = "closed";
        message = m.reason;
        setStatus("closed");
        sessionId = null;
        break;
      case "error":
        phase = "error";
        message = m.message;
        setStatus("error");
        sessionId = null;
        break;
    }
  }

  async function connect() {
    phase = "connecting";
    notices = [];
    message = "";
    setStatus("connecting");
    const r = wrap.getBoundingClientRect();
    try {
      sessionId = await api.desktopOpen(tab.hostId, protocol, Math.round(r.width), Math.round(r.height), onMessage);
    } catch (e) {
      onMessage({ type: "error", message: errorText(e) });
    }
  }

  // ------------------------------------------------------------ input

  // Pointer moves are coalesced to one per frame; everything else goes out at once so
  // key presses and releases are never delayed (a late release causes remote auto-repeat).
  function send(e: DesktopInput) {
    if (!sessionId || phase !== "open") return;
    if (e.type === "mouseMove") {
      lastMove = e;
      if (!flushQueued) {
        flushQueued = true;
        requestAnimationFrame(flush);
      }
      return;
    }
    if (lastMove) { pending.push(lastMove); lastMove = null; }
    pending.push(e);
    flush();
  }

  function flush() {
    flushQueued = false;
    if (lastMove) { pending.push(lastMove); lastMove = null; }
    if (sessionId && pending.length) api.desktopInput(sessionId, pending);
    pending = [];
  }

  function pos(e: MouseEvent) {
    const r = canvas.getBoundingClientRect();
    const x = Math.round(((e.clientX - r.left) / r.width) * canvas.width);
    const y = Math.round(((e.clientY - r.top) / r.height) * canvas.height);
    return { x: Math.max(0, Math.min(canvas.width - 1, x)), y: Math.max(0, Math.min(canvas.height - 1, y)) };
  }

  const onMove = (e: MouseEvent) => send({ type: "mouseMove", ...pos(e) });
  function onButton(e: MouseEvent, down: boolean) {
    e.preventDefault();
    if (down) wrap.focus();
    send({ type: "mouseMove", ...pos(e) });
    send({ type: "mouseButton", button: e.button, down });
  }
  function onWheel(e: WheelEvent) {
    e.preventDefault();
    const vertical = Math.abs(e.deltaY) >= Math.abs(e.deltaX);
    const delta = vertical ? e.deltaY : e.deltaX;
    if (!delta) return;
    const notches = Math.max(1, Math.round(Math.abs(delta) / 100));
    // RDP: positive = away from the user (scroll up / right).
    send({ type: "wheel", vertical, units: (delta < 0 ? 1 : -1) * 120 * notches * (vertical ? 1 : -1) });
  }

  function isAppShortcut(e: KeyboardEvent) {
    const k = e.key.toLowerCase();
    return e.ctrlKey && e.shiftKey && (k === "k" || k === "w");
  }

  // Remember the keysym sent on key-down so the release matches even if Shift changed in between.
  const downSyms = new Map<string, number>();

  function onKey(e: KeyboardEvent, down: boolean) {
    if (isAppShortcut(e)) return;
    e.preventDefault();
    e.stopPropagation();
    const sc = SCANCODES[e.code];
    let sym = down ? keysym(e) : (downSyms.get(e.code) ?? keysym(e));
    if (down && sym !== undefined) downSyms.set(e.code, sym);
    if (!down) downSyms.delete(e.code);
    if (protocol === "vnc") {
      if (sym !== undefined) send({ type: "key", keysym: sym, down });
    } else if (sc !== undefined) send({ type: "key", scancode: sc, down });
    else if (e.key.length === 1) send({ type: "unicode", ch: e.key, down });
  }

  function ctrlAltDel() {
    const keys: DesktopInput[] =
      protocol === "vnc"
        ? CTRL_ALT_DEL_KEYSYMS.map((k) => ({ type: "key", keysym: k, down: true }))
        : CTRL_ALT_DEL.map((sc) => ({ type: "key", scancode: sc, down: true }));
    for (const k of keys) send(k);
    for (const k of [...keys].reverse()) send({ ...k, down: false } as DesktopInput);
    wrap.focus();
  }

  function requestResize() {
    clearTimeout(resizeTimer);
    resizeTimer = setTimeout(() => {
      if (!sessionId || phase !== "open" || !visible) return;
      const r = wrap.getBoundingClientRect();
      const w = Math.round(r.width), h = Math.round(r.height);
      if (w < 200 || h < 200) return;
      if (Math.abs(w - size.width) > 8 || Math.abs(h - size.height) > 8) send({ type: "resize", width: w, height: h });
    }, 700);
  }

  onMount(() => {
    ctx = canvas.getContext("2d", { alpha: false })!;
    observer = new ResizeObserver(requestResize);
    observer.observe(wrap);
    requestAnimationFrame(connect);
  });

  $effect(() => {
    if (visible && phase === "open") requestAnimationFrame(() => wrap?.focus());
  });

  onDestroy(() => {
    observer?.disconnect();
    clearTimeout(resizeTimer);
    if (sessionId) api.desktopClose(sessionId);
  });
</script>

<div class="rd">
  <div class="bar">
    <span class="dot" class:on={phase === "open"} class:busy={phase === "connecting"} class:err={phase === "error"}></span>
    <span class="name">{host?.name}</span>
    <span class="proto">{protocol}</span>
    {#if size.width}<span class="mono muted">{size.width}×{size.height}</span>{/if}
    <span class="spacer"></span>
    <button class="btn small" disabled={phase !== "open"} onclick={ctrlAltDel}>Ctrl+Alt+Del</button>
    {#if phase === "open"}
      <button class="btn small" onclick={() => sessionId && api.desktopClose(sessionId)}>Disconnect</button>
    {:else if phase !== "connecting"}
      <button class="btn small" onclick={connect}><Icon name="refresh" size={13} /> Reconnect</button>
    {/if}
  </div>

  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
  <div
    class="screen"
    bind:this={wrap}
    tabindex="0"
    role="application"
    aria-label="Remote desktop of {host?.name}"
    onkeydown={(e) => onKey(e, true)}
    onkeyup={(e) => onKey(e, false)}
    onblur={() => { downSyms.clear(); send({ type: "releaseAll" }); }}
  >
    <canvas
      bind:this={canvas}
      class:hidden={phase !== "open"}
      onmousemove={onMove}
      onmousedown={(e) => onButton(e, true)}
      onmouseup={(e) => onButton(e, false)}
      onwheel={onWheel}
      oncontextmenu={(e) => e.preventDefault()}
    ></canvas>

    {#if phase !== "open"}
      <div class="overlay">
        {#if phase === "connecting"}
          <div class="spinner"></div>
          <p>Connecting to {host?.name}…</p>
        {:else if phase === "error"}
          <p class="err">{message}</p>
          <button class="btn primary" onclick={connect}>Try again</button>
        {:else}
          <p>Session ended. {message}</p>
          <button class="btn primary" onclick={connect}>Reconnect</button>
        {/if}
        {#each notices as n, i (i)}<small class="muted mono">{n}</small>{/each}
      </div>
    {/if}
  </div>
</div>

<style>
  .rd { height: 100%; display: grid; grid-template-rows: auto 1fr; background: var(--term-bg); min-height: 0; }
  .bar { display: flex; align-items: center; gap: 10px; padding: 5px 10px; background: var(--panel); border-bottom: 1px solid var(--line); font-size: 12px; }
  .name { font-weight: 600; }
  .spacer { flex: 1; }
  .btn.small { padding: 3px 9px; font-size: 12px; }
  .screen { position: relative; min-height: 0; overflow: hidden; display: grid; place-items: center; outline: none; background: #000; }
  .screen:focus-visible { box-shadow: inset 0 0 0 2px var(--accent); }
  canvas { max-width: 100%; max-height: 100%; object-fit: contain; image-rendering: auto; }
  canvas.hidden { visibility: hidden; position: absolute; }
  .overlay { position: absolute; inset: 0; display: grid; place-content: center; justify-items: center; gap: 12px; padding: 24px; text-align: center; background: var(--bg); color: var(--fg); }
  .overlay p { margin: 0; max-width: 60ch; }
  .overlay small { max-width: 70ch; overflow-wrap: anywhere; }
  .err { color: var(--bad); }
  .spinner { width: 26px; height: 26px; border-radius: 50%; border: 3px solid var(--line); border-top-color: var(--accent); animation: spin 0.9s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
</style>
