<!-- SPDX-FileCopyrightText: 2026 BunnyCloud.IT -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial -->
<script lang="ts">
  import Modal from "./Modal.svelte";
  import { api, errorText } from "../api";
  import type { Host } from "../api";
  import { app, folderPath, hostById, reload, toast } from "../state.svelte";

  let { hostIds }: { hostIds: string[] } = $props();

  // svelte-ignore state_referenced_locally
  const hosts = hostIds.map(hostById).filter((h): h is Host => !!h);
  const same = <T,>(values: T[]) => (values.every((v) => v === values[0]) ? values[0] : undefined);

  // Each field is left alone unless ticked; it starts at the value all hosts share.
  let setFolder = $state(false);
  let folderId = $state<string | null>(same(hosts.map((h) => h.folderId ?? null)) ?? null);
  let setCredential = $state(false);
  let credentialId = $state<string | null>(same(hosts.map((h) => h.overrides.credentialId ?? null)) ?? null);
  let addTags = $state("");
  let removeTags = $state<string[]>([]);
  let busy = $state(false);
  let error = $state("");

  const folders = $derived(
    app.library.folders.map((f) => ({ id: f.id, path: folderPath(f.id) })).sort((a, b) => a.path.localeCompare(b.path)),
  );
  /** Tags on any of the hosts, with how many have each. */
  const tags = $derived.by(() => {
    const n = new Map<string, number>();
    for (const t of hosts.flatMap((h) => h.tags)) n.set(t, (n.get(t) ?? 0) + 1);
    return [...n].sort((a, b) => a[0].localeCompare(b[0]));
  });
  const parseTags = (s: string) => s.split(/[,\s]+/).map((t) => t.replace(/^#/, "").trim()).filter(Boolean);
  const changes = $derived(setFolder || setCredential || parseTags(addTags).length > 0 || removeTags.length > 0);
  const names = hosts.length > 6 ? `${hosts.slice(0, 5).map((h) => h.name).join(", ")} and ${hosts.length - 5} more` : hosts.map((h) => h.name).join(", ");

  function toggleRemove(t: string) {
    removeTags = removeTags.includes(t) ? removeTags.filter((x) => x !== t) : [...removeTags, t];
  }

  async function save() {
    busy = true;
    error = "";
    const add = parseTags(addTags);
    let done = 0;
    try {
      for (const h of hosts) {
        const next = $state.snapshot(h) as Host;
        if (setFolder) next.folderId = folderId;
        if (setCredential) next.overrides = { ...next.overrides, credentialId };
        next.tags = [...new Set([...next.tags.filter((t) => !removeTags.includes(t)), ...add])];
        await api.saveHost(next);
        done++;
      }
      toast(`Updated ${done} ${done === 1 ? "host" : "hosts"}.`);
      app.modal = null;
    } catch (e) {
      error = `${errorText(e)} ${done} of ${hosts.length} hosts were updated before this.`;
    } finally {
      busy = false;
      await reload();
    }
  }
</script>

<Modal title="Edit {hosts.length} hosts" width={520}>
  <p class="muted names">{names}</p>

  <div class="field">
    <label class="check">
      <input id="bulk-set-folder" type="checkbox" bind:checked={setFolder} />
      <span>Move to folder</span>
    </label>
    <select id="bulk-folder" bind:value={folderId} disabled={!setFolder}>
      <option value={null}>No folder</option>
      {#each folders as f (f.id)}<option value={f.id}>{f.path}</option>{/each}
    </select>
  </div>

  <div class="field">
    <label class="check">
      <input id="bulk-set-credential" type="checkbox" bind:checked={setCredential} />
      <span>Set credential</span>
    </label>
    <select id="bulk-credential" bind:value={credentialId} disabled={!setCredential}>
      <option value={null}>Inherit from the folder</option>
      {#each app.library.credentials as c (c.id)}<option value={c.id}>{c.name}{c.username ? ` · ${c.username}` : ""}</option>{/each}
    </select>
  </div>

  <label class="field">
    <span>Add tags</span>
    <input id="bulk-add-tags" class="mono" bind:value={addTags} placeholder="prod, web" />
  </label>

  {#if tags.length}
    <div class="field">
      <span class="lbl">Remove tags <small class="muted">(click to remove; the count is how many of these hosts have it)</small></span>
      <div class="tags">
        {#each tags as [t, n] (t)}
          <button type="button" class="chip" class:remove={removeTags.includes(t)} aria-pressed={removeTags.includes(t)} onclick={() => toggleRemove(t)}>
            #{t} <span class="n">{n}</span>
          </button>
        {/each}
      </div>
    </div>
  {/if}

  {#if error}<p class="error">{error}</p>{/if}

  {#snippet footer()}
    <button class="btn" onclick={() => (app.modal = null)}>Cancel</button>
    <button class="btn primary" onclick={save} disabled={!changes || busy}>{busy ? "Saving…" : `Update ${hosts.length} hosts`}</button>
  {/snippet}
</Modal>

<style>
  p { margin: 0; }
  .names { overflow-wrap: anywhere; }
  .field { display: grid; gap: 5px; }
  .field > span, .lbl { font-size: 11.5px; color: var(--muted); }
  .check { display: flex; gap: 8px; align-items: center; cursor: pointer; font-size: 13px; }
  .check input { accent-color: var(--accent); }
  .tags { display: flex; flex-wrap: wrap; gap: 5px; }
  .chip { font: 11.5px var(--mono); padding: 2px 9px; border-radius: 12px; border: 1px solid var(--line); color: var(--fg); }
  .chip .n { color: var(--faint); margin-left: 2px; }
  .chip.remove { border-color: var(--bad); color: var(--bad); text-decoration: line-through; }
  .error { color: var(--bad); }
</style>
