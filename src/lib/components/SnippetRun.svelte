<!-- SPDX-FileCopyrightText: 2026 BunnyCloud.IT -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial -->
<script lang="ts">
  import Modal from "./Modal.svelte";
  import type { Snippet } from "../api";
  import { app, fillPlaceholders, sendSnippet, type Placeholder } from "../state.svelte";

  let { snippet, tabId, fields, values: initial }: { snippet: Snippet; tabId: string; fields: Placeholder[]; values: Record<string, string> } = $props();

  // svelte-ignore state_referenced_locally
  let values = $state({ ...initial });
  const preview = $derived(fillPlaceholders(snippet.command, values));
  const tab = $derived(app.tabs.find((t) => t.id === tabId));

  function send(e: SubmitEvent) {
    e.preventDefault();
    app.modal = null;
    sendSnippet(snippet, tabId, $state.snapshot(values));
  }
</script>

<Modal title={snippet.name} width={520}>
  <form id="snippet-run" onsubmit={send}>
    {#each fields as f, i (f.name)}
      <label class="field">
        <span>{f.name}</span>
        <!-- svelte-ignore a11y_autofocus -->
        <input class="mono" bind:value={values[f.name]} placeholder={f.default} autofocus={i === 0} spellcheck="false" />
      </label>
    {/each}
  </form>
  <div class="field">
    <span class="lbl">Sends to {tab?.title ?? "the terminal"}{snippet.sendEnter ? "" : " (without pressing Enter)"}</span>
    <pre class="preview">{preview}</pre>
  </div>

  {#snippet footer()}
    <button class="btn" onclick={() => (app.modal = null)}>Cancel</button>
    <button class="btn primary" type="submit" form="snippet-run" disabled={!tab}>Send</button>
  {/snippet}
</Modal>

<style>
  form { display: grid; gap: 10px; }
  .lbl { font-size: 11.5px; color: var(--muted); }
  .preview { margin: 0; padding: 8px 10px; max-height: 160px; overflow: auto; white-space: pre-wrap; word-break: break-all; font: 12px var(--mono); background: var(--panel-2); border-radius: 6px; color: var(--accent); }
</style>
