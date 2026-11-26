//! Desk's answer as it streams, and the sessions and missions it names: a
//! `@session(<pane> <name>)` or `@mission(<id> <title>)` token is read the
//! moment its `)` arrives, so its chip shows while the rest of the answer is
//! still being written (mission-106 answer, picture section 2).

const SESSION_OPEN: &str = "@session(";
const MISSION_OPEN: &str = "@mission(";
/// Both opens are this long, so a tail one byte shorter may still be the
/// start of one.
const OPEN_LEN: usize = 9;
/// The longest token read as a chip, its open and `)` included: a name that
/// runs longer is words, so a token left open costs a chunk this much
/// reading again at most.
const MAX_TOKEN_BYTES: usize = 1024;

/// One answer's words as the agent wrote them, with each token that closed
/// written as its chip names it (a quoted name without its quotes), and the
/// sessions and missions those tokens name, in order and once per id.
///
/// [`Answer::push`] reads only what may still change: the chunk, and a token
/// the chunk before left open (bounded by [`MAX_TOKEN_BYTES`] and its line),
/// so a chunk costs its own length plus at most one open token, never the
/// whole answer again.
#[derive(Debug, Default)]
pub struct Answer {
    text: String,
    /// Bytes the agent wrote, before any quotes were taken out: what the
    /// answer's cap and a refusal's place count.
    written: usize,
    /// Where reading starts on the next chunk: the open token, or the tail
    /// that could still begin one.
    open_at: usize,
    sessions: Vec<(String, String)>,
    missions: Vec<(String, String)>,
}

impl Answer {
    /// An answer read whole, as a finished one is.
    pub fn read(text: &str) -> Self {
        let mut answer = Self::default();
        answer.push(text);
        answer
    }

    /// Adds the agent's next words and reads every token they close.
    pub fn push(&mut self, chunk: &str) {
        self.written += chunk.len();
        self.text.push_str(chunk);
        self.read_open();
    }

    /// The words so far, each closed token as its chip names it.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// How many bytes the agent wrote.
    pub fn written(&self) -> usize {
        self.written
    }

    /// The sessions named so far, as `(pane, name)`.
    pub fn sessions(&self) -> &[(String, String)] {
        &self.sessions
    }

    /// The missions named so far, as `(id, title)`.
    pub fn missions(&self) -> &[(String, String)] {
        &self.missions
    }

    fn read_open(&mut self) {
        let mut from = self.open_at;
        loop {
            let Some((start, open)) = next_open(&self.text, from) else {
                // No token starts past `from`; the last bytes may begin one.
                let mut keep = self.text.len().saturating_sub(OPEN_LEN - 1).max(from);
                while !self.text.is_char_boundary(keep) {
                    keep += 1;
                }
                self.open_at = keep;
                return;
            };
            let inner = start + OPEN_LEN;
            let close = match closing(&self.text[inner..]) {
                Close::Open => {
                    self.open_at = start;
                    return;
                }
                Close::Words => {
                    from = inner;
                    continue;
                }
                Close::At(close) => close,
            };
            let end = inner + close + 1;
            from = end;
            let Some((id, name, quoted)) = named(&self.text[inner..end - 1]) else {
                continue;
            };
            let (id, name) = (id.to_owned(), name.to_owned());
            if quoted {
                let token = format!("{open}{id} {name})");
                self.text.replace_range(start..end, &token);
                from = start + token.len();
            }
            let found = if open == SESSION_OPEN {
                &mut self.sessions
            } else {
                &mut self.missions
            };
            if !found.iter().any(|(seen, _)| *seen == id) {
                found.push((id, name));
            }
        }
    }
}

/// The sessions `text` names as `@session(<pane> <name>)`, in order, each
/// once. Every one is a chip (PRD session-docs-and-assistant D-86): the
/// reply's own byte cap bounds how many it can name, so no count cap leaves a
/// later one as raw text. A name the agent quoted is read without its quotes.
pub fn session_mentions(text: &str) -> Vec<(String, String)> {
    Answer::read(text).sessions
}

/// The first `@session(` or `@mission(` at or past `from`.
fn next_open(text: &str, from: usize) -> Option<(usize, &'static str)> {
    let mut at = from;
    while let Some(found) = text[at..].find('@') {
        let start = at + found;
        for open in [SESSION_OPEN, MISSION_OPEN] {
            if text[start..].starts_with(open) {
                return Some((start, open));
            }
        }
        at = start + 1;
    }
    None
}

/// What follows a token's open.
enum Close {
    /// Its `)`, this far in.
    At(usize),
    /// The line ends, or the token runs past [`MAX_TOKEN_BYTES`], before it
    /// closes: it stays words.
    Words,
    /// The text ends with it still open: the next chunk may close it.
    Open,
}

/// Where the parenthesis already open before `text` closes, on its line.
/// The name may hold parentheses of its own, `(11-25)`, as long as they
/// close.
fn closing(text: &str) -> Close {
    let mut depth = 0usize;
    for (at, char) in text.char_indices() {
        if at + OPEN_LEN >= MAX_TOKEN_BYTES {
            return Close::Words;
        }
        match char {
            '\n' => return Close::Words,
            '(' => depth += 1,
            ')' if depth == 0 => return Close::At(at),
            ')' => depth -= 1,
            _ => {}
        }
    }
    Close::Open
}

/// A token's id and name, the name trimmed and without one pair of matching
/// quotes, and whether it had them; none for a token with no name.
fn named(inner: &str) -> Option<(&str, &str, bool)> {
    let (id, name) = inner.trim().split_once(char::is_whitespace)?;
    let name = name.trim();
    let unquoted = unquote(name);
    if unquoted.is_empty() {
        return None;
    }
    Some((id, unquoted, unquoted.len() != name.len()))
}

/// `name` without one pair of matching quotes around it (`"..."`, `'...'`,
/// `“...”`, `‘...’`); any other name as it is.
fn unquote(name: &str) -> &str {
    for (left, right) in [('"', '"'), ('\'', '\''), ('“', '”'), ('‘', '’')] {
        if let Some(inner) = name
            .strip_prefix(left)
            .and_then(|rest| rest.strip_suffix(right))
        {
            return inner.trim();
        }
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(found: &[(String, String)]) -> Vec<(&str, &str)> {
        found
            .iter()
            .map(|(id, name)| (id.as_str(), name.as_str()))
            .collect()
    }

    #[test]
    fn a_reply_s_missions_are_read_with_parentheses_in_their_titles() {
        let text = "@mission(mission-63 결제 흐름 정리 (11-25)) runs; again \
                    @mission(mission-63 결제 흐름 정리 (11-25)), @mission(mission-9) and \
                    @mission(mission-7 open (one\nline) then @mission(mission-8 last)";
        assert_eq!(
            pairs(Answer::read(text).missions()),
            [
                ("mission-63", "결제 흐름 정리 (11-25)"),
                ("mission-8", "last")
            ]
        );
    }

    #[test]
    fn a_reply_s_session_names_are_read_once_each_in_order() {
        let text = "@session(w1:p2 fix login) is done; @session(w3:p1 docs) waits. \
                    Again @session(w1:p2 fix login), then @session(broken) and @session(w4:p1 tail";
        assert_eq!(
            session_mentions(text),
            [
                ("w1:p2".to_owned(), "fix login".to_owned()),
                ("w3:p1".to_owned(), "docs".to_owned()),
            ]
        );
    }

    #[test]
    fn a_quoted_session_name_is_read_without_its_quotes_and_the_kept_reply_writes_it_so() {
        let text = "@session(h1:p2 \"note-claude\") wrote it; @session(h1:p3 'pi run') too, \
                    @session(h1:p4 “결제”), @session(h1:p5 \"\") and @session(h1:p6 say \"hi\")";
        let answer = Answer::read(text);
        assert_eq!(
            pairs(answer.sessions()),
            [
                ("h1:p2", "note-claude"),
                ("h1:p3", "pi run"),
                ("h1:p4", "결제"),
                ("h1:p6", "say \"hi\""),
            ]
        );
        assert_eq!(
            answer.text(),
            "@session(h1:p2 note-claude) wrote it; @session(h1:p3 pi run) too, \
             @session(h1:p4 결제), @session(h1:p5 \"\") and @session(h1:p6 say \"hi\")"
        );
        assert_eq!(answer.written(), text.len());
        assert_eq!(
            Answer::read("@mission(mission-9 \"정리 (11-25)\") ok").text(),
            "@mission(mission-9 정리 (11-25)) ok"
        );
    }

    #[test]
    fn every_session_a_long_reply_names_is_read_past_the_sixteenth() {
        let text: String = (1..=40)
            .map(|n| format!("- @session(w{n}:p1 job-{n}) runs\n"))
            .collect();
        let found = session_mentions(&text);
        assert_eq!(found.len(), 40);
        assert_eq!(found[16], ("w17:p1".to_owned(), "job-17".to_owned()));
        assert_eq!(found[39], ("w40:p1".to_owned(), "job-40".to_owned()));
    }

    #[test]
    fn a_token_is_read_the_moment_its_parenthesis_closes_while_the_answer_streams() {
        let mut answer = Answer::default();
        answer.push("지휘 세션 @ses");
        assert!(answer.sessions().is_empty());
        answer.push("sion(h1:p2 \"Hide 앱 Play");
        assert!(
            answer.sessions().is_empty(),
            "a name not yet closed stays words"
        );
        answer.push(" 스토어\")에 안 읽은 편");
        assert_eq!(pairs(answer.sessions()), [("h1:p2", "Hide 앱 Play 스토어")]);
        assert_eq!(
            answer.text(),
            "지휘 세션 @session(h1:p2 Hide 앱 Play 스토어)에 안 읽은 편"
        );
        answer.push("지가 있고 @mission(mission-63 결제 (11-25");
        assert!(
            answer.missions().is_empty(),
            "its own parentheses keep it open"
        );
        answer.push(")) 도 기다립니다.");
        assert_eq!(pairs(answer.missions()), [("mission-63", "결제 (11-25)")]);
    }

    #[test]
    fn a_streamed_answer_reads_as_the_same_answer_read_whole() {
        let text = "1. @session(w1:p2 \"fix login\") — done\n\
                    2. @mission(mission-7 open (one\nline) then @mission(mission-8 \"last\")\n\
                    3. `@session(w3:p1 docs)` and @session(w1:p2 fix login) again; @session(broken)";
        let whole = Answer::read(text);
        // Every split, by one character at a time, reads the same.
        let mut streamed = Answer::default();
        for char in text.chars() {
            streamed.push(char.encode_utf8(&mut [0; 4]));
        }
        assert_eq!(streamed.text(), whole.text());
        assert_eq!(streamed.sessions(), whole.sessions());
        assert_eq!(streamed.missions(), whole.missions());
        assert_eq!(streamed.written(), text.len());
        assert_eq!(
            pairs(whole.sessions()),
            [("w1:p2", "fix login"), ("w3:p1", "docs")]
        );
        assert_eq!(pairs(whole.missions()), [("mission-8", "last")]);
    }

    #[test]
    fn a_chunk_reads_its_own_words_and_at_most_one_open_token_again() {
        let mut answer = Answer::default();
        // A long answer with no token leaves nothing to read again.
        answer.push(&"word ".repeat(10_000));
        assert!(answer.text().len() - answer.open_at < OPEN_LEN);
        // An open token is read again only from its own start.
        answer.push("@session(w1:p2 still");
        assert_eq!(answer.open_at, 50_000);
        // A name that runs past the cap is words, and reading moves past it.
        answer.push(&"x".repeat(MAX_TOKEN_BYTES));
        assert!(answer.open_at > 50_000 + MAX_TOKEN_BYTES);
        answer.push(")");
        assert!(answer.sessions().is_empty());
    }
}
