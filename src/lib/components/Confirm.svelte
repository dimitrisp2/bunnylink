<!-- SPDX-FileCopyrightText: 2026 BunnyCloud.IT -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial -->
<script lang="ts">
  import Modal from "./Modal.svelte";
  import { app } from "../state.svelte";
  let { title, body, confirm, run }: { title: string; body: string; confirm: string; run: () => Promise<void> | void } = $props();
  let busy = $state(false);
</script>

<Modal {title} width={420} dismissable>
  <p>{body}</p>
  {#snippet footer()}
    <button class="btn" onclick={() => (app.modal = null)}>Cancel</button>
    <!-- svelte-ignore a11y_autofocus -->
    <button class="btn primary danger-fill" disabled={busy} autofocus onclick={async () => { busy = true; await run(); app.modal = null; }}>{confirm}</button>
  {/snippet}
</Modal>

<style>
  p { margin: 0; color: var(--muted); }
  .danger-fill { background: var(--bad); border-color: var(--bad); color: #fff; }
</style>
