<!-- SPDX-FileCopyrightText: 2026 BunnyCloud.IT -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial -->
<script lang="ts">
  import { api, errorText } from "../api";
  import type { Theme } from "../api";
  import { app, afterUnlock, saveSettings } from "../state.svelte";

  let password = $state("");
  let confirm = $state("");
  let error = $state("");
  let busy = $state(false);
  let portable = $state(false);
  const creating = $derived(!app.vault.initialized);

  const themes: { id: Theme; label: string }[] = [
    { id: "dark", label: "Dark" },
    { id: "light", label: "Light" },
    { id: "auto", label: "Auto" },
  ];
  const setTheme = (theme: Theme) => saveSettings({ ...app.settings, theme });
  // A new installation starts in Light; the choice is saved along with the new database.
  if (app.vault.firstRun) setTheme("light");

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    error = "";
    if (creating && password !== confirm) {
      error = "The passwords don't match.";
      return;
    }
    busy = true;
    try {
      if (creating) await api.vaultCreate(password, portable);
      else await api.vaultUnlock(password);
      password = confirm = "";
      await afterUnlock();
    } catch (err) {
      error = errorText(err);
    } finally {
      busy = false;
    }
  }
</script>

<div class="wrap">
  <form class="card" onsubmit={submit}>
    <img class="logo" src="/logo-small.png" alt="" />
    <div class="brand">Bunny<b>Link</b></div>
    {#if creating}
      <h1>Create your vault</h1>
      <p class="muted">
        Passwords and keys are encrypted with a master password. It is never stored, so it cannot be recovered if you forget it.
      </p>
    {:else}
      <h1>Unlock</h1>
      <p class="muted">Enter your master password to open your connections.</p>
    {/if}
    <label class="field">
      <span>Master password</span>
      <!-- svelte-ignore a11y_autofocus -->
      <input id="vault-password" type="password" bind:value={password} autofocus autocomplete="current-password" />
    </label>
    {#if creating}
      <label class="field">
        <span>Confirm master password</span>
        <input id="vault-confirm" type="password" bind:value={confirm} autocomplete="new-password" />
      </label>
      {#if app.vault.firstRun}
        <div class="field">
          <span>Theme</span>
          <div class="seg" role="radiogroup" aria-label="Theme">
            {#each themes as t (t.id)}
              <button type="button" class:act={app.settings.theme === t.id} onclick={() => setTheme(t.id)}>{t.label}</button>
            {/each}
          </div>
          <small class="muted">Auto follows your operating system's light or dark mode.</small>
        </div>
        <label class="portable">
          <input type="checkbox" bind:checked={portable} />
          <span>
            <b>Portable mode</b>
            <span class="muted">Keep the database next to the app, so you can carry both on a USB drive.</span>
          </span>
        </label>
      {/if}
    {/if}
    {#if error}<p class="error">{error}</p>{/if}
    <button class="btn primary" type="submit" disabled={busy || !password}>
      {busy ? "Working…" : creating ? "Create vault" : "Unlock"}
    </button>
  </form>
</div>

<style>
  .wrap { height: 100%; display: grid; place-items: center; background: var(--bg); }
  .card {
    width: min(380px, calc(100% - 32px)); display: grid; gap: 14px; padding: 28px;
    background: var(--panel); border: 1px solid var(--line); border-radius: 10px; box-shadow: var(--shadow);
  }
  .logo { width: 96px; height: 96px; border-radius: 16px; justify-self: center; }
  .brand { font-weight: 700; font-size: 15px; }
  .brand b { color: var(--accent); }
  h1 { margin: 0; font-size: 20px; }
  p { margin: 0; }
  .error { color: var(--bad); }
  .seg { display: inline-flex; gap: 2px; padding: 2px; border: 1px solid var(--line); border-radius: 7px; justify-self: start; }
  .seg button { padding: 5px 16px; border-radius: 5px; color: var(--muted); }
  .seg button.act { background: var(--accent); color: var(--accent-fg); font-weight: 600; }
  .portable { display: flex; gap: 10px; align-items: flex-start; cursor: pointer; }
  .portable input { margin-top: 3px; accent-color: var(--accent); }
  .portable > span { display: grid; gap: 2px; }
  .btn { justify-content: center; padding: 8px 12px; }
</style>
