//! Claude Code's record of a paste. A paste of four lines or more, or of
//! 800 characters or more, reaches its prompt hook and its session file as a
//! block, `\n\n<pasted_content id="d974">\n<the pasted text>\n
//! </pasted_content id="d974">\n`, whose id is four lowercase hex digits; a
//! shorter one arrives as typed (Claude Code 2.1.316, measured 2026-11-01).
//! Inside a block, and in the typed words beside one, Claude Code writes a
//! backslash after each `<` that starts a `pasted_content` tag, so a pasted
//! text cannot close its own block.
//!
//! This reads blocks as Claude Code's own reader does (`C0t` and `Mde` in the
//! 2.1.316 bundle): a block opens at `<pasted_content id="` with a valid id
//! and a line break, closes at the first line that starts with its own close
//! tag, and takes up to two line breaks on each side with it. Every reader of
//! a person's message that needs the words as typed goes through here: the
//! thread keeps pasted text apart from typed words, and the prompt digest
//! compares what Hide pasted with what the agent took.

use std::borrow::Cow;

const OPEN: &str = "<pasted_content id=\"";
const OPEN_END: &str = "\">\n";
/// The longest run of characters between `<\` and the tag name Claude
/// Code's escape reads as part of the tag, such as the `/` of a close tag.
const ESCAPE_FILLER: usize = 8;

/// One piece of a person's message: words they typed, or a paste's body.
enum Piece<'a> {
    Typed(&'a str),
    Pasted(&'a str),
}

/// `text` in the pieces Claude Code's reader cuts it into.
fn pieces(text: &str) -> Vec<Piece<'_>> {
    let bytes = text.as_bytes();
    let mut pieces = Vec::new();
    let mut taken = 0;
    let mut from = 0;
    while let Some(found) = text[from..].find(OPEN) {
        let open = from + found;
        let id_at = open + OPEN.len();
        let id = text.get(id_at..id_at + 4).filter(|id| valid_id(id));
        let (Some(id), true) = (
            id,
            text.get(id_at + 4..)
                .is_some_and(|rest| rest.starts_with(OPEN_END)),
        ) else {
            from = id_at;
            continue;
        };
        let body_at = id_at + 4 + OPEN_END.len();
        let close = format!("\n</pasted_content id=\"{id}\">");
        // The close tag's line break may be the open tag's own: an empty body.
        let Some(found) = text[body_at - 1..].find(&close) else {
            break;
        };
        let close_at = body_at - 1 + found + 1;
        let mut start = open;
        for _ in 0..2 {
            if start > taken && bytes[start - 1] == b'\n' {
                start -= 1;
            }
        }
        if start > taken {
            pieces.push(Piece::Typed(&text[taken..start]));
        }
        let mut end = close_at + close.len() - 1;
        for _ in 0..2 {
            if bytes.get(end) == Some(&b'\n') {
                end += 1;
            }
        }
        pieces.push(Piece::Pasted(text.get(body_at..close_at - 1).unwrap_or("")));
        taken = end;
        from = end;
    }
    if taken < text.len() {
        pieces.push(Piece::Typed(&text[taken..]));
    }
    pieces
}

fn valid_id(id: &str) -> bool {
    id.bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// `text` cut at each paste, the pasted text marked as pasted and without
/// its tags; pieces that hold only white space are left out.
pub fn pasted_pieces(text: &str) -> Vec<(&str, bool)> {
    let mut pieces: Vec<_> = pieces(text)
        .into_iter()
        .map(|piece| match piece {
            Piece::Typed(text) => (text, false),
            Piece::Pasted(body) => (body, true),
        })
        .collect();
    pieces.retain(|(piece, _)| !piece.trim().is_empty());
    pieces
}

/// What the person typed and pasted, in order, from Claude Code's record of
/// it: each paste's body in place of its block, with Claude Code's escape of
/// a `pasted_content` tag undone. Repeated until nothing changes, so a paste
/// that itself held a block, written escaped inside the outer one, reads as
/// the same text whether Claude Code wrapped it or not. Any other text comes
/// back as it is.
pub fn typed_text(record: &str) -> Cow<'_, str> {
    let mut text = Cow::Borrowed(record);
    loop {
        let joined = join(&text);
        let next = unescape(&joined).into_owned();
        if next == *text {
            return text;
        }
        // Each pass takes tags or a backslash out, so the text shortens.
        text = Cow::Owned(next);
    }
}

fn join(text: &str) -> Cow<'_, str> {
    let pieces = pieces(text);
    if pieces.iter().all(|piece| matches!(piece, Piece::Typed(_))) {
        return Cow::Borrowed(text);
    }
    Cow::Owned(
        pieces
            .into_iter()
            .map(|piece| match piece {
                Piece::Typed(text) | Piece::Pasted(text) => text,
            })
            .collect(),
    )
}

/// `text` with the backslash taken out of each `<\` that Claude Code put
/// before a `pasted_content` tag (any case, `-`, `_` or nothing between the
/// words, a few other characters such as `/` before them). Look-alike letters
/// Claude Code also escapes stay as written.
fn unescape(text: &str) -> Cow<'_, str> {
    let mut out = String::new();
    let mut copied = 0;
    let mut from = 0;
    while let Some(found) = text[from..].find("<\\") {
        let at = from + found;
        let rest = &text[at + 2..];
        let filler = rest
            .char_indices()
            .take(ESCAPE_FILLER + 1)
            .find(|(_, c)| c.is_ascii_alphanumeric())
            .map(|(index, _)| index);
        if filler.is_some_and(|index| names_the_tag(&rest[index..])) {
            out.push_str(&text[copied..at + 1]);
            copied = at + 2;
        }
        from = at + 2;
    }
    if copied == 0 {
        return Cow::Borrowed(text);
    }
    out.push_str(&text[copied..]);
    Cow::Owned(out)
}

fn names_the_tag(text: &str) -> bool {
    let starts = |text: &str, word: &str| {
        text.get(..word.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(word))
    };
    if !starts(text, "pasted") {
        return false;
    }
    let rest = &text["pasted".len()..];
    let rest = rest.strip_prefix(['-', '_']).unwrap_or(rest);
    starts(rest, "content")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What Claude Code 2.1.316 writes for `pasted` (`hSe`): its escape of a
    /// tag, the block around it with a line break before the close tag.
    fn claude_block(id: &str, pasted: &str) -> String {
        let escaped = pasted
            .replace("<pasted_content", "<\\pasted_content")
            .replace("</pasted_content", "<\\/pasted_content");
        let end = if escaped.ends_with('\n') { "" } else { "\n" };
        format!("\n\n<pasted_content id=\"{id}\">\n{escaped}{end}</pasted_content id=\"{id}\">\n")
    }

    #[test]
    fn a_paste_reads_as_the_text_that_was_pasted() {
        let brief = "# Brief\n\nRun `hide agent show`.";
        assert_eq!(typed_text(&claude_block("d974", brief)), brief);
        // Typed words beside it stay, as Claude Code's reader keeps them.
        let mixed = format!("fix this{}why?", claude_block("0a1f", "panic at main.rs:3"));
        assert_eq!(typed_text(&mixed), "fix thispanic at main.rs:3why?");
        assert_eq!(
            pasted_pieces(&mixed),
            [
                ("fix this", false),
                ("panic at main.rs:3", true),
                ("why?", false)
            ]
        );
    }

    #[test]
    fn a_paste_that_quotes_another_paste_reads_as_typed_both_ways() {
        // A parent's brief quoting the operator's paste: Claude Code escapes
        // the quoted tags inside the child's own block.
        let quoted = claude_block("fc7e", "the operator's log");
        let brief = format!("Look at this:{quoted}and fix it.");
        let recorded = claude_block("d974", &brief);
        assert!(recorded.contains("<\\/pasted_content id=\"fc7e\">"));
        assert_eq!(typed_text(&recorded), typed_text(&brief));
        assert_eq!(
            typed_text(&brief),
            "Look at this:the operator's logand fix it."
        );
    }

    #[test]
    fn only_a_block_as_claude_code_writes_one_is_a_paste() {
        for text in [
            // Not four lowercase hex digits.
            "<pasted_content id=\"p1\">\nx\n</pasted_content id=\"p1\">",
            // No line break after the open tag or before the close tag.
            "<pasted_content id=\"a1b2\">x\n</pasted_content id=\"a1b2\">",
            "<pasted_content id=\"a1b2\">\nx</pasted_content id=\"a1b2\">",
            // Another block's close tag, or none.
            "<pasted_content id=\"a1b2\">\nx\n</pasted_content id=\"c3d4\">",
            "<pasted_content id=\"a1b2\">\nnever closed",
        ] {
            assert_eq!(typed_text(text), text, "{text:?}");
            assert_eq!(pasted_pieces(text), [(text, false)], "{text:?}");
        }
    }

    #[test]
    fn only_a_backslash_claude_code_put_before_the_tag_is_taken_out() {
        assert_eq!(
            unescape("<\\pasted_content <\\/PASTED-CONTENT <\\ /pastedcontent"),
            "<pasted_content </PASTED-CONTENT < /pastedcontent"
        );
        for text in ["<\\pasted", "<\\b>", "a <\\ b", "<\\pasted_contents?"] {
            let expected = if text.starts_with("<\\pasted_content") {
                "<pasted_contents?"
            } else {
                text
            };
            assert_eq!(unescape(text), expected, "{text:?}");
        }
    }
}
