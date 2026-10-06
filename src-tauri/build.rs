// SPDX-FileCopyrightText: 2026 BunnyCloud.IT
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial
fn main() {
    // Official builds get the update server and its release key from the environment
    // (GitHub secrets); rebuild when they change.
    println!("cargo:rerun-if-env-changed=BUNNYLINK_UPDATE_URL");
    println!("cargo:rerun-if-env-changed=BUNNYLINK_UPDATE_KEY");
    tauri_build::build()
}
