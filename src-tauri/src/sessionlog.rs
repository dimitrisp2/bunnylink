// SPDX-FileCopyrightText: 2026 BunnyCloud.IT
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial
//! Terminal session logs. The `.log` file keeps everything the terminal showed, escape
//! codes included, so the built-in viewer can replay it with colours; when the session
//! ends a `.txt` copy with the codes removed is written next to it.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::Serialize;

use crate::error::{AppError, AppResult};
use crate::ssh::TermEvent;

/// The log of one terminal session; finished (flushed, clean copy written) when dropped.
pub struct SessionLog {
    path: PathBuf,
    out: BufWriter<File>,
}

impl SessionLog {
    /// Starts `<folder>/<host>_<date>_<time>.log`, creating the folder if needed.
    pub fn create(folder: &Path, host: &str, protocol: &str) -> std::io::Result<Self> {
        std::fs::create_dir_all(folder)?;
        let now = chrono::Local::now();
        let stem = format!("{}_{}", file_safe(host), now.format("%Y-%m-%d_%H-%M-%S"));
        // Two sessions to one host in the same second get a counter.
        let mut path = folder.join(format!("{stem}.log"));
        let mut n = 1;
        while path.exists() {
            n += 1;
            path = folder.join(format!("{stem}_{n}.log"));
        }
        let mut log = SessionLog { out: BufWriter::new(File::create(&path)?), path };
        log.dim(&format!("BunnyLink session log: {host} ({protocol}), {}", now.format("%Y-%m-%d %H:%M:%S %:z")));
        Ok(log)
    }

    /// Writes an event the way the terminal shows it.
    pub fn record(&mut self, event: &TermEvent) {
        match event {
            TermEvent::Data { data } => self.write(data),
            TermEvent::Prompt { text } => self.write(text.as_bytes()),
            TermEvent::Notice { text } => self.dim(text),
            TermEvent::Exit { code } => {
                let code = code.map(|c| format!(" (exit {c})")).unwrap_or_default();
                self.write(b"\r\n");
                self.dim(&format!("Session ended{code}."));
            }
            TermEvent::Error { message } => {
                self.write(format!("\r\n\x1b[31m{message}\x1b[0m\r\n").as_bytes());
            }
        }
        // Kept on disk as it arrives, so a crash loses nothing that was shown.
        let _ = self.out.flush();
    }

    fn write(&mut self, data: &[u8]) {
        let _ = self.out.write_all(data);
    }

    fn dim(&mut self, text: &str) {
        self.write(format!("\x1b[2m{text}\x1b[0m\r\n").as_bytes());
    }
}

impl Drop for SessionLog {
    fn drop(&mut self) {
        let _ = self.out.flush();
        if let Ok(raw) = std::fs::read(&self.path) {
            let _ = std::fs::write(self.path.with_extension("txt"), clean(&raw));
        }
    }
}

/// Host names become part of a file name; anything Windows rejects becomes `_`.
fn file_safe(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c })
        .collect();
    let s = s.trim().trim_end_matches('.').to_string();
    if s.is_empty() { "session".into() } else { s.chars().take(60).collect() }
}

/// Plain text from raw terminal output: escape codes are removed and carriage returns,
/// backspaces and line erases are applied, so progress bars and edited command lines
/// read as they ended up. Full-screen programs (editors, top) don't survive this; the
/// viewer replays those.
pub fn clean(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let mut chars = text.chars().peekable();
    let mut out = String::with_capacity(raw.len());
    let mut line: Vec<char> = Vec::new();
    let mut col = 0usize;

    let put = |line: &mut Vec<char>, col: &mut usize, c: char| {
        if *col < line.len() {
            line[*col] = c;
        } else {
            line.resize(*col, ' ');
            line.push(c);
        }
        *col += 1;
    };
    let end_line = |out: &mut String, line: &mut Vec<char>, col: &mut usize| {
        let s: String = line.iter().collect();
        out.push_str(s.trim_end());
        out.push('\n');
        line.clear();
        *col = 0;
    };

    while let Some(c) = chars.next() {
        match c {
            '\x1b' => match chars.next() {
                // CSI: parameters, then a final byte from @ to ~.
                Some('[') => {
                    let mut params = String::new();
                    let mut fin = None;
                    for p in chars.by_ref() {
                        if ('@'..='~').contains(&p) {
                            fin = Some(p);
                            break;
                        }
                        params.push(p);
                    }
                    let n = params.trim_start_matches('?').split(';').next().and_then(|v| v.parse::<usize>().ok());
                    match fin {
                        // Erase in line: 0 (default) to the end, 1 to the start, 2 all.
                        Some('K') => match n.unwrap_or(0) {
                            0 => line.truncate(col),
                            1 => line.iter_mut().take(col + 1).for_each(|c| *c = ' '),
                            _ => line.clear(),
                        },
                        Some('C') => col += n.unwrap_or(1).max(1),
                        Some('D') => col = col.saturating_sub(n.unwrap_or(1).max(1)),
                        Some('G') => col = n.unwrap_or(1).max(1) - 1,
                        _ => {}
                    }
                }
                // OSC (window titles and the like): up to BEL or ESC \.
                Some(']') => {
                    while let Some(p) = chars.next() {
                        if p == '\x07' || (p == '\x1b' && chars.next_if_eq(&'\\').is_some()) {
                            break;
                        }
                    }
                }
                // DCS, SOS, PM, APC strings: up to ESC \.
                Some('P' | 'X' | '^' | '_') => {
                    while let Some(p) = chars.next() {
                        if p == '\x1b' && chars.next_if_eq(&'\\').is_some() {
                            break;
                        }
                    }
                }
                // Character set choices take one more character.
                Some('(' | ')' | '*' | '+' | '-' | '.' | '/' | '#' | '%') => {
                    chars.next();
                }
                _ => {}
            },
            '\r' => col = 0,
            '\n' => end_line(&mut out, &mut line, &mut col),
            '\x08' => col = col.saturating_sub(1),
            '\t' => {
                let next = (col / 8 + 1) * 8;
                while col < next {
                    put(&mut line, &mut col, ' ');
                }
            }
            c if c.is_control() => {}
            c => put(&mut line, &mut col, c),
        }
    }
    if !line.is_empty() {
        end_line(&mut out, &mut line, &mut col);
    }
    out
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogFile {
    pub name: String,
    pub path: String,
    pub size: u64,
    /// Seconds since the Unix epoch.
    pub modified: i64,
    /// The plain-text copy exists.
    pub clean_path: Option<String>,
}

/// The session logs in `folder`, newest first. A missing folder has none.
pub fn list(folder: &Path) -> AppResult<Vec<LogFile>> {
    let entries = match std::fs::read_dir(folder) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(AppError::Invalid(format!("Could not read {}: {e}", folder.display()))),
    };
    let mut logs: Vec<LogFile> = entries
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("log")))
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            let path = e.path();
            let txt = path.with_extension("txt");
            Some(LogFile {
                name: e.file_name().to_string_lossy().into_owned(),
                size: meta.len(),
                modified: meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs() as i64,
                clean_path: txt.exists().then(|| txt.to_string_lossy().into_owned()),
                path: path.to_string_lossy().into_owned(),
            })
        })
        .collect();
    logs.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| b.name.cmp(&a.name)));
    Ok(logs)
}

/// The path of a log named in the list, refusing anything that isn't a plain `.log`
/// file name in `folder`.
pub fn resolve(folder: &Path, name: &str) -> AppResult<PathBuf> {
    let plain = !name.contains(['/', '\\']) &&Path::new(name).file_name().is_some_and(|f| f == name);
    if !plain || !name.to_ascii_lowercase().ends_with(".log") {
        return Err(AppError::Invalid("Not a session log.".into()));
    }
    Ok(folder.join(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_strips_codes_and_applies_edits() {
        let raw = b"\x1b]0;user@web: ~\x07\x1b[01;32muser@web\x1b[00m:~$ lss\x08 \x08\r\n\
                    file.txt\r\n\
                    10%\r50%\r100% done\r\n\
                    abc\x1b[2Dx\x1b[K\r\n\
                    a\tb\x1b(B\r\n\
                    tail";
        assert_eq!(clean(raw), "user@web:~$ ls\nfile.txt\n100% done\nax\na       b\ntail\n");
    }

    #[test]
    fn records_events_like_the_terminal_and_writes_a_clean_copy() {
        let dir = std::env::temp_dir().join(format!("bunnylink-log-{}", uuid::Uuid::new_v4()));
        let path = {
            let mut log = SessionLog::create(&dir, "web:01/prod", "ssh").unwrap();
            log.record(&TermEvent::Notice { text: "Connecting to web01…".into() });
            log.record(&TermEvent::Data { data: b"\x1b[1mhello\x1b[0m\r\n".to_vec() });
            log.record(&TermEvent::Exit { code: Some(0) });
            log.path.clone()
        };
        assert!(path.file_name().unwrap().to_string_lossy().starts_with("web_01_prod_"));
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\x1b[1mhello"));
        let txt = std::fs::read_to_string(path.with_extension("txt")).unwrap();
        let lines: Vec<_> = txt.lines().collect();
        assert!(lines[0].starts_with("BunnyLink session log: web:01/prod (ssh), "));
        assert_eq!(&lines[1..], ["Connecting to web01…", "hello", "", "Session ended (exit 0)."]);

        let listed = list(&dir).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].clean_path.is_some());
        assert_eq!(resolve(&dir, &listed[0].name).unwrap(), path);
        for bad in ["../x.log", "x.txt", "a/b.log", r"a\b.log"] {
            assert!(resolve(&dir, bad).is_err(), "{bad}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
