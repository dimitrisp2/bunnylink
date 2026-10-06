// SPDX-FileCopyrightText: 2026 BunnyCloud.IT
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial
//! Serial console sessions (USB-serial adapters, COM ports, console cables).

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serialport::{DataBits, FlowControl, Parity, StopBits};
use tokio::sync::mpsc;

use crate::error::{AppError, AppResult};
use crate::model::SerialSettings;
use crate::ssh::{TermEvent, TermInput};

pub struct SerialTarget {
    pub device: String,
    pub settings: SerialSettings,
}

pub fn describe(s: &SerialSettings) -> String {
    let parity = match s.parity.as_str() {
        "odd" => "O",
        "even" => "E",
        _ => "N",
    };
    format!("{} {}{}{}", s.baud, s.data_bits, parity, s.stop_bits)
}

fn open(t: &SerialTarget) -> AppResult<Box<dyn serialport::SerialPort>> {
    let s = &t.settings;
    serialport::new(&t.device, s.baud)
        .data_bits(match s.data_bits {
            5 => DataBits::Five,
            6 => DataBits::Six,
            7 => DataBits::Seven,
            _ => DataBits::Eight,
        })
        .parity(match s.parity.as_str() {
            "odd" => Parity::Odd,
            "even" => Parity::Even,
            _ => Parity::None,
        })
        .stop_bits(if s.stop_bits == 2 { StopBits::Two } else { StopBits::One })
        .flow_control(match s.flow.as_str() {
            "hardware" => FlowControl::Hardware,
            "software" => FlowControl::Software,
            _ => FlowControl::None,
        })
        .timeout(Duration::from_millis(100))
        .open()
        .map_err(|e| AppError::Other(format!("Could not open {}: {e}", t.device)))
}

/// Ports the OS knows about, for the host editor's suggestions.
pub fn ports() -> Vec<String> {
    let mut v: Vec<String> = serialport::available_ports()
        .map(|p| p.into_iter().map(|p| p.port_name).collect())
        .unwrap_or_default();
    v.sort();
    v
}

pub async fn run(
    t: SerialTarget,
    mut input: mpsc::UnboundedReceiver<TermInput>,
    emit: impl Fn(TermEvent) + Send + Sync + 'static,
) {
    emit(TermEvent::Notice { text: format!("Opening {} ({})…", t.device, describe(&t.settings)) });
    let port = match open(&t) {
        Ok(p) => p,
        Err(e) => return emit(TermEvent::Error { message: e.to_string() }),
    };
    let mut writer = match port.try_clone() {
        Ok(w) => w,
        Err(e) => return emit(TermEvent::Error { message: e.to_string() }),
    };
    emit(TermEvent::Notice { text: "Connected. Press Enter if the device shows nothing yet.".into() });

    // Reads block, so they run on their own thread.
    let emit = Arc::new(emit);
    let stop = Arc::new(AtomicBool::new(false));
    let (done_tx, mut done_rx) = mpsc::unbounded_channel::<Option<String>>();
    {
        let emit = emit.clone();
        let stop = stop.clone();
        let mut reader = port;
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            let err = loop {
                if stop.load(Ordering::Relaxed) {
                    break None;
                }
                match reader.read(&mut buf) {
                    Ok(0) => {}
                    Ok(n) => emit(TermEvent::Data { data: buf[..n].to_vec() }),
                    Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                    Err(e) => break Some(format!("The serial port stopped working: {e}")),
                }
            };
            let _ = done_tx.send(err);
        });
    }

    let error = loop {
        tokio::select! {
            msg = input.recv() => match msg {
                Some(TermInput::Data(d)) => {
                    if let Err(e) = writer.write_all(&d) {
                        break Some(format!("Could not write to {}: {e}", t.device));
                    }
                }
                Some(TermInput::Resize { .. }) => {}
                Some(TermInput::Close) | None => break None,
            },
            err = done_rx.recv() => break err.flatten(),
        }
    };
    stop.store(true, Ordering::Relaxed);
    match error {
        Some(message) => emit(TermEvent::Error { message }),
        None => emit(TermEvent::Exit { code: None }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live test, skipped unless `BUNNYLINK_TEST_SERIAL=/dev/ttyX` points at a device
    /// with a shell on the other end (e.g. a socat pty pair).
    #[test]
    fn live_session() {
        let Ok(device) = std::env::var("BUNNYLINK_TEST_SERIAL") else { return };
        let t = SerialTarget { device, settings: SerialSettings::default() };
        let (tx, rx) = mpsc::unbounded_channel();
        let (otx, orx) = std::sync::mpsc::channel::<TermEvent>();
        let worker = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            rt.block_on(run(t, rx, move |e| {
                let _ = otx.send(e);
            }));
        });
        std::thread::sleep(Duration::from_millis(500));
        tx.send(TermInput::Data(b"echo serial-$((20+22))\r".to_vec())).unwrap();
        let mut seen = String::new();
        let start = std::time::Instant::now();
        while !seen.contains("serial-42") && start.elapsed() < Duration::from_secs(5) {
            match orx.recv_timeout(Duration::from_millis(200)) {
                Ok(TermEvent::Data { data }) => seen.push_str(&String::from_utf8_lossy(&data)),
                Ok(TermEvent::Error { message }) => panic!("{message}"),
                _ => {}
            }
        }
        tx.send(TermInput::Close).unwrap();
        worker.join().unwrap();
        assert!(seen.contains("serial-42"), "output: {seen}");
    }
}
