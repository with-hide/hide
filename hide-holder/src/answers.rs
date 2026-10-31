//! Answers to the program's questions, which go out whether or not a node is
//! connected (PRD engine-bundle-2-pane-holder D-9).
//!
//! Who answers what is fixed, so no question is answered twice:
//!
//! | Question | Answered by |
//! | --- | --- |
//! | DSR 5n, 6n; DECRQM; DECRQSS; XTVERSION; kitty keyboard `CSI ? u`; DA1, DA2, DA3 | libghostty |
//! | OSC 10, 11, 12 and OSC 4 colors, `CSI ? 996 n` | libghostty, from the theme the node sent ([`crate::pane`]) |
//! | XTGETTCAP | libghostty, less what this path does not carry ([`UNSUPPORTED`]) |
//! | XTWINOPS 18t, the text area in cells | the holder ([`Scanner`]) |
//! | XTWINOPS 14t, 16t | nobody: the cell's pixel size is not known, and libghostty answers neither without it |
//! | mode 2048 size reports (`CSI 48 ; ... t`) | nobody: libghostty's carry pixels it does not know, so the holder drops them ([`keeps`]) |
//! | `CSI ? 997 n` appearance changes | the holder, on a theme whose appearance changed while mode 2031 is on |
//! | ENQ, OSC 52 reads, 21t title | nobody |
//!
//! DA1 is libghostty's default, VT220 with ANSI color (`CSI ? 62 ; 22 c`),
//! which this path carries. The holder's own answer goes in at the byte the
//! question ended at, between libghostty's answers to what came before and
//! after it, so a burst of questions is answered in order.

/// XTGETTCAP capabilities libghostty knows that this path does not carry
/// to the screen or from the node: the clipboard (OSC 52), underline color,
/// the kitty keyboard, and focus reports.
pub const UNSUPPORTED: [&str; 5] = ["Ms", "Setulc", "fullkbd", "kxIN", "kxOUT"];

/// What the holder must act on at a point of the program's output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// `CSI 18 t`: report the text area in cells.
    SizeQuery,
    /// DECSCUSR with its shape; 0 hands the shape back to the receiver.
    CursorShape(u8),
    /// RIS: the terminal was reset.
    Reset,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Ground,
    Escape,
    Csi,
    /// An OSC, DCS, APC, PM or SOS string, whose content is not a sequence.
    Text,
    TextEscape,
}

/// Finds the sequences the holder acts on in the program's output, across
/// reads that split them. Seven-bit only: libghostty reads a raw C1 byte in
/// UTF-8 as a replacement character, not as a sequence.
pub struct Scanner {
    state: State,
    params: Vec<u8>,
    overflowed: bool,
}

/// The longest CSI the scanner keeps; a longer one is none it acts on.
const MAX_PARAMS: usize = 32;

impl Default for Scanner {
    fn default() -> Self {
        Self {
            state: State::Ground,
            params: Vec::with_capacity(MAX_PARAMS),
            overflowed: false,
        }
    }
}

impl Scanner {
    /// The marks in `bytes`, each with the offset just past the byte that
    /// ended it.
    pub fn scan(&mut self, bytes: &[u8], marks: &mut Vec<(usize, Mark)>) {
        for (index, &byte) in bytes.iter().enumerate() {
            // CAN and SUB end any sequence.
            if byte == 0x18 || byte == 0x1a {
                self.state = State::Ground;
                continue;
            }
            match self.state {
                State::Ground => {
                    if byte == 0x1b {
                        self.state = State::Escape;
                    }
                }
                State::Escape => self.escape(byte, index, marks),
                State::Csi => match byte {
                    0x1b => self.state = State::Escape,
                    0x20..=0x3f => {
                        if self.params.len() < MAX_PARAMS {
                            self.params.push(byte);
                        } else {
                            self.overflowed = true;
                        }
                    }
                    0x40..=0x7e => {
                        if !self.overflowed
                            && let Some(mark) = csi_mark(&self.params, byte)
                        {
                            marks.push((index + 1, mark));
                        }
                        self.state = State::Ground;
                    }
                    // C0 controls run inside a CSI without ending it.
                    _ => {}
                },
                State::Text => match byte {
                    0x07 => self.state = State::Ground,
                    0x1b => self.state = State::TextEscape,
                    _ => {}
                },
                State::TextEscape => {
                    if byte == b'\\' {
                        self.state = State::Ground;
                    } else {
                        self.escape(byte, index, marks);
                    }
                }
            }
        }
    }

    fn escape(&mut self, byte: u8, index: usize, marks: &mut Vec<(usize, Mark)>) {
        self.state = match byte {
            b'[' => {
                self.params.clear();
                self.overflowed = false;
                State::Csi
            }
            b']' | b'P' | b'_' | b'^' | b'X' => State::Text,
            b'c' => {
                marks.push((index + 1, Mark::Reset));
                State::Ground
            }
            0x1b => State::Escape,
            // Intermediates keep the escape open; anything else ends it.
            0x20..=0x2f => State::Escape,
            _ => State::Ground,
        };
    }
}

fn csi_mark(params: &[u8], last: u8) -> Option<Mark> {
    match (params, last) {
        (b"18", b't') => Some(Mark::SizeQuery),
        (params, b'q') => {
            let shape = params.strip_suffix(b" ")?;
            if shape.is_empty() {
                return Some(Mark::CursorShape(0));
            }
            let shape: u8 = std::str::from_utf8(shape).ok()?.parse().ok()?;
            (shape <= 6).then_some(Mark::CursorShape(shape))
        }
        _ => None,
    }
}

/// Whether libghostty's `answer` is one this path keeps: an XTGETTCAP
/// answer naming a capability in [`UNSUPPORTED`] is dropped, as one
/// libghostty does not know is, and so is an in-band size report, whose
/// pixels nobody knows.
pub fn keeps(answer: &[u8]) -> bool {
    if answer.starts_with(b"\x1b[48;") && answer.ends_with(b"t") {
        return false;
    }
    let Some(rest) = answer.strip_prefix(b"\x1bP1+r") else {
        return true;
    };
    let end = rest
        .iter()
        .position(|byte| *byte == b'=' || *byte == 0x1b)
        .unwrap_or(rest.len());
    let Some(name) = decode_hex(&rest[..end]) else {
        return true;
    };
    !UNSUPPORTED
        .iter()
        .any(|unsupported| unsupported.as_bytes() == name)
}

fn decode_hex(hex: &[u8]) -> Option<Vec<u8>> {
    if hex.len() % 2 != 0 {
        return None;
    }
    hex.chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marks(chunks: &[&[u8]]) -> Vec<(usize, usize, Mark)> {
        let mut scanner = Scanner::default();
        let mut found = Vec::new();
        for (chunk_index, chunk) in chunks.iter().enumerate() {
            let mut marks = Vec::new();
            scanner.scan(chunk, &mut marks);
            found.extend(marks.into_iter().map(|(at, mark)| (chunk_index, at, mark)));
        }
        found
    }

    #[test]
    fn a_size_question_is_found_where_it_ends_even_split() {
        assert_eq!(marks(&[b"ab\x1b[18t"]), [(0, 7, Mark::SizeQuery)]);
        assert_eq!(marks(&[b"\x1b[1", b"8t"]), [(1, 2, Mark::SizeQuery)]);
        assert_eq!(marks(&[b"\x1b[14t\x1b[?18t\x1b[18;1t"]), []);
    }

    #[test]
    fn a_question_inside_a_string_is_not_one() {
        assert_eq!(
            marks(&[b"\x1b]0;[18t\x07\x1b[18t"]),
            [(0, 14, Mark::SizeQuery)]
        );
        assert_eq!(
            marks(&[b"\x1bP1;2|[18t\x1b\\\x1b[18t"]),
            [(0, 17, Mark::SizeQuery)]
        );
    }

    #[test]
    fn a_cursor_shape_and_a_reset_are_found() {
        assert_eq!(
            marks(&[b"\x1b[5 q\x1b[ q\x1b[9 q\x1bc"]),
            [
                (0, 5, Mark::CursorShape(5)),
                (0, 9, Mark::CursorShape(0)),
                (0, 16, Mark::Reset)
            ]
        );
    }

    #[test]
    fn only_unsupported_capabilities_are_dropped() {
        assert!(!keeps(b"\x1bP1+r4D73=5C45\x1b\\"));
        assert!(!keeps(b"\x1bP1+r536574756C63=x\x1b\\"));
        assert!(keeps(b"\x1bP1+r5463=5C45\x1b\\"));
        assert!(keeps(b"\x1b[?62;22c"));
        assert!(!keeps(b"\x1b[48;30;100;30;100t"));
        assert!(keeps(b"\x1b[8;30;100t"));
    }
}
