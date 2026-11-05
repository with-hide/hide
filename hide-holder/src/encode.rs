// Derived from Herdr v0.9.5 (https://github.com/herdrdev/herdr, commit 2ecf19a),
// src/protocol/render_ansi.rs.
// Copyright 2025 herdr contributors. Licensed under the Apache License,
// Version 2.0; see NOTICE at the repository root.
// Changed by Hide: encodes the holder's own rows (`crate::screen`) instead of
// ratatui frames, skips a row the capture shares unchanged, starts a full
// screen with a display reset so any earlier state of the receiver is gone,
// splits a screen between rows into fragments of at most `MAX_FRAGMENT`
// bytes, writes overline, says which rows soft-wrap (`WRAP_OSC`), and drops
// the drawn cursor, the surface patches, the profiling and the IME anchor
// repeat, which only Herdr's own client uses.

//! Screen fragments: the ANSI that draws a captured screen on a receiver,
//! whole or as the cells that changed since the last one.
//!
//! Every fragment is wrapped in synchronized output and hides the cursor
//! before it writes a cell. A full one first puts the receiver back to a
//! known display: the normal screen, a soft reset (DECSTR: scroll region,
//! origin mode, insert mode, character sets, attributes), autowrap and
//! reverse video off, and a clear. It never turns on an input mode or asks
//! the receiver anything, so a terminal that draws it answers nothing (PRD
//! engine-bundle-2-pane-holder D-7).
//!
//! A full fragment, and a delta whose rows soft-wrap differently from the
//! last sent, also says which rows soft-wrap, as [`WRAP_OSC`]: the cells are
//! placed with autowrap off, so the receiver cannot tell a row the program's
//! text ran past from one it ended with a line break. A program's own OSC
//! never reaches the receiver, since a fragment is drawn from the parsed
//! cells, so only the holder writes this one.

use std::cmp;
use std::io::Write;

use hide_holder_protocol::MAX_FRAGMENT;

use crate::screen::{Cell, Color, Frame, MAX_LINK, MAX_TEXT, Row, attrs};

const SYNC_START: &[u8] = b"\x1b[?2026h";
const SYNC_END: &[u8] = b"\x1b[?2026l";

/// What a full screen writes before its cells: the normal screen, a soft
/// reset, autowrap and reverse video off (the encoder places every cell, and
/// a write in the last column must not wrap), plain attributes, a clear.
const RESET: &[u8] = b"\x1b[?1049l\x1b[!p\x1b[?7l\x1b[?5l\x1b[r\x1b[0m\x1b[H\x1b[2J";

/// The private OSC that says which rows of the screen soft-wrap:
/// `ESC ] 7719 ; <cols> ; <rows> ; <hex> ESC \`, where hex digit `k` holds
/// rows `4k` to `4k + 3`, row `4k + j` in bit `1 << j`, with trailing zero
/// digits left out (empty when no row wraps). The web shell reads it
/// (`web/src/terminalWrap.ts`); a terminal that does not know it drops it.
pub const WRAP_OSC: u16 = 7719;

/// The most bytes [`WRAP_OSC`] takes: its frame, two sizes, a digit per four rows.
const MAX_WRAP: usize = 24 + hide_holder_protocol::MAX_ROWS as usize / 4;

/// The most bytes one cell can encode to: a move, the longest attributes,
/// a hyperlink closed and opened, and its text.
const MAX_CELL: usize = 16 + 64 + 8 + (MAX_LINK + 8) + MAX_TEXT;

/// What a fragment keeps free for the cursor and the end of the screen.
const TAIL: usize = 64;

// One row always fits a fragment, so a screen is split, never refused.
const _: () = assert!(
    MAX_CELL * hide_holder_protocol::MAX_COLS as usize
        + SYNC_START.len()
        + RESET.len()
        + MAX_WRAP
        + TAIL
        < MAX_FRAGMENT
);

/// A screen encoded: whether it is full, and its pieces in order, each at
/// most [`MAX_FRAGMENT`] bytes. No pieces when nothing changed.
pub struct Encoded {
    pub full: bool,
    pub pieces: Vec<Vec<u8>>,
}

/// Remembers what the receiver was last sent.
#[derive(Default)]
pub struct Encoder {
    last: Option<Frame>,
    shape: u8,
}

impl Encoder {
    /// Encodes `frame` against what was sent last: full when `full` is
    /// asked, nothing was sent yet, or the size changed.
    pub fn encode(&mut self, frame: Frame, full: bool) -> Encoded {
        let previous = self
            .last
            .take()
            .filter(|last| !full && last.cols == frame.cols && last.rows == frame.rows);
        let mut out = Pieces::default();
        out.push(SYNC_START);
        // Unlinked cells must not inherit a hyperlink left open.
        out.push(b"\x1b]8;;\x1b\\");
        let full = previous.is_none();
        let mut wrote = false;
        match &previous {
            None => {
                // After the reset, whose soft reset shows the cursor again.
                out.push(RESET);
                out.push(b"\x1b[?25l");
                write_wrapped(&mut out, &frame);
                write_all_cells(&mut out, &frame);
                self.shape = 0;
                wrote = true;
            }
            Some(previous) => {
                out.push(b"\x1b[?25l");
                if frame.wrapped != previous.wrapped {
                    write_wrapped(&mut out, &frame);
                    wrote = true;
                }
                wrote |= write_changed_cells(&mut out, &frame, previous);
            }
        }
        let unchanged = previous
            .as_ref()
            .is_some_and(|previous| previous.cursor == frame.cursor && !wrote);
        if unchanged {
            self.last = previous;
            return Encoded {
                full: false,
                pieces: Vec::new(),
            };
        }
        write_cursor(&mut out, &frame, &mut self.shape, full);
        out.push(SYNC_END);
        self.last = Some(frame);
        Encoded {
            full,
            pieces: out.finish(),
        }
    }
}

/// Fragments being written: a row goes whole into the current one, or
/// starts the next when it would pass the bound.
#[derive(Default)]
struct Pieces {
    done: Vec<Vec<u8>>,
    current: Vec<u8>,
}

impl Pieces {
    fn push(&mut self, bytes: &[u8]) {
        self.current.extend_from_slice(bytes);
    }

    fn row(&mut self, row: &[u8]) {
        if self.current.len() + row.len() + TAIL > MAX_FRAGMENT {
            self.done.push(std::mem::take(&mut self.current));
        }
        self.current.extend_from_slice(row);
    }

    fn finish(mut self) -> Vec<Vec<u8>> {
        self.done.push(self.current);
        self.done
    }
}

/// What the receiver's attributes and hyperlink are at a point.
#[derive(Default)]
struct Pen {
    sgr: String,
    style: Option<(Color, Color, u16)>,
    link: Option<String>,
}

fn color_sgr(color: Color, foreground: bool, out: &mut String) {
    use std::fmt::Write as _;
    let base = if foreground { 38 } else { 48 };
    match color {
        Color::Default => out.push_str(if foreground { "39" } else { "49" }),
        Color::Indexed(index) => {
            let _ = write!(out, "{base};5;{index}");
        }
        Color::Rgb(r, g, b) => {
            let _ = write!(out, "{base};2;{r};{g};{b}");
        }
    }
}

/// A complete SGR for a cell's style, from a reset.
fn build_sgr(fg: Color, bg: Color, bits: u16) -> String {
    let mut sgr = String::from("\x1b[0");
    for (bit, part) in [(attrs::BOLD, "1"), (attrs::DIM, "2"), (attrs::ITALIC, "3")] {
        if bits & bit != 0 {
            sgr.push(';');
            sgr.push_str(part);
        }
    }
    if bits & attrs::UNDERLINE != 0 {
        sgr.push(';');
        sgr.push_str(
            match (bits & attrs::UNDERLINE_STYLE) >> attrs::UNDERLINE_STYLE_SHIFT {
                2 => "4:2",
                3 => "4:3",
                4 => "4:4",
                5 => "4:5",
                _ => "4",
            },
        );
    }
    for (bit, part) in [
        (attrs::BLINK, "5"),
        (attrs::INVISIBLE, "8"),
        (attrs::STRIKE, "9"),
        (attrs::OVERLINE, "53"),
    ] {
        if bits & bit != 0 {
            sgr.push(';');
            sgr.push_str(part);
        }
    }
    sgr.push(';');
    color_sgr(fg, true, &mut sgr);
    sgr.push(';');
    color_sgr(bg, false, &mut sgr);
    sgr.push('m');
    sgr
}

fn write_cursor_position(out: &mut Vec<u8>, x: u16, y: u16) {
    let _ = write!(out, "\x1b[{};{}H", y + 1, x + 1);
}

fn write_cell(out: &mut Vec<u8>, at: Option<(u16, u16)>, row: &Row, cell: &Cell, pen: &mut Pen) {
    if let Some((x, y)) = at {
        write_cursor_position(out, x, y);
    }
    let style = (cell.fg, cell.bg, cell.attrs);
    if pen.style != Some(style) {
        let sgr = build_sgr(cell.fg, cell.bg, cell.attrs);
        if sgr != pen.sgr {
            out.extend_from_slice(sgr.as_bytes());
            pen.sgr = sgr;
        }
        pen.style = Some(style);
    }
    let link = row.link(cell);
    if pen.link.as_deref() != link {
        if pen.link.is_some() {
            out.extend_from_slice(b"\x1b]8;;\x1b\\");
        }
        pen.link = link.map(str::to_owned);
        if let Some(uri) = link {
            let _ = write!(out, "\x1b]8;;{uri}\x1b\\");
        }
    }
    out.extend_from_slice(row.text(cell).as_bytes());
}

fn close_link(out: &mut Vec<u8>, pen: &mut Pen) {
    if pen.link.take().is_some() {
        out.extend_from_slice(b"\x1b]8;;\x1b\\");
    }
}

fn write_all_cells(out: &mut Pieces, frame: &Frame) {
    let mut pen = Pen::default();
    let mut bytes = Vec::new();
    for (y, row) in frame.lines.iter().enumerate() {
        let y = y as u16;
        bytes.clear();
        let mut to_skip = 0usize;
        let mut next_inline = None;
        for (x, cell) in row.cells().iter().enumerate() {
            let x = x as u16;
            if to_skip > 0 {
                to_skip -= 1;
                continue;
            }
            if cell.width == 0 {
                next_inline = None;
                continue;
            }
            let at = (next_inline != Some(x)).then_some((x, y));
            write_cell(&mut bytes, at, row, cell, &mut pen);
            next_inline =
                (row.text(cell).is_ascii() && cell.width == 1).then_some(x.saturating_add(1));
            to_skip = usize::from(cell.width).saturating_sub(1);
        }
        out.row(&bytes);
    }
    bytes.clear();
    close_link(&mut bytes, &mut pen);
    bytes.extend_from_slice(b"\x1b[0m");
    out.push(&bytes);
}

/// Writes the cells that changed; answers whether it wrote any.
fn write_changed_cells(out: &mut Pieces, frame: &Frame, previous: &Frame) -> bool {
    let mut pen = Pen::default();
    let mut bytes = Vec::new();
    for (y, (row, before)) in frame.lines.iter().zip(&previous.lines).enumerate() {
        if std::sync::Arc::ptr_eq(row, before) {
            continue;
        }
        let y = y as u16;
        bytes.clear();
        let mut invalidated = 0usize;
        let mut to_skip = 0usize;
        // Autowrap is off, so safe cells advance inline.
        let mut next_inline = None;
        for (x, (cell, old)) in row.cells().iter().zip(before.cells()).enumerate() {
            let x = x as u16;
            if cell.width != 0 && (!row.same(cell, before, old) || invalidated > 0) && to_skip == 0
            {
                let at = (next_inline != Some(x) || invalidated > 0).then_some((x, y));
                write_cell(&mut bytes, at, row, cell, &mut pen);
                next_inline =
                    (row.text(cell).is_ascii() && cell.width == 1).then_some(x.saturating_add(1));
            }
            to_skip = usize::from(cell.width).saturating_sub(1);
            let affected = cmp::max(usize::from(cell.width), usize::from(old.width));
            invalidated = cmp::max(affected, invalidated).saturating_sub(1);
        }
        if !bytes.is_empty() {
            out.row(&bytes);
        }
    }
    bytes.clear();
    close_link(&mut bytes, &mut pen);
    let wrote = !pen.sgr.is_empty();
    if wrote {
        bytes.extend_from_slice(b"\x1b[0m");
    }
    out.push(&bytes);
    wrote
}

/// Writes [`WRAP_OSC`] for `frame`.
fn write_wrapped(out: &mut Pieces, frame: &Frame) {
    let mut bytes = Vec::with_capacity(24 + frame.wrapped.len() / 4);
    let _ = write!(bytes, "\x1b]{WRAP_OSC};{};{};", frame.cols, frame.rows);
    let used = frame.wrapped.iter().rposition(|wraps| *wraps).map_or(0, |last| last / 4 + 1);
    for digit in frame.wrapped.chunks(4).take(used) {
        let bits = digit
            .iter()
            .enumerate()
            .fold(0u8, |bits, (j, wraps)| bits | (u8::from(*wraps) << j));
        bytes.push(b"0123456789abcdef"[usize::from(bits)]);
    }
    bytes.extend_from_slice(b"\x1b\\");
    out.push(&bytes);
}

fn write_cursor(out: &mut Pieces, frame: &Frame, last_shape: &mut u8, full: bool) {
    let cursor = frame.cursor;
    let mut bytes = Vec::new();
    write_cursor_position(&mut bytes, cursor.x, cursor.y);
    if full || cursor.shape != *last_shape {
        let _ = write!(bytes, "\x1b[{} q", cursor.shape);
        *last_shape = cursor.shape;
    }
    bytes.extend_from_slice(if cursor.visible {
        b"\x1b[?25h"
    } else {
        b"\x1b[?25l"
    });
    out.push(&bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_style_is_written_whole_from_a_reset() {
        assert_eq!(
            build_sgr(Color::Indexed(3), Color::Rgb(1, 2, 3), attrs::BOLD),
            "\x1b[0;1;38;5;3;48;2;1;2;3m"
        );
        assert_eq!(
            build_sgr(
                Color::Default,
                Color::Default,
                attrs::UNDERLINE | (3 << attrs::UNDERLINE_STYLE_SHIFT) | attrs::OVERLINE
            ),
            "\x1b[0;4:3;53;39;49m"
        );
    }

    fn pane(cols: u16, rows: u16) -> crate::pane::Pane {
        crate::pane::Pane::new(hide_holder_protocol::Size { rows, cols }, None).unwrap()
    }

    fn wrap_sequences(encoded: &Encoded) -> Vec<String> {
        let text = String::from_utf8_lossy(&encoded.pieces.concat()).into_owned();
        text.match_indices("\x1b]7719;")
            .map(|(at, _)| {
                let rest = &text[at + 2..];
                rest[..rest.find("\x1b\\").unwrap()].to_owned()
            })
            .collect()
    }

    // The URL of with-hide/hide-private#312 fills a 29-column row exactly.
    const URL: &str = "http://127.0.0.1:41811/a.html";

    #[test]
    fn a_full_screen_says_which_rows_soft_wrap() {
        let mut ended = pane(29, 4);
        ended.feed(format!("{URL}\r\nosc8 label").as_bytes(), &mut Vec::new());
        let mut wrapped = pane(29, 4);
        wrapped.feed(format!("{URL}osc8 label").as_bytes(), &mut Vec::new());

        let ended = Encoder::default().encode(ended.take(true).unwrap(), true);
        let wrapped = Encoder::default().encode(wrapped.take(true).unwrap(), true);

        assert_eq!(wrap_sequences(&ended), ["7719;29;4;"]);
        assert_eq!(wrap_sequences(&wrapped), ["7719;29;4;1"]);
    }

    #[test]
    fn a_delta_says_the_wraps_again_only_when_they_change() {
        let mut pane = pane(29, 6);
        let mut encoder = Encoder::default();
        pane.feed(URL.as_bytes(), &mut Vec::new());
        assert_eq!(wrap_sequences(&encoder.encode(pane.take(false).unwrap(), false)), ["7719;29;6;"]);

        // The row is marked wrapped when the next character lands, on the row after it.
        pane.feed(b"osc8", &mut Vec::new());
        let wrapped = encoder.encode(pane.take(false).unwrap(), false);
        assert!(!wrapped.full);
        assert_eq!(wrap_sequences(&wrapped), ["7719;29;6;1"]);

        // Output that changes cells but no wrap carries no wrap bytes.
        pane.feed(b" label\r\nplain", &mut Vec::new());
        let unchanged_wraps = encoder.encode(pane.take(false).unwrap(), false);
        assert!(!unchanged_wraps.pieces.is_empty());
        assert!(wrap_sequences(&unchanged_wraps).is_empty());

        // Nothing changed: no bytes at all.
        assert!(encoder.encode(pane.take(false).unwrap(), false).pieces.is_empty());

        // A line erased and rewritten short ends with a break again.
        pane.feed(b"\x1b[H\x1b[2Kshort", &mut Vec::new());
        assert_eq!(wrap_sequences(&encoder.encode(pane.take(false).unwrap(), false)), ["7719;29;6;"]);
    }

    #[test]
    fn rows_past_the_fourth_land_in_later_digits() {
        let mut pane = pane(4, 12);
        // Rows 0, 1 and 9 wrap.
        pane.feed(b"abcdefghijk\r\n\n\n\n\n\n\nlmnopq", &mut Vec::new());
        assert_eq!(
            wrap_sequences(&Encoder::default().encode(pane.take(true).unwrap(), true)),
            ["7719;4;12;302"]
        );
    }

    #[test]
    fn a_program_cannot_write_the_wrap_sequence_itself() {
        let mut pane = pane(29, 4);
        pane.feed(b"\x1b]7719;29;4;f\x1b\\short", &mut Vec::new());
        let encoded = Encoder::default().encode(pane.take(true).unwrap(), true);
        assert_eq!(wrap_sequences(&encoded), ["7719;29;4;"]);
    }

    #[test]
    fn a_screen_past_the_fragment_bound_is_split_between_rows() {
        // Every cell changes color, so each one carries a full style.
        let size = hide_holder_protocol::Size {
            rows: 512,
            cols: 512,
        };
        let mut pane = crate::pane::Pane::new(size, None).unwrap();
        let mut screen = String::new();
        for row in 0..size.rows {
            screen.push_str(&format!("\x1b[{};1H", row + 1));
            for col in 0..size.cols {
                let shade = (row ^ col) % 2 * 200;
                screen.push_str(&format!("\x1b[38;2;{shade};1;2;48;2;3;{shade};4mx"));
            }
        }
        pane.feed(screen.as_bytes(), &mut Vec::new());
        let encoded = Encoder::default().encode(pane.take(true).unwrap(), true);

        assert!(encoded.full);
        assert!(encoded.pieces.len() > 1, "{}", encoded.pieces.len());
        assert!(encoded.pieces[0].starts_with(SYNC_START));
        assert!(encoded.pieces.last().unwrap().ends_with(SYNC_END));
        // A later piece starts at the start of a row after the last one's.
        let mut last_row = 0;
        for piece in &encoded.pieces[1..] {
            let row: u16 = std::str::from_utf8(&piece[2..piece.len().min(12)])
                .unwrap()
                .split_once(";1H")
                .and_then(|(row, _)| row.parse().ok())
                .unwrap_or_else(|| panic!("{:?}", &piece[..12]));
            assert!(row > last_row, "row {row} after row {last_row}");
            last_row = row;
        }
        for piece in &encoded.pieces {
            assert!(piece.len() <= MAX_FRAGMENT, "{}", piece.len());
        }
    }
}
