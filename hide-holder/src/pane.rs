// Partly derived from Herdr v0.9.5 (https://github.com/herdrdev/herdr, commit
// 2ecf19a), src/pane/terminal.rs: the wheel's routing and
// `ghostty_recent_read_range`.
// Copyright 2025 herdr contributors. Licensed under the Apache License,
// Version 2.0; see NOTICE at the repository root.
// Changed by Hide: a wheel arrives as lines with an optional cell instead of
// a terminal event, and a read is cut to the protocol's byte bound.

//! The pane's parsed terminal and everything read from it or written for
//! it: the program's output and the answers to its questions, the screen a
//! fragment draws, input that means something (PRD
//! engine-bundle-2-pane-holder D-8), the theme, and its recent text (D-18).
//! One lock holds it; nothing here blocks.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use hide_holder_protocol::{
    Answer, Appearance, MAX_READ_BYTES, Modifiers, Mouse, MouseAction, MouseButton, Scroll, Size,
    Theme,
};
use hide_vt::{ActiveScreen, DefaultColor, RgbColor, Terminal};

use crate::answers::{self, Mark, Scanner};
use crate::screen::{Capture, Defaults, Frame};

/// The parser's scrollback budget, libghostty's approximate bytes (PRD
/// engine-bundle-2-pane-holder D-17).
const SCROLLBACK_BYTES: usize = 2 * 1024 * 1024;

/// How long the screen waits on a program that turned synchronized output
/// on before it is drawn anyway, as Ghostty and kitty bound it.
pub const SYNC_LIMIT: Duration = Duration::from_secs(1);

/// What a wheel turned into.
pub enum Wheel {
    /// Bytes for the program: a mouse report or an arrow key.
    Input(Vec<u8>),
    /// The screen moved through its scrollback.
    Moved,
    /// Nothing changed.
    Still,
}

pub struct Pane {
    terminal: Terminal,
    capture: Capture,
    /// What libghostty answered while it parsed the last output.
    replies: Arc<Mutex<Vec<Vec<u8>>>>,
    scanner: Scanner,
    marks: Vec<(usize, Mark)>,
    defaults: Defaults,
    appearance: Appearance,
    /// The program chose its cursor's shape (DECSCUSR other than 0).
    shaped: bool,
    /// Since when the program has held synchronized output on.
    sync_since: Option<Instant>,
    /// The next capture reads every row: the colors or the size changed.
    everything: bool,
}

impl Pane {
    pub fn new(size: Size) -> Result<Self, String> {
        let mut terminal = Terminal::new(size.cols, size.rows, SCROLLBACK_BYTES)
            .map_err(|error| format!("the terminal parser: {error}"))?;
        let replies = Arc::new(Mutex::new(Vec::new()));
        terminal
            .set_write_pty_callback({
                let replies = replies.clone();
                move |bytes: &[u8]| {
                    replies
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push(bytes.to_vec());
                }
            })
            .map_err(|error| format!("the terminal parser's answers: {error}"))?;
        let mut capture =
            Capture::new().map_err(|error| format!("the terminal parser's screen: {error}"))?;
        let (initial_fg, initial_bg) = capture
            .colors(&terminal)
            .map_err(|error| format!("the terminal parser's colors: {error}"))?;
        Ok(Self {
            terminal,
            capture,
            replies,
            scanner: Scanner::default(),
            marks: Vec::new(),
            defaults: Defaults {
                initial_fg: Some(initial_fg),
                initial_bg: Some(initial_bg),
                ..Defaults::default()
            },
            appearance: Appearance::Unknown,
            shaped: false,
            sync_since: None,
            everything: true,
        })
    }

    /// Parses what the program wrote and answers what it asked, in the
    /// order it asked.
    pub fn feed(&mut self, bytes: &[u8], answers: &mut Vec<u8>) {
        self.marks.clear();
        self.scanner.scan(bytes, &mut self.marks);
        let mut start = 0;
        for index in 0..self.marks.len() {
            let (end, mark) = self.marks[index];
            self.terminal.write(&bytes[start..end]);
            self.take_replies(answers);
            match mark {
                Mark::SizeQuery => {
                    let (rows, cols) = (
                        self.terminal.rows().unwrap_or(0),
                        self.terminal.cols().unwrap_or(0),
                    );
                    answers.extend_from_slice(format!("\x1b[8;{rows};{cols}t").as_bytes());
                }
                Mark::CursorShape(shape) => self.shaped = shape != 0,
                Mark::Reset => self.shaped = false,
            }
            start = end;
        }
        self.terminal.write(&bytes[start..]);
        self.take_replies(answers);
        let synchronized = self
            .terminal
            .mode_get(hide_vt::MODE_SYNCHRONIZED_OUTPUT)
            .unwrap_or(false);
        self.sync_since = match (synchronized, self.sync_since) {
            (true, Some(since)) => Some(since),
            (true, None) => Some(Instant::now()),
            (false, _) => None,
        };
    }

    fn take_replies(&mut self, answers: &mut Vec<u8>) {
        let replies =
            std::mem::take(&mut *self.replies.lock().unwrap_or_else(PoisonError::into_inner));
        for reply in replies {
            if answers::keeps(&reply) {
                answers.extend_from_slice(&reply);
            }
        }
    }

    /// When the screen may be drawn: now, or once the program ends its
    /// synchronized output or [`SYNC_LIMIT`] passes.
    pub fn drawable_at(&self) -> Option<Instant> {
        self.sync_since.map(|since| since + SYNC_LIMIT)
    }

    /// The screen as it is now; `everything` reads every row again.
    pub fn take(&mut self, everything: bool) -> Result<Frame, hide_vt::Error> {
        let everything = everything || std::mem::take(&mut self.everything);
        self.capture
            .take(&self.terminal, &self.defaults, self.shaped, everything)
    }

    /// Resizes the parser; answers what it says of the new size, which with
    /// no pixel size known is nothing (D-9).
    pub fn resize(&mut self, size: Size, answers: &mut Vec<u8>) -> Result<(), hide_vt::Error> {
        self.everything = true;
        let resized = self.terminal.resize(size.cols, size.rows, 0, 0);
        self.take_replies(answers);
        resized
    }

    /// Takes the node's theme: the colors questions are answered with until
    /// the program sets its own. Answers a change of appearance to a program
    /// that asked to be told (mode 2031).
    pub fn theme(&mut self, theme: &Theme, answers: &mut Vec<u8>) -> Result<(), hide_vt::Error> {
        let rgb = |color: Option<hide_holder_protocol::Rgb>| {
            color.map(|color| RgbColor {
                r: color.r,
                g: color.g,
                b: color.b,
            })
        };
        self.terminal
            .set_default_color(DefaultColor::Foreground, rgb(theme.foreground))?;
        self.terminal
            .set_default_color(DefaultColor::Background, rgb(theme.background))?;
        self.terminal
            .set_default_color(DefaultColor::Cursor, rgb(theme.cursor))?;
        let mut palette = hide_vt::default_palette();
        for (index, color) in &theme.palette {
            palette[usize::from(*index)] = rgb(Some(*color)).unwrap_or_default();
        }
        self.terminal.set_default_palette(&palette)?;
        self.terminal.set_color_scheme(match theme.appearance {
            Appearance::Unknown => None,
            Appearance::Dark => Some(hide_vt::ColorScheme::Dark),
            Appearance::Light => Some(hide_vt::ColorScheme::Light),
        });
        self.defaults.theme_fg = rgb(theme.foreground);
        self.defaults.theme_bg = rgb(theme.background);
        self.everything = true;
        self.take_replies(answers);
        let changed = self.appearance != theme.appearance;
        self.appearance = theme.appearance;
        if changed && self.terminal.mode_get(hide_vt::MODE_COLOR_SCHEME_REPORT)? {
            match theme.appearance {
                Appearance::Dark => answers.extend_from_slice(b"\x1b[?997;1n"),
                Appearance::Light => answers.extend_from_slice(b"\x1b[?997;2n"),
                Appearance::Unknown => {}
            }
        }
        Ok(())
    }

    /// Pasted text for the program: wrapped only when it turned bracketed
    /// paste on.
    pub fn paste(&self, text: &[u8]) -> Result<Vec<u8>, hide_vt::Error> {
        let bracketed = self.terminal.mode_get(hide_vt::MODE_BRACKETED_PASTE)?;
        hide_vt::encode_paste(text, bracketed)
    }

    /// A pointer event as the program asked to be told of it, or `None` when
    /// it tracks no mouse, or asked for pixels, which are not known.
    pub fn mouse(&self, mouse: &Mouse) -> Result<Option<Vec<u8>>, hide_vt::Error> {
        let button = mouse.button.map(|button| match button {
            MouseButton::Left => hide_vt::MOUSE_BUTTON_LEFT,
            MouseButton::Middle => hide_vt::MOUSE_BUTTON_MIDDLE,
            MouseButton::Right => hide_vt::MOUSE_BUTTON_RIGHT,
        });
        let action = match mouse.action {
            MouseAction::Press => hide_vt::MOUSE_ACTION_PRESS,
            MouseAction::Release => hide_vt::MOUSE_ACTION_RELEASE,
            MouseAction::Motion => hide_vt::MOUSE_ACTION_MOTION,
        };
        self.report(action, button, (mouse.col, mouse.row), mouse.modifiers)
    }

    fn report(
        &self,
        action: hide_vt::ffi::GhosttyMouseAction,
        button: Option<hide_vt::ffi::GhosttyMouseButton>,
        (col, row): (u16, u16),
        modifiers: Modifiers,
    ) -> Result<Option<Vec<u8>>, hide_vt::Error> {
        if !self.terminal.mouse_tracking_enabled()?
            || self.terminal.mode_get(hide_vt::MODE_MOUSE_SGR_PIXELS)?
        {
            return Ok(None);
        }
        let cols = self.terminal.cols()?;
        let rows = self.terminal.rows()?;
        let mut encoder = hide_vt::MouseEncoder::new()?;
        encoder.set_from_terminal(&self.terminal);
        encoder.set_size(u32::from(cols), u32::from(rows), 1, 1);
        let mut event = hide_vt::MouseEvent::new()?;
        event.set_action(action);
        match button {
            Some(button) => event.set_button(button),
            None => event.clear_button(),
        }
        event.set_mods(mods(modifiers));
        event.set_position(
            f32::from(col.min(cols.saturating_sub(1))),
            f32::from(row.min(rows.saturating_sub(1))),
        );
        let bytes = encoder.encode(&event)?;
        Ok((!bytes.is_empty()).then_some(bytes))
    }

    /// A wheel, as Herdr routes it: a report to a program that tracks the
    /// mouse, an arrow key on the alternate screen with mode 1007, or else
    /// the screen moves through its scrollback.
    pub fn scroll(&mut self, scroll: &Scroll) -> Result<Wheel, hide_vt::Error> {
        if scroll.lines == 0 {
            return Ok(Wheel::Still);
        }
        let up = scroll.lines > 0;
        if self.terminal.mouse_tracking_enabled()? {
            let button = if up {
                hide_vt::MOUSE_BUTTON_WHEEL_UP
            } else {
                hide_vt::MOUSE_BUTTON_WHEEL_DOWN
            };
            return Ok(
                match self.report(
                    hide_vt::MOUSE_ACTION_PRESS,
                    Some(button),
                    scroll.cell.unwrap_or((0, 0)),
                    scroll.modifiers,
                )? {
                    Some(bytes) => Wheel::Input(bytes),
                    None => Wheel::Still,
                },
            );
        }
        if self.terminal.active_screen()? == ActiveScreen::Alternate
            && self
                .terminal
                .mode_get(hide_vt::MODE_MOUSE_ALTERNATE_SCROLL)?
        {
            let mut encoder = hide_vt::KeyEncoder::new()?;
            encoder.set_from_terminal(&self.terminal);
            let mut event = hide_vt::KeyEvent::new()?;
            event.set_action(hide_vt::ffi::GhosttyKeyAction_GHOSTTY_KEY_ACTION_PRESS);
            event.set_key(if up {
                hide_vt::KEY_UP
            } else {
                hide_vt::KEY_DOWN
            });
            let bytes = encoder.encode(&event)?;
            return Ok(if bytes.is_empty() {
                Wheel::Still
            } else {
                Wheel::Input(bytes)
            });
        }
        let before = self.terminal.scrollbar()?.offset;
        self.terminal
            .scroll_viewport_delta(-isize::from(scroll.lines));
        Ok(if self.terminal.scrollbar()?.offset == before {
            Wheel::Still
        } else {
            Wheel::Moved
        })
    }

    /// Puts the screen back at the bottom, as typing does; answers whether
    /// it moved.
    pub fn to_bottom(&mut self) -> bool {
        let Ok(before) = self.terminal.scrollbar() else {
            return false;
        };
        if before.offset + before.len >= before.total {
            return false;
        }
        self.terminal.scroll_viewport_bottom();
        true
    }

    /// The last `lines` rows as text with soft wraps joined (Herdr's recent
    /// unwrapped read), cut from the oldest side to [`MAX_READ_BYTES`]. The
    /// screen's scroll position is not touched: rows are read by their place
    /// in the whole screen.
    pub fn read(&self, lines: u16) -> Result<Answer, hide_vt::Error> {
        let lines = usize::from(lines);
        let total = self.terminal.total_rows()?;
        let cols = self.terminal.cols()?;
        let Some((start, end)) = self.recent_range(lines)? else {
            return Ok(Answer::Text {
                text: String::new(),
                truncated: false,
            });
        };
        let mut text = self.terminal.read_text_screen(
            (0, start as u32),
            (cols.saturating_sub(1), end as u32),
            false,
        )?;
        let mut truncated = total > lines;
        if text.len() > MAX_READ_BYTES {
            let mut cut = text.len() - MAX_READ_BYTES;
            while !text.is_char_boundary(cut) {
                cut += 1;
            }
            text.drain(..cut);
            truncated = true;
        }
        Ok(Answer::Text { text, truncated })
    }

    /// Herdr's `ghostty_recent_read_range`: on the alternate screen its last
    /// rows; on the primary one, rows up to the last that has text or the
    /// cursor's, whichever is lower.
    fn recent_range(&self, lines: usize) -> Result<Option<(usize, usize)>, hide_vt::Error> {
        let terminal = &self.terminal;
        let total = terminal.total_rows()?;
        let cols = terminal.cols()?;
        if total == 0 || cols == 0 || lines == 0 {
            return Ok(None);
        }
        let last = total - 1;
        if terminal.active_screen()? != ActiveScreen::Primary {
            return Ok(Some(((last + 1).saturating_sub(lines), last)));
        }
        let rows = usize::from(terminal.rows()?);
        let viewport_start = total.saturating_sub(rows);
        let cursor_row = (viewport_start + usize::from(terminal.cursor_y()?)).min(last);
        let mut content = None;
        for row in (viewport_start..total).rev() {
            let mut blank = true;
            terminal.for_each_screen_row_cell(row as u32, |_, graphemes| {
                if graphemes
                    .iter()
                    .any(|code| char::from_u32(*code).is_some_and(|ch| !ch.is_whitespace()))
                {
                    blank = false;
                }
            })?;
            if !blank {
                content = Some(row);
                break;
            }
        }
        let end = content.map_or(last, |row| row.max(cursor_row));
        Ok(Some(((end + 1).saturating_sub(lines), end)))
    }
}

fn mods(modifiers: Modifiers) -> u16 {
    let mut mods = 0;
    for (bit, ghostty) in [
        (Modifiers::SHIFT, hide_vt::MOD_SHIFT),
        (Modifiers::CTRL, hide_vt::MOD_CTRL),
        (Modifiers::ALT, hide_vt::MOD_ALT),
        (Modifiers::SUPER, hide_vt::MOD_SUPER),
    ] {
        if modifiers.0 & bit != 0 {
            mods |= ghostty;
        }
    }
    mods
}

#[cfg(test)]
mod tests {
    use hide_holder_protocol::Rgb;

    use super::*;

    fn pane() -> Pane {
        Pane::new(Size { rows: 24, cols: 80 }).unwrap()
    }

    fn answers(pane: &mut Pane, chunks: &[&[u8]]) -> String {
        let mut answers = Vec::new();
        for chunk in chunks {
            pane.feed(chunk, &mut answers);
        }
        String::from_utf8(answers).unwrap()
    }

    fn theme(pane: &mut Pane, appearance: Appearance) -> String {
        let mut answers = Vec::new();
        pane.theme(
            &Theme {
                foreground: Some(Rgb {
                    r: 0x10,
                    g: 0x20,
                    b: 0x30,
                }),
                background: Some(Rgb { r: 1, g: 2, b: 3 }),
                cursor: None,
                palette: vec![(1, Rgb { r: 9, g: 9, b: 9 })],
                appearance,
            },
            &mut answers,
        )
        .unwrap();
        String::from_utf8(answers).unwrap()
    }

    #[test]
    fn each_question_is_answered_once_in_the_order_asked() {
        let mut pane = pane();
        let asked = answers(
            &mut pane,
            &[
                b"\x1b[6n\x1b[c\x1b[>c\x1b[=c\x1b[18t\x1b[14t\x1b[16t\x1b]10;?\x07\x1b]4;1;?\x07\
                \x1bP+q544E;4D73;5463;536574756C63;66756C6C6B6264\x1b\\\x1b[>q\x1b[?2004$p\x05",
            ],
        );
        assert_eq!(
            asked,
            "\x1b[1;1R\x1b[?62;22c\x1b[>1;0;0c\x1bP!|00000000\x1b\\\x1b[8;24;80t\
             \x1b]4;1;rgb:cccc/6666/6666\x07\x1bP1+r544E=787465726D2D323536636F6C6F72\x1b\\\
             \x1bP1+r5463\x1b\\\x1bP>|libghostty\x1b\\\x1b[?2004;2$y"
        );
    }

    #[test]
    fn a_question_split_across_reads_is_answered_once() {
        let mut pane = pane();
        assert_eq!(
            answers(&mut pane, &[b"\x1b[", b"6n\x1b[1", b"8", b"t"]),
            "\x1b[1;1R\x1b[8;24;80t"
        );
    }

    #[test]
    fn pixel_questions_go_unanswered_while_the_cell_size_is_unknown() {
        let mut pane = pane();
        assert_eq!(answers(&mut pane, &[b"\x1b[14t\x1b[16t\x1b[?2048h"]), "");
        let mut resized = Vec::new();
        pane.resize(
            Size {
                rows: 30,
                cols: 100,
            },
            &mut resized,
        )
        .unwrap();
        assert!(resized.is_empty(), "{resized:?}");
        assert_eq!(answers(&mut pane, &[b"\x1b[18t"]), "\x1b[8;30;100t");
    }

    #[test]
    fn a_theme_answers_colors_until_the_program_sets_its_own_and_after_it_resets_them() {
        let mut pane = pane();
        assert_eq!(answers(&mut pane, &[b"\x1b]10;?\x07\x1b]11;?\x07"]), "");
        theme(&mut pane, Appearance::Dark);
        let asked = b"\x1b]10;?\x07\x1b]11;?\x1b\\\x1b]4;1;?\x07";
        let themed = "\x1b]10;rgb:1010/2020/3030\x07\x1b]11;rgb:0101/0202/0303\x1b\\\
                      \x1b]4;1;rgb:0909/0909/0909\x07";
        assert_eq!(answers(&mut pane, &[asked]), themed);
        answers(&mut pane, &[b"\x1b]10;#abcdef\x07\x1b]4;1;#123456\x07"]);
        let own = "\x1b]10;rgb:abab/cdcd/efef\x07\x1b]11;rgb:0101/0202/0303\x1b\\\
                   \x1b]4;1;rgb:1212/3434/5656\x07";
        assert_eq!(answers(&mut pane, &[asked]), own);
        // A theme that changes later does not override the program's own.
        theme(&mut pane, Appearance::Dark);
        assert_eq!(answers(&mut pane, &[asked]), own);
        answers(&mut pane, &[b"\x1b]110\x07\x1b]104;1\x07"]);
        assert_eq!(answers(&mut pane, &[asked]), themed);
    }

    #[test]
    fn an_appearance_change_is_told_only_to_a_program_that_asked() {
        let mut pane = pane();
        assert_eq!(theme(&mut pane, Appearance::Dark), "");
        assert_eq!(answers(&mut pane, &[b"\x1b[?996n"]), "\x1b[?997;1n");
        answers(&mut pane, &[b"\x1b[?2031h"]);
        assert_eq!(theme(&mut pane, Appearance::Light), "\x1b[?997;2n");
        assert_eq!(theme(&mut pane, Appearance::Light), "");
    }

    #[test]
    fn a_paste_is_wrapped_only_in_bracketed_paste_mode() {
        let mut pane = pane();
        assert_eq!(pane.paste(b"a\nb\x1bc").unwrap(), b"a\rb c");
        answers(&mut pane, &[b"\x1b[?2004h"]);
        assert_eq!(
            pane.paste(b"a\nb\x1bc").unwrap(),
            b"\x1b[200~a\nb c\x1b[201~"
        );
    }

    fn press(col: u16, row: u16) -> Mouse {
        Mouse {
            action: MouseAction::Press,
            button: Some(MouseButton::Left),
            col,
            row,
            modifiers: Modifiers::default(),
        }
    }

    fn wheel(lines: i16) -> Scroll {
        Scroll {
            lines,
            cell: Some((1, 2)),
            modifiers: Modifiers::default(),
        }
    }

    #[test]
    fn a_mouse_event_follows_the_programs_mouse_mode_or_is_dropped() {
        let mut pane = pane();
        assert_eq!(pane.mouse(&press(3, 4)).unwrap(), None);
        answers(&mut pane, &[b"\x1b[?1000h\x1b[?1006h"]);
        assert_eq!(
            pane.mouse(&press(3, 4)).unwrap().as_deref(),
            Some(&b"\x1b[<0;4;5M"[..])
        );
        // Pixels the holder does not know.
        answers(&mut pane, &[b"\x1b[?1016h"]);
        assert_eq!(pane.mouse(&press(3, 4)).unwrap(), None);
    }

    fn wheel_input(pane: &mut Pane, lines: i16) -> Option<Vec<u8>> {
        match pane.scroll(&wheel(lines)).unwrap() {
            Wheel::Input(bytes) => Some(bytes),
            Wheel::Moved | Wheel::Still => None,
        }
    }

    #[test]
    fn a_wheel_is_routed_as_herdr_routes_it() {
        let mut pane = pane();
        answers(&mut pane, &[b"\x1b[?1000h\x1b[?1006h"]);
        assert_eq!(
            wheel_input(&mut pane, 3).as_deref(),
            Some(&b"\x1b[<64;2;3M"[..])
        );
        answers(&mut pane, &[b"\x1b[?1000l\x1b[?1049h\x1b[?1007h"]);
        assert_eq!(wheel_input(&mut pane, 1).as_deref(), Some(&b"\x1b[A"[..]));
        assert_eq!(wheel_input(&mut pane, -1).as_deref(), Some(&b"\x1b[B"[..]));
        answers(&mut pane, &[b"\x1b[?1h"]);
        assert_eq!(wheel_input(&mut pane, 1).as_deref(), Some(&b"\x1bOA"[..]));
        answers(&mut pane, &[b"\x1b[?1049l"]);
        assert!(matches!(pane.scroll(&wheel(3)).unwrap(), Wheel::Still));
        let lines: String = (0..60).map(|row| format!("row {row}\r\n")).collect();
        answers(&mut pane, &[lines.as_bytes()]);
        assert!(matches!(pane.scroll(&wheel(3)).unwrap(), Wheel::Moved));
        assert!(pane.to_bottom());
        assert!(!pane.to_bottom());
    }

    #[test]
    fn a_read_gives_the_recent_rows_joined_and_says_when_rows_were_left_out() {
        let mut pane = pane();
        let lines: String = (0..30).map(|row| format!("row {row}\r\n")).collect();
        answers(
            &mut pane,
            &[lines.as_bytes(), b"a long line ", &[b'x'; 100]],
        );
        let Answer::Text { text, truncated } = pane.read(3).unwrap() else {
            panic!("not a text");
        };
        assert!(truncated);
        // Rows, not lines: the long line takes the last two.
        assert_eq!(text, format!("row 29\na long line {}", "x".repeat(100)));
        let Answer::Text { text, truncated } = pane.read(2000).unwrap() else {
            panic!("not a text");
        };
        assert!(!truncated, "{text}");
        assert!(text.starts_with("row 0\n"), "{text}");
    }

    #[test]
    fn a_read_past_its_byte_bound_keeps_the_newest_text() {
        // Each cell carries a letter and four combining marks, nine bytes,
        // so 512 rows of 512 hold far more than the bound.
        let mut pane = Pane::new(Size {
            rows: 512,
            cols: 512,
        })
        .unwrap();
        let cell = "e\u{301}\u{302}\u{303}\u{304}";
        let mut screen = String::new();
        for row in 0..512 {
            screen.push_str(&format!("\x1b[{};1H", row + 1));
            screen.push_str(&cell.repeat(500));
            screen.push_str(&format!("{row:>12}"));
        }
        answers(&mut pane, &[screen.as_bytes()]);
        let Answer::Text { text, truncated } = pane.read(512).unwrap() else {
            panic!("not a text");
        };
        assert!(truncated);
        assert!(text.len() <= MAX_READ_BYTES, "{}", text.len());
        assert!(text.len() > MAX_READ_BYTES - 16, "{}", text.len());
        assert!(
            text.ends_with(&format!("{:>12}", 511)),
            "the newest row was cut"
        );
        assert!(
            !text.contains(&format!("{:>12}\n", 0)),
            "the oldest row was kept"
        );
    }
}
