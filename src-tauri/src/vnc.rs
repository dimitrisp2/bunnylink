// SPDX-FileCopyrightText: 2026 BunnyCloud.IT
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial
//! VNC sessions via vnc-rs. Keeps a local framebuffer, applies the server's updates and
//! streams changed rectangles to the UI in the same format as RDP.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use vnc::{PixelFormat, VncConnector, VncEncoding, VncError, VncEvent, X11Event};

use crate::display::{encode_cursor, Damage, DesktopEvent, DesktopInput, Out, Pixels, Rect, MSG_CURSOR_HIDDEN};
use crate::error::{AppError, AppResult};
use crate::rdp::open_stream;
use crate::ssh;
use crate::store::Store;

pub struct VncTarget {
    pub label: String,
    pub address: String,
    pub port: u16,
    /// VNC authentication only uses a password; `None` for servers without one.
    pub password: Option<crate::model::SecretString>,
    pub jump: Option<ssh::Target>,
}

struct Framebuffer {
    width: u16,
    height: u16,
    data: Vec<u8>,
}

impl Framebuffer {
    fn new(width: u16, height: u16) -> Self {
        Self { width, height, data: vec![0; width as usize * height as usize * 4] }
    }

    fn pixels(&self) -> Pixels<'_> {
        Pixels { data: &self.data, stride: self.width as usize * 4, width: self.width, height: self.height }
    }

    fn put(&mut self, r: &vnc::Rect, src: &[u8]) -> bool {
        if r.x as usize + r.width as usize > self.width as usize
            || r.y as usize + r.height as usize > self.height as usize
            || src.len() < r.width as usize * r.height as usize * 4
        {
            return false;
        }
        let row = r.width as usize * 4;
        for y in 0..r.height as usize {
            let dst = ((r.y as usize + y) * self.width as usize + r.x as usize) * 4;
            self.data[dst..dst + row].copy_from_slice(&src[y * row..(y + 1) * row]);
        }
        true
    }

    fn copy(&mut self, dst: &vnc::Rect, src: &vnc::Rect) -> bool {
        let (w, h) = (dst.width as usize, dst.height as usize);
        let fits = |r: &vnc::Rect| r.x as usize + w <= self.width as usize && r.y as usize + h <= self.height as usize;
        if !fits(dst) || !fits(src) {
            return false;
        }
        // Copy through a temporary so overlapping regions are handled.
        let mut tmp = vec![0u8; w * h * 4];
        for y in 0..h {
            let s = ((src.y as usize + y) * self.width as usize + src.x as usize) * 4;
            tmp[y * w * 4..(y + 1) * w * 4].copy_from_slice(&self.data[s..s + w * 4]);
        }
        let r = vnc::Rect { x: dst.x, y: dst.y, width: dst.width, height: dst.height };
        self.put(&r, &tmp)
    }
}

fn rect(r: &vnc::Rect) -> Rect {
    Rect { x: r.x, y: r.y, w: r.width, h: r.height }
}

fn friendly(e: VncError, label: &str) -> AppError {
    match e {
        VncError::WrongPassword => AppError::Other(format!("{label} rejected the VNC password.")),
        // Servers report a failed check with a free-text reason.
        VncError::General(m) if m.to_lowercase().contains("password") || m.to_lowercase().contains("auth") => {
            AppError::Other(format!("{label} rejected the VNC password."))
        }
        VncError::NoPassword => AppError::Other(format!("{label} needs a VNC password. Pick a password credential.")),
        other => AppError::Other(format!("VNC: {other}")),
    }
}

/// Web mouse buttons (0 left, 1 middle, 2 right) to the RFB button mask.
fn button_bit(button: u8) -> u8 {
    match button {
        0 => 1,
        1 => 2,
        2 => 4,
        _ => 0,
    }
}

pub async fn run(
    target: VncTarget,
    store: Arc<Store>,
    mut input: mpsc::UnboundedReceiver<DesktopInput>,
    emit: impl Fn(Out) + Send + Sync + 'static,
) {
    emit(Out::Event(DesktopEvent::Notice { text: format!("Connecting to {}…", target.label) }));
    let result = session(&target, &store, &mut input, &emit).await;
    match result {
        Ok(reason) => emit(Out::Event(DesktopEvent::Closed { reason })),
        Err(e) => emit(Out::Event(DesktopEvent::Error { message: e.to_string() })),
    }
}

async fn session(
    t: &VncTarget,
    store: &Arc<Store>,
    input: &mut mpsc::UnboundedReceiver<DesktopInput>,
    emit: &(impl Fn(Out) + Send + Sync),
) -> AppResult<String> {
    let (stream, _) = open_stream(&t.label, &t.address, t.port, t.jump.as_ref(), store, emit).await?;
    // vnc-rs takes a plain String; that copy is out of our hands.
    let password = t.password.as_deref().cloned();
    let vnc = tokio::time::timeout(Duration::from_secs(20), async {
        VncConnector::new(stream)
            .set_auth_method(async move { password.ok_or(VncError::NoPassword) })
            .add_encoding(VncEncoding::Zrle)
            .add_encoding(VncEncoding::CopyRect)
            .add_encoding(VncEncoding::Raw)
            .add_encoding(VncEncoding::CursorPseudo)
            .add_encoding(VncEncoding::DesktopSizePseudo)
            .add_encoding(VncEncoding::ExtendedDesktopSizePseudo)
            .allow_shared(true)
            .set_pixel_format(PixelFormat::rgba())
            .build()?
            .try_start()
            .await?
            .finish()
    })
    .await
    .map_err(|_| AppError::Other(format!("Timed out connecting to {}.", t.label)))?
    .map_err(|e| friendly(e, &t.label))?;

    let mut fb: Option<Framebuffer> = None;
    let mut damage = Damage::default();
    let mut buttons = 0u8;
    let mut pointer = (0u16, 0u16);
    let mut tick = tokio::time::interval(Duration::from_millis(16));
    let send = |e: X11Event| {
        let vnc = vnc.clone();
        async move { vnc.input(e).await.map_err(|e| friendly(e, "")) }
    };

    let reason = loop {
        tokio::select! {
            ev = vnc.recv_event() => {
                let ev = ev.map_err(|e| friendly(e, &t.label))?;
                match ev {
                    VncEvent::SetResolution(s) => {
                        let first = fb.is_none();
                        fb = Some(Framebuffer::new(s.width, s.height));
                        damage.clear();
                        emit(Out::Event(if first {
                            DesktopEvent::Connected { width: s.width, height: s.height }
                        } else {
                            DesktopEvent::Resized { width: s.width, height: s.height }
                        }));
                    }
                    VncEvent::DesktopUpdate(u) => {
                        if let (Some(layout), Some(cur)) = (u.layout, fb.as_ref()) {
                            if (layout.width, layout.height) != (cur.width, cur.height) {
                                fb = Some(Framebuffer::new(layout.width, layout.height));
                                damage.clear();
                                emit(Out::Event(DesktopEvent::Resized { width: layout.width, height: layout.height }));
                            }
                        }
                    }
                    VncEvent::RawImage(r, data) => {
                        if let Some(fb) = fb.as_mut() {
                            if fb.put(&r, &data) {
                                damage.add(rect(&r));
                            }
                        }
                    }
                    VncEvent::Copy(dst, src) => {
                        if let Some(fb) = fb.as_mut() {
                            if fb.copy(&dst, &src) {
                                damage.add(rect(&dst));
                            }
                        }
                    }
                    VncEvent::SetCursor(r, data) => {
                        if r.width == 0 || r.height == 0 || data.is_empty() {
                            emit(Out::Binary(vec![MSG_CURSOR_HIDDEN]));
                        } else {
                            // For cursors the rect's position is the hotspot.
                            emit(Out::Binary(encode_cursor(r.width, r.height, r.x, r.y, &data)));
                        }
                    }
                    VncEvent::Error(e) => return Err(AppError::Other(format!("VNC: {e}"))),
                    _ => {}
                }
                if let Some(fb) = fb.as_ref() {
                    if let Some(bytes) = damage.flush(&fb.pixels(), false) {
                        emit(Out::Binary(bytes));
                    }
                }
            }
            msg = input.recv() => match msg {
                None | Some(DesktopInput::Close) => break "Disconnected.".to_string(),
                Some(DesktopInput::MouseMove { x, y }) => {
                    pointer = (x, y);
                    send(X11Event::PointerEvent((x, y, buttons).into())).await?;
                }
                Some(DesktopInput::MouseButton { button, down }) => {
                    let bit = button_bit(button);
                    if down { buttons |= bit } else { buttons &= !bit }
                    send(X11Event::PointerEvent((pointer.0, pointer.1, buttons).into())).await?;
                }
                Some(DesktopInput::Wheel { vertical, units }) => {
                    // Wheel "buttons": 4 up, 5 down, 6 left, 7 right; one click per notch.
                    let bit = match (vertical, units > 0) {
                        (true, true) => 8,
                        (true, false) => 16,
                        (false, true) => 64,
                        (false, false) => 32,
                    };
                    for _ in 0..(units.unsigned_abs() / 120).max(1) {
                        send(X11Event::PointerEvent((pointer.0, pointer.1, buttons | bit).into())).await?;
                        send(X11Event::PointerEvent((pointer.0, pointer.1, buttons).into())).await?;
                    }
                }
                Some(DesktopInput::Key { keysym: Some(k), down, .. }) => {
                    send(X11Event::KeyEvent((k, down).into())).await?;
                }
                Some(DesktopInput::Unicode { ch, down }) => {
                    let cp = ch as u32;
                    let keysym = if cp < 0x100 { cp } else { 0x0100_0000 | cp };
                    send(X11Event::KeyEvent((keysym, down).into())).await?;
                }
                Some(DesktopInput::ReleaseAll) => {
                    if buttons != 0 {
                        buttons = 0;
                        send(X11Event::PointerEvent((pointer.0, pointer.1, 0).into())).await?;
                    }
                }
                Some(DesktopInput::Resize { width, height }) => {
                    // Only honoured by servers with ExtendedDesktopSize; otherwise the UI scales.
                    let vnc = vnc.clone();
                    tokio::spawn(async move {
                        let _ = vnc.resize_desktop(width, height).await;
                    });
                }
                Some(DesktopInput::Key { .. }) => {}
            },
            _ = tick.tick() => {
                if let Some(fb) = fb.as_ref() {
                    if let Some(bytes) = damage.flush(&fb.pixels(), true) {
                        emit(Out::Binary(bytes));
                    }
                }
                // Ask for the next incremental update.
                send(X11Event::Refresh).await?;
            }
        }
    };
    let _ = vnc.close().await;
    Ok(reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framebuffer_copy_overlapping() {
        let mut fb = Framebuffer::new(4, 1);
        for (i, px) in fb.data.chunks_exact_mut(4).enumerate() {
            px[0] = i as u8;
        }
        let r = |x| vnc::Rect { x, y: 0, width: 3, height: 1 };
        assert!(fb.copy(&r(1), &r(0)));
        let reds: Vec<u8> = fb.data.chunks_exact(4).map(|p| p[0]).collect();
        assert_eq!(reds, vec![0, 0, 1, 2]);
        assert!(!fb.copy(&r(2), &r(0)), "out of bounds must be rejected");
    }

    /// Live test, skipped unless `BUNNYLINK_TEST_VNC=host:port:password` is set.
    #[test]
    fn live_session() {
        let Ok(spec) = std::env::var("BUNNYLINK_TEST_VNC") else { return };
        let p: Vec<&str> = spec.splitn(3, ':').collect();
        let target = VncTarget {
            label: "test".into(),
            address: p[0].into(),
            port: p[1].parse().unwrap(),
            password: Some(p[2].to_string().into()),
            jump: None,
        };
        let store = Arc::new(Store::in_memory().unwrap());
        let (tx, rx) = mpsc::unbounded_channel();
        let (otx, orx) = std::sync::mpsc::channel::<Out>();
        let worker = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            rt.block_on(run(target, store, rx, move |o| {
                let _ = otx.send(o);
            }));
        });
        let mut connected = None;
        let mut rects = 0;
        let mut rects_at_input = None;
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(12) {
            match orx.recv_timeout(Duration::from_millis(200)) {
                Ok(Out::Event(DesktopEvent::Connected { width, height })) => connected = Some((width, height)),
                Ok(Out::Event(DesktopEvent::Error { message })) => panic!("VNC error: {message}"),
                Ok(Out::Binary(b)) if b[0] == crate::display::MSG_RECTS => rects += 1,
                _ => {}
            }
            if rects_at_input.is_none() && start.elapsed() > Duration::from_secs(4) {
                rects_at_input = Some(rects);
                // Type "x" + Return into the focused terminal on the server.
                for k in [0x78u32, 0xff0d] {
                    tx.send(DesktopInput::Key { scancode: None, keysym: Some(k), down: true }).unwrap();
                    tx.send(DesktopInput::Key { scancode: None, keysym: Some(k), down: false }).unwrap();
                }
            }
        }
        tx.send(DesktopInput::Close).unwrap();
        worker.join().unwrap();
        assert!(connected.is_some(), "never connected");
        assert!(rects > 0, "no frames");
        assert!(rects > rects_at_input.unwrap(), "no updates after input");
    }

    #[test]
    fn wrong_password_is_reported() {
        let Ok(spec) = std::env::var("BUNNYLINK_TEST_VNC") else { return };
        let p: Vec<&str> = spec.splitn(3, ':').collect();
        let target = VncTarget {
            label: "test".into(),
            address: p[0].into(),
            port: p[1].parse().unwrap(),
            password: Some(String::from("not-it").into()),
            jump: None,
        };
        let (_tx, rx) = mpsc::unbounded_channel();
        let (otx, orx) = std::sync::mpsc::channel::<Out>();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(run(target, Arc::new(Store::in_memory().unwrap()), rx, move |o| {
            let _ = otx.send(o);
        }));
        let errors: Vec<String> = orx
            .try_iter()
            .filter_map(|o| match o {
                Out::Event(DesktopEvent::Error { message }) => Some(message),
                _ => None,
            })
            .collect();
        assert!(errors.iter().any(|m| m.contains("rejected the VNC password")), "{errors:?}");
    }
}
