//! Cutting a styled read (an [`Ask::Rows`](crate::Ask::Rows) or
//! [`Ask::Screen`](crate::Ask::Screen) answer) to a bound, the one rule the
//! holder's [`MAX_READ_BYTES`](crate::MAX_READ_BYTES) cut and hided's phone
//! frame both follow: what is kept is the newest part, it never starts inside
//! a UTF-8 character or an escape sequence, and the style in effect where it
//! starts is opened again in front of it, so its first character keeps its
//! color.
//!
//! The text is the parser's VT formatter output (hide-vt, Ghostty's
//! `formatter.zig` and `Style.formatterVt` at the pinned source), and the
//! rule leans on what that output can hold rather than on a general ANSI
//! parser: a cell's style is written whole at every change as a reset
//! (`ESC [ 0 m`) and one SGR per attribute, at most [`STYLE_MAX`] bytes, so
//! the style in effect at a point is the run of SGRs right after the last
//! reset before it; a VT selection writes no hyperlink (OSC 8) and no other
//! sequence; a cell's text is at most 65 code points (a base and
//! `grapheme_max_len` 64 more), 260 bytes.
//!
//! `cost` says what a piece of text takes against the room (its bytes, or the
//! bytes its JSON string takes) and must add up over concatenation.

/// The most bytes the formatter writes for one cell's style: `Style.formatterVt`
/// assembles it in a 128-byte buffer (a reset, seven flags, overline and
/// three colors). A style run longer than this is not the formatter's and is
/// not opened again.
pub const STYLE_MAX: usize = 128;

/// The most bytes below its room that [`newest_bytes`] may leave unused when
/// it cuts, for the formatter's output with `cost` its bytes: less than one
/// UTF-8 character (3) that did not fit, the part of one style it steps past
/// so as not to start inside it (under [`STYLE_MAX`]), and the bytes it gives
/// up to make room for the style it opens again in front (at most
/// [`STYLE_MAX`], plus the one character or sequence that crosses it, under
/// [`STYLE_MAX`] again). A cut therefore keeps more than `room - CUT_SLACK`.
pub const CUT_SLACK: usize = 3 + 3 * STYLE_MAX;

const ESC: u8 = 0x1b;

/// The newest part of `text` that costs at most `room`, and whether anything
/// was left out. It starts at a character or at an escape sequence, never
/// inside one, and when it cuts, the style in effect at its start is put in
/// front of it.
pub fn newest_bytes(text: &str, room: usize, cost: &dyn Fn(&str) -> usize) -> (String, bool) {
    let mut start = text.len();
    let mut used = 0;
    let mut whole = true;
    for (index, ch) in text.char_indices().rev() {
        let len = cost(&text[index..index + ch.len_utf8()]);
        if used + len > room {
            whole = false;
            break;
        }
        used += len;
        start = index;
    }
    if whole {
        return (text.to_owned(), false);
    }
    let bytes = text.as_bytes();
    start = past_sequence(bytes, start);
    let mut style = style_at(text, start);
    let mut tail = cost(&text[start..]);
    // The style takes room too: give up the oldest characters, or whole
    // sequences, until both fit. A reset or an SGR given up changes the
    // style the rest starts in.
    while start < text.len() && cost(&style) + tail > room {
        let next = token_end(bytes, start);
        let token = &text[start..next];
        tail -= cost(token);
        if token == "\x1b[0m" || token == "\x1b[m" {
            style.clear();
        } else if is_sgr(token) {
            style.push_str(token);
        }
        start = next;
    }
    if start == text.len() {
        return (String::new(), true);
    }
    style.push_str(&text[start..]);
    (style, true)
}

/// The newest whole lines of `text` that cost at most `room`, and whether any
/// were left out; an older line that does not fit is left out whole. A line
/// that alone costs more than `room` is the one exception: its newest part
/// ([`newest_bytes`]) fills what the newer lines left, since a terminal's
/// newest characters are what it means.
pub fn newest_lines(text: &str, room: usize, cost: &dyn Fn(&str) -> usize) -> (String, bool) {
    let bytes = text.as_bytes();
    let (mut start, mut used) = (text.len(), 0);
    while start > 0 {
        let line = bytes[..start - 1]
            .iter()
            .rposition(|&byte| byte == b'\n')
            .map_or(0, |newline| newline + 1);
        let len = cost(&text[line..start]);
        if used + len <= room {
            used += len;
            start = line;
            continue;
        }
        if len <= room {
            return (text[start..].to_owned(), true);
        }
        let (mut kept, _) = newest_bytes(&text[line..start], room - used, cost);
        kept.push_str(&text[start..]);
        return (kept, true);
    }
    (text.to_owned(), false)
}

/// `at`, or the end of the escape sequence `at` falls inside.
fn past_sequence(bytes: &[u8], at: usize) -> usize {
    match bytes[..at].iter().rposition(|&byte| byte == ESC) {
        Some(escape) => sequence_end(bytes, escape).max(at),
        None => at,
    }
}

/// Where the escape sequence that starts at `escape` ends (exclusive): a CSI
/// at its final byte, an OSC at its BEL or string terminator, any other at
/// its second byte; the text's end when it ends unterminated.
fn sequence_end(bytes: &[u8], escape: usize) -> usize {
    let after = escape + 1;
    match bytes.get(after) {
        Some(b'[') => bytes[after + 1..]
            .iter()
            .position(|byte| (0x40..=0x7e).contains(byte))
            .map_or(bytes.len(), |end| after + 1 + end + 1),
        Some(b']') => {
            let body = &bytes[after + 1..];
            let mut index = 0;
            while index < body.len() {
                match body[index] {
                    0x07 => return after + 1 + index + 1,
                    ESC if body.get(index + 1) == Some(&b'\\') => return after + 1 + index + 2,
                    _ => index += 1,
                }
            }
            bytes.len()
        }
        Some(_) => after + 1,
        None => bytes.len(),
    }
}

/// The end of the character or escape sequence that starts at `at`.
fn token_end(bytes: &[u8], at: usize) -> usize {
    if bytes[at] == ESC {
        return sequence_end(bytes, at);
    }
    let mut end = at + 1;
    while end < bytes.len() && (bytes[end] & 0xc0) == 0x80 {
        end += 1;
    }
    end
}

fn is_sgr(token: &str) -> bool {
    token.starts_with("\x1b[") && token.ends_with('m')
}

/// The SGR sequences right after the last reset before `start` (or from the
/// text's start when there is none), which open the style in effect there.
fn style_at(text: &str, start: usize) -> String {
    let head = &text[..start];
    let after = ["\x1b[0m", "\x1b[m"]
        .iter()
        .filter_map(|reset| head.rfind(reset).map(|at| at + reset.len()))
        .max()
        .unwrap_or(0);
    let bytes = text.as_bytes();
    let mut at = after;
    while at < start && bytes[at] == ESC && bytes.get(at + 1) == Some(&b'[') {
        let end = sequence_end(bytes, at);
        if end > start || bytes[end - 1] != b'm' {
            break;
        }
        at = end;
    }
    let style = &text[after..at];
    // Past the formatter's longest style the run is not one it wrote: the
    // rest starts in the default style rather than behind an unbounded one.
    if style.len() > STYLE_MAX {
        return String::new();
    }
    style.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(piece: &str) -> usize {
        piece.len()
    }

    #[test]
    fn a_text_within_its_room_is_kept_whole() {
        let text = "\x1b[0m\x1b[31mred\x1b[0m\r\nplain";
        assert_eq!(
            newest_bytes(text, text.len(), &bytes),
            (text.to_owned(), false)
        );
        assert_eq!(
            newest_lines(text, text.len(), &bytes),
            (text.to_owned(), false)
        );
    }

    #[test]
    fn a_cut_keeps_the_newest_characters_and_opens_their_color_again() {
        let text = format!("\x1b[0m\x1b[38;2;255;0;0m{}END", "x".repeat(100));
        let style = "\x1b[38;2;255;0;0m";
        let (kept, cut) = newest_bytes(&text, style.len() + 20, &bytes);
        assert!(cut);
        assert_eq!(kept, format!("{style}{}END", "x".repeat(17)));
    }

    #[test]
    fn a_cut_never_starts_inside_a_character_or_a_sequence() {
        // Three bytes each: a room of 7 cannot hold the third character's
        // first byte, so two whole characters are kept.
        assert_eq!(newest_bytes("가나다", 7, &bytes), ("나다".to_owned(), true));
        // A room that ends inside `ESC [ 3 4 m` starts after it, with that
        // style opened again, within the room.
        let text = "\x1b[0mab\x1b[0m\x1b[34mcd";
        for room in 1..text.len() {
            let (kept, cut) = newest_bytes(text, room, &bytes);
            assert!(cut && kept.len() <= room, "{room}: {kept:?}");
            // It starts with one of the text's whole sequences or one of its
            // characters, never with what is left of a sequence.
            assert!(
                kept.is_empty()
                    || kept.starts_with("\x1b[0m")
                    || kept.starts_with("\x1b[34m")
                    || kept.starts_with(['a', 'b', 'c', 'd']),
                "{room}: {kept:?}"
            );
            // `c` and `d` were drawn blue: whatever keeps them keeps the blue.
            if let Some(first) = kept.find(['c', 'd']) {
                assert!(kept[..first].contains("\x1b[34m"), "{room}: {kept:?}");
            }
        }
        // A hyperlink's address is never cut either: a cut inside it starts
        // after it.
        let link = "\x1b]8;;https://example.com/a/long/path\x1b\\";
        let text = format!("{link}link-text\x1b]8;;\x1b\\");
        let (kept, _) = newest_bytes(&text, text.len() - 10, &bytes);
        assert_eq!(kept, "link-text\x1b]8;;\x1b\\");
    }

    /// The formatter's longest style: a reset, the seven flags, overline and
    /// three 24-bit colors.
    fn longest_style(red: u8) -> String {
        format!(
            "\x1b[0m\x1b[1m\x1b[2m\x1b[3m\x1b[4m\x1b[5m\x1b[7m\x1b[8m\x1b[9m\x1b[53m\
             \x1b[38;2;{red};255;255m\x1b[48;2;255;{red};255m\x1b[58;2;255;255;{red}m"
        )
    }

    #[test]
    fn a_cut_of_the_longest_styles_and_widest_characters_leaves_less_than_the_slack() {
        assert!(longest_style(255).len() <= STYLE_MAX);
        // Every cell changes style, and its text is a four-byte character
        // or a three-byte one, so a room can end anywhere in either.
        let text: String = (0..4_000)
            .map(|cell| {
                let red = (cell % 250) as u8;
                format!(
                    "{}{}",
                    longest_style(red),
                    if cell % 2 == 0 { "😀" } else { "가" }
                )
            })
            .collect();
        for room in (1_000..60_000).step_by(997) {
            let (kept, cut) = newest_bytes(&text, room, &bytes);
            assert!(
                cut && kept.len() <= room && kept.len() + CUT_SLACK > room,
                "{room}: {}",
                kept.len()
            );
            // It opens with the whole style of its first character, whose
            // red is the cell's.
            let first = kept.find(['😀', '가']).unwrap();
            let opened = &kept[..first];
            assert!(
                opened.starts_with("\x1b[1m") || opened.starts_with("\x1b[0m"),
                "{room}: {opened:?}"
            );
            // The kept characters are the newest cells, so the first is
            // the cell that many from the end.
            let cell = 4_000 - kept.matches(['😀', '가']).count();
            assert!(
                opened.contains(&format!("\x1b[38;2;{};255;255m", (cell % 250) as u8)),
                "{room}: cell {cell} opened {opened:?}"
            );
        }
    }

    #[test]
    fn a_run_longer_than_any_style_the_formatter_writes_is_not_opened_again() {
        let text = format!("\x1b[0m{}{}", "\x1b[1m".repeat(100), "x".repeat(100));
        let (kept, cut) = newest_bytes(&text, 50, &bytes);
        assert_eq!((kept, cut), ("x".repeat(50), true));
    }

    #[test]
    fn lines_are_kept_whole_newest_first_and_an_older_line_that_does_not_fit_goes_whole() {
        let text = "old line\r\nmiddle line\r\nnew";
        let (kept, cut) = newest_lines(text, "middle line\r\nnew".len() + 3, &bytes);
        assert_eq!((kept.as_str(), cut), ("middle line\r\nnew", true));
    }

    #[test]
    fn a_line_longer_than_the_whole_room_keeps_its_newest_part() {
        let long = format!("\x1b[0m\x1b[32m{}END\x1b[0m", "x".repeat(1_000));
        let text = format!("older\r\n{long}\r\nnewest");
        let (kept, cut) = newest_lines(&text, 200, &bytes);
        assert!(cut && kept.len() <= 200, "{}", kept.len());
        assert!(kept.starts_with("\x1b[32mx"), "{kept:?}");
        assert!(kept.ends_with("END\x1b[0m\r\nnewest"), "{kept:?}");
        assert!(!kept.contains("older"));
    }
}
