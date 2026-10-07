<!-- SPDX-FileCopyrightText: 2026 BunnyCloud.IT -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial -->
<script lang="ts">
  import { openUrl } from "@tauri-apps/plugin-opener";
  import Icon from "./Icon.svelte";
  import { api } from "../api";
  import type { Theme } from "../api";
  import { app, attempt, restartToUpdate, saveSettings } from "../state.svelte";

  interface License { name: string; version: string; ecosystem: "rust" | "npm"; license: string; repository: string | null; texts: { file: string; text: number }[] }

  type Section = "appearance" | "security" | "licenses" | "updates" | "about";
  const sections: { id: Section; label: string }[] = [
    { id: "appearance", label: "Appearance" },
    { id: "security", label: "Security" },
    { id: "licenses", label: "Open source" },
    { id: "updates", label: "Updates" },
    { id: "about", label: "About" },
  ];
  let section = $state<Section>("appearance");
  const update = $derived(app.update.status);

  const themes: { id: Theme; label: string }[] = [
    { id: "dark", label: "Dark" },
    { id: "light", label: "Light" },
    { id: "auto", label: "Auto" },
  ];
  const set = <K extends keyof typeof app.settings>(k: K, v: (typeof app.settings)[K]) => saveSettings({ ...app.settings, [k]: v });

  // Open source: loaded the first time the section is opened.
  let licenses = $state<License[] | null>(null);
  let licenseTexts = $state.raw<string[]>([]);
  let licenseError = $state("");
  let openLicense = $state<string | null>(null);
  $effect(() => {
    if (section !== "licenses" || licenses) return;
    fetch("/licenses.json")
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(`HTTP ${r.status}`))))
      .then((d: { texts: string[]; packages: License[] }) => {
        licenseTexts = d.texts;
        licenses = d.packages;
      })
      .catch(() => (licenseError = "The license list is not available in this build."));
  });
  const keyOf = (l: License) => `${l.ecosystem}:${l.name}@${l.version}`;
</script>

<div class="settings">
  <nav>
    {#each sections as s (s.id)}
      <button class:act={section === s.id} onclick={() => (section = s.id)}>{s.label}</button>
    {/each}
  </nav>
  <div class="content">
    {#if section === "appearance"}
      <h1>Appearance</h1>
      <div class="field">
        <span class="lbl">Theme</span>
        <div class="seg" role="radiogroup" aria-label="Theme">
          {#each themes as t (t.id)}
            <button class:act={app.settings.theme === t.id} onclick={() => set("theme", t.id)}>{t.label}</button>
          {/each}
        </div>
        <small class="muted">Auto follows your operating system's light or dark mode.</small>
      </div>
      <label class="field narrow">
        <span>Terminal font size</span>
        <input id="set-font" type="number" min="9" max="28" value={app.settings.terminalFontSize}
          onchange={(e) => set("terminalFontSize", Math.min(28, Math.max(9, +e.currentTarget.value || 14)))} />
      </label>
    {:else if section === "security"}
      <h1>Security</h1>
      <label class="field narrow">
        <span>Lock the vault after idle minutes</span>
        <input id="set-lock" type="number" min="0" max="1440" value={app.settings.autoLockMinutes}
          onchange={(e) => set("autoLockMinutes", Math.max(0, +e.currentTarget.value || 0))} />
        <small class="muted">0 never locks automatically. Open sessions stay connected when the vault locks.</small>
      </label>
      <label class="field narrow">
        <span>Clear copied passwords after seconds</span>
        <input id="set-clip" type="number" min="0" max="600" value={app.settings.clipboardClearSeconds}
          onchange={(e) => set("clipboardClearSeconds", Math.max(0, +e.currentTarget.value || 0))} />
      </label>
      <div>
        <button class="btn" onclick={async () => { await attempt(() => api.vaultLock()); app.vault.unlocked = false; }}>
          <Icon name="lock" size={14} /> Lock now
        </button>
      </div>
    {:else if section === "about"}
      <h1 class="sr-only">About</h1>
      <!-- The banner is the logo's own navy, so the artwork blends in; it stays dark in both themes. -->
      <div class="banner">
        <img src="/logo.jpg" alt="BunnyLink logo: a white rabbit leaping across a glowing chain in front of a blue cloud" />
        <div class="banner-text">
          <div class="banner-name">Bunny<b>Link</b> <span class="banner-version">{app.update.current || "…"}</span></div>
          <div class="banner-tagline">Remote connections: SSH, RDP, VNC, files and tunnels</div>
          <div class="banner-copy">© 2026 BunnyCloud.IT</div>
        </div>
      </div>
      <h2>About BunnyLink</h2>
      <p>
        BunnyLink is a remote connection manager for Windows. SSH, Telnet and serial terminals, RDP and VNC desktops,
        SFTP, FTP/FTPS and SMB file browsing, web consoles and tunnels live in one app, and every protocol runs inside
        it, without launching external clients. Your hosts, passwords and keys stay in one file on your own disk, with
        passwords and keys encrypted by your master password.
      </p>
      <h2>License</h2>
      <p class="lic-text">
        BunnyLink is free under the GNU GPL v3 or later. Companies that want to ship it, or products built on it,
        without the GPL's obligations can get a commercial license.
      </p>
      <div class="lic-links">
        <button class="link" onclick={() => openUrl("https://spdx.org/licenses/GPL-3.0-or-later.html")}>Read the GPL</button>
        <span class="muted">·</span>
        <button class="link" onclick={() => openUrl("https://bunnycloud.gr/contact")}>Ask about a commercial license</button>
      </div>
    {:else if section === "updates"}
      <h1>Updates</h1>
      <dl class="meta"><dt>Version</dt><dd class="mono">{app.update.current || "…"}</dd></dl>
      {#if update.state === "unavailable"}
        <p class="muted">Updates are not available in this build. Official releases update themselves.</p>
      {:else}
        <div class="update">
          <p>
            {#if update.state === "checking"}Checking for updates…
            {:else if update.state === "downloading"}Downloading {update.version}… {Math.round((update.done / Math.max(1, update.total)) * 100)}%
            {:else if update.state === "ready"}Version {update.version} is downloaded and verified.
            {:else if update.state === "upToDate"}BunnyLink is up to date. Last checked {new Date(update.checkedAt * 1000).toLocaleString()}.
            {:else if update.state === "error"}<span class="err">{update.message}</span>
            {:else}Not checked yet.{/if}
          </p>
          <div class="update-btns">
            {#if update.state === "ready"}
              <button class="btn primary" onclick={restartToUpdate}>Restart to update</button>
            {:else}
              <button class="btn" disabled={update.state === "checking" || update.state === "downloading"} onclick={() => attempt(() => api.updateCheck())}>Check for updates</button>
            {/if}
          </div>
          <label class="check">
            <input id="set-auto-update" type="checkbox" checked={app.settings.autoUpdate} onchange={(e) => set("autoUpdate", e.currentTarget.checked)} />
            <span>Check for updates automatically <small class="muted">(at startup and every 12 hours)</small></span>
          </label>
        </div>
        {#if update.state === "ready" && update.changelog}
          <h2>Changelog</h2>
          <pre class="changelog">{update.changelog}</pre>
        {/if}
      {/if}
      {#if app.update.currentChangelog}
        <h2>What's new in {app.update.current}</h2>
        <pre class="changelog">{app.update.currentChangelog}</pre>
      {/if}
    {:else}
      <h1>Open source licenses</h1>
      <p class="muted">BunnyLink is built with these open source components.</p>
      {#if licenseError}
        <p class="muted">{licenseError}</p>
      {:else if !licenses}
        <p class="muted">Loading…</p>
      {:else}
        <p class="muted count">{licenses.length} components</p>
        <div class="licenses">
          {#each licenses as l (keyOf(l))}
            <div class="lic">
              <button class="lic-head" onclick={() => (openLicense = openLicense === keyOf(l) ? null : keyOf(l))} aria-expanded={openLicense === keyOf(l)}>
                <span class="twisty" class:open={openLicense === keyOf(l)}><Icon name="chevron" size={11} /></span>
                <b>{l.name}</b>
                <span class="mono muted">{l.version}</span>
                <span class="tag">{l.ecosystem === "rust" ? "Rust" : "npm"}</span>
                <span class="spacer"></span>
                <span class="mono license">{l.license}</span>
              </button>
              {#if openLicense === keyOf(l)}
                <div class="lic-body">
                  {#if l.repository}<p class="mono muted">{l.repository}</p>{/if}
                  {#each l.texts as t (t.file)}
                    <h3 class="mono">{t.file}</h3>
                    <pre>{licenseTexts[t.text]}</pre>
                  {:else}
                    <p class="muted">No license file is included with this package. License: {l.license}.</p>
                  {/each}
                </div>
              {/if}
            </div>
          {/each}
        </div>
      {/if}
    {/if}
  </div>
</div>

<style>
  .settings { display: grid; grid-template-columns: 180px 1fr; height: 100%; min-height: 0; }
  nav { display: grid; gap: 2px; align-content: start; padding: 18px 10px; border-right: 1px solid var(--line); }
  nav button { text-align: left; padding: 6px 10px; border-radius: 6px; color: var(--muted); }
  nav button.act { background: var(--sel); color: var(--fg); }
  .content { display: grid; gap: 16px; align-content: start; padding: 22px 26px; overflow: auto; min-width: 0; }
  h1 { margin: 0; font-size: 20px; }
  h2 { margin: 8px 0 0; font-size: 14px; }
  .field { display: grid; gap: 5px; max-width: 520px; }
  .field > span, .lbl { font-size: 11.5px; color: var(--muted); }
  .narrow input { max-width: 120px; }
  .seg { display: inline-flex; gap: 2px; padding: 2px; border: 1px solid var(--line); border-radius: 7px; justify-self: start; }
  .seg button { padding: 5px 16px; border-radius: 5px; color: var(--muted); }
  .seg button.act { background: var(--accent); color: var(--accent-fg); font-weight: 600; }
  p { margin: 0; }
  .sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); white-space: nowrap; }
  /* Fixed colours on purpose: the banner continues the logo's artwork, whatever the theme. */
  .banner {
    display: flex; align-items: center; gap: 8px; min-height: 168px; padding-right: 28px; border-radius: 12px; overflow: hidden;
    background: #121826; color: #e4e8f2;
  }
  .banner img { width: 168px; height: 168px; flex: none; display: block; }
  .banner-text { display: grid; gap: 4px; min-width: 0; }
  .banner-name { font-size: 26px; font-weight: 700; letter-spacing: -0.01em; }
  .banner-name b { color: #78a2e4; }
  .banner-version { font: 500 13px var(--mono); color: #98a3bb; margin-left: 6px; vertical-align: middle; }
  .banner-tagline { color: #c3cbe0; }
  .banner-copy { color: #98a3bb; font-size: 12px; }
  .lic-links { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
  .meta { display: grid; grid-template-columns: max-content 1fr; gap: 6px 18px; margin: 0; }
  .meta dt { color: var(--muted); }
  .meta dd { margin: 0; }
  .update { display: grid; gap: 10px; max-width: 640px; }
  .update-btns { display: flex; gap: 8px; }
  .err { color: var(--bad); }
  .changelog { margin: 0; padding: 10px; max-height: 260px; overflow: auto; white-space: pre-wrap; font: 12px var(--mono); background: var(--bg); border-radius: 6px; user-select: text; }
  .check { display: flex; gap: 8px; align-items: center; cursor: pointer; }
  .check input { accent-color: var(--accent); }
  .link { color: var(--accent); padding: 0; text-align: left; text-decoration: underline; text-underline-offset: 2px; justify-self: start; }
  .licenses { display: grid; border: 1px solid var(--line); border-radius: 8px; }
  .lic { border-bottom: 1px solid var(--line); min-width: 0; }
  .lic:last-child { border-bottom: 0; }
  .lic-head { display: flex; align-items: center; gap: 8px; width: 100%; padding: 6px 10px; text-align: left; }
  .lic-head:hover { background: var(--panel-2); }
  .twisty { display: grid; color: var(--faint); transition: transform .12s; }
  .twisty.open { transform: rotate(90deg); }
  .tag { font-size: 10.5px; padding: 1px 6px; border-radius: 4px; background: var(--panel-2); color: var(--muted); }
  .spacer { flex: 1; }
  .license { font-size: 11.5px; color: var(--accent); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 45%; }
  .lic-body { display: grid; gap: 6px; padding: 4px 12px 12px 32px; min-width: 0; }
  h3 { margin: 6px 0 0; font-size: 11.5px; color: var(--muted); font-weight: 600; }
  pre { margin: 0; padding: 10px; max-height: 320px; overflow: auto; white-space: pre-wrap; font-size: 11.5px; background: var(--bg); border-radius: 6px; user-select: text; }
</style>
