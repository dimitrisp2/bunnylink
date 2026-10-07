<!-- SPDX-FileCopyrightText: 2026 BunnyCloud.IT -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial -->
<script lang="ts">
  import Modal from "./Modal.svelte";
  import { api, errorText } from "../api";
  import type { Snippet } from "../api";
  import {
    app, allTags, BUILTIN_PLACEHOLDERS, deleteSnippet, folderPath, newSnippet, placeholders, reload, runSnippet, snippetTab,
  } from "../state.svelte";

  let { snippet }: { snippet: Snippet | null } = $props();

  // svelte-ignore state_referenced_locally
  let draft = $state<Snippet>(snippet ? ($state.snapshot(snippet) as Snippet) : newSnippet());
  // A snippet limited to a deleted folder stays limited (to nothing) rather than turning global.
  let scoped = $state(draft.folderIds.length > 0 || draft.tags.length > 0);
  draft.folderIds = draft.folderIds.filter((id) => app.library.folders.some((f) => f.id === id));
  let tags = $state(draft.tags.join(", "));
  let error = $state("");

  const folders = $derived(
    app.library.folders.map((f) => ({ id: f.id, path: folderPath(f.id) })).sort((a, b) => a.path.localeCompare(b.path)),
  );
  const parsedTags = $derived(tags.split(/[,\s]+/).map((t) => t.replace(/^#/, "").trim()).filter(Boolean));
  const fields = $derived(placeholders(draft.command));
  const target = snippetTab();

  function toggleFolder(id: string) {
    draft.folderIds = draft.folderIds.includes(id) ? draft.folderIds.filter((x) => x !== id) : [...draft.folderIds, id];
  }
  function toggleTag(t: string) {
    tags = (parsedTags.includes(t) ? parsedTags.filter((x) => x !== t) : [...parsedTags, t]).join(", ");
  }

  async function save(send: boolean) {
    error = "";
    const s = $state.snapshot(draft) as Snippet;
    s.name = s.name.trim();
    s.tags = scoped ? [...new Set(parsedTags)] : [];
    if (!scoped) s.folderIds = [];
    if (scoped && !s.folderIds.length && !s.tags.length) {
      error = "Pick at least one folder or tag, or make it available for all hosts.";
      return;
    }
    try {
      const saved = await api.saveSnippet(s);
      await reload();
      app.modal = null;
      app.sidebar = "snippets";
      if (send && target) await runSnippet(saved, target.id);
    } catch (e) {
      error = errorText(e);
    }
  }
</script>

<Modal title={snippet ? `Edit ${snippet.name}` : "New snippet"} width={580}>
  <div class="grid">
    <!-- svelte-ignore a11y_autofocus -->
    <label class="field"><span>Name</span><input id="snip-name" bind:value={draft.name} placeholder="Disk usage" autofocus /></label>
    <label class="field"><span>Description</span><input id="snip-desc" bind:value={draft.description} placeholder="Optional" /></label>
  </div>

  <label class="field">
    <span>Command</span>
    <textarea id="snip-cmd" rows="4" bind:value={draft.command} placeholder={"df -h {{path:/}}"} spellcheck="false"></textarea>
  </label>
  <p class="muted hint">
    Write <code>{"{{name}}"}</code> or <code>{"{{name:default}}"}</code> to be asked for a value when sending.
    <code>{"{{host}}"}</code>, <code>{"{{address}}"}</code> and <code>{"{{user}}"}</code> are filled in from the host.
    Each line is sent as a separate command.
  </p>
  {#if fields.length}
    <div class="vars">
      {#each fields as f (f.name)}
        <span class="var mono" class:auto={BUILTIN_PLACEHOLDERS.includes(f.name)}>
          {f.name}{#if f.default}<small> = {f.default}</small>{/if}{#if BUILTIN_PLACEHOLDERS.includes(f.name)}<small> (from host)</small>{/if}
        </span>
      {/each}
    </div>
  {/if}

  <label class="check">
    <input id="snip-enter" type="checkbox" bind:checked={draft.sendEnter} />
    <span>Press Enter after sending <small class="muted">(off: the last line waits for you to check it)</small></span>
  </label>

  <div class="field">
    <span class="lbl">Available for</span>
    <div class="kinds" role="radiogroup" aria-label="Available for">
      <button type="button" role="radio" aria-checked={!scoped} class:act={!scoped} onclick={() => (scoped = false)}>All hosts</button>
      <button type="button" role="radio" aria-checked={scoped} class:act={scoped} onclick={() => (scoped = true)}>Chosen folders and tags</button>
    </div>
  </div>
  {#if scoped}
    <div class="grid">
      <div class="field">
        <span class="lbl">Folders <small class="muted">(and their subfolders)</small></span>
        <div class="pick">
          {#each folders as f (f.id)}
            <label class="check"><input type="checkbox" checked={draft.folderIds.includes(f.id)} onchange={() => toggleFolder(f.id)} /><span>{f.path}</span></label>
          {:else}
            <span class="muted">No folders yet.</span>
          {/each}
        </div>
      </div>
      <div class="field">
        <label class="field"><span>Tags</span><input id="snip-tags" class="mono" bind:value={tags} placeholder="prod, web" /></label>
        {#if allTags().length}
          <div class="chips">
            {#each allTags() as t (t)}
              <button type="button" class="chip mono" class:act={parsedTags.includes(t)} onclick={() => toggleTag(t)}>#{t}</button>
            {/each}
          </div>
        {/if}
      </div>
    </div>
  {/if}
  {#if error}<p class="error">{error}</p>{/if}

  {#snippet footer()}
    {#if snippet}<button class="btn danger" style="margin-right:auto" onclick={() => deleteSnippet(snippet)}>Delete</button>{/if}
    <button class="btn" class:primary={!target} onclick={() => save(false)}>Save</button>
    {#if target}<button class="btn primary" onclick={() => save(true)} title="Send to {target.title}">Save and send</button>{/if}
  {/snippet}
</Modal>

<style>
  .grid { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
  .hint { margin: -4px 0 0; font-size: 12px; line-height: 1.5; }
  .hint code { font-size: 11.5px; color: var(--fg); }
  .lbl { font-size: 11.5px; color: var(--muted); }
  .vars { display: flex; flex-wrap: wrap; gap: 5px; }
  .var { font-size: 11px; padding: 2px 8px; border-radius: 10px; background: var(--accent-soft); color: var(--fg); }
  .var.auto { background: var(--panel-2); color: var(--muted); }
  .var small { color: var(--muted); }
  .check { display: flex; gap: 8px; align-items: center; cursor: pointer; }
  .check input { accent-color: var(--accent); }
  .kinds { display: inline-flex; gap: 2px; padding: 2px; border: 1px solid var(--line); border-radius: 7px; justify-self: start; }
  .kinds button { padding: 4px 14px; border-radius: 5px; color: var(--muted); }
  .kinds button.act { background: var(--accent); color: var(--accent-fg); }
  .pick { display: grid; gap: 4px; max-height: 160px; overflow: auto; padding: 6px 8px; border: 1px solid var(--line); border-radius: 6px; }
  .chips { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 6px; }
  .chip { font-size: 11px; padding: 2px 8px; border-radius: 10px; border: 1px solid var(--line); color: var(--muted); }
  .chip.act { background: var(--accent-soft); border-color: var(--accent); color: var(--fg); }
  .error { margin: 0; color: var(--bad); }
</style>
