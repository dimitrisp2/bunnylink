# BunnyLink

A remote connection manager for Windows 10 and 11, a modern take on mRemoteNG.
Every session runs inside the app; nothing external needs to be installed.
macOS and Linux builds are not decided yet.

Built with [Tauri 2](https://tauri.app) (Rust core) and Svelte 5.

## Status

| Area | State |
| --- | --- |
| Encrypted vault (master password, Argon2id + XChaCha20-Poly1305) | Done |
| Hosts, folders with inherited defaults, tags, pinning | Done |
| Command palette (`Ctrl K`), tabs, split view | Done |
| Themes: Dark (default), Light, Auto | Done |
| SSH terminal (xterm.js), host key trust-on-first-use | Done |
| Jump hosts (nested) | Done |
| Tunnels: SSH local, SSH remote, SOCKS5 | Done |
| Run command, copy credentials (auto-clearing clipboard) | Done |
| Web console (HTTP/HTTPS in an app window) | Done |
| SFTP file browser: upload/download (folders too), drag and drop, rename, delete | Done |
| FTP / FTPS file browser (FTPS used automatically when offered, certificate trust-on-first-use) | Done |
| SMB file browser (SMB 2/3, NTLM; browse shares or open one directly) | Done |
| RDP in a tab (IronRDP): NLA/TLS, certificate trust-on-first-use, keyboard, mouse, server cursors, resize | Done |
| VNC in a tab (vnc-rs): VNC password auth, ZRLE/CopyRect, server cursors, keyboard, mouse, wheel | Done |
| RDP, VNC, Telnet, FTP and SMB through an SSH jump host | Done |
| SPICE | Deferred |
| Telnet terminal (option negotiation, window size, automatic login at prompts) | Done |
| Serial console (baud, data bits, parity, stop bits, flow control; port suggestions) | Done |
| SSH agent auth (OpenSSH agent, Pageant on Windows) | Done |
| Import from mRemoteNG (incl. encrypted files and passwords), PuTTY sessions and `~/.ssh/config` | Done |
| Key passphrases: saved, or asked on every connection (inside the terminal for SSH tabs) | Done |
| Portable mode (database next to the executable, chosen on first run) | Done |
| Signed self-updates (official builds) | Done |

Planned actions already show up in the UI, marked "Soon".

## Development

Prerequisites: Rust (stable), Node.js 20+, and the
[Tauri system prerequisites](https://tauri.app/start/prerequisites/) for your OS
(on Windows: WebView2, which ships with Windows 10/11).

```sh
npm install
npm run tauri dev       # run the app with hot reload
npm run tauri build     # build src-tauri/target/release/bunnylink.exe (no installers)
```

A local build has no updater: the update server and its release key are only
compiled into official builds (see Releasing).

## Releasing

1. Set the new version in `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` and
   `package.json`, and commit.
2. Tag it with an annotated tag; its message becomes the release notes:
   `git tag -a v0.2.0 -m "What changed"`, then `git push --follow-tags`.

The tag starts `.github/workflows/windows.yml`, which runs in the `release`
environment (tags `v*` only). It checks that the tag matches the app version,
builds `bunnylink.exe` with the update server compiled in, signs and publishes it
to the update server (`scripts/publish-update.mjs`), and attaches `bunnylink.exe`
to a draft GitHub release. No installers are built.

The environment holds four secrets: `UPDATE_SERVER_URL`, `UPDATE_PUBLIC_KEY`,
`US_SIGNING_KEY` (Ed25519, base64) and `US_TOKEN`. The app installs an update only
when its size, SHA-256 and signature match the public key built into it.

Checks:

```sh
npm run check                         # Svelte + TypeScript
cd src-tauri && cargo test            # Rust unit tests
```

The protocol tests run against real servers when configured; otherwise
they are skipped:

```sh
BUNNYLINK_TEST_SSH=127.0.0.1:22:user:password \
BUNNYLINK_TEST_RDP=127.0.0.1:3389:user:password \
BUNNYLINK_TEST_VNC=127.0.0.1:5900:password \
BUNNYLINK_TEST_TELNET=127.0.0.1:23:user:password \
BUNNYLINK_TEST_SERIAL=/dev/ttyUSB0 \
BUNNYLINK_TEST_FTP=127.0.0.1:21:user:password \
BUNNYLINK_TEST_SMB=127.0.0.1:445:share:user:password cargo test
```

`BUNNYLINK_TEST_AGENT=1` (with `BUNNYLINK_TEST_SSH`) also tests agent login; the
agent must hold a key the SSH server accepts. With `BUNNYLINK_TEST_SSH` set, the
FTP and SMB tests also run through the SSH server as a jump host.

Convenient test servers on Linux: FreeRDP's shadow server (`freerdp-shadow-cli3`)
and `x11vnc` on an `Xvfb` display, `busybox telnetd`, `vsftpd`, `smbd`, and a
`socat` pty pair with a shell for serial (the serial test types into a shell).

## Layout

```
src/                     Svelte frontend
  lib/api.ts             typed wrappers around the Rust commands
  lib/state.svelte.ts    app state, tabs/panes, host actions
  lib/components/        UI components
src-tauri/src/
  lib.rs                 Tauri commands and app state
  model.rs               data model (hosts, folders, credentials, tunnels)
  store.rs               SQLite storage and folder inheritance
  vault.rs               master-password encryption
  ssh.rs                 SSH connections, terminal, exec, tunnels
  net.rs                 TCP connections, directly or through an SSH jump host
  telnet.rs              Telnet terminal
  serial.rs              serial consoles
  files.rs               remote file systems: shared transfers and progress
  sftp.rs / ftp.rs / smbfs.rs   the SFTP, FTP(S) and SMB backends
  display.rs             shared remote-desktop plumbing (frame format, damage, input)
  rdp.rs                 RDP sessions
  vnc.rs                 VNC sessions
  import.rs              importers: mRemoteNG, PuTTY, OpenSSH config
  updater.rs             self-update: check, download, verify, swap the executable
scripts/
  licenses.mjs           builds static/licenses.json (Settings → Open source) before dev/build
  publish-update.mjs     signs a release and uploads it to the update server (CI)
```

## Data and security

- Everything is stored in one SQLite file (`bunnylink.db`) in the OS app-data
  folder. SQLite is compiled in, so there is nothing to install.
- Portable mode: on first run, when the vault is created, the user can choose to
  keep `bunnylink.db` next to the executable instead (the folder must be
  writable). At startup a database next to the executable wins over the
  app-data one.
- Secrets (passwords, private keys) are encrypted individually with a key derived
  from the master password. The master password and the key are never written to
  disk. The rest of the library (host names, addresses) is not encrypted.
- The vault locks after a configurable idle time (default 15 min). Open sessions
  stay connected.
- Host keys are trusted on first use and checked on every connection after that.
  A changed key blocks the connection until the user forgets the old key.
- Web console windows get no access to the app's commands.

## Licence

Copyright © 2026 BunnyCloud.IT.

BunnyLink is dual-licensed:

- **GPL-3.0-or-later**: free to use, change and share. Anyone who distributes
  BunnyLink or software based on it must release that software's source under
  the GPL too. See [LICENSE](LICENSE).
- **Commercial licence**: for companies that want to ship it, or products built
  on it, without the GPL's obligations. See [COMMERCIAL.md](COMMERCIAL.md) or ask
  through <https://bunnycloud.gr/contact>.

Bundled open-source components keep their own licences; the full list is in
Settings → About.
