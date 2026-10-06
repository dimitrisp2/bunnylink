<!-- SPDX-FileCopyrightText: 2026 BunnyCloud.IT -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial -->
<script lang="ts">
  import Icon from "./Icon.svelte";
  import { api, errorText } from "../api";
  import { app, hostById, type Tab } from "../state.svelte";

  let { tab }: { tab: Tab } = $props();
  const host = $derived(hostById(tab.hostId));

  interface Run { command: string; output: string; exitCode: number | null; error?: boolean; ms: number }
  let command = $state("");
  let runs = $state<Run[]>([]);
  let busy = $state(false);

  const setStatus = (s: Tab["status"]) => {
    const t = app.tabs.find((x) => x.id === tab.id);
    if (t) t.status = s;
  };
  setStatus("open");

  async function run(e: SubmitEvent) {
    e.preventDefault();
    const cmd = command.trim();
    if (!cmd || busy) return;
    busy = true;
    setStatus("connecting");
    const start = performance.now();
    try {
      const r = await api.runCommand(tab.hostId, cmd);
      runs.unshift({ command: cmd, output: r.output, exitCode: r.exitCode, ms: performance.now() - start });
    } catch (err) {
      runs.unshift({ command: cmd, output: errorText(err), exitCode: null, error: true, ms: performance.now() - start });
    } finally {
      busy = false;
      setStatus("open");
    }
  }
</script>

<div class="wrap">
  <form onsubmit={run}>
    <span class="prompt mono">{host?.name ?? "host"} $</span>
    <!-- svelte-ignore a11y_autofocus -->
    <input id="cmd-{tab.id}" class="mono" placeholder="Command to run, e.g. uptime" bind:value={command} autofocus />
    <button class="btn primary" type="submit" disabled={busy || !command.trim()}><Icon name="play" size={13} /> {busy ? "Running…" : "Run"}</button>
  </form>
  <div class="runs">
    {#each runs as r, i (runs.length - i)}
      <section>
        <header>
          <code>{r.command}</code>
          <span class="status" class:bad={r.error || (r.exitCode ?? 0) !== 0}>
            {r.error ? "failed" : `exit ${r.exitCode ?? "?"}`} · {(r.ms / 1000).toFixed(1)} s
          </span>
        </header>
        <pre>{r.output || "(no output)"}</pre>
      </section>
    {:else}
      <p class="muted">Runs a single command over SSH and shows its output. Nothing is kept running afterwards.</p>
    {/each}
  </div>
</div>

<style>
  .wrap { height: 100%; display: grid; grid-template-rows: auto 1fr; background: var(--bg); min-height: 0; }
  form { display: flex; gap: 8px; align-items: center; padding: 12px 16px; border-bottom: 1px solid var(--line); background: var(--panel); }
  .prompt { color: var(--accent); white-space: nowrap; }
  .runs { overflow: auto; padding: 12px 16px; display: grid; gap: 12px; align-content: start; }
  section { border: 1px solid var(--line); border-radius: 8px; overflow: hidden; }
  section header { display: flex; justify-content: space-between; gap: 12px; padding: 6px 10px; background: var(--panel); border-bottom: 1px solid var(--line); }
  code { font-family: var(--mono); font-size: 12px; }
  .status { color: var(--good); font-size: 11.5px; white-space: nowrap; }
  .status.bad { color: var(--bad); }
  pre { margin: 0; padding: 10px; font: 12px/1.5 var(--mono); background: var(--term-bg); color: var(--term-fg); overflow: auto; max-height: 420px; user-select: text; }
  p { margin: 0; }
</style>
