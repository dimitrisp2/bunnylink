// SPDX-FileCopyrightText: 2026 BunnyCloud.IT
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial
//! RDP sessions via IronRDP. The decoded desktop is streamed to the UI as changed
//! rectangles of RGBA pixels; keyboard and mouse input comes back as RDP fast-path events.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use ironrdp::connector::connection_activation::ConnectionActivationState;
use ironrdp::connector::{self, ClientConnector, ConnectionResult, Credentials, DesktopSize};
use ironrdp::core::WriteBuf;
use ironrdp::displaycontrol::client::DisplayControlClient;
use ironrdp::displaycontrol::pdu::MonitorLayoutEntry;
use ironrdp::dvc::DrdynvcClient;
use ironrdp::graphics::image_processing::PixelFormat;
use ironrdp::input::{Database, MouseButton, MousePosition, Operation, Scancode, WheelRotations};
use ironrdp::pdu::gcc::KeyboardType;
use ironrdp::pdu::geometry::InclusiveRectangle;
use ironrdp::pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp::pdu::rdp::client_info::{PerformanceFlags, TimezoneInfo};
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{fast_path, ActiveStage, ActiveStageOutput};
use ironrdp_tokio::{single_sequence_step_read, split_tokio_framed, FramedWrite, TokioFramed};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

use crate::display::{
    encode_cursor, Damage, DesktopEvent as RdpEvent, DesktopInput as RdpInput, Out, Pixels, Rect, MSG_CURSOR_DEFAULT,
    MSG_CURSOR_HIDDEN,
};
use crate::error::{AppError, AppResult};
use crate::ssh;
use crate::store::Store;

pub struct RdpTarget {
    pub label: String,
    pub address: String,
    pub port: u16,
    pub username: String,
    pub password: crate::model::SecretString,
    pub width: u16,
    pub height: u16,
    /// SSH server to tunnel the connection through.
    pub jump: Option<ssh::Target>,
}

use crate::net::Stream;
type Framed = TokioFramed<Box<dyn Stream>>;

fn err(context: &str, e: impl std::fmt::Display) -> AppError {
    AppError::Other(format!("{context}: {e}"))
}

/// Splits `DOMAIN\user` and `user@domain` forms.
fn split_domain(username: &str) -> (String, Option<String>) {
    if let Some((d, u)) = username.split_once('\\') {
        // `MicrosoftAccount\user@live.com`: sspi rejects a domain next to a UPN, and
        // the UPN alone already names the account.
        if u.contains('@') {
            return (u.to_string(), None);
        }
        (u.to_string(), Some(d.to_string()))
    } else {
        (username.to_string(), None)
    }
}

fn config(t: &RdpTarget) -> connector::Config {
    let (username, domain) = split_domain(&t.username);
    connector::Config {
        // IronRDP takes a plain String; that copy is out of our hands.
        credentials: Credentials::UsernamePassword { username, password: t.password.to_string() },
        domain,
        enable_tls: true,
        enable_credssp: true,
        keyboard_type: KeyboardType::IbmEnhanced,
        keyboard_subtype: 0,
        keyboard_layout: 0,
        keyboard_functional_keys_count: 12,
        ime_file_name: String::new(),
        dig_product_id: String::new(),
        desktop_size: DesktopSize { width: t.width, height: t.height },
        bitmap: None,
        client_build: 0,
        client_name: "BunnyLink".to_owned(),
        client_dir: "C:\\Windows\\System32\\mstscax.dll".to_owned(),
        #[cfg(windows)]
        platform: MajorPlatformType::WINDOWS,
        #[cfg(target_os = "macos")]
        platform: MajorPlatformType::MACINTOSH,
        #[cfg(not(any(windows, target_os = "macos")))]
        platform: MajorPlatformType::UNIX,
        enable_server_pointer: true,
        pointer_software_rendering: false,
        request_data: None,
        autologon: false,
        enable_audio_playback: false,
        performance_flags: PerformanceFlags::default(),
        desktop_scale_factor: 0,
        hardware_id: None,
        license_cache: None,
        timezone_info: TimezoneInfo::default(),
    }
}

/// TCP to the server, directly or through an SSH jump host.
pub(crate) async fn open_stream(
    label: &str,
    address: &str,
    port: u16,
    jump: Option<&ssh::Target>,
    store: &Arc<Store>,
    emit: &(impl Fn(Out) + Send + Sync),
) -> AppResult<(Box<dyn Stream>, SocketAddr)> {
    let mut notices = Vec::new();
    let result = crate::net::connect(label, address, port, jump, store, &mut notices).await;
    for text in notices {
        emit(Out::Event(RdpEvent::Notice { text }));
    }
    result
}

/// Connects, checks the server's key (trust on first use) and completes the RDP handshake.
async fn connect(
    t: &RdpTarget,
    store: &Arc<Store>,
    emit: &(impl Fn(Out) + Send + Sync),
) -> AppResult<(ConnectionResult, Framed)> {
    let (tcp, client_addr) = open_stream(&t.label, &t.address, t.port, t.jump.as_ref(), store, emit).await?;

    let drdynvc = DrdynvcClient::new().with_dynamic_channel(DisplayControlClient::new(|_| Ok(Vec::new())));
    let mut connector = ClientConnector::new(config(t), client_addr).with_static_channel(drdynvc);

    let mut framed = TokioFramed::new(tcp);
    let should_upgrade = ironrdp_tokio::connect_begin(&mut framed, &mut connector)
        .await
        .map_err(|e| err("RDP negotiation failed", e))?;
    let (tcp, leftover) = framed.into_inner();
    let (tls, cert) = ironrdp_tls::upgrade(tcp, &t.address).await.map_err(|e| err("TLS handshake failed", e))?;
    let public_key = ironrdp_tls::extract_tls_server_public_key(&cert)
        .ok_or_else(|| AppError::Other("The server's certificate has no usable public key.".into()))?
        .to_vec();

    let fingerprint = format!(
        "SHA256:{}",
        Sha256::digest(&public_key).iter().map(|b| format!("{b:02x}")).collect::<String>()
    );
    let key_host = format!("rdp:{}", t.address);
    match store.known_host(&key_host, t.port)? {
        Some(known) if known == fingerprint => {}
        Some(known) => {
            return Err(AppError::Other(format!(
                "The certificate key for {} has changed. Expected {known}, got {fingerprint}. \
                 If the server was reinstalled, forget the old key and try again.",
                t.label
            )))
        }
        None => {
            store.save_known_host(&key_host, t.port, &fingerprint)?;
            emit(Out::Event(RdpEvent::Notice { text: format!("Trusted new certificate key for {}: {fingerprint}", t.label) }));
        }
    }

    let upgraded = ironrdp_tokio::mark_as_upgraded(should_upgrade, &mut connector);
    let stream: Box<dyn Stream> = Box::new(tls);
    let mut framed = TokioFramed::new_with_leftover(stream, leftover);
    // sspi switches NLA to Kerberos when it can find a KDC for the user's domain,
    // and Kerberos has to talk to that KDC over the network.
    let mut network_client = ironrdp_tokio::reqwest::ReqwestNetworkClient::new();
    let result = ironrdp_tokio::connect_finalize(
        upgraded,
        &mut framed,
        connector,
        t.address.clone().into(),
        public_key,
        Some(&mut network_client),
        None,
    )
    .await
    .map_err(|e| {
        // The report includes the sspi error underneath, e.g. SEC_E_LOGON_DENIED.
        let text = e.report().to_string();
        if text.to_lowercase().contains("logon") || text.contains("CredSSP") || text.contains("SEC_E") {
            AppError::Other(format!("{} rejected the login for \"{}\". ({text})", t.label, t.username))
        } else {
            err("RDP connection failed", text)
        }
    })?;
    Ok((result, framed))
}

fn rect(r: &InclusiveRectangle) -> Rect {
    Rect { x: r.left, y: r.top, w: r.right.saturating_sub(r.left) + 1, h: r.bottom.saturating_sub(r.top) + 1 }
}

fn pixels(image: &DecodedImage) -> Pixels<'_> {
    Pixels { data: image.data(), stride: image.stride(), width: image.width(), height: image.height() }
}

fn to_operations(input: RdpInput) -> Vec<Operation> {
    match input {
        RdpInput::MouseMove { x, y } => vec![Operation::MouseMove(MousePosition { x, y })],
        RdpInput::MouseButton { button, down } => match MouseButton::from_web_button(button) {
            Some(b) if down => vec![Operation::MouseButtonPressed(b)],
            Some(b) => vec![Operation::MouseButtonReleased(b)],
            None => vec![],
        },
        RdpInput::Wheel { vertical, units } => {
            vec![Operation::WheelRotations(WheelRotations { is_vertical: vertical, rotation_units: units })]
        }
        RdpInput::Key { scancode: Some(scancode), down, .. } => {
            let sc = Scancode::from_u16(scancode);
            vec![if down { Operation::KeyPressed(sc) } else { Operation::KeyReleased(sc) }]
        }
        RdpInput::Unicode { ch, down } => {
            vec![if down { Operation::UnicodeKeyPressed(ch) } else { Operation::UnicodeKeyReleased(ch) }]
        }
        _ => vec![],
    }
}

pub async fn run(
    target: RdpTarget,
    store: Arc<Store>,
    mut input: mpsc::UnboundedReceiver<RdpInput>,
    emit: impl Fn(Out) + Send + Sync + 'static,
) {
    emit(Out::Event(RdpEvent::Notice { text: format!("Connecting to {}…", target.label) }));
    let result = async {
        let (connection, framed) = connect(&target, &store, &emit).await?;
        session(connection, framed, &mut input, &emit).await
    }
    .await;
    match result {
        Ok(reason) => emit(Out::Event(RdpEvent::Closed { reason })),
        Err(e) => emit(Out::Event(RdpEvent::Error { message: e.to_string() })),
    }
}

async fn session(
    connection: ConnectionResult,
    framed: Framed,
    input: &mut mpsc::UnboundedReceiver<RdpInput>,
    emit: &(impl Fn(Out) + Send + Sync),
) -> AppResult<String> {
    let (mut reader, mut writer) = split_tokio_framed(framed);
    let size = connection.desktop_size;
    let mut image = DecodedImage::new(PixelFormat::RgbA32, size.width, size.height);
    let mut stage = ActiveStage::new(connection);
    let mut keys = Database::new();
    let mut damage = Damage::default();
    let mut tick = tokio::time::interval(Duration::from_millis(16));
    emit(Out::Event(RdpEvent::Connected { width: size.width, height: size.height }));

    loop {
        let outputs = tokio::select! {
            frame = reader.read_pdu() => {
                let (action, payload) = frame.map_err(|e| err("Connection lost", e))?;
                stage.process(&mut image, action, &payload).map_err(|e| err("RDP", e))?
            }
            msg = input.recv() => match msg {
                None | Some(RdpInput::Close) => {
                    let out = stage.graceful_shutdown().map_err(|e| err("RDP", e))?;
                    for o in out {
                        if let ActiveStageOutput::ResponseFrame(f) = o {
                            let _ = writer.write_all(&f).await;
                        }
                    }
                    return Ok("Disconnected.".into());
                }
                Some(RdpInput::Resize { width, height }) => {
                    let (w, h) = MonitorLayoutEntry::adjust_display_size(width.into(), height.into());
                    match stage.encode_resize(w, h, None, None) {
                        Some(frame) => vec![ActiveStageOutput::ResponseFrame(frame.map_err(|e| err("RDP", e))?)],
                        // The server cannot resize; the UI scales the picture instead.
                        None => vec![],
                    }
                }
                Some(RdpInput::ReleaseAll) => {
                    let events = keys.release_all();
                    stage.process_fastpath_input(&mut image, &events).map_err(|e| err("RDP", e))?
                }
                Some(other) => {
                    let events = keys.apply(to_operations(other));
                    stage.process_fastpath_input(&mut image, &events).map_err(|e| err("RDP", e))?
                }
            },
            _ = tick.tick() => {
                if let Some(bytes) = damage.flush(&pixels(&image), true) {
                    emit(Out::Binary(bytes));
                }
                continue;
            }
        };

        for out in outputs {
            match out {
                ActiveStageOutput::ResponseFrame(f) => writer.write_all(&f).await.map_err(|e| err("Connection lost", e))?,
                ActiveStageOutput::GraphicsUpdate(r) => damage.add(rect(&r)),
                ActiveStageOutput::PointerDefault => emit(Out::Binary(vec![MSG_CURSOR_DEFAULT])),
                ActiveStageOutput::PointerHidden => emit(Out::Binary(vec![MSG_CURSOR_HIDDEN])),
                ActiveStageOutput::PointerPosition { .. } => {}
                ActiveStageOutput::PointerBitmap(p) => {
                    emit(Out::Binary(encode_cursor(p.width, p.height, p.hotspot_x, p.hotspot_y, &p.bitmap_data)))
                }
                ActiveStageOutput::Terminate(reason) => return Ok(reason.description()),
                ActiveStageOutput::DeactivateAll(mut sequence) => {
                    // Deactivation-reactivation: happens on resize and some server-side changes.
                    let mut buf = WriteBuf::new();
                    loop {
                        let written = single_sequence_step_read(&mut reader, &mut *sequence, &mut buf)
                            .await
                            .map_err(|e| err("RDP reactivation", e))?;
                        if written.size().is_some() {
                            writer.write_all(buf.filled()).await.map_err(|e| err("Connection lost", e))?;
                        }
                        if let ConnectionActivationState::Finalized {
                            io_channel_id,
                            user_channel_id,
                            desktop_size,
                            enable_server_pointer,
                            pointer_software_rendering,
                        } = sequence.state
                        {
                            image = DecodedImage::new(PixelFormat::RgbA32, desktop_size.width, desktop_size.height);
                            stage.set_fastpath_processor(
                                fast_path::ProcessorBuilder {
                                    io_channel_id,
                                    user_channel_id,
                                    enable_server_pointer,
                                    pointer_software_rendering,
                                }
                                .build(),
                            );
                            stage.set_enable_server_pointer(enable_server_pointer);
                            damage.clear();
                            emit(Out::Event(RdpEvent::Resized { width: desktop_size.width, height: desktop_size.height }));
                            break;
                        }
                    }
                }
            }
        }
        if let Some(bytes) = damage.flush(&pixels(&image), false) {
            emit(Out::Binary(bytes));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn domain_forms() {
        assert_eq!(split_domain("CORP\\admin"), ("admin".into(), Some("CORP".into())));
        assert_eq!(split_domain("admin@corp.local"), ("admin@corp.local".into(), None));
    }

    /// Live test against a real RDP server, skipped unless
    /// `BUNNYLINK_TEST_RDP=host:port:user:password` is set. Writes the final frame to
    /// `$BUNNYLINK_TEST_RDP_PPM` when given.
    #[test]
    fn live_session() {
        let Ok(spec) = std::env::var("BUNNYLINK_TEST_RDP") else { return };
        let p: Vec<&str> = spec.splitn(4, ':').collect();
        let target = RdpTarget {
            label: "test".into(),
            address: p[0].into(),
            port: p[1].parse().unwrap(),
            username: p[2].into(),
            password: p[3].to_string().into(),
            width: 1024,
            height: 768,
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

        let (mut w, mut h) = (0usize, 0usize);
        let mut frame = Vec::new();
        let mut rects = 0;
        let mut notices = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut sent_input = false;
        let mut first_frame: Option<Instant> = None;
        let mut rects_at_input = 0;
        while Instant::now() < deadline {
            match orx.recv_timeout(Duration::from_millis(200)) {
                Ok(Out::Event(RdpEvent::Connected { width, height } | RdpEvent::Resized { width, height })) => {
                    (w, h) = (width as usize, height as usize);
                    frame = vec![0u8; w * h * 4];
                }
                Ok(Out::Event(RdpEvent::Notice { text })) => notices.push(text),
                Ok(Out::Event(RdpEvent::Error { message })) => panic!("RDP error: {message}"),
                Ok(Out::Event(_)) => {}
                Ok(Out::Binary(b)) if b[0] == crate::display::MSG_RECTS => {
                    let n = u16::from_le_bytes([b[1], b[2]]);
                    let mut at = 3;
                    for _ in 0..n {
                        let v: Vec<usize> = (0..4).map(|i| u16::from_le_bytes([b[at + i * 2], b[at + i * 2 + 1]]) as usize).collect();
                        at += 8;
                        for row in 0..v[3] {
                            let dst = ((v[1] + row) * w + v[0]) * 4;
                            frame[dst..dst + v[2] * 4].copy_from_slice(&b[at..at + v[2] * 4]);
                            at += v[2] * 4;
                        }
                        rects += 1;
                    }
                }
                Ok(Out::Binary(_)) => {}
                Err(_) => {}
            }
            if rects > 0 && first_frame.is_none() {
                first_frame = Some(Instant::now());
            }
            // Wait for the screen to settle so later updates must come from our input.
            if !sent_input && first_frame.is_some_and(|t| t.elapsed() > Duration::from_secs(4)) {
                sent_input = true;
                rects_at_input = rects;
                eprintln!("rects before input: {rects}");
                // Type into whatever has focus and move the mouse; the session must stay healthy.
                // "h", "i"
                for sc in [0x23u16, 0x17] {
                    tx.send(RdpInput::Key { scancode: Some(sc), keysym: None, down: true }).unwrap();
                    tx.send(RdpInput::Key { scancode: Some(sc), keysym: None, down: false }).unwrap();
                }
                tx.send(RdpInput::MouseMove { x: 200, y: 200 }).unwrap();
                tx.send(RdpInput::MouseButton { button: 0, down: true }).unwrap();
                tx.send(RdpInput::MouseButton { button: 0, down: false }).unwrap();
            }
        }
        tx.send(RdpInput::Close).unwrap();
        worker.join().unwrap();

        eprintln!("rects after input: {}", rects - rects_at_input);
        assert!(rects > rects_at_input, "no screen updates after input");
        assert!(notices.iter().any(|n| n.contains("Trusted new certificate key")), "{notices:?}");
        assert!(w > 0 && rects > 0, "no frames received");
        let lit = frame.chunks_exact(4).filter(|p| p[0] | p[1] | p[2] != 0).count();
        assert!(lit > w * h / 4, "frame mostly empty ({lit} lit pixels)");
        if let Ok(path) = std::env::var("BUNNYLINK_TEST_RDP_PPM") {
            let mut ppm = format!("P6 {w} {h} 255\n").into_bytes();
            for px in frame.chunks_exact(4) {
                ppm.extend_from_slice(&px[..3]);
            }
            std::fs::write(path, ppm).unwrap();
        }
    }
}
