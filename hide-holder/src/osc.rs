// Derived from Herdr v0.9.5 (https://github.com/herdrdev/herdr, commit
// 2ecf19a), src/pane/osc.rs: `OscStreamCollector`, the OSC 0/2 title and
// OSC 9 progress of `AgentOscStateTracker`, `sanitize_agent_osc_string`, and
// `parse_reported_cwd` with `parse_file_uri_cwd` and `percent_decode_utf8`.
// Copyright 2025 herdr contributors. Licensed under the Apache License,
// Version 2.0; see NOTICE at the repository root.
// Changed by Hide: the tracker also counts OSC 133;A prompt marks, keeps the
// title the terminal shows apart from the title and progress the screen rules
// read, and keeps no pane id or log; the default color and debug trackers are
// not taken.

//! What a pane's program says of itself in OSC sequences, read from its raw
//! output beside the parser: its title (OSC 0 and 2), its progress (OSC 9),
//! the prompts its shell marks (OSC 133;A), and how a reported folder (OSC 7,
//! OSC 9;9, which the parser hands over) reads as a path.

use std::path::PathBuf;

/// The longest OSC body the collector keeps; a longer one is dropped whole.
const MAX_BODY_BYTES: usize = 4096;

/// The most characters a title or a progress payload keeps: they are the
/// program's own text.
pub const MAX_CHARS: usize = 256;

/// Collects complete OSC bodies from a raw byte stream. Consumers receive only
/// bodies, keeping the framing state machine independent from OSC commands.
#[derive(Debug, Default)]
struct OscStreamCollector {
    state: OscStreamState,
    body: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum OscStreamState {
    #[default]
    Ground,
    Escape,
    Body,
    BodyEscape,
    IgnoringString,
    IgnoringStringEscape,
    Discarding,
    DiscardingEscape,
}

fn is_ignored_string_intro(byte: u8) -> bool {
    matches!(byte, b'P' | b'_' | b'^' | b'X')
}

impl OscStreamCollector {
    fn observe(&mut self, bytes: &[u8], mut receive: impl FnMut(&[u8])) {
        let mut cursor = 0;
        while cursor < bytes.len() {
            if matches!(
                self.state,
                OscStreamState::Ground | OscStreamState::IgnoringString
            ) {
                let Some(offset) = bytes[cursor..].iter().position(|&byte| byte == 0x1b) else {
                    break;
                };
                cursor += offset;
            }
            let byte = bytes[cursor];
            cursor += 1;
            match self.state {
                OscStreamState::Ground => {
                    if byte == 0x1b {
                        self.state = OscStreamState::Escape;
                    }
                }
                OscStreamState::Escape => match byte {
                    b']' => {
                        self.body.clear();
                        self.state = OscStreamState::Body;
                    }
                    0x1b => self.state = OscStreamState::Escape,
                    byte if is_ignored_string_intro(byte) => {
                        self.state = OscStreamState::IgnoringString;
                    }
                    _ => self.state = OscStreamState::Ground,
                },
                OscStreamState::Body => match byte {
                    0x07 => self.finish(&mut receive),
                    0x1b => self.state = OscStreamState::BodyEscape,
                    _ => self.push(byte),
                },
                OscStreamState::BodyEscape => match byte {
                    b'\\' => self.finish(&mut receive),
                    0x07 => {
                        self.push(0x1b);
                        if matches!(self.state, OscStreamState::Body) {
                            self.finish(&mut receive);
                        } else {
                            self.state = OscStreamState::Ground;
                        }
                    }
                    0x1b => {
                        self.push(0x1b);
                        self.state = match self.state {
                            OscStreamState::Body => OscStreamState::BodyEscape,
                            OscStreamState::Discarding => OscStreamState::DiscardingEscape,
                            state => state,
                        };
                    }
                    _ => {
                        self.push(0x1b);
                        if matches!(self.state, OscStreamState::Body) {
                            self.push(byte);
                        }
                    }
                },
                OscStreamState::IgnoringString => {
                    if byte == 0x1b {
                        self.state = OscStreamState::IgnoringStringEscape;
                    }
                }
                OscStreamState::IgnoringStringEscape => {
                    if byte == b'\\' {
                        self.state = OscStreamState::Ground;
                    } else if byte != 0x1b {
                        self.state = OscStreamState::IgnoringString;
                    }
                }
                OscStreamState::Discarding => {
                    if byte == 0x07 {
                        self.state = OscStreamState::Ground;
                    } else if byte == 0x1b {
                        self.state = OscStreamState::DiscardingEscape;
                    }
                }
                OscStreamState::DiscardingEscape => {
                    if byte == b'\\' {
                        self.state = OscStreamState::Ground;
                    } else if byte != 0x1b {
                        self.state = OscStreamState::Discarding;
                    }
                }
            }
        }
    }

    fn push(&mut self, byte: u8) {
        self.body.push(byte);
        if self.body.len() > MAX_BODY_BYTES {
            self.body.clear();
            self.state = OscStreamState::Discarding;
        } else {
            self.state = OscStreamState::Body;
        }
    }

    fn finish(&mut self, receive: &mut impl FnMut(&[u8])) {
        receive(&self.body);
        self.body.clear();
        self.state = OscStreamState::Ground;
    }
}

/// The title, progress and prompt marks a pane's program sent, each kept
/// from the last complete sequence, so one split across reads counts once
/// it ends.
#[derive(Debug, Default)]
pub struct OscState {
    collector: OscStreamCollector,
    /// The title the terminal shows: the last OSC 0 or 2, `None` when it
    /// was empty or none came.
    title: Option<String>,
    /// The title and progress the screen rules read, which a change of the
    /// pane's agent drops ([`OscState::clear_retained`]) so a new agent does
    /// not inherit an earlier one's.
    rules_title: Option<String>,
    progress: Option<String>,
    /// OSC 133;A marks since the pane began.
    prompts: u64,
}

impl OscState {
    pub fn observe(&mut self, bytes: &[u8]) {
        let Self {
            collector,
            title,
            rules_title,
            progress,
            prompts,
        } = self;
        collector.observe(bytes, |body| {
            let Some((command, payload)) = split_body(body) else {
                return;
            };
            match command {
                b"0" | b"2" => {
                    let text = sanitize(payload, MAX_CHARS);
                    let text = (!text.is_empty()).then_some(text);
                    *title = text.clone();
                    *rules_title = text;
                }
                b"9" => *progress = Some(sanitize(payload, MAX_CHARS)),
                b"133" if payload == b"A" || payload.starts_with(b"A;") => {
                    *prompts = prompts.saturating_add(1);
                }
                _ => {}
            }
        });
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// The title the screen rules read, empty when none.
    pub fn rules_title(&self) -> &str {
        self.rules_title.as_deref().unwrap_or("")
    }

    /// The progress payload the screen rules read (what follows `9;`),
    /// empty when none.
    pub fn progress(&self) -> &str {
        self.progress.as_deref().unwrap_or("")
    }

    pub fn prompts(&self) -> u64 {
        self.prompts
    }

    /// Drops the title and progress the screen rules read, as is done
    /// when a pane's agent changes; a sequence in flight still finishes and
    /// counts for the new one.
    pub fn clear_retained(&mut self) {
        self.rules_title = None;
        self.progress = None;
    }
}

/// Splits an OSC body at the first `;`, returning `(command, payload)`.
/// Returns `None` if there is no `;`.
fn split_body(body: &[u8]) -> Option<(&[u8], &[u8])> {
    let sep = body.iter().position(|&b| b == b';')?;
    Some((&body[..sep], &body[sep + 1..]))
}

fn sanitize(payload: &[u8], max_chars: usize) -> String {
    let text = String::from_utf8_lossy(payload);
    let mut out = String::new();
    for ch in text.chars().filter(|ch| !ch.is_control()).take(max_chars) {
        out.push(ch);
    }
    out
}

/// A folder the shell reported (OSC 7's `file://` URI, or OSC 9;9's plain
/// path), as a path; `None` for a URI of another host or text that is not
/// one.
pub fn parse_reported_cwd(value: &[u8]) -> Option<PathBuf> {
    let value = std::str::from_utf8(value).ok()?.trim();
    if value.starts_with("file://") {
        return parse_file_uri_cwd(value);
    }
    let path = value.trim_matches('"');
    (!path.is_empty()).then(|| PathBuf::from(path))
}

fn parse_file_uri_cwd(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let path = if rest.starts_with('/') {
        rest
    } else if let Some(slash) = rest.find('/') {
        let host = &rest[..slash];
        if !(host.is_empty() || host.eq_ignore_ascii_case("localhost")) {
            return None;
        }
        &rest[slash..]
    } else {
        rest
    };
    let path = percent_decode_utf8(path)?;

    #[cfg(windows)]
    {
        let mut path = path;
        if path.len() >= 3
            && path.as_bytes()[0] == b'/'
            && path.as_bytes()[2] == b':'
            && path.as_bytes()[1].is_ascii_alphabetic()
        {
            path.remove(0);
        }
        Some(PathBuf::from(path.replace('/', "\\")))
    }

    #[cfg(not(windows))]
    Some(PathBuf::from(path))
}

fn percent_decode_utf8(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut idx = 0;
    while idx < bytes.len() {
        if bytes[idx] == b'%' {
            let hi = *bytes.get(idx + 1)?;
            let lo = *bytes.get(idx + 2)?;
            output.push(hex_value(hi)? * 16 + hex_value(lo)?);
            idx += 3;
        } else {
            output.push(bytes[idx]);
            idx += 1;
        }
    }
    String::from_utf8(output).ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observed(chunks: &[&[u8]]) -> OscState {
        let mut state = OscState::default();
        for chunk in chunks {
            state.observe(chunk);
        }
        state
    }

    #[test]
    fn a_title_split_across_reads_counts_once_it_ends_and_an_empty_one_clears_it() {
        let state = observed(&[b"\x1b]0;first", b" half\x07"]);
        assert_eq!(state.title(), Some("first half"));
        assert_eq!(state.rules_title(), "first half");
        let state = observed(&[b"\x1b]2;named\x1b\\", b"\x1b]0;\x07"]);
        assert_eq!(state.title(), None);
        assert_eq!(state.rules_title(), "");
        let state = observed(&[b"\x1b]0;unfinished"]);
        assert_eq!(state.title(), None);
    }

    #[test]
    fn a_title_keeps_its_first_characters_without_control_characters() {
        let long = format!("\x1b]0;a\tb{}\x07", "x".repeat(MAX_CHARS * 2));
        let state = observed(&[long.as_bytes()]);
        let title = state.title().unwrap();
        assert_eq!(title.chars().count(), MAX_CHARS);
        assert!(title.starts_with("abx"), "{title}");
    }

    #[test]
    fn prompts_count_each_133_a_mark_and_nothing_else() {
        let state = observed(&[
            b"\x1b]133;A\x07$ \x1b]133;B\x07ls\x1b]133;C\x07",
            b"\x1b]133;D;0\x07\x1b]133;A;cl=m\x1b\\",
            b"\x1b]133;",
            b"A\x07",
        ]);
        assert_eq!(state.prompts(), 3);
    }

    #[test]
    fn progress_is_kept_and_a_change_of_agent_drops_what_the_rules_read() {
        let mut state = observed(&[b"\x1b]9;4;3;\x07\x1b]0;\xe2\x9c\xb3 task\x07"]);
        assert_eq!(state.progress(), "4;3;");
        state.clear_retained();
        assert_eq!(state.progress(), "");
        assert_eq!(state.rules_title(), "");
        assert_eq!(state.title(), Some("\u{2733} task"));
    }

    #[test]
    fn a_reported_folder_reads_as_a_path_and_another_hosts_does_not() {
        #[cfg(not(windows))]
        assert_eq!(
            parse_reported_cwd(b"file:///tmp/hide%20repo"),
            Some(PathBuf::from("/tmp/hide repo"))
        );
        #[cfg(windows)]
        assert_eq!(
            parse_reported_cwd(b"file:///C:/hide%20repo"),
            Some(PathBuf::from("C:\\hide repo"))
        );
        assert_eq!(
            parse_reported_cwd(b"file://localhost/tmp"),
            Some(PathBuf::from(if cfg!(windows) { "\\tmp" } else { "/tmp" }))
        );
        assert_eq!(
            parse_reported_cwd(b"\"C:\\my proj\""),
            Some(PathBuf::from("C:\\my proj"))
        );
        assert_eq!(parse_reported_cwd(b""), None);
        assert_eq!(parse_reported_cwd(b"\xff"), None);
        assert_eq!(parse_reported_cwd(b"file://remote/tmp"), None);
        assert_eq!(parse_reported_cwd(b"file:///tmp/%zz"), None);
    }
}
