<!-- SPDX-FileCopyrightText: 2026 BunnyCloud.IT -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial -->
<script lang="ts">
  import Modal from "./Modal.svelte";
  import { api, errorText } from "../api";
  import type { Tunnel, TunnelKind } from "../api";
  import { app, attempt, hostById, reload, toggleTunnel, tunnelSummary } from "../state.svelte";

  let { tunnel, hostId, targetPort }: { tunnel: Tunnel | null; hostId?: string; targetPort?: number } = $props();

  const sshHosts = $derived(app.library.hosts.filter((h) => h.endpoints.some((e) => e.protocol === "ssh")));
  // svelte-ignore state_referenced_locally
  let draft = $state<Tunnel>(
    tunnel
      ? ($state.snapshot(tunnel) as Tunnel)
      : {
          id: "",
          name: "",
          kind: "local",
          hostId: hostId ?? "",
          bindPort: targetPort ? (targetPort < 1024 ? targetPort + 10000 : targetPort) : 8080,
          targetHost: "localhost",
          targetPort: targetPort ?? 80,
        },
  );
  // svelte-ignore state_referenced_locally
  if (!draft.hostId && sshHosts[0]) draft.hostId = sshHosts[0].id;
  let error = $state("");

  const kinds: { id: TunnelKind; label: string; help: string }[] = [
    { id: "local", label: "Local", help: "Reach a service behind the SSH host from this computer." },
    { id: "remote", label: "Remote", help: "Let the SSH host reach a service on this computer." },
    { id: "socks", label: "SOCKS", help: "Proxy any traffic through the SSH host. Point your browser at it." },
  ];

  async function save(start: boolean) {
    error = "";
    const t = $state.snapshot(draft) as Tunnel;
    if (!t.name.trim()) t.name = `${hostById(t.hostId)?.name ?? "tunnel"} ${t.kind === "socks" ? "SOCKS" : t.targetPort}`;
    if (t.kind === "socks") { t.targetHost = null; t.targetPort = null; }
    try {
      const saved = await api.saveTunnel(t);
      await reload();
      app.modal = null;
      app.sidebar = "tunnels";
      if (start) await toggleTunnel(saved);
    } catch (e) {
      error = errorText(e);
    }
  }
  function remove() {
    const t = tunnel!;
    app.modal = {
      kind: "confirm", title: `Delete tunnel ${t.name}?`, body: "The tunnel is closed if it is open.", confirm: "Delete tunnel",
      run: async () => { await attempt(() => api.deleteTunnel(t.id)); await reload(); },
    };
  }
</script>

<Modal title={tunnel ? `Edit ${tunnel.name}` : "New tunnel"}>
  <div class="kinds" role="radiogroup" aria-label="Tunnel type">
    {#each kinds as k (k.id)}
      <button type="button" class:act={draft.kind === k.id} onclick={() => (draft.kind = k.id)}>{k.label}</button>
    {/each}
  </div>
  <p class="muted hint">{kinds.find((k) => k.id === draft.kind)?.help}</p>

  <div class="grid">
    <label class="field"><span>Name</span><input id="tun-name" bind:value={draft.name} placeholder="Optional" /></label>
    <label class="field">
      <span>Through SSH host</span>
      <select id="tun-host" bind:value={draft.hostId}>
        {#each sshHosts as h (h.id)}<option value={h.id}>{h.name}</option>{/each}
      </select>
    </label>
    <label class="field">
      <span>{draft.kind === "remote" ? "Port on the SSH host" : "Local port"}</span>
      <input id="tun-bind" class="mono" type="number" min="1" max="65535" bind:value={draft.bindPort} />
    </label>
    {#if draft.kind !== "socks"}
      <span></span>
      <label class="field">
        <span>{draft.kind === "remote" ? "Forward to host (from this computer)" : "Destination host (from the SSH host)"}</span>
        <input id="tun-target" class="mono" bind:value={draft.targetHost} placeholder="localhost" />
      </label>
      <label class="field"><span>Destination port</span><input id="tun-tport" class="mono" type="number" min="1" max="65535" bind:value={draft.targetPort} /></label>
    {/if}
  </div>
  {#if draft.hostId}<p class="summary mono">{tunnelSummary(draft)}</p>{/if}
  {#if error}<p class="error">{error}</p>{/if}

  {#snippet footer()}
    {#if tunnel}<button class="btn danger" style="margin-right:auto" onclick={remove}>Delete</button>{/if}
    <button class="btn" onclick={() => save(false)} disabled={!draft.hostId}>Save</button>
    <button class="btn primary" onclick={() => save(true)} disabled={!draft.hostId}>Save and open</button>
  {/snippet}
</Modal>

<style>
  .kinds { display: inline-flex; gap: 2px; padding: 2px; border: 1px solid var(--line); border-radius: 7px; justify-self: start; }
  .kinds button { padding: 4px 14px; border-radius: 5px; color: var(--muted); }
  .kinds button.act { background: var(--accent); color: var(--accent-fg); }
  .hint { margin: 0; font-size: 12px; }
  .grid { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
  .summary { margin: 0; padding: 8px 10px; border-radius: 6px; background: var(--panel-2); color: var(--accent); font-size: 12px; }
  .error { margin: 0; color: var(--bad); }
</style>
