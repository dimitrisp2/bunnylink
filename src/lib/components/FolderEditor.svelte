<!-- SPDX-FileCopyrightText: 2026 BunnyCloud.IT -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial -->
<script lang="ts">
  import Modal from "./Modal.svelte";
  import { api, errorText } from "../api";
  import type { Folder } from "../api";
  import { app, attempt, folderPath, reload } from "../state.svelte";

  let { folder, parentId = null }: { folder: Folder | null; parentId?: string | null } = $props();
  // svelte-ignore state_referenced_locally
  let draft = $state<Folder>(folder ? ($state.snapshot(folder) as Folder) : { id: "", parentId, name: "", defaults: {} });
  let error = $state("");

  const descendant = (id: string): boolean => {
    let f = app.library.folders.find((x) => x.id === id);
    while (f) {
      if (f.id === draft.id) return true;
      f = app.library.folders.find((x) => x.id === f!.parentId);
    }
    return false;
  };
  const parents = $derived(
    app.library.folders.filter((f) => !draft.id || !descendant(f.id)).map((f) => ({ id: f.id, path: folderPath(f.id) })),
  );
  const sshHosts = $derived(app.library.hosts.filter((h) => h.endpoints.some((e) => e.protocol === "ssh")));

  async function save() {
    error = "";
    for (const k of ["username", "credentialId", "jumpHostId"] as const) if (!draft.defaults[k]) draft.defaults[k] = null;
    try {
      await api.saveFolder($state.snapshot(draft) as Folder);
      await reload();
      app.modal = null;
    } catch (e) {
      error = errorText(e);
    }
  }
  function remove() {
    const f = folder!;
    app.modal = {
      kind: "confirm",
      title: `Delete folder ${f.name}?`,
      body: "Hosts and subfolders inside it move up one level. Nothing else is deleted.",
      confirm: "Delete folder",
      run: async () => {
        await attempt(() => api.deleteFolder(f.id));
        await reload();
      },
    };
  }
</script>

<Modal title={folder ? `Edit ${folder.name}` : "New folder"}>
  <div class="grid">
    <!-- svelte-ignore a11y_autofocus -->
    <label class="field"><span>Name</span><input id="folder-name" bind:value={draft.name} placeholder="Production" autofocus /></label>
    <label class="field">
      <span>Inside</span>
      <select id="folder-parent" bind:value={draft.parentId}>
        <option value={null}>Top level</option>
        {#each parents as p (p.id)}<option value={p.id}>{p.path}</option>{/each}
      </select>
    </label>
  </div>
  <p class="muted hint">Defaults apply to every host in this folder and its subfolders, unless a host sets its own.</p>
  <div class="grid">
    <label class="field"><span>Default username</span><input id="folder-user" bind:value={draft.defaults.username} autocomplete="off" /></label>
    <label class="field">
      <span>Default credential</span>
      <select id="folder-cred" bind:value={draft.defaults.credentialId}>
        <option value={null}>None</option>
        {#each app.library.credentials as c (c.id)}<option value={c.id}>{c.name}</option>{/each}
      </select>
    </label>
    <label class="field">
      <span>Default jump host</span>
      <select id="folder-jump" bind:value={draft.defaults.jumpHostId}>
        <option value={null}>Direct connection</option>
        {#each sshHosts as h (h.id)}<option value={h.id}>{h.name}</option>{/each}
      </select>
    </label>
  </div>
  {#if error}<p class="error">{error}</p>{/if}
  {#snippet footer()}
    {#if folder}<button class="btn danger" style="margin-right:auto" onclick={remove}>Delete</button>{/if}
    <button class="btn" onclick={() => (app.modal = null)}>Cancel</button>
    <button class="btn primary" disabled={!draft.name.trim()} onclick={save}>{folder ? "Save" : "Create folder"}</button>
  {/snippet}
</Modal>

<style>
  .grid { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
  .hint { margin: 0; font-size: 12px; }
  .error { margin: 0; color: var(--bad); }
</style>
