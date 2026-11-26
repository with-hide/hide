//! A terminal's escape sequences taken out of an agent's text. An agent's
//! command can answer with what it writes to a terminal, colours and links
//! included (omp's `/context`); Desk shows text, so an `ESC` and the
//! sequence it starts (ECMA-48: CSI, a string such as OSC or DCS up to its
//! terminator, or a two-part escape) never reach the answer, whichever agent
//! wrote it.

/// Where the text read so far stopped: a sequence can be split between two
/// chunks of a reply, so the next chunk goes on from it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum State {
    #[default]
    Text,
    /// Right after `ESC`.
    Escape,
    /// An escape's intermediate bytes, before its final one.
    Intermediate,
    /// A CSI's parameters, before its final byte.
    Csi,
    /// A string (OSC, DCS, SOS, PM, APC) before its terminator.
    String,
    /// `ESC` inside a string, which `\` makes its terminator.
    StringEscape,
}

/// The characters a string sequence (OSC, DCS, SOS, PM, APC) is held for
/// before its terminator. A terminal's longest, a link's address or a
/// window title, fits well under; past it the sequence is taken as one that
/// never ends, so its head alone goes and what it held shows as text.
const MAX_STRING_CHARS: usize = 4096;

/// Takes escape sequences out of a stream of text, chunk by chunk.
#[derive(Debug, Default)]
pub struct Escapes {
    state: State,
    /// What the open string sequence holds after its head, until its
    /// terminator drops it or it turns out never to end.
    held: String,
    held_chars: usize,
}

impl Escapes {
    /// `text` without the escape sequences in it, going on from where the
    /// last chunk stopped.
    pub fn strip(&mut self, text: &str) -> String {
        let mut plain = String::with_capacity(text.len());
        for ch in text.chars() {
            self.step(ch, &mut plain);
        }
        plain
    }

    /// What a string sequence still open when the text ends held, as text:
    /// its head is gone, its words are not lost.
    pub fn finish(&mut self) -> String {
        let mut plain = String::new();
        // What it held may open another string that is not closed either;
        // each pass reads less, since a head is gone each time.
        while matches!(self.state, State::String | State::StringEscape) {
            self.release(&mut plain);
        }
        plain
    }

    /// Reads one character. The state moves in one place, at the end, but
    /// for a string that never ends: its release reads what it held again
    /// and leaves the state where that reading stopped.
    fn step(&mut self, ch: char, plain: &mut String) {
        let next = match (self.state, ch) {
            (State::String, '\x07') => self.drop_held(),
            (State::String, '\x1b') => State::StringEscape,
            (State::StringEscape, '\\') => self.drop_held(),
            (State::StringEscape, '\x1b') => {
                self.hold('\x1b');
                State::StringEscape
            }
            (State::String | State::StringEscape, ch) => {
                if self.state == State::StringEscape {
                    self.hold('\x1b');
                }
                self.hold(ch);
                if self.held_chars > MAX_STRING_CHARS {
                    self.release(plain);
                    return;
                }
                State::String
            }
            (_, '\x1b') => State::Escape,
            (State::Text, ch) => {
                plain.push(ch);
                State::Text
            }
            (State::Escape, '[') => State::Csi,
            (State::Escape, ']' | 'P' | 'X' | '^' | '_') => State::String,
            (State::Escape | State::Intermediate, '\x20'..='\x2f') => State::Intermediate,
            (State::Escape | State::Intermediate, _) => State::Text,
            (State::Csi, '\x40'..='\x7e') => State::Text,
            (State::Csi, _) => State::Csi,
        };
        self.state = next;
    }

    fn hold(&mut self, ch: char) {
        self.held.push(ch);
        self.held_chars += 1;
    }

    /// The string ended at its terminator: what it held was the sequence's.
    fn drop_held(&mut self) -> State {
        self.held.clear();
        self.held_chars = 0;
        State::Text
    }

    /// The string never ended: what it held is read again as text, so an
    /// escape inside it is still taken out.
    fn release(&mut self, plain: &mut String) {
        let held = std::mem::take(&mut self.held);
        self.held_chars = 0;
        self.state = State::Text;
        for ch in held.chars() {
            self.step(ch, plain);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Escapes;

    fn strip(chunks: &[&str]) -> String {
        let mut escapes = Escapes::default();
        chunks.iter().map(|chunk| escapes.strip(chunk)).collect()
    }

    #[test]
    fn colours_and_links_leave_only_their_words() {
        assert_eq!(strip(&["\x1b[38;5;243m░░\x1b[39m used"]), "░░ used");
        assert_eq!(
            strip(&["see \x1b]8;;https://example.com\x07docs\x1b]8;;\x1b\\ now"]),
            "see docs now"
        );
        assert_eq!(strip(&["\x1bP1$r0m\x1b\\a\x1b(Bb\x1b7c"]), "abc");
    }

    #[test]
    fn a_sequence_split_between_chunks_is_taken_out_whole() {
        assert_eq!(
            strip(&["░\x1b[38;5", ";243m░", "\x1b", "[0m done"]),
            "░░ done"
        );
        assert_eq!(strip(&["a\x1b]0;ti", "tle\x1b", "\\b"]), "ab");
    }

    /// letter-48550: a string sequence that never ends loses its head
    /// only: past the cap, or at the text's end, what it held shows.
    #[test]
    fn a_string_that_never_ends_keeps_its_words() {
        let long = "x".repeat(super::MAX_STRING_CHARS);
        assert_eq!(
            strip(&["a\x1b]", &long, "y rest"]),
            format!("a{long}y rest")
        );
        let mut escapes = Escapes::default();
        let said = escapes.strip("see \x1b]8;;the answer");
        assert_eq!(said + &escapes.finish(), "see 8;;the answer");
        // Two that never end, one inside the other, give back every word:
        // the outer one's release opens the inner one, which keeps going.
        let mut escapes = Escapes::default();
        let said = escapes.strip(&format!("a\x1b]{}\x1b]{long}z rest", "x".repeat(10)));
        assert_eq!(said + &escapes.finish(), format!("a{}{long}z rest", "x".repeat(10)));
        // One that ends within the cap is still taken out whole.
        assert_eq!(strip(&["a\x1b]", &long[1..], "\x07b"]), "ab");
    }

    #[test]
    fn text_without_escapes_is_kept_as_it_is() {
        assert_eq!(strip(&["안녕 [1m] ~ \\ ]\n\tok"]), "안녕 [1m] ~ \\ ]\n\tok");
    }
}
