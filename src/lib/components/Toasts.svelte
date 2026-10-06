<!-- SPDX-FileCopyrightText: 2026 BunnyCloud.IT -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial -->
<script lang="ts">
  import { app } from "../state.svelte";
</script>

<div class="toasts" aria-live="polite">
  {#each app.toasts as t (t.id)}
    <div class="toast" class:error={t.kind === "error"}>{t.text}</div>
  {/each}
</div>

<style>
  .toasts { position: fixed; right: 16px; bottom: 36px; display: grid; gap: 8px; z-index: 60; max-width: min(420px, calc(100% - 32px)); }
  .toast { overflow-wrap: anywhere; padding: 10px 14px; border-radius: 8px; background: var(--panel); border: 1px solid var(--line-strong); box-shadow: var(--shadow); user-select: text; }
  .toast.error { border-color: var(--bad); }
</style>
