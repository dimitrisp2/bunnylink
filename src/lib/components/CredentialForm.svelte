<!-- SPDX-FileCopyrightText: 2026 BunnyCloud.IT -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial -->
<script lang="ts">
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { api, errorText } from "../api";
  import type { Credential, CredentialKind } from "../api";
  import { reload } from "../state.svelte";
  import { readSecret, takeSecret } from "../secret";

  let {
    credential = null,
    suggestedName = "",
    onSaved,
    onCancel,
    compact = false,
  }: {
    credential?: Credential | null;
    suggestedName?: string;
    onSaved: (c: Credential) => void;
    onCancel?: () => void;
    compact?: boolean;
  } = $props();

  // svelte-ignore state_referenced_locally
  let name = $state(credential?.name ?? suggestedName);
  // svelte-ignore state_referenced_locally
  let username = $state(credential?.username ?? "");
  // svelte-ignore state_referenced_locally
  let kind = $state<CredentialKind>(credential?.kind ?? "password");
  // Not bound to state: read once when saved (see ../secret). One secret field shows at a time.
  let secretInput = $state<HTMLInputElement | HTMLTextAreaElement>();
  let passphraseInput = $state<HTMLInputElement>();
  /** A key file picked with "Load from file…": only its path is kept; the backend reads it. */
  let keyFile = $state<string | null>(null);
  // svelte-ignore state_referenced_locally
  let askPassphrase = $state(credential?.askPassphrase ?? false);
  let error = $state("");
  let busy = $state(false);

  async function save() {
    error = "";
    busy = true;
    const file = kind === "key" ? keyFile : null;
    let secret = kind !== "agent" && !file ? takeSecret(secretInput) : "";
    let passphrase = !askPassphrase ? takeSecret(passphraseInput) : "";
    try {
      const saved = await api.saveCredential(
        { id: credential?.id ?? "", name: name.trim() || "Untitled", username: username.trim() || null, kind, askPassphrase: kind === "key" && askPassphrase },
        secret || undefined,
        passphrase || undefined,
        file ?? undefined,
      );
      await reload();
      onSaved(saved);
    } catch (e) {
      error = errorText(e);
      // Put them back so a pasted key isn't lost to a typo elsewhere in the form.
      if (secretInput && !readSecret(secretInput)) secretInput.value = secret;
      if (passphraseInput && !readSecret(passphraseInput)) passphraseInput.value = passphrase;
    } finally {
      secret = passphrase = "";
      busy = false;
    }
  }

  async function pickKeyFile() {
    const chosen = await openDialog({ title: "Choose a private key file", multiple: false, directory: false });
    if (typeof chosen === "string") {
      keyFile = chosen;
      error = "";
    }
  }
  const fileName = (p: string) => p.split(/[\\/]/).pop() ?? p;
</script>

<div class="form" class:compact>
  <div class="grid">
    <label class="field"><span>Name</span><input id="cred-name" bind:value={name} placeholder="e.g. Production deploy" /></label>
    <label class="field"><span>Username</span><input id="cred-user" bind:value={username} placeholder="Optional" autocomplete="off" /></label>
  </div>
  <div class="kind" role="radiogroup" aria-label="Credential type">
    <button type="button" class:act={kind === "password"} onclick={() => (kind = "password")}>Password</button>
    <button type="button" class:act={kind === "key"} onclick={() => (kind = "key")}>Private key</button>
    <button type="button" class:act={kind === "agent"} onclick={() => (kind = "agent")}>SSH agent</button>
  </div>
  {#if kind === "agent"}
    <p class="muted hint">Keys are taken from the running SSH agent (OpenSSH agent or Pageant on Windows). Nothing secret is stored.</p>
  {:else if kind === "password"}
    <label class="field">
      <span>Password{credential ? " (leave empty to keep the saved one)" : ""}</span>
      <input id="cred-secret" type="password" bind:this={secretInput} autocomplete="new-password" />
    </label>
  {:else}
    <div class="field">
      <span id="cred-key-label">Private key, OpenSSH, PEM or PuTTY (.ppk){credential ? " (leave empty to keep the saved one)" : ""}</span>
      {#if keyFile}
        <div class="keyfile">
          <span class="mono" title={keyFile}>{fileName(keyFile)}</span>
          <small class="muted">Read from the file when you save; it never passes through this window.</small>
          <button type="button" class="link" onclick={() => (keyFile = null)}>Use pasted key instead</button>
        </div>
      {:else}
        <textarea id="cred-key" aria-labelledby="cred-key-label" rows="5" bind:this={secretInput} placeholder="-----BEGIN OPENSSH PRIVATE KEY-----"></textarea>
      {/if}
    </div>
    <div class="row">
      <button type="button" class="btn" onclick={pickKeyFile}>Load from file…</button>
      {#if !askPassphrase}
        <label class="field grow">
          <span>Key passphrase{credential ? " (leave empty to keep the saved one)" : ""}</span>
          <input id="cred-pass" type="password" bind:this={passphraseInput} placeholder="If the key has one" />
        </label>
      {/if}
    </div>
    <label class="check">
      <input id="cred-ask" type="checkbox" bind:checked={askPassphrase} />
      <span>Ask for the passphrase when connecting <small class="muted">(it is not saved)</small></span>
    </label>
  {/if}
  {#if error}<p class="error">{error}</p>{/if}
  <div class="actions">
    {#if onCancel}<button type="button" class="btn" onclick={onCancel}>Cancel</button>{/if}
    <button type="button" class="btn primary" disabled={busy} onclick={save}>{credential ? "Save credential" : "Add credential"}</button>
  </div>
</div>

<style>
  .form { display: grid; gap: 10px; }
  .form.compact { padding: 12px; border: 1px solid var(--line); border-radius: 8px; background: var(--panel-2); }
  .grid { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; }
  .kind { display: inline-flex; gap: 2px; padding: 2px; border: 1px solid var(--line); border-radius: 7px; justify-self: start; }
  .kind button { padding: 4px 12px; border-radius: 5px; color: var(--muted); }
  .kind button.act { background: var(--accent); color: var(--accent-fg); }
  .row { display: flex; gap: 10px; align-items: flex-end; }
  .grow { flex: 1; }
  .check { display: flex; gap: 8px; align-items: center; cursor: pointer; }
  .check input { accent-color: var(--accent); }
  .keyfile { display: grid; gap: 2px; padding: 8px 10px; border: 1px solid var(--line); border-radius: 6px; background: var(--panel-2); }
  .keyfile .mono { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .keyfile .link { justify-self: start; padding: 0; color: var(--accent); font-size: 12px; }
  .keyfile .link:hover { text-decoration: underline; }
  .hint { margin: 0; font-size: 12px; }
  .error { margin: 0; color: var(--bad); }
  .actions { display: flex; gap: 8px; justify-content: flex-end; }
</style>
