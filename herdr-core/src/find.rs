//! Searching a pane's whole scrollback.
//!
//! Herdr renders the pane and keeps its history, so the terminal view the
//! shell draws holds only the rows currently on screen. Searching that view
//! can only ever find what is already visible, and a counter built from it
//! reports "1/1" while the buffer holds seven matches. The buffer lives in
//! Herdr, so the search has to ask Herdr for it.
//!
//! Everything here is pure: [`find_matches`] searches text, and
//! [`viewport_anchor`] locates the visible rows inside the whole buffer so a
//! match can be turned into a scroll distance. Fetching either one is
//! [`crate::live`]'s job, off the runtime mutex.

pub(crate) mod pane;

/// How a term is compared. Every option applies to the whole buffer: an option
/// that searched only the visible rows would put back the counter this module
/// exists to fix.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PaneFindOptions {
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
}

/// A match's position in the buffer that was searched: `line` indexes the
/// lines of that buffer, `column` and `length` are in characters, not bytes,
/// because that is what a terminal grid counts in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PaneFindMatch {
    pub line: usize,
    pub column: usize,
    pub length: usize,
}

/// Every match for `term`, in buffer order.
///
/// An empty term matches nothing rather than everything: the caller's question
/// is "where is this", and before anything is typed the answer is nowhere.
///
/// A pattern the regex engine rejects is an error, not zero matches. Half of a
/// pattern is a normal thing to be holding while typing one, and "No matches"
/// would tell the reader their text is absent when it was never searched for.
pub fn find_matches(
    text: &str,
    term: &str,
    options: &PaneFindOptions,
) -> Result<Vec<PaneFindMatch>, String> {
    if term.is_empty() {
        return Ok(Vec::new());
    }
    if options.regex {
        let pattern = compile(term, options)?;
        return Ok(text
            .lines()
            .enumerate()
            .flat_map(|(line_index, line)| regex_line_matches(line, &pattern, line_index))
            .collect());
    }
    Ok(text
        .lines()
        .enumerate()
        .flat_map(|(line_index, line)| line_matches(line, term, options, line_index))
        .collect())
}

fn compile(term: &str, options: &PaneFindOptions) -> Result<regex::Regex, String> {
    // Whole word wraps the caller's pattern rather than being spliced into it,
    // so an alternation stays one unit and `a|b` does not become `\ba|b\b`.
    let pattern = if options.whole_word {
        format!(r"\b(?:{term})\b")
    } else {
        term.to_owned()
    };
    regex::RegexBuilder::new(&pattern)
        .case_insensitive(!options.case_sensitive)
        .build()
        .map_err(|_| "Not a valid pattern".to_owned())
}

fn regex_line_matches(line: &str, pattern: &regex::Regex, line_index: usize) -> Vec<PaneFindMatch> {
    pattern
        .find_iter(line)
        .filter(|found| !found.is_empty())
        .map(|found| {
            // The engine reports byte offsets; a terminal grid counts cells, so
            // both ends are converted before they leave this module.
            let column = line[..found.start()].chars().count();
            PaneFindMatch {
                line: line_index,
                column,
                length: found.as_str().chars().count(),
            }
        })
        .collect()
}

fn line_matches(
    line: &str,
    term: &str,
    options: &PaneFindOptions,
    line_index: usize,
) -> Vec<PaneFindMatch> {
    let haystack: Vec<char> = if options.case_sensitive {
        line.chars().collect()
    } else {
        line.chars().flat_map(char::to_lowercase).collect()
    };
    let needle: Vec<char> = if options.case_sensitive {
        term.chars().collect()
    } else {
        term.chars().flat_map(char::to_lowercase).collect()
    };
    // Lowercasing can change a string's length, and a match reported in
    // lowercased coordinates would land on the wrong cell of the real line.
    // Comparing per character keeps the two in step for every case-folding
    // that maps one character to one character, and the fold that does not
    // (ß to ss) simply does not match, which is better than pointing at the
    // wrong column.
    if needle.is_empty() || haystack.len() < needle.len() {
        return Vec::new();
    }

    let mut matches = Vec::new();
    let mut start = 0;
    while start + needle.len() <= haystack.len() {
        if haystack[start..start + needle.len()] == needle[..]
            && (!options.whole_word || is_whole_word(&haystack, start, needle.len()))
        {
            matches.push(PaneFindMatch {
                line: line_index,
                column: start,
                length: needle.len(),
            });
            start += needle.len();
        } else {
            start += 1;
        }
    }
    matches
}

fn is_whole_word(haystack: &[char], start: usize, length: usize) -> bool {
    let before = start.checked_sub(1).map(|index| haystack[index]);
    let after = haystack.get(start + length).copied();
    !before.is_some_and(is_word_character) && !after.is_some_and(is_word_character)
}

fn is_word_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

/// Where the visible rows begin inside the whole buffer.
///
/// The two reads are separate requests and do not agree on length - the buffer
/// read carries a trailing blank the visible read does not, and either can
/// gain a line between the calls - so the offset is found by locating the
/// visible rows rather than by subtracting counts. Without it a match's line
/// number cannot be turned into a scroll distance.
///
/// The search runs from the end because a terminal repeats itself: a prompt,
/// or a rerun of the same command, appears many times, and the one on screen
/// is the most recent.
pub fn viewport_anchor(buffer: &str, visible: &str) -> Option<usize> {
    let buffer_lines: Vec<&str> = buffer.lines().collect();
    let visible_lines: Vec<&str> = trim_trailing_blanks(visible.lines().collect());
    if visible_lines.is_empty() || visible_lines.len() > buffer_lines.len() {
        return None;
    }
    (0..=buffer_lines.len() - visible_lines.len())
        .rev()
        .find(|&start| buffer_lines[start..start + visible_lines.len()] == visible_lines[..])
}

fn trim_trailing_blanks(mut lines: Vec<&str>) -> Vec<&str> {
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    lines
}

/// How far to scroll, in lines, to put `line` in the middle of a viewport that
/// currently starts at `viewport_top`.
///
/// Positive scrolls back through history, negative scrolls toward the newest
/// output, and zero means the line is already where it should be. The result
/// is recomputed from the reported viewport every time rather than accumulated,
/// so a step that lands short - or a pane that scrolled underneath - corrects
/// itself on the next one instead of drifting.
pub fn scroll_delta(line: usize, viewport_top: usize, viewport_rows: usize) -> i64 {
    if viewport_rows == 0 {
        return 0;
    }
    // Already on screen: leave the viewport alone. Recentring a match the
    // reader can already see would make every step jump the pane.
    if line >= viewport_top && line < viewport_top + viewport_rows {
        return 0;
    }
    let target_top = line.saturating_sub(viewport_rows / 2);
    viewport_top as i64 - target_top as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(case_sensitive: bool, whole_word: bool) -> PaneFindOptions {
        PaneFindOptions {
            case_sensitive,
            whole_word,
            regex: false,
        }
    }

    fn regex_options(whole_word: bool) -> PaneFindOptions {
        PaneFindOptions {
            case_sensitive: false,
            whole_word,
            regex: true,
        }
    }

    fn matches_of(text: &str, term: &str, options: PaneFindOptions) -> Vec<PaneFindMatch> {
        find_matches(text, term, &options).expect("the pattern compiles")
    }

    #[test]
    fn a_match_is_reported_for_every_occurrence_across_every_line() {
        let text = "alpha needle\nbeta\nneedle needle\n";
        let found =
            find_matches(text, "needle", &options(false, false)).expect("a literal term compiles");
        assert_eq!(
            found,
            vec![
                PaneFindMatch {
                    line: 0,
                    column: 6,
                    length: 6
                },
                PaneFindMatch {
                    line: 2,
                    column: 0,
                    length: 6
                },
                PaneFindMatch {
                    line: 2,
                    column: 7,
                    length: 6
                },
            ]
        );
    }

    /// The count is the whole point of searching the buffer instead of the
    /// screen, so it is asserted on a buffer far longer than any viewport.
    #[test]
    fn matches_below_the_visible_rows_are_counted_too() {
        let mut text = String::new();
        for index in 0..200 {
            if index % 40 == 1 {
                text.push_str(&format!("line {index} NEEDLEWORD here\n"));
            } else {
                text.push_str(&format!("line {index} filler\n"));
            }
        }
        let found = find_matches(&text, "NEEDLEWORD", &options(false, false))
            .expect("a literal term compiles");
        assert_eq!(found.len(), 5);
        assert_eq!(found.first().map(|m| m.line), Some(1));
        assert_eq!(found.last().map(|m| m.line), Some(161));
    }

    #[test]
    fn case_folding_is_the_default_and_case_sensitivity_narrows_it() {
        let text = "Needle needle NEEDLE\n";
        assert_eq!(matches_of(text, "needle", options(false, false)).len(), 3);
        assert_eq!(matches_of(text, "needle", options(true, false)).len(), 1);
    }

    #[test]
    fn a_whole_word_search_skips_a_term_inside_a_longer_word() {
        let text = "cat catalog the_cat cat_alog cat.\n";
        let found = matches_of(text, "cat", options(false, true));
        assert_eq!(
            found.iter().map(|m| m.column).collect::<Vec<_>>(),
            vec![0, 29]
        );
    }

    #[test]
    fn an_empty_term_matches_nothing_rather_than_everything() {
        assert!(matches_of("anything at all\n", "", options(false, false)).is_empty());
    }

    #[test]
    fn a_column_counts_characters_not_bytes() {
        // Each of these is multi-byte, so a byte offset would report 6.
        let found = matches_of("한국어 needle\n", "needle", options(false, false));
        assert_eq!(found.first().map(|m| m.column), Some(4));
    }

    #[test]
    fn a_pattern_matches_across_the_whole_buffer_the_same_way_a_literal_does() {
        let text = "error 404 here\nfine\nerror 500 here\n";
        let found = matches_of(text, r"error \d+", regex_options(false));
        assert_eq!(
            found,
            vec![
                PaneFindMatch {
                    line: 0,
                    column: 0,
                    length: 9
                },
                PaneFindMatch {
                    line: 2,
                    column: 0,
                    length: 9
                },
            ]
        );
    }

    /// Whole word wraps the caller's pattern instead of being spliced into it,
    /// so an alternation stays one unit.
    #[test]
    fn a_whole_word_pattern_keeps_an_alternation_together() {
        let text = "cat dog category\n";
        let found = matches_of(text, "cat|dog", regex_options(true));
        assert_eq!(
            found.iter().map(|m| m.column).collect::<Vec<_>>(),
            vec![0, 4]
        );
    }

    /// Half a pattern is a normal thing to be holding while typing one, so it
    /// is reported as what it is. "No matches" would say the text is absent
    /// when it was never searched for.
    #[test]
    fn a_pattern_the_engine_rejects_is_an_error_not_an_empty_result() {
        assert_eq!(
            find_matches("anything\n", "[unclosed", &regex_options(false)),
            Err("Not a valid pattern".to_owned())
        );
    }

    #[test]
    fn a_pattern_reports_columns_in_characters_like_a_literal_does() {
        let found = matches_of("한국어 e404\n", r"e\d+", regex_options(false));
        assert_eq!(found.first().map(|m| m.column), Some(4));
    }

    /// A pattern that can match nothing would otherwise report a match on every
    /// cell and make stepping impossible.
    #[test]
    fn a_pattern_that_matches_the_empty_string_reports_no_matches() {
        assert!(matches_of("some tale\n", "x*", regex_options(false)).is_empty());
    }

    #[test]
    fn the_visible_rows_are_located_inside_the_buffer() {
        let buffer = "one\ntwo\nthree\nfour\nfive\n";
        assert_eq!(viewport_anchor(buffer, "three\nfour\nfive\n"), Some(2));
        assert_eq!(viewport_anchor(buffer, "one\ntwo\n"), Some(0));
    }

    /// A terminal repeats itself, so the visible rows can appear more than once
    /// in the buffer. The one on screen is the most recent.
    #[test]
    fn a_repeated_viewport_anchors_to_its_most_recent_appearance() {
        let buffer = "prompt\nls\nprompt\nls\nprompt\n";
        assert_eq!(viewport_anchor(buffer, "prompt\nls\n"), Some(2));
    }

    #[test]
    fn the_blank_rows_a_visible_read_pads_with_do_not_defeat_the_anchor() {
        let buffer = "one\ntwo\nthree\n";
        assert_eq!(viewport_anchor(buffer, "two\nthree\n\n\n"), Some(1));
    }

    #[test]
    fn a_viewport_that_is_not_in_the_buffer_reports_no_anchor() {
        assert_eq!(viewport_anchor("one\ntwo\n", "nothing like it\n"), None);
        assert_eq!(viewport_anchor("one\n", "one\ntwo\nthree\n"), None);
    }

    #[test]
    fn a_match_already_on_screen_does_not_move_the_viewport() {
        assert_eq!(scroll_delta(50, 40, 43), 0);
        assert_eq!(scroll_delta(40, 40, 43), 0);
        assert_eq!(scroll_delta(82, 40, 43), 0);
    }

    #[test]
    fn a_match_above_the_viewport_scrolls_back_and_one_below_scrolls_forward() {
        // Centring row 10 in a 43-row viewport puts the top at 0, which is 40
        // lines back from where it is now.
        assert_eq!(scroll_delta(10, 40, 43), 40);
        // Row 200 centres at top 179, which is 139 lines forward.
        assert_eq!(scroll_delta(200, 40, 43), -139);
    }

    #[test]
    fn a_viewport_with_no_rows_is_not_scrolled() {
        assert_eq!(scroll_delta(10, 0, 0), 0);
    }
}
