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

/// The characters a string sequence (OSC, DCS, SOS, PM, APC) may hold
/// before its terminator: one that never ends stops being read as a
/// sequence here, so it costs the reply at most this much and what follows
/// shows (a link's address, the longest a terminal sends, fits well under).
const MAX_STRING_CHARS: usize = 4096;

/// Takes escape sequences out of a stream of text, chunk by chunk.
#[derive(Debug, Default)]
pub struct Escapes {
    state: State,
    /// The characters the open string sequence has held.
    string: usize,
}

impl Escapes {
    /// `text` without the escape sequences in it, going on from where the
    /// last chunk stopped.
    pub fn strip(&mut self, text: &str) -> String {
        let mut plain = String::with_capacity(text.len());
        for ch in text.chars() {
            if matches!(self.state, State::String | State::StringEscape) {
                self.string += 1;
                if self.string > MAX_STRING_CHARS {
                    self.state = State::Text;
                }
            } else {
                self.string = 0;
            }
            self.state = match (self.state, ch) {
                (_, '\x1b') if self.state != State::String => State::Escape,
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
                (State::String, '\x07') => State::Text,
                (State::String, '\x1b') => State::StringEscape,
                (State::String, _) => State::String,
                (State::StringEscape, '\\') => State::Text,
                (State::StringEscape, _) => State::String,
            };
        }
        plain
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

    /// letter-48549: an OSC that never ends costs the reply at most
    /// `MAX_STRING_CHARS`, and the words after it show.
    #[test]
    fn a_string_that_never_ends_stops_at_its_cap() {
        let lost = "x".repeat(super::MAX_STRING_CHARS);
        assert_eq!(strip(&["a\x1b]", &lost, "the rest"]), "athe rest");
        // A string that ends within the cap is still taken out whole.
        assert_eq!(strip(&["a\x1b]", &lost[1..], "\x07b"]), "ab");
    }

    #[test]
    fn text_without_escapes_is_kept_as_it_is() {
        assert_eq!(strip(&["안녕 [1m] ~ \\ ]\n\tok"]), "안녕 [1m] ~ \\ ]\n\tok");
    }
}
