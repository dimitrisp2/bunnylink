<!-- SPDX-FileCopyrightText: 2026 BunnyCloud.IT -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial -->
<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "./Icon.svelte";
  import { api } from "../api";
  import {
    app, hostActions, openSettings, toggleTunnel, tunnelSummary, saveSettings, unsplit, attempt,
  } from "../state.svelte";

  interface Item { id: string; icon: string; title: string; host?: string; detail: string; hay: string; name: string; weight: number; soon?: boolean; run: () => void }

  let query = $state("");
  let index = $state(0);
  let input: HTMLInputElement;
  let list: HTMLDivElement;

  // Return focus (e.g. to a terminal) when the palette closes.
  const previous = document.activeElement as HTMLElement | null;
  const close = () => {
    app.paletteOpen = false;
    if (!app.modal) requestAnimationFrame(() => previous?.focus());
  };
  onMount(() => requestAnimationFrame(() => input?.focus()));

  const items = $derived.by(() => {
    const out: Item[] = [];
    const numbers = query.match(/\b\d{1,5}\b/g)?.map(Number) ?? [];
    for (const h of app.library.hosts) {
      const recency = h.lastUsed ? 1 : 0;
      for (const a of hostActions(h)) {
        const run =
          a.key === "tunnel" && numbers.length
            ? () => (app.modal = { kind: "tunnel", tunnel: null, hostId: h.id, targetPort: numbers[0] })
            : a.run;
        out.push({
          id: `${h.id}:${a.key}`,
          icon: a.icon,
          title: a.label,
          host: h.name,
          detail: a.key === "tunnel" && numbers.length ? `forward port ${numbers[0]}` : a.detail,
          hay: `${h.name} ${h.address} ${h.tags.join(" ")} ${a.label} ${a.protocol ?? ""} ${a.key === "tunnel" ? "forward tunnel socks" : ""} ${a.key === "terminal" ? "ssh shell console" : ""}`.toLowerCase(),
          name: h.name.toLowerCase(),
          weight: (a.ready ? 2 : 0) + recency,
          soon: !a.ready,
          run,
        });
      }
    }
    for (const t of app.library.tunnels) {
      const st = app.tunnelState[t.id]?.state ?? "closed";
      out.push({
        id: `tunnel:${t.id}`, icon: "tunnel", title: st === "closed" ? "Open tunnel" : "Close tunnel", host: t.name,
        detail: tunnelSummary(t), hay: `${t.name} tunnel ${tunnelSummary(t)}`.toLowerCase(), name: t.name.toLowerCase(), weight: 1,
        run: () => toggleTunnel(t),
      });
    }
    const cmd = (id: string, icon: string, title: string, detail: string, run: () => void, extra = "") =>
      out.push({ id, icon, title, detail, hay: `${title} ${extra}`.toLowerCase(), name: title.toLowerCase(), weight: 0, run });
    cmd("new-host", "plus", "New host", "Add a server or device", () => (app.modal = { kind: "host", host: null }), "add create");
    cmd("new-folder", "folder", "New folder", "Group hosts and share settings", () => (app.modal = { kind: "folder", folder: null }), "add create");
    cmd("new-tunnel", "tunnel", "New tunnel", "Local, remote or SOCKS", () => (app.modal = { kind: "tunnel", tunnel: null }), "add create forward port");
    cmd("new-cred", "key", "New credential", "Password or private key", () => (app.modal = { kind: "credential", credential: null }), "add create password key");
    cmd("import", "download", "Import connections", "From mRemoteNG, PuTTY or ~/.ssh/config", () => (app.modal = { kind: "import" }), "mremoteng putty openssh ssh config");
    cmd("settings", "settings", "Settings", "Appearance, security, about", openSettings, "preferences options version licenses");
    cmd("credentials", "key", "Credentials", "Manage saved passwords and keys", () => (app.sidebar = "credentials"), "passwords keys");
    cmd("theme-dark", "settings", "Theme: Dark", "Appearance", () => saveSettings({ ...app.settings, theme: "dark" }));
    cmd("theme-light", "settings", "Theme: Light", "Appearance", () => saveSettings({ ...app.settings, theme: "light" }));
    cmd("theme-auto", "settings", "Theme: Auto", "Follow the system", () => saveSettings({ ...app.settings, theme: "auto" }));
    if (app.panes.right) cmd("unsplit", "split", "Close split view", "Back to one pane", unsplit);
    cmd("lock", "lock", "Lock vault", "Requires the master password to continue", async () => {
      await attempt(() => api.vaultLock());
      app.vault.unlocked = false;
    });
    return out;
  });

  function score(it: Item, tokens: string[]): number {
    let s = it.weight;
    for (const tok of tokens) {
      if (/^\d+$/.test(tok)) continue;
      if (it.name.startsWith(tok)) s += 30;
      else if (it.name.includes(tok)) s += 20;
      else if (it.hay.includes(tok)) s += 10;
      else if (subsequence(tok, it.hay)) s += 2;
      else return -1;
    }
    return s;
  }
  const subsequence = (needle: string, hay: string) => {
    let i = 0;
    for (const ch of hay) if (ch === needle[i]) i++;
    return i === needle.length;
  };

  const results = $derived.by(() => {
    const tokens = query.toLowerCase().split(/\s+/).filter(Boolean);
    if (!tokens.length) {
      const recent = app.library.hosts
        .filter((h) => h.lastUsed)
        .sort((a, b) => (b.lastUsed ?? 0) - (a.lastUsed ?? 0))
        .slice(0, 5)
        .map((h) => items.find((i) => i.id.startsWith(`${h.id}:`) && !i.soon))
        .filter((x): x is Item => !!x);
      return [...recent, ...items.filter((i) => !i.host)].slice(0, 12);
    }
    return items
      .map((it) => ({ it, s: score(it, tokens) }))
      .filter((x) => x.s >= 0)
      .sort((a, b) => b.s - a.s)
      .slice(0, 30)
      .map((x) => x.it);
  });

  $effect(() => {
    void query;
    index = 0;
  });
  $effect(() => {
    list?.querySelector(`[data-i="${index}"]`)?.scrollIntoView({ block: "nearest" });
  });

  function choose(it: Item | undefined) {
    if (!it) return;
    app.paletteOpen = false;
    it.run();
  }
  function onKey(e: KeyboardEvent) {
    if (e.key === "ArrowDown") { index = Math.min(index + 1, results.length - 1); e.preventDefault(); }
    else if (e.key === "ArrowUp") { index = Math.max(index - 1, 0); e.preventDefault(); }
    else if (e.key === "Enter") { choose(results[index]); e.preventDefault(); }
    else if (e.key === "Escape") { close(); e.preventDefault(); }
  }
</script>

<div class="backdrop" role="presentation" onmousedown={(e) => e.target === e.currentTarget && close()}>
  <div class="palette" role="dialog" aria-label="Command palette">
    <div class="q">
      <Icon name="search" />
      <input id="palette-input" bind:this={input} bind:value={query} onkeydown={onKey} placeholder="Host, then action. e.g. “db01 forward 5432”" />
      <kbd>Esc</kbd>
    </div>
    <div class="list" bind:this={list}>
      {#each results as it, i (it.id)}
        <button class="it" class:act={i === index} class:soon={it.soon} data-i={i} onmouseenter={() => (index = i)} onclick={() => choose(it)}>
          <Icon name={it.icon} size={15} />
          <span class="t">{it.title}{#if it.host}<b>{it.host}</b>{/if}</span>
          <small class="mono">{it.soon ? "coming soon" : it.detail}</small>
        </button>
      {:else}
        <p class="none muted">Nothing matches “{query}”.</p>
      {/each}
    </div>
  </div>
</div>

<style>
  .backdrop { position: fixed; inset: 0; background: rgb(0 0 0 / .25); z-index: 50; display: flex; justify-content: center; padding-top: 12vh; }
  .palette { width: min(600px, calc(100% - 32px)); max-height: 60vh; display: grid; grid-template-rows: auto 1fr; background: var(--panel); border: 1px solid var(--line-strong); border-radius: 10px; box-shadow: var(--shadow); overflow: hidden; align-self: flex-start; }
  .q { display: flex; align-items: center; gap: 10px; padding: 10px 14px; border-bottom: 1px solid var(--line); color: var(--muted); }
  .q input { border: 0; background: none; padding: 4px 0; font-size: 15px; color: var(--fg); }
  .list { overflow: auto; padding: 4px 0; }
  .it { display: flex; align-items: center; gap: 10px; width: 100%; padding: 7px 14px; text-align: left; color: var(--muted); }
  .it.act { background: var(--sel); color: var(--fg); }
  .it.soon { opacity: .55; }
  .t { color: var(--fg); white-space: nowrap; }
  .t b { color: var(--accent); font-weight: 600; margin-left: .35em; }
  small { margin-left: auto; font-size: 11px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; min-width: 0; }
  .none { padding: 12px 14px; margin: 0; }
</style>
