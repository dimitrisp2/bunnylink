<!-- SPDX-FileCopyrightText: 2026 BunnyCloud.IT -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial -->
<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";
  import { app } from "../state.svelte";

  // Forms are not dismissed by a stray click outside; only by Esc, Cancel or the close button.
  let {
    title,
    width = 520,
    dismissable = false,
    children,
    footer,
    onclose,
  }: { title: string; width?: number; dismissable?: boolean; children: Snippet; footer?: Snippet; onclose?: () => void } = $props();
  const close = () => (onclose ? onclose() : (app.modal = null));
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && close()} />

<div class="backdrop" role="presentation" onmousedown={(e) => dismissable && e.target === e.currentTarget && close()}>
  <div class="modal" role="dialog" aria-modal="true" aria-label={title} style:width="min({width}px, calc(100% - 32px))">
    <header>
      <h2>{title}</h2>
      <button class="icon-btn" onclick={close} aria-label="Close"><Icon name="x" /></button>
    </header>
    <div class="body">{@render children()}</div>
    {#if footer}<footer>{@render footer()}</footer>{/if}
  </div>
</div>

<style>
  /* Anchored near the top so the dialog doesn't jump as its content grows. */
  .backdrop { position: fixed; inset: 0; background: rgb(0 0 0 / .35); z-index: 40; display: grid; justify-items: center; align-items: start; padding-top: min(10vh, 72px); }
  .modal { max-height: calc(100vh - min(10vh, 72px) - 24px); display: grid; grid-template-rows: auto 1fr auto; background: var(--panel); border: 1px solid var(--line-strong); border-radius: 10px; box-shadow: var(--shadow); }
  header { display: flex; align-items: center; justify-content: space-between; padding: 14px 18px 8px; }
  h2 { margin: 0; font-size: 16px; }
  .body { padding: 8px 18px 16px; overflow: auto; display: grid; gap: 12px; align-content: start; }
  footer { display: flex; gap: 8px; justify-content: flex-end; padding: 12px 18px; border-top: 1px solid var(--line); }
</style>
