// SPDX-FileCopyrightText: 2026 BunnyCloud.IT
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial
//! Shared plumbing for graphical sessions (RDP, VNC): the message format sent to the
//! UI, damage tracking and the input events the UI sends back.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Input from the remote-desktop view. Keys carry both a PC scancode (RDP) and an X11
/// keysym (VNC); each protocol uses the one it needs.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum DesktopInput {
    MouseMove { x: u16, y: u16 },
    MouseButton { button: u8, down: bool },
    Wheel { vertical: bool, units: i16 },
    Key { scancode: Option<u16>, keysym: Option<u32>, down: bool },
    Unicode { ch: char, down: bool },
    ReleaseAll,
    Resize { width: u16, height: u16 },
    Close,
}

/// Messages to the UI. Pixels and cursors travel as binary messages (see `encode_*`).
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum DesktopEvent {
    Notice { text: String },
    Connected { width: u16, height: u16 },
    Resized { width: u16, height: u16 },
    Closed { reason: String },
    Error { message: String },
}

pub enum Out {
    Event(DesktopEvent),
    Binary(Vec<u8>),
}

pub const MSG_RECTS: u8 = 1;
pub const MSG_CURSOR: u8 = 2;
pub const MSG_CURSOR_DEFAULT: u8 = 3;
pub const MSG_CURSOR_HIDDEN: u8 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

/// A framebuffer view: tightly or loosely packed 4-byte pixels in R, G, B, (A) order.
pub struct Pixels<'a> {
    pub data: &'a [u8],
    pub stride: usize,
    pub width: u16,
    pub height: u16,
}

/// Accumulates changed areas and sends them at most ~60 times a second.
pub struct Damage {
    rects: Vec<Rect>,
    last: Instant,
}

impl Default for Damage {
    fn default() -> Self {
        Self { rects: vec![], last: Instant::now() }
    }
}

impl Damage {
    pub fn add(&mut self, r: Rect) {
        if r.w == 0 || r.h == 0 {
            return;
        }
        self.rects.push(r);
        // Many small updates: collapse into one bounding box.
        if self.rects.len() > 48 {
            let (mut x0, mut y0, mut x1, mut y1) = (u16::MAX, u16::MAX, 0u16, 0u16);
            for r in &self.rects {
                x0 = x0.min(r.x);
                y0 = y0.min(r.y);
                x1 = x1.max(r.x.saturating_add(r.w));
                y1 = y1.max(r.y.saturating_add(r.h));
            }
            self.rects = vec![Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }];
        }
    }

    pub fn clear(&mut self) {
        self.rects.clear();
    }

    pub fn flush(&mut self, px: &Pixels, force: bool) -> Option<Vec<u8>> {
        if self.rects.is_empty() || (!force && self.last.elapsed() < Duration::from_millis(16)) {
            return None;
        }
        self.last = Instant::now();
        Some(encode_rects(px, &std::mem::take(&mut self.rects)))
    }
}

/// `[1][count u16]` then per rect `[x y w h: u16]` followed by `w*h*4` RGBA bytes (little endian).
pub fn encode_rects(px: &Pixels, rects: &[Rect]) -> Vec<u8> {
    let mut out = Vec::with_capacity(3 + rects.iter().map(|r| 8 + r.w as usize * r.h as usize * 4).sum::<usize>());
    out.push(MSG_RECTS);
    let count_pos = out.len();
    out.extend_from_slice(&0u16.to_le_bytes());
    let mut count = 0u16;
    for r in rects {
        let x = r.x.min(px.width);
        let y = r.y.min(px.height);
        let w = r.w.min(px.width - x);
        let h = r.h.min(px.height - y);
        if w == 0 || h == 0 {
            continue;
        }
        for v in [x, y, w, h] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for row in y..y + h {
            let start = row as usize * px.stride + x as usize * 4;
            let at = out.len();
            out.extend_from_slice(&px.data[start..start + w as usize * 4]);
            // Decoders leave alpha unset; the canvas needs it opaque.
            for p in out[at..].chunks_exact_mut(4) {
                p[3] = 255;
            }
        }
        count += 1;
    }
    out[count_pos..count_pos + 2].copy_from_slice(&count.to_le_bytes());
    out
}

/// `[2][w h hotspot_x hotspot_y: u16]` followed by `w*h*4` RGBA bytes.
pub fn encode_cursor(w: u16, h: u16, hot_x: u16, hot_y: u16, rgba: &[u8]) -> Vec<u8> {
    let mut out = vec![MSG_CURSOR];
    for v in [w, h, hot_x, hot_y] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(rgba);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_encoding() {
        let data = vec![7u8; 4 * 3 * 4];
        let px = Pixels { data: &data, stride: 16, width: 4, height: 3 };
        let bytes = encode_rects(&px, &[Rect { x: 1, y: 1, w: 2, h: 2 }, Rect { x: 3, y: 2, w: 9, h: 9 }]);
        assert_eq!(bytes[0], MSG_RECTS);
        assert_eq!(u16::from_le_bytes([bytes[1], bytes[2]]), 2);
        assert_eq!(&bytes[3..11], &[1, 0, 1, 0, 2, 0, 2, 0]);
        // The second rect is clipped to the framebuffer: 1x1.
        assert_eq!(&bytes[27..35], &[3, 0, 2, 0, 1, 0, 1, 0]);
        assert_eq!(bytes.len(), 35 + 4);
        assert!(bytes[11..27].chunks(4).all(|p| p == [7, 7, 7, 255]));
    }

    #[test]
    fn damage_collapses() {
        let mut d = Damage::default();
        // The 49th rect triggers a collapse into one bounding box; later ones append.
        for i in 0..50 {
            d.add(Rect { x: i, y: i, w: 1, h: 1 });
        }
        assert_eq!(d.rects, vec![Rect { x: 0, y: 0, w: 49, h: 49 }, Rect { x: 49, y: 49, w: 1, h: 1 }]);
    }
}
