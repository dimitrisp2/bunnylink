<!-- SPDX-FileCopyrightText: 2026 BunnyCloud.IT -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial -->
<script module lang="ts">
  type Part = { kind: "heading"; text: string } | { kind: "text"; text: string } | { kind: "list"; items: string[] };
  interface Release { version: string; parts: Part[] }

  const BULLET = /^\s*[-*•]\s+/;

  /**
   * Reads release notes as written in tag messages: "## <version>" starts a release,
   * a line followed by "- " items is a section heading, the rest are paragraphs.
   */
  export function parseNotes(text: string, version = ""): Release[] {
    const releases: Release[] = [];
    let cur: { version: string; lines: string[] } = { version, lines: [] };
    const blocks = [cur];
    for (const line of text.replace(/\r/g, "").split("\n")) {
      const m = line.match(/^##\s+(.+)$/);
      if (m) blocks.push((cur = { version: m[1].trim(), lines: [] }));
      else cur.lines.push(line);
    }
    for (const b of blocks) {
      const lines = b.lines.filter((l) => l.trim());
      // The tag's title line only repeats the version.
      if (lines.length && b.version && lines[0].trim().toLowerCase() === `bunnylink ${b.version}`.toLowerCase()) lines.shift();
      if (!lines.length) continue;
      const parts: Part[] = [];
      lines.forEach((line, i) => {
        const last = parts.at(-1);
        if (BULLET.test(line)) {
          const item = line.replace(BULLET, "").trim();
          if (last?.kind === "list") last.items.push(item);
          else parts.push({ kind: "list", items: [item] });
        } else if (last?.kind === "list" && /^\s/.test(line)) {
          // An indented line continues the item above it.
          last.items[last.items.length - 1] += " " + line.trim();
        } else {
          const heading = BULLET.test(lines[i + 1] ?? "") || /^#+\s/.test(line);
          const text = line.replace(/^#+\s+/, "").trim();
          parts.push(heading ? { kind: "heading", text } : { kind: "text", text });
        }
      });
      releases.push({ version: b.version, parts });
    }
    return releases;
  }
</script>

<script lang="ts">
  let { text, version = "" }: { text: string; version?: string } = $props();
  const releases = $derived(parseNotes(text, version));
</script>

<div class="notes">
  {#each releases as r, i (i)}
    {#if releases.length > 1 && r.version}<h3>{r.version}</h3>{/if}
    {#each r.parts as p, j (j)}
      {#if p.kind === "heading"}<h4>{p.text}</h4>
      {:else if p.kind === "text"}<p>{p.text}</p>
      {:else}<ul>{#each p.items as item, k (k)}<li>{item}</li>{/each}</ul>{/if}
    {/each}
  {/each}
</div>

<style>
  .notes { padding: 4px 14px 12px; max-height: 340px; overflow: auto; background: var(--bg); border-radius: 6px; user-select: text; line-height: 1.5; }
  h3 { margin: 12px 0 2px; font-size: 13px; }
  h4 { margin: 10px 0 2px; font-size: 11px; letter-spacing: .06em; text-transform: uppercase; color: var(--muted); }
  p { margin: 8px 0 0; }
  ul { margin: 2px 0 0; padding-left: 18px; }
  li + li { margin-top: 3px; }
</style>
