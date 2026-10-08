<!-- SPDX-FileCopyrightText: 2026 BunnyCloud.IT -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial -->
<script lang="ts">
  import { onMount } from "svelte";
  import Modal from "./Modal.svelte";
  import { api } from "../api";
  import type { LoginRequest } from "../api";
  import { app } from "../state.svelte";
  import { takeSecret } from "../secret";

  let { request }: { request: LoginRequest } = $props();
  // Not bound to state: read once when sent (see ../secret).
  const inputs: HTMLInputElement[] = $state([]);

  // `autofocus` only applies when nothing else has focus, and the button or terminal that
  // started the connection usually still does.
  onMount(() => requestAnimationFrame(() => inputs[0]?.focus()));

  function answer(values: string[] | null) {
    // Read the id first: removing the request from the queue unsets this prop.
    const id = request.id;
    app.loginPrompts = app.loginPrompts.filter((r) => r.id !== id);
    api.loginReply(id, values);
  }
  function submit(e: SubmitEvent) {
    e.preventDefault();
    // Answers may be empty: some servers ask for an optional value.
    answer(request.prompts.map((_, i) => takeSecret(inputs[i])));
  }
</script>

<Modal title={request.name.trim() || "Login"} width={440} onclose={() => answer(null)}>
  <form id="login-form" onsubmit={submit} novalidate>
    <p class="muted"><b>{request.host}</b> asks for more information to log in.</p>
    {#if request.instructions.trim()}<p class="instructions">{request.instructions.trim()}</p>{/if}
    {#each request.prompts as p, i (i)}
      <label class="field">
        <span>{p.prompt.trim().replace(/:$/, "")}</span>
        <input id="login-input-{i}" type={p.echo ? "text" : "password"} bind:this={inputs[i]} autocomplete="off" spellcheck="false" />
      </label>
    {/each}
  </form>
  {#snippet footer()}
    <button class="btn" onclick={() => answer(null)}>Cancel</button>
    <button class="btn primary" type="submit" form="login-form">Continue</button>
  {/snippet}
</Modal>

<style>
  form { display: grid; gap: 12px; }
  p { margin: 0; }
  .instructions { white-space: pre-wrap; }
</style>
