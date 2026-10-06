<!-- SPDX-FileCopyrightText: 2026 BunnyCloud.IT -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial -->
<script lang="ts">
  import Icon from "./Icon.svelte";
  import { api } from "../api";
  import { app, attempt, restartToUpdate } from "../state.svelte";

  const sessions = $derived(app.tabs.filter((t) => t.status === "open").length);
  const openTunnels = $derived(app.library.tunnels.filter((t) => app.tunnelState[t.id]?.state === "open"));
</script>

<footer>
  <span><span class="dot" class:on={sessions > 0}></span>{sessions} {sessions === 1 ? "session" : "sessions"}</span>
  <button onclick={() => (app.sidebar = "tunnels")}>
    <span class="dot" class:on={openTunnels.length > 0}></span>
    {openTunnels.length ? openTunnels.map((t) => t.name).join(", ") : "No tunnels open"}
  </button>
  <span class="spacer"></span>
  {#if app.update.status.state === "ready"}
    <button class="update" title="Install the downloaded update and restart" onclick={restartToUpdate}>
      Update {app.update.status.version} ready · Restart
    </button>
  {/if}
  <span>{app.library.hosts.length} hosts</span>
  <button title="Lock vault" onclick={async () => { await attempt(() => api.vaultLock()); app.vault.unlocked = false; }}>
    <Icon name="lock" size={12} /> Vault unlocked
  </button>
</footer>

<style>
  footer { display: flex; gap: 18px; align-items: center; padding: 0 12px; height: 24px; border-top: 1px solid var(--line); background: var(--panel); color: var(--muted); font-size: 11.5px; white-space: nowrap; overflow: hidden; }
  footer span, footer button { display: inline-flex; align-items: center; gap: 6px; }
  footer button:hover { color: var(--fg); }
  .spacer { flex: 1; }
  .update { color: var(--accent); font-weight: 600; }
</style>
