// Derived from Herdr v0.9.5 (https://github.com/herdrdev/herdr, commit 2ecf19a),
// src/protocol/render_ansi.rs.
// Copyright 2025 herdr contributors. Licensed under the Apache License,
// Version 2.0; see NOTICE at the repository root.
// Changed by Hide: encodes the holder's own rows (`crate::screen`) instead of
// ratatui frames, skips a row the capture shares unchanged, starts a full
// screen with a display reset so any earlier state of the receiver is gone,
// splits a screen between rows into fragments of at most `MAX_FRAGMENT`
// bytes, writes overline, and drops the drawn cursor, the surface patches,
// the profiling and the IME anchor repeat, which only Herdr's own client
// uses.

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

/// The most bytes one cell can encode to: a move, the longest attributes,
/// a hyperlink closed and opened, and its text.
const MAX_CELL: usize = 16 + 64 + 8 + (MAX_LINK + 8) + MAX_TEXT;

/// What a fragment keeps free for the cursor and the end of the screen.
const TAIL: usize = 64;

// One row always fits a fragment, so a screen is split, never refused.
const _: () = assert!(
    MAX_CELL * hide_holder_protocol::MAX_COLS as usize + SYNC_START.len() + RESET.len() + TAIL
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
        out.push(b"\x1b[?25l");
        // Unlinked cells must not inherit a hyperlink left open.
        out.push(b"\x1b]8;;\x1b\\");
        let full = previous.is_none();
        let mut wrote = false;
        match &previous {
            None => {
                out.push(RESET);
                write_all_cells(&mut out, &frame);
                self.shape = 0;
                wrote = true;
            }
            Some(previous) => wrote |= write_changed_cells(&mut out, &frame, previous),
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
}
