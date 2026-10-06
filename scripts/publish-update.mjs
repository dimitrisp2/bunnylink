// SPDX-FileCopyrightText: 2026 BunnyCloud.IT
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial
// Signs a release package and publishes it to the BunnyCloud update server.
//
//   node scripts/publish-update.mjs --version=0.2.0 [--changelog=notes.md] [--draft] package.zip
//
// Reads from the environment (GitHub secrets in CI):
//   UPDATE_SERVER_URL   server base URL
//   UPDATE_PUBLIC_KEY   release public key (base64); the upload is refused if the signing key doesn't match
//   US_SIGNING_KEY      libsodium secret key (base64, 64 bytes: seed + public key)
//   US_TOKEN            API token for the bunnylink product
//
// The signature covers "bunnysite-release-v1\n{product}\n{version}\n{sha256 hex}\n{size}",
// the message the server and the app's updater verify.
import { createHash, createPrivateKey, createPublicKey, sign, verify } from "node:crypto";
import { readFileSync } from "node:fs";
import { basename } from "node:path";

const PRODUCT = "bunnylink";
const CONTEXT = "bunnysite-release-v1";

function fail(message) {
  console.error(`Error: ${message}`);
  process.exit(1);
}

const args = Object.fromEntries(
  process.argv.slice(2).filter((a) => a.startsWith("--")).map((a) => {
    const [k, ...v] = a.slice(2).split("=");
    return [k, v.length ? v.join("=") : "1"];
  }),
);
const packagePath = process.argv.slice(2).find((a) => !a.startsWith("--")) ?? fail("Give the package zip.");
const version = args.version ?? fail("Give --version=X.Y.Z.");
const env = (name) => (process.env[name] ?? "").trim() || fail(`Set ${name}.`);
const server = env("UPDATE_SERVER_URL").replace(/\/+$/, "");
const expectedPublic = env("UPDATE_PUBLIC_KEY");
const token = env("US_TOKEN");

// libsodium secret key = 32-byte seed + 32-byte public key.
const secret = Buffer.from(env("US_SIGNING_KEY"), "base64");
if (secret.length !== 64) fail("US_SIGNING_KEY must be a base64 libsodium secret key (64 bytes).");
const pkcs8 = Buffer.concat([Buffer.from("302e020100300506032b657004220420", "hex"), secret.subarray(0, 32)]);
const privateKey = createPrivateKey({ key: pkcs8, format: "der", type: "pkcs8" });
const publicRaw = createPublicKey(privateKey).export({ format: "der", type: "spki" }).subarray(-32);
if (publicRaw.toString("base64") !== expectedPublic) {
  fail("The signing key does not match UPDATE_PUBLIC_KEY. Nothing was uploaded.");
}

const data = readFileSync(packagePath);
const sha256 = createHash("sha256").update(data).digest("hex");
const message = `${CONTEXT}\n${PRODUCT}\n${version}\n${sha256}\n${data.length}`;
const signatureRaw = sign(null, Buffer.from(message), privateKey);
if (!verify(null, Buffer.from(message), createPublicKey(privateKey), signatureRaw)) fail("The signature does not verify.");
const signature = signatureRaw.toString("base64");

const form = new FormData();
form.set("product", PRODUCT);
form.set("version", version);
form.set("channel", "stable");
form.set("status", args.draft ? "draft" : "published");
form.set("changelog", args.changelog ? readFileSync(args.changelog, "utf8") : "");
form.set("signature", signature);
form.set("package", new Blob([data], { type: "application/zip" }), basename(packagePath));

let response;
try {
  response = await fetch(`${server}/api/v1/releases`, {
    method: "POST",
    headers: { Authorization: `Bearer ${token}`, Accept: "application/json" },
    body: form,
  });
} catch (e) {
  fail(`Could not reach the update server: ${e.cause?.code ?? e.message}`);
}
const text = await response.text();
let body;
try {
  body = JSON.parse(text);
} catch {
  fail(`The server answered ${response.status} with something other than JSON.`);
}
if (!response.ok) fail(`The server refused the release (${response.status}): ${body.error ?? text}`);
console.log(`Published ${PRODUCT} ${version} (${data.length} bytes, sha256 ${sha256}) as ${body.status}.`);
