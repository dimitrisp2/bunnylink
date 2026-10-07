// SPDX-FileCopyrightText: 2026 BunnyCloud.IT
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial
use std::path::{Path, PathBuf};

fn main() {
    // Official builds get the update server and its release key from the environment
    // (GitHub secrets); rebuild when they change.
    println!("cargo:rerun-if-env-changed=BUNNYLINK_UPDATE_URL");
    println!("cargo:rerun-if-env-changed=BUNNYLINK_UPDATE_KEY");
    embed_changelog();
    tauri_build::build()
}

/// Copies this release's notes (`BUNNYLINK_CHANGELOG`, a path relative to `src-tauri`)
/// into the build, so the app can show them. Builds without it embed an empty file.
fn embed_changelog() {
    println!("cargo:rerun-if-env-changed=BUNNYLINK_CHANGELOG");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("changelog.md");
    let text = match std::env::var("BUNNYLINK_CHANGELOG").ok().filter(|p| !p.trim().is_empty()) {
        Some(p) => {
            let path = Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join(p.trim());
            println!("cargo:rerun-if-changed={}", path.display());
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("Could not read the changelog {}: {e}", path.display()))
        }
        None => String::new(),
    };
    std::fs::write(out, text.trim()).unwrap();
}
