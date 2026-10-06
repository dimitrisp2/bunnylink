<!-- SPDX-FileCopyrightText: 2026 BunnyCloud.IT -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial -->
<script lang="ts">
  import { app, type ContextMenu } from "../state.svelte";

  let { menu }: { menu: ContextMenu } = $props();
  let el = $state<HTMLDivElement>();
  let pos = $state({ x: 0, y: 0 });

  // Keep the menu inside the window.
  $effect(() => {
    if (!el) return;
    const r = el.getBoundingClientRect();
    pos = {
      x: Math.max(4, Math.min(menu.x, window.innerWidth - r.width - 4)),
      y: Math.max(4, Math.min(menu.y, window.innerHeight - r.height - 4)),
    };
  });

  const close = () => (app.menu = null);
  const pick = (run: () => void) => () => {
    close();
    run();
  };
</script>

<svelte:window
  onmousedown={(e) => !el?.contains(e.target as Node) && close()}
  onkeydown={(e) => e.key === "Escape" && close()}
  onblur={close}
  onresize={close}
/>

<div class="menu" bind:this={el} style:left="{pos.x}px" style:top="{pos.y}px" role="menu" tabindex="-1" oncontextmenu={(e) => e.preventDefault()}>
  {#each menu.items as item, i (i)}
    {#if item === "separator"}
      {#if i > 0 && i < menu.items.length - 1 && menu.items[i - 1] !== "separator"}<hr />{/if}
    {:else}
      <button role="menuitem" class:danger={item.danger} disabled={item.disabled} onclick={pick(item.run)}>{item.label}</button>
    {/if}
  {/each}
</div>

<style>
  .menu {
    position: fixed; z-index: 50; min-width: 190px; padding: 4px; display: grid;
    background: var(--panel); border: 1px solid var(--line-strong); border-radius: 8px; box-shadow: var(--shadow);
  }
  .menu button { text-align: left; padding: 6px 10px; border-radius: 5px; white-space: nowrap; }
  .menu button:hover:not(:disabled) { background: var(--sel); }
  .menu button:disabled { color: var(--faint); }
  .menu .danger { color: var(--bad); }
  hr { border: 0; border-top: 1px solid var(--line); margin: 4px 0; width: 100%; }
</style>
