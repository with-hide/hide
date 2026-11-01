//! Claude Code's record of a paste. A paste of four lines or more, or of
//! 800 characters or more, reaches its prompt hook and its session file
//! wrapped as `\n\n<pasted_content id="d974">\n<the pasted text, trimmed>\n
//! </pasted_content id="d974">\n`, the id its own (Claude Code 2.1.316,
//! measured 2026-11-01); a shorter one arrives as typed. Every reader of a
//! person's message that needs the words as typed goes through here: the
//! thread keeps pasted text apart from typed words, and the prompt digest
//! compares what Hide pasted with what the agent took.

const OPEN: &str = "<pasted_content";
const CLOSE: &str = "</pasted_content";

/// `text` cut at each pasted-content wrapper, the wrapped text marked as
/// pasted and without its tags; pieces that hold only white space are left
/// out. A wrapper closes at the first close tag naming the same attributes
/// as its open tag, or at a bare `</pasted_content>`; an unclosed wrapper is
/// the person's text.
pub fn pasted_pieces(text: &str) -> Vec<(&str, bool)> {
    let mut pieces = Vec::new();
    let mut rest = text;
    while let Some((open, inner, end)) = next_wrapper(rest) {
        pieces.push((&rest[..open], false));
        pieces.push((inner, true));
        rest = &rest[end..];
    }
    pieces.push((rest, false));
    pieces.retain(|(piece, _)| !piece.trim().is_empty());
    pieces
}

/// The pasted text when `text` is one paste and nothing else: what a person,
/// or Hide, pasted into an empty composer.
pub fn only_pasted(text: &str) -> Option<&str> {
    match pasted_pieces(text).as_slice() {
        [(inner, true)] => Some(inner),
        _ => None,
    }
}

/// The first closed wrapper in `text`: where its open tag starts, the text
/// between the tags, and where its close tag ends.
fn next_wrapper(text: &str) -> Option<(usize, &str, usize)> {
    let mut from = 0;
    loop {
        let open = from + text[from..].find(OPEN)?;
        let after = open + OPEN.len();
        // `<pasted_contents` or `<pasted_content-x` is another tag.
        if !text[after..].starts_with(['>', ' ']) {
            from = after;
            continue;
        }
        let tag_end = after + text[after..].find('>')? + 1;
        let attributes = &text[after..tag_end - 1];
        let body = &text[tag_end..];
        let named = format!("{CLOSE}{attributes}>");
        let bare = format!("{CLOSE}>");
        let close = body
            .find(&named)
            .map(|at| (at, named.len()))
            .or_else(|| body.find(&bare).map(|at| (at, bare.len())))?;
        return Some((open, &body[..close.0], tag_end + close.0 + close.1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The shape Claude Code 2.1.316's `UserPromptSubmit` hook and session
    // file carried for a 3.5 KB markdown brief typed by `agent.prompt`.
    const WRAPPED: &str = "\n\n<pasted_content id=\"d974\">\n# Brief\n\nRun `hide agent show`.\n</pasted_content id=\"d974\">\n";

    #[test]
    fn pasted_content_is_cut_out_of_the_typed_words() {
        let text = "look at this\n<pasted_content id=\"a1\">\nlog line\n</pasted_content>\nwhy?";
        assert_eq!(
            pasted_pieces(text),
            vec![
                ("look at this\n", false),
                ("\nlog line\n", true),
                ("\nwhy?", false)
            ]
        );
        assert_eq!(
            pasted_pieces("<pasted_content id=\"a\">never closed"),
            vec![("<pasted_content id=\"a\">never closed", false)]
        );
    }

    #[test]
    fn a_whole_paste_reads_as_the_text_that_was_pasted() {
        assert_eq!(
            only_pasted(WRAPPED),
            Some("\n# Brief\n\nRun `hide agent show`.\n")
        );
    }

    #[test]
    fn a_paste_beside_typed_words_or_unclosed_is_not_a_whole_paste() {
        assert_eq!(
            only_pasted("fix this\n<pasted_content id=\"a\">\nx\n</pasted_content id=\"a\">"),
            None
        );
        assert_eq!(only_pasted("<pasted_content id=\"a\">\nx\n"), None);
        assert_eq!(only_pasted("plain words"), None);
    }

    #[test]
    fn a_wrapper_closes_at_its_own_close_tag_only() {
        // A pasted text quoting another paste's close tag keeps it.
        let text = "<pasted_content id=\"b\">\nsee </pasted_content id=\"a\"> here\n</pasted_content id=\"b\">";
        assert_eq!(
            only_pasted(text),
            Some("\nsee </pasted_content id=\"a\"> here\n")
        );
        assert_eq!(
            pasted_pieces("<pasted_contents>x</pasted_contents>"),
            [("<pasted_contents>x</pasted_contents>", false)]
        );
    }
}
