// SPDX-FileCopyrightText: 2026 BunnyCloud.IT
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial
//! Telnet client: option negotiation (echo, suppress-go-ahead, terminal type, window
//! size), IAC escaping, and optional automatic login at the usual prompts.

use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::error::AppResult;
use crate::ssh::{self, TermEvent, TermInput};
use crate::store::Store;

pub struct TelnetTarget {
    pub label: String,
    pub address: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub jump: Option<ssh::Target>,
}

const IAC: u8 = 255;
const DONT: u8 = 254;
const DO: u8 = 253;
const WONT: u8 = 252;
const WILL: u8 = 251;
const SB: u8 = 250;
const SE: u8 = 240;

const ECHO: u8 = 1;
const SGA: u8 = 3;
const TTYPE: u8 = 24;
const NAWS: u8 = 31;

enum State {
    Data,
    Iac,
    Verb(u8),
    Sub,
    SubIac,
}

/// Decodes the server's byte stream, producing terminal data and protocol replies.
pub struct Telnet {
    state: State,
    sub: Vec<u8>,
    pub naws: bool,
    cols: u16,
    rows: u16,
    answered: Vec<(u8, u8)>,
}

impl Telnet {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self { state: State::Data, sub: vec![], naws: false, cols, rows, answered: vec![] }
    }

    fn reply(&mut self, out: &mut Vec<u8>, verb: u8, opt: u8) {
        // Answer each request once to avoid negotiation loops.
        if self.answered.contains(&(verb, opt)) {
            return;
        }
        self.answered.push((verb, opt));
        out.extend_from_slice(&[IAC, verb, opt]);
    }

    pub fn naws_bytes(&self) -> Vec<u8> {
        let mut out = vec![IAC, SB, NAWS];
        for b in self.cols.to_be_bytes().into_iter().chain(self.rows.to_be_bytes()) {
            out.push(b);
            if b == IAC {
                out.push(IAC);
            }
        }
        out.extend_from_slice(&[IAC, SE]);
        out
    }

    pub fn resize(&mut self, cols: u16, rows: u16) -> Option<Vec<u8>> {
        self.cols = cols;
        self.rows = rows;
        self.naws.then(|| self.naws_bytes())
    }

    /// Returns (bytes for the terminal, bytes to send back to the server).
    pub fn feed(&mut self, input: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let mut data = Vec::with_capacity(input.len());
        let mut reply = Vec::new();
        for &b in input {
            self.state = match std::mem::replace(&mut self.state, State::Data) {
                State::Data if b == IAC => State::Iac,
                State::Data => {
                    data.push(b);
                    State::Data
                }
                State::Iac => match b {
                    IAC => {
                        data.push(IAC);
                        State::Data
                    }
                    WILL | WONT | DO | DONT => State::Verb(b),
                    SB => {
                        self.sub.clear();
                        State::Sub
                    }
                    _ => State::Data, // NOP, GA, etc.
                },
                State::Verb(verb) => {
                    match (verb, b) {
                        (WILL, ECHO | SGA) => self.reply(&mut reply, DO, b),
                        (WILL, _) => self.reply(&mut reply, DONT, b),
                        (DO, TTYPE | SGA) => self.reply(&mut reply, WILL, b),
                        (DO, NAWS) => {
                            self.reply(&mut reply, WILL, NAWS);
                            self.naws = true;
                            reply.extend(self.naws_bytes());
                        }
                        (DO, _) => self.reply(&mut reply, WONT, b),
                        _ => {}
                    }
                    State::Data
                }
                State::Sub if b == IAC => State::SubIac,
                State::Sub => {
                    self.sub.push(b);
                    State::Sub
                }
                State::SubIac if b == SE => {
                    // Terminal type request: IAC SB TTYPE SEND IAC SE.
                    if self.sub == [TTYPE, 1] {
                        reply.extend_from_slice(&[IAC, SB, TTYPE, 0]);
                        reply.extend_from_slice(b"XTERM-256COLOR");
                        reply.extend_from_slice(&[IAC, SE]);
                    }
                    State::Data
                }
                State::SubIac => {
                    self.sub.push(b);
                    State::Sub
                }
            };
        }
        (data, reply)
    }
}

/// Escapes IAC and turns a bare CR (Enter) into CR NUL as the NVT requires.
pub fn encode_input(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len() + 4);
    for (i, &b) in input.iter().enumerate() {
        match b {
            IAC => out.extend_from_slice(&[IAC, IAC]),
            b'\r' if input.get(i + 1) != Some(&b'\n') => out.extend_from_slice(b"\r\0"),
            _ => out.push(b),
        }
    }
    out
}

/// Watches output for login prompts and answers each one once.
struct AutoLogin {
    username: Option<String>,
    password: Option<String>,
    tail: String,
}

impl AutoLogin {
    fn check(&mut self, data: &[u8]) -> Option<String> {
        self.tail.push_str(&String::from_utf8_lossy(data).to_lowercase());
        if self.tail.len() > 256 {
            let cut = self.tail.len() - 256;
            let cut = (cut..self.tail.len()).find(|&i| self.tail.is_char_boundary(i)).unwrap_or(0);
            self.tail.drain(..cut);
        }
        let end = self.tail.trim_end();
        let answer = if end.ends_with("password:") {
            self.password.take()
        } else if end.ends_with("login:") || end.ends_with("username:") || end.ends_with("user name:") {
            self.username.take()
        } else {
            None
        };
        if answer.is_some() {
            self.tail.clear();
        }
        answer
    }
}

pub async fn run(
    t: TelnetTarget,
    store: Arc<Store>,
    cols: u32,
    rows: u32,
    mut input: mpsc::UnboundedReceiver<TermInput>,
    emit: impl Fn(TermEvent) + Send + 'static,
) {
    emit(TermEvent::Notice { text: format!("Connecting to {}…", t.label) });
    let result: AppResult<()> = async {
        let mut notices = Vec::new();
        let conn = crate::net::connect(&t.label, &t.address, t.port, t.jump.as_ref(), &store, &mut notices).await;
        for text in notices {
            emit(TermEvent::Notice { text });
        }
        let (stream, _) = conn?;
        emit(TermEvent::Notice { text: "Telnet is not encrypted. Prefer SSH where you can.".into() });
        let (mut rd, mut wr) = tokio::io::split(stream);
        let mut telnet = Telnet::new(cols as u16, rows as u16);
        let mut login = AutoLogin { username: t.username.clone(), password: t.password.clone(), tail: String::new() };
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            tokio::select! {
                n = rd.read(&mut buf) => {
                    let n = n?;
                    if n == 0 {
                        break;
                    }
                    let (data, reply) = telnet.feed(&buf[..n]);
                    if !reply.is_empty() {
                        wr.write_all(&reply).await?;
                    }
                    if !data.is_empty() {
                        if let Some(answer) = login.check(&data) {
                            wr.write_all(&encode_input(format!("{answer}\r").as_bytes())).await?;
                        }
                        emit(TermEvent::Data { data });
                    }
                }
                msg = input.recv() => match msg {
                    Some(TermInput::Data(d)) => wr.write_all(&encode_input(&d)).await?,
                    Some(TermInput::Resize { cols, rows }) => {
                        if let Some(b) = telnet.resize(cols as u16, rows as u16) {
                            wr.write_all(&b).await?;
                        }
                    }
                    Some(TermInput::Close) | None => break,
                },
            }
        }
        Ok(())
    }
    .await;
    match result {
        Ok(()) => emit(TermEvent::Exit { code: None }),
        Err(e) => emit(TermEvent::Error { message: e.to_string() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiation() {
        let mut t = Telnet::new(80, 24);
        let (data, reply) = t.feed(&[b'h', IAC, DO, NAWS, IAC, WILL, ECHO, IAC, DO, 39, b'i', IAC, IAC]);
        assert_eq!(data, vec![b'h', b'i', IAC]);
        assert_eq!(
            reply,
            vec![IAC, WILL, NAWS, IAC, SB, NAWS, 0, 80, 0, 24, IAC, SE, IAC, DO, ECHO, IAC, WONT, 39]
        );
        // The same request is not answered twice.
        assert_eq!(t.feed(&[IAC, WILL, ECHO]).1, Vec::<u8>::new());
        // Terminal type, split across reads.
        let (_, r1) = t.feed(&[IAC, SB, TTYPE]);
        let (_, r2) = t.feed(&[1, IAC, SE]);
        assert!(r1.is_empty());
        assert_eq!(&r2[..4], &[IAC, SB, TTYPE, 0]);
        assert_eq!(t.resize(100, 40).unwrap(), vec![IAC, SB, NAWS, 0, 100, 0, 40, IAC, SE]);
    }

    #[test]
    fn input_encoding() {
        assert_eq!(encode_input(b"ls\r"), b"ls\r\0");
        assert_eq!(encode_input(b"a\r\nb"), b"a\r\nb");
        assert_eq!(encode_input(&[IAC]), vec![IAC, IAC]);
    }

    #[test]
    fn auto_login() {
        let mut a = AutoLogin { username: Some("admin".into()), password: Some("pw".into()), tail: String::new() };
        assert_eq!(a.check(b"Welcome\r\nrouter "), None);
        assert_eq!(a.check(b"login: "), Some("admin".into()));
        assert_eq!(a.check(b"Password: "), Some("pw".into()));
        assert_eq!(a.check(b"login: "), None, "answered only once");
    }

    /// Live test, skipped unless `BUNNYLINK_TEST_TELNET=host:port:user:password` is set.
    /// Logs in automatically and runs a command.
    #[test]
    fn live_session() {
        let Ok(spec) = std::env::var("BUNNYLINK_TEST_TELNET") else { return };
        let p: Vec<&str> = spec.splitn(4, ':').collect();
        let t = TelnetTarget {
            label: "test".into(),
            address: p[0].into(),
            port: p[1].parse().unwrap(),
            username: Some(p[2].into()),
            password: Some(p[3].into()),
            jump: None,
        };
        let (tx, rx) = mpsc::unbounded_channel();
        let (otx, orx) = std::sync::mpsc::channel::<TermEvent>();
        let worker = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            rt.block_on(run(t, Arc::new(Store::in_memory().unwrap()), 80, 24, rx, move |e| {
                let _ = otx.send(e);
            }));
        });
        let mut seen = String::new();
        let mut sent = false;
        let start = std::time::Instant::now();
        while !seen.contains("telnet-42") && start.elapsed() < std::time::Duration::from_secs(15) {
            match orx.recv_timeout(std::time::Duration::from_millis(200)) {
                Ok(TermEvent::Data { data }) => seen.push_str(&String::from_utf8_lossy(&data)),
                Ok(TermEvent::Error { message }) => panic!("{message}"),
                _ => {}
            }
            // Once logged in (a shell prompt appears), run a command.
            if !sent && (seen.ends_with("$ ") || seen.trim_end().ends_with('$')) {
                sent = true;
                tx.send(TermInput::Data(b"echo telnet-$((40+2))\r".to_vec())).unwrap();
            }
        }
        tx.send(TermInput::Close).unwrap();
        worker.join().unwrap();
        assert!(seen.contains("telnet-42"), "output: {seen}");
    }
}
