//! Text another program wrote (an agent's title, a letter sender's name,
//! a label line, a request or reply), made fit for a row the operator reads.
//!
//! Every Unicode default-ignorable code point is dropped: they draw nothing
//! but can reorder the text around them (bidirectional controls) or make
//! two different names look the same (zero-width characters, Hangul
//! fillers, variation selectors, tags). Control characters become spaces.
//! The list is the `Default_Ignorable_Code_Point` property from Unicode's
//! DerivedCoreProperties, which has been stable since Unicode 6.

use unicode_normalization::UnicodeNormalization;

fn ignorable(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{061C}'
            | '\u{115F}'..='\u{1160}'
            | '\u{17B4}'..='\u{17B5}'
            | '\u{180B}'..='\u{180F}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{3164}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FEFF}'
            | '\u{FFA0}'
            | '\u{FFF0}'..='\u{FFF8}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0000}'..='\u{E0FFF}'
    )
}

/// One line: ignorable characters dropped, controls and runs of whitespace
/// as one space, at most `max_chars` characters; `None` when nothing is left.
pub(crate) fn one_line(text: &str, max_chars: usize) -> Option<String> {
    let kept: String = text
        .chars()
        .filter(|&c| !ignorable(c))
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let line: String = kept
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max_chars)
        .collect();
    (!line.is_empty()).then_some(line)
}

fn bidi_control(c: char) -> bool {
    matches!(
        c,
        '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}

/// Text kept as written, line breaks and tabs included, with the
/// bidirectional controls that reorder it and every other control dropped.
/// Joiners and variation selectors stay: a request or a reply is prose
/// (emoji sequences, Persian and Indic script), not a name to tell apart.
pub(crate) fn block(text: &str) -> String {
    text.chars()
        .filter(|&c| !bidi_control(c) && (!c.is_control() || matches!(c, '\n' | '\t')))
        .collect()
}

/// What a name looks like once compatibility forms are folded (full-width
/// letters, jamo that compose to a syllable), case folded, and everything
/// but letters and digits dropped (ignorables, spaces, blanks such as
/// U+2800, punctuation): two names with the same skeleton read alike.
pub(crate) fn skeleton(text: &str) -> String {
    text.nfkc()
        .filter(|&c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_keeps_what_draws_and_nothing_that_reorders_or_hides() {
        assert_eq!(
            one_line("ci\u{202E}-lead\u{200B}\n  봐 줘\u{FEFF}", 64).as_deref(),
            Some("ci-lead 봐 줘")
        );
        assert_eq!(one_line("\u{200B}\u{3164} ", 64), None);
        assert_eq!(one_line("가나다라", 2).as_deref(), Some("가나"));
        assert_eq!(block("첫 줄\u{202E}\n\t둘째\u{0007}"), "첫 줄\n\t둘째");
        assert_eq!(block("👩\u{200D}💻 ❤\u{FE0F}"), "👩\u{200D}💻 ❤\u{FE0F}");
    }

    #[test]
    fn names_that_read_the_same_share_a_skeleton() {
        for lookalike in [
            "나\u{200B}",
            " 나 ",
            "\u{1102}\u{1161}",
            "ㄴㅏ",
            "나\u{2800}",
            "나\u{FFFC}",
        ] {
            assert_eq!(skeleton(lookalike), skeleton("나"), "{lookalike:?}");
        }
        assert_eq!(skeleton("ｏｐｅｒａｔｏｒ"), "operator");
        assert_eq!(skeleton("Oper\u{00AD}ator"), "operator");
        assert_ne!(skeleton("ci-lead"), skeleton("나"));
    }
}
