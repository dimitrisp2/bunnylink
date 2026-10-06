// SPDX-FileCopyrightText: 2026 BunnyCloud.IT
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial
// Generates static/licenses.json: the open-source licenses of everything shipped
// in the app (Rust crates linked into the binary + npm packages bundled into the
// frontend). Runs automatically before `npm run dev` / `npm run build`.
import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, statSync, writeFileSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const OUT = join(ROOT, "static", "licenses.json");
const LICENSE_RE = /^(licen[cs]e|copying|notice)/i;

function readText(file) {
  try {
    return readFileSync(file, "utf8").replace(/\r\n/g, "\n");
  } catch {
    return null;
  }
}

/** License-ish files in a directory, plus optional extra files. Dedupes identical texts. */
function collectTexts(dir, extra = []) {
  const files = [];
  try {
    for (const name of readdirSync(dir).sort()) {
      if (!LICENSE_RE.test(name)) continue;
      const full = join(dir, name);
      try {
        if (statSync(full).isFile()) files.push(full);
      } catch {}
    }
  } catch {}
  for (const f of extra) if (f && !files.includes(f)) files.push(f);

  const texts = [];
  const seen = new Set();
  for (const f of files) {
    const text = readText(f);
    if (text == null) continue;
    const key = text.trim();
    if (seen.has(key)) continue;
    seen.add(key);
    texts.push({ file: f.split(/[\\/]/).pop(), text });
  }
  return texts;
}

function cleanRepo(url) {
  if (!url || typeof url !== "string") return null;
  let u = url.trim();
  if (u.startsWith("git+")) u = u.slice(4);
  if (u.endsWith(".git")) u = u.slice(0, -4);
  if (/^github:/.test(u)) u = "https://github.com/" + u.slice(7);
  else if (/^[\w.-]+\/[\w.-]+$/.test(u)) u = "https://github.com/" + u;
  return u || null;
}

// ---------------------------------------------------------------- Rust
function rustPackages() {
  const raw = execFileSync(
    "cargo",
    [
      "metadata",
      "--format-version",
      "1",
      "--manifest-path",
      join(ROOT, "src-tauri", "Cargo.toml"),
      "--filter-platform",
      "x86_64-pc-windows-msvc",
    ],
    { cwd: ROOT, encoding: "utf8", maxBuffer: 1024 * 1024 * 1024 },
  );
  const meta = JSON.parse(raw);
  const pkgById = new Map(meta.packages.map((p) => [p.id, p]));
  const nodeById = new Map(meta.resolve.nodes.map((n) => [n.id, n]));
  const members = new Set(meta.workspace_members);
  const rootId = meta.resolve.root;

  const visited = new Set([rootId]);
  const stack = [rootId];
  while (stack.length) {
    const node = nodeById.get(stack.pop());
    if (!node) continue;
    for (const dep of node.deps ?? []) {
      const normal = (dep.dep_kinds ?? []).some((k) => k.kind == null);
      if (!normal || visited.has(dep.pkg)) continue;
      visited.add(dep.pkg);
      stack.push(dep.pkg);
    }
  }

  const out = [];
  for (const id of visited) {
    if (id === rootId || members.has(id)) continue;
    const p = pkgById.get(id);
    if (!p) continue;
    const dir = dirname(p.manifest_path);
    const licenseFile = p.license_file ? resolve(dir, p.license_file) : null;
    out.push({
      name: p.name,
      version: p.version,
      ecosystem: "rust",
      license: p.license || "Unknown",
      repository: cleanRepo(p.repository),
      texts: collectTexts(dir, licenseFile ? [licenseFile] : []),
    });
  }
  return out;
}

// ---------------------------------------------------------------- npm
// Dev dependencies whose runtime code is bundled into the frontend anyway. Their own
// dependencies are listed too, which may include a few that only run at build time.
const BUNDLED_DEV_DEPENDENCIES = ["svelte", "@sveltejs/kit"];

function npmLicense(pj) {
  const l = pj.license ?? pj.licenses;
  if (typeof l === "string") return l;
  if (Array.isArray(l)) {
    const parts = l.map((x) => (typeof x === "string" ? x : x?.type)).filter(Boolean);
    return parts.length ? parts.join(" OR ") : "Unknown";
  }
  if (l && typeof l === "object" && l.type) return l.type;
  return "Unknown";
}

function findPackageDir(name, fromDir) {
  // Node-style lookup: <dir>/node_modules/<name>, walking up to the project root.
  let dir = fromDir;
  while (true) {
    const candidate = join(dir, "node_modules", ...name.split("/"));
    if (existsSync(join(candidate, "package.json"))) return candidate;
    if (dir === ROOT || dirname(dir) === dir) break;
    dir = dirname(dir);
    if (!dir.startsWith(ROOT)) break;
  }
  return null;
}

function npmPackages() {
  const rootPj = JSON.parse(readFileSync(join(ROOT, "package.json"), "utf8"));
  const seen = new Map(); // dir -> package entry
  const roots = [...Object.keys(rootPj.dependencies ?? {}), ...BUNDLED_DEV_DEPENDENCIES];
  const queue = roots.map((name) => ({ name, from: ROOT, optional: false }));

  while (queue.length) {
    const { name, from, optional } = queue.shift();
    const dir = findPackageDir(name, from);
    if (!dir) {
      if (!optional) console.warn(`[licenses] warning: npm package not installed: ${name}`);
      continue;
    }
    if (seen.has(dir)) continue;
    const pj = JSON.parse(readFileSync(join(dir, "package.json"), "utf8"));
    const repo = typeof pj.repository === "string" ? pj.repository : pj.repository?.url;
    seen.set(dir, {
      name: pj.name ?? name,
      version: pj.version ?? "0.0.0",
      ecosystem: "npm",
      license: npmLicense(pj),
      repository: cleanRepo(repo),
      texts: collectTexts(dir),
    });
    const optionalDeps = new Set(Object.keys(pj.optionalDependencies ?? {}));
    for (const dep of Object.keys(pj.dependencies ?? {})) {
      queue.push({ name: dep, from: dir, optional: optionalDeps.has(dep) });
    }
    for (const dep of optionalDeps) {
      if (!(pj.dependencies ?? {})[dep]) queue.push({ name: dep, from: dir, optional: true });
    }
  }
  return [...seen.values()];
}

// ---------------------------------------------------------------- main
const packages = [...rustPackages(), ...npmPackages()];

// Dedupe identical name+version+ecosystem (e.g. hoisted copies found twice).
const unique = new Map();
for (const p of packages) {
  const key = `${p.ecosystem}\0${p.name}\0${p.version}`;
  if (!unique.has(key)) unique.set(key, p);
}
const sorted = [...unique.values()].sort(
  (a, b) =>
    a.ecosystem.localeCompare(b.ecosystem) ||
    a.name.toLowerCase().localeCompare(b.name.toLowerCase()) ||
    a.version.localeCompare(b.version, undefined, { numeric: true }),
);

// Many packages ship the same license text: store each text once, and give each
// package's file the index of its text.
const texts = [];
const textIndex = new Map();
for (const p of sorted) {
  p.texts = p.texts.map(({ file, text }) => {
    if (!textIndex.has(text)) {
      textIndex.set(text, texts.length);
      texts.push(text);
    }
    return { file, text: textIndex.get(text) };
  });
}

const json = JSON.stringify({ generated: new Date().toISOString(), texts, packages: sorted });
mkdirSync(dirname(OUT), { recursive: true });
writeFileSync(OUT, json);

const count = (eco) => sorted.filter((p) => p.ecosystem === eco).length;
console.log(
  `[licenses] ${count("rust")} Rust crates, ${count("npm")} npm packages, ${texts.length} distinct texts -> static/licenses.json (${(
    Buffer.byteLength(json) / 1024
  ).toFixed(0)} KiB)`,
);
