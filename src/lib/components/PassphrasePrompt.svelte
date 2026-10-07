<!-- SPDX-FileCopyrightText: 2026 BunnyCloud.IT -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial -->
<script lang="ts">
  import { onMount } from "svelte";
  import Modal from "./Modal.svelte";
  import { api } from "../api";
  import type { PassphraseRequest } from "../api";
  import { app } from "../state.svelte";

  let { request }: { request: PassphraseRequest } = $props();
  let passphrase = $state("");
  let input: HTMLInputElement;

  // `autofocus` only applies when nothing else has focus, and the button or terminal that
  // started the connection usually still does.
  onMount(() => requestAnimationFrame(() => input?.focus()));

  function answer(value: string | null) {
    // Read the id first: removing the request from the queue unsets this prop.
    const id = request.id;
    app.passphrasePrompts = app.passphrasePrompts.filter((r) => r.id !== id);
    api.passphraseReply(id, value);
  }
  function submit(e: SubmitEvent) {
    e.preventDefault();
    if (passphrase) answer(passphrase);
  }
</script>

<Modal title="Key passphrase" width={420} onclose={() => answer(null)}>
  <form id="passphrase-form" onsubmit={submit}>
    <p class="muted">Connecting to <b>{request.host}</b> with the key <b>{request.key}</b>.</p>
    {#if request.retry}<p class="error">Wrong passphrase, try again.</p>{/if}
    <label class="field">
      <span>Passphrase</span>
      <input id="passphrase-input" type="password" bind:this={input} bind:value={passphrase} autocomplete="off" />
    </label>
  </form>
  {#snippet footer()}
    <button class="btn" onclick={() => answer(null)}>Cancel</button>
    <button class="btn primary" type="submit" form="passphrase-form" disabled={!passphrase}>Connect</button>
  {/snippet}
</Modal>

<style>
  form { display: grid; gap: 12px; }
  p { margin: 0; }
  .error { color: var(--bad); }
</style>
