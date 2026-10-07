// SPDX-FileCopyrightText: 2026 BunnyCloud.IT
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial
//! Copying credentials. On Windows the text is marked so clipboard history (Win+V), cloud
//! clipboard sync and clipboard monitors leave it alone, the way password managers do;
//! clearing the clipboard later then really removes it.

use tauri::AppHandle;

use crate::error::{AppError, AppResult};

#[cfg(windows)]
pub fn copy_private(app: &AppHandle, text: &str) -> AppResult<()> {
    use tauri::Manager;
    // SetClipboardData fails without an owner window.
    let window = app.get_webview_window("main").ok_or_else(|| failed("no window"))?;
    let hwnd = window.hwnd().map_err(|_| failed("no window"))?.0;
    write(hwnd, text)
}

/// Elsewhere there is no history flag to set; a plain copy.
#[cfg(not(windows))]
pub fn copy_private(app: &AppHandle, text: &str) -> AppResult<()> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    app.clipboard().write_text(text.to_string()).map_err(|e| AppError::Other(e.to_string()))
}

fn failed(what: &str) -> AppError {
    AppError::Other(format!("Could not copy to the clipboard ({what})."))
}

/// Formats that, set to a DWORD 0, tell Windows not to keep, sync or monitor the text.
#[cfg(windows)]
const PRIVATE_FORMATS: [&str; 3] =
    ["ExcludeClipboardContentFromMonitorProcessing", "CanIncludeInClipboardHistory", "CanUploadToCloudClipboard"];

#[cfg(windows)]
fn format_id(name: &str) -> u32 {
    use windows_sys::Win32::System::DataExchange::RegisterClipboardFormatW;
    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    // SAFETY: `wide` is a NUL-terminated UTF-16 string.
    unsafe { RegisterClipboardFormatW(wide.as_ptr()) }
}

/// The Windows part of `copy_private`, with the owner window given.
#[cfg(windows)]
fn write(hwnd: *mut std::ffi::c_void, text: &str) -> AppResult<()> {
    use windows_sys::Win32::Foundation::{GlobalFree, HANDLE};
    use windows_sys::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
    use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    use zeroize::Zeroizing;

    const CF_UNICODETEXT: u32 = 13;

    /// Copies `bytes` into a new global memory block, as the clipboard requires.
    fn global(bytes: &[u8]) -> Option<HANDLE> {
        // SAFETY: a fresh block of the right size, locked only while copying into it.
        unsafe {
            let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len());
            if h.is_null() {
                return None;
            }
            let p = GlobalLock(h);
            if p.is_null() {
                GlobalFree(h);
                return None;
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), p.cast::<u8>(), bytes.len());
            GlobalUnlock(h);
            Some(h)
        }
    }
    /// Hands a block to the open clipboard, which then owns it; frees it if refused.
    fn put(format: u32, bytes: &[u8]) -> bool {
        let Some(h) = global(bytes) else { return false };
        // SAFETY: only called while the clipboard is open; `h` is a global block from `global`.
        unsafe {
            if SetClipboardData(format, h).is_null() {
                GlobalFree(h);
                return false;
            }
        }
        true
    }

    let mut utf16 = Zeroizing::new(Vec::<u8>::with_capacity((text.len() + 1) * 2));
    for unit in text.encode_utf16().chain(Some(0)) {
        utf16.extend_from_slice(&unit.to_le_bytes());
    }

    // Another program may hold the clipboard for a moment; retry briefly.
    let mut opened = false;
    for _ in 0..10 {
        // SAFETY: opens with our own window; closed below on every path.
        if unsafe { OpenClipboard(hwnd) } != 0 {
            opened = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    if !opened {
        return Err(failed("it is in use by another program"));
    }
    // SAFETY: the clipboard is open and owned by our window until CloseClipboard.
    let ok = unsafe { EmptyClipboard() } != 0
        && put(CF_UNICODETEXT, &utf16)
        && PRIVATE_FORMATS.iter().all(|name| put(format_id(name), &0u32.to_le_bytes()));
    // SAFETY: opened above.
    unsafe { CloseClipboard() };
    if ok { Ok(()) } else { Err(failed("Windows refused the data")) }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    };
    use windows_sys::Win32::System::Memory::{GlobalLock, GlobalUnlock};
    use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, HWND_MESSAGE};

    /// Writes to the real clipboard, so it only runs when asked:
    /// `cargo test clipboard -- --ignored`
    #[test]
    #[ignore]
    fn copies_text_marked_private() {
        let class: Vec<u16> = "STATIC".encode_utf16().chain(Some(0)).collect();
        // SAFETY: a message-only window of a system class, destroyed at the end.
        let hwnd = unsafe {
            CreateWindowExW(0, class.as_ptr(), std::ptr::null(), 0, 0, 0, 0, 0, HWND_MESSAGE, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null())
        };
        assert!(!hwnd.is_null());
        write(hwnd, "pässwörd ✓").unwrap();

        // SAFETY: reads back under an open clipboard; the data stays owned by the clipboard.
        unsafe {
            assert_ne!(OpenClipboard(hwnd), 0);
            for name in PRIVATE_FORMATS {
                assert_ne!(IsClipboardFormatAvailable(format_id(name)), 0, "{name} missing");
            }
            let h = GetClipboardData(13);
            assert!(!h.is_null());
            let p = GlobalLock(h).cast::<u16>();
            let len = (0..).take_while(|&i| *p.add(i) != 0).count();
            let text = String::from_utf16(std::slice::from_raw_parts(p, len)).unwrap();
            GlobalUnlock(h);
            CloseClipboard();
            DestroyWindow(hwnd);
            assert_eq!(text, "pässwörd ✓");
        }
    }
}
