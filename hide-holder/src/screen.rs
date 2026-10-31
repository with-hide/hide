//! The screen as cells: what the parser shows, captured for a fragment.
//!
//! A capture reads the parser's render state, which keeps which rows changed
//! since the last capture; a row that did not change is the same shared row
//! as before, so the encoder skips it by identity. Colors follow Herdr's
//! rules (`src/pane/terminal.rs` in Herdr v0.9.5): the pane's default colors
//! go out as the receiver's own defaults unless the program changed them, a
//! palette entry stays an index unless the program redefined it, and inverse
//! and invisible text are resolved to colors here.

use std::sync::Arc;

use hide_vt::{
    CellBasicData, CellColor, CellWide, CursorVisualStyle, RenderState, RgbColor, RowCellIter,
    RowCells, RowIterator, Terminal,
};
use unicode_width::UnicodeWidthStr;

/// The most bytes of one cell's text; a longer grapheme keeps its first
/// character. Together with [`MAX_LINK`] it bounds one row's encoding, so
/// any screen fits in fragments split between rows.
pub const MAX_TEXT: usize = 64;

/// The most bytes of one cell's hyperlink; a longer one is left out.
pub const MAX_LINK: usize = 2048;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    /// The receiver's own default for the place.
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

impl From<RgbColor> for Color {
    fn from(color: RgbColor) -> Self {
        Self::Rgb(color.r, color.g, color.b)
    }
}

/// A cell's attributes, one bit each, and its underline style in
/// [`UNDERLINE_STYLE`].
pub mod attrs {
    pub const BOLD: u16 = 1 << 0;
    pub const DIM: u16 = 1 << 1;
    pub const ITALIC: u16 = 1 << 2;
    pub const UNDERLINE: u16 = 1 << 3;
    pub const BLINK: u16 = 1 << 4;
    pub const STRIKE: u16 = 1 << 5;
    pub const OVERLINE: u16 = 1 << 6;
    pub const UNDERLINE_STYLE_SHIFT: u16 = 8;
    pub const UNDERLINE_STYLE: u16 = 0xf << UNDERLINE_STYLE_SHIFT;
}

/// One cell. Its text and hyperlink live in its row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    start: u32,
    len: u8,
    /// Columns the text takes as the receiver measures it: 0 for the column
    /// a wide character covers.
    pub width: u8,
    pub fg: Color,
    pub bg: Color,
    pub attrs: u16,
    /// 0 for none, else one more than its index in the row's links.
    link: u16,
}

/// One row of cells.
#[derive(Clone, Debug, Default)]
pub struct Row {
    cells: Vec<Cell>,
    text: Vec<u8>,
    links: Vec<String>,
}

impl Row {
    fn with_capacity(cols: u16) -> Self {
        Self {
            cells: Vec::with_capacity(usize::from(cols)),
            text: Vec::with_capacity(usize::from(cols)),
            links: Vec::new(),
        }
    }

    fn blank(cols: u16, fg: Color, bg: Color) -> Self {
        let mut row = Self::with_capacity(cols);
        for _ in 0..cols {
            row.push(" ", 1, fg, bg, 0, None);
        }
        row
    }

    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    pub fn text(&self, cell: &Cell) -> &str {
        let start = cell.start as usize;
        // Only whole text is ever pushed, so the slice is UTF-8.
        std::str::from_utf8(&self.text[start..start + usize::from(cell.len)]).unwrap_or(" ")
    }

    pub fn link(&self, cell: &Cell) -> Option<&str> {
        let index = usize::from(cell.link.checked_sub(1)?);
        self.links.get(index).map(String::as_str)
    }

    /// Whether `cell` here looks the same as `other` in `row`.
    pub fn same(&self, cell: &Cell, row: &Row, other: &Cell) -> bool {
        cell.fg == other.fg
            && cell.bg == other.bg
            && cell.attrs == other.attrs
            && cell.width == other.width
            && self.text(cell) == row.text(other)
            && self.link(cell) == row.link(other)
    }

    fn push(
        &mut self,
        text: &str,
        width: u8,
        fg: Color,
        bg: Color,
        attrs: u16,
        link: Option<&str>,
    ) {
        let start = self.text.len() as u32;
        self.text.extend_from_slice(text.as_bytes());
        let link = match link {
            Some(uri) => match self.links.iter().position(|known| known == uri) {
                Some(index) => index as u16 + 1,
                None => {
                    self.links.push(uri.to_owned());
                    self.links.len() as u16
                }
            },
            None => 0,
        };
        self.cells.push(Cell {
            start,
            len: text.len() as u8,
            width,
            fg,
            bg,
            attrs,
            link,
        });
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub x: u16,
    pub y: u16,
    pub visible: bool,
    /// The DECSCUSR shape, 0 for the receiver's default.
    pub shape: u8,
}

/// The screen at one moment.
#[derive(Clone, Debug)]
pub struct Frame {
    pub cols: u16,
    pub rows: u16,
    pub lines: Vec<Arc<Row>>,
    pub cursor: Cursor,
}

/// What decides how the pane's default colors go out: the theme's colors,
/// and the parser's defaults before any theme.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Defaults {
    pub theme_fg: Option<RgbColor>,
    pub theme_bg: Option<RgbColor>,
    pub initial_fg: Option<RgbColor>,
    pub initial_bg: Option<RgbColor>,
}

/// The colors a capture resolves cells with; a change re-reads every row.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Palette {
    default_fg: Option<Color>,
    default_bg: Option<Color>,
    resolved_fg: Color,
    resolved_bg: Color,
    overrides: [Option<RgbColor>; 256],
}

/// The parser's render state and the last frame taken from it.
pub struct Capture {
    render: RenderState,
    rows: RowIterator,
    cells: RowCells,
    bytes: Vec<u8>,
    text: String,
    palette: Option<Palette>,
    frame: Option<Frame>,
}

impl Capture {
    pub fn new() -> Result<Self, hide_vt::Error> {
        Ok(Self {
            render: RenderState::new()?,
            rows: RowIterator::new()?,
            cells: RowCells::new()?,
            bytes: Vec::new(),
            text: String::new(),
            palette: None,
            frame: None,
        })
    }

    /// The parser's colors before anything changed them.
    pub fn colors(&mut self, terminal: &Terminal) -> Result<(RgbColor, RgbColor), hide_vt::Error> {
        self.render.update(terminal)?;
        let colors = self.render.colors()?;
        Ok((colors.foreground, colors.background))
    }

    /// Takes the screen as it is now; `everything` reads every row again
    /// rather than the changed ones. `shaped` says the program chose its
    /// cursor's shape.
    pub fn take(
        &mut self,
        terminal: &Terminal,
        defaults: &Defaults,
        shaped: bool,
        everything: bool,
    ) -> Result<Frame, hide_vt::Error> {
        self.render.update(terminal)?;
        let cols = self.render.cols()?;
        let rows = self.render.rows()?;
        let colors = self.render.colors()?;
        let palette = palette(
            colors.foreground,
            colors.background,
            &colors.palette,
            &terminal.default_palette()?,
            defaults,
        );
        let previous = self
            .frame
            .take()
            .filter(|frame| frame.cols == cols && frame.rows == rows);
        let everything = everything
            || previous.is_none()
            || self.palette != Some(palette)
            || self.render.dirty()? == hide_vt::Dirty::Full;
        self.palette = Some(palette);
        let default_fg = palette.default_fg.unwrap_or(Color::Default);
        let default_bg = palette.default_bg.unwrap_or(Color::Default);
        let mut lines = Vec::with_capacity(usize::from(rows));
        {
            let mut iterator = self.render.populate_row_iterator(&mut self.rows)?;
            let mut y = 0u16;
            while y < rows && iterator.next() {
                let kept = match &previous {
                    Some(frame) if !everything && !iterator.dirty()? => {
                        Some(frame.lines[usize::from(y)].clone())
                    }
                    _ => None,
                };
                let line = match kept {
                    Some(line) => line,
                    None => {
                        let mut cells = iterator.populate_cells(&mut self.cells)?;
                        Arc::new(read_row(
                            &mut cells,
                            terminal,
                            y,
                            cols,
                            &palette,
                            &mut self.bytes,
                            &mut self.text,
                        )?)
                    }
                };
                iterator.set_dirty(false)?;
                lines.push(line);
                y += 1;
            }
            while y < rows {
                lines.push(Arc::new(Row::blank(cols, default_fg, default_bg)));
                y += 1;
            }
        }
        self.render.set_dirty(hide_vt::Dirty::Clean)?;
        let cursor = self.render.cursor()?;
        let cursor = match cursor.viewport {
            Some(at) => Cursor {
                x: at.x.min(cols.saturating_sub(1)),
                y: at.y.min(rows.saturating_sub(1)),
                visible: cursor.visible,
                shape: if shaped {
                    shape(cursor.visual_style, cursor.blinking)
                } else {
                    0
                },
            },
            // Scrolled back past the cursor: no cursor shows.
            None => Cursor {
                x: cols.saturating_sub(1),
                y: rows.saturating_sub(1),
                visible: false,
                shape: 0,
            },
        };
        let frame = Frame {
            cols,
            rows,
            lines,
            cursor,
        };
        self.frame = Some(frame.clone());
        Ok(frame)
    }
}

fn shape(style: CursorVisualStyle, blinking: bool) -> u8 {
    match (style, blinking) {
        (CursorVisualStyle::Block | CursorVisualStyle::BlockHollow, true) => 1,
        (CursorVisualStyle::Block | CursorVisualStyle::BlockHollow, false) => 2,
        (CursorVisualStyle::Underline, true) => 3,
        (CursorVisualStyle::Underline, false) => 4,
        (CursorVisualStyle::Bar, true) => 5,
        (CursorVisualStyle::Bar, false) => 6,
    }
}

fn palette(
    foreground: RgbColor,
    background: RgbColor,
    active: &[RgbColor; 256],
    default: &[RgbColor; 256],
    defaults: &Defaults,
) -> Palette {
    let mut overrides = [None; 256];
    for (index, (active, default)) in active.iter().zip(default).enumerate() {
        if active != default {
            overrides[index] = Some(*active);
        }
    }
    Palette {
        default_fg: default_color(foreground, defaults.theme_fg, defaults.initial_fg),
        default_bg: default_color(background, defaults.theme_bg, defaults.initial_bg),
        resolved_fg: foreground.into(),
        resolved_bg: background.into(),
        overrides,
    }
}

/// The pane's default color goes out as the receiver's default, unless the
/// program set one of its own (Herdr's `ghostty_default_fg` and `_bg`).
fn default_color(
    color: RgbColor,
    theme: Option<RgbColor>,
    initial: Option<RgbColor>,
) -> Option<Color> {
    let own = match theme {
        Some(theme) => theme != color,
        None => initial.is_some_and(|initial| initial != color),
    };
    own.then(|| color.into())
}

fn cell_color(color: CellColor, palette: &Palette) -> Color {
    match color {
        CellColor::Palette(index) => match palette.overrides[usize::from(index)] {
            Some(color) => color.into(),
            None => Color::Indexed(index),
        },
        CellColor::Rgb(color) => color.into(),
    }
}

/// A cell's colors and attributes (Herdr's `ghostty_cell_style`).
fn style(cells: &RowCellIter<'_>, basic: &CellBasicData, palette: &Palette) -> (Color, Color, u16) {
    let style = &basic.style;
    let mut fg = style
        .fg_color
        .map(|color| cell_color(color, palette))
        .or_else(|| cells.fg_color().ok().flatten().map(Color::from))
        .or(palette.default_fg);
    let mut bg = cells
        .content_bg_color()
        .ok()
        .flatten()
        .or(style.bg_color)
        .map(|color| cell_color(color, palette))
        .or_else(|| cells.bg_color().ok().flatten().map(Color::from))
        .or(palette.default_bg);
    if style.invisible {
        fg = bg.or(palette.default_bg);
    }
    if style.inverse {
        // A default background swapped into the foreground would draw in
        // the receiver's default foreground, the same hue as the new
        // background: resolve both first.
        let bg_resolved = bg.unwrap_or(palette.resolved_bg);
        let fg_resolved = fg.unwrap_or(palette.resolved_fg);
        fg = Some(bg_resolved);
        bg = Some(fg_resolved);
    }
    let mut bits = 0;
    for (on, bit) in [
        (style.bold, attrs::BOLD),
        (style.faint, attrs::DIM),
        (style.italic, attrs::ITALIC),
        (style.blink, attrs::BLINK),
        (style.strikethrough, attrs::STRIKE),
        (style.overline, attrs::OVERLINE),
    ] {
        if on {
            bits |= bit;
        }
    }
    if style.underlined {
        bits |= attrs::UNDERLINE | (u16::from(style.underline) << attrs::UNDERLINE_STYLE_SHIFT);
    }
    (
        fg.unwrap_or(Color::Default),
        bg.unwrap_or(Color::Default),
        bits,
    )
}

fn read_row(
    cells: &mut RowCellIter<'_>,
    terminal: &Terminal,
    y: u16,
    cols: u16,
    palette: &Palette,
    bytes: &mut Vec<u8>,
    text: &mut String,
) -> Result<Row, hide_vt::Error> {
    let default_fg = palette.default_fg.unwrap_or(Color::Default);
    let default_bg = palette.default_bg.unwrap_or(Color::Default);
    let mut row = Row::with_capacity(cols);
    let mut x = 0u16;
    while x < cols && cells.next() {
        if cells.is_default_blank()? {
            row.push(" ", 1, default_fg, default_bg, 0, None);
            x += 1;
            continue;
        }
        let basic = cells.basic_data()?;
        let (fg, bg, bits) = style(cells, &basic, palette);
        symbol(cells, basic.wide, bytes, text)?;
        let link = if basic.has_hyperlink {
            terminal
                .viewport_hyperlink_uri(x, u32::from(y))?
                .and_then(|uri| sanitized_link(&uri))
        } else {
            None
        };
        let width = cell_width(text) as u8;
        row.push(text, width, fg, bg, bits, link.as_deref());
        x += 1;
    }
    while x < cols {
        row.push(" ", 1, default_fg, default_bg, 0, None);
        x += 1;
    }
    Ok(row)
}

/// A cell's text (Herdr's `ghostty_buffer_symbol_into`): the column a wide
/// character covers is empty, and text whose width the receiver would
/// measure differently from the parser is drawn as blank.
fn symbol(
    cells: &RowCellIter<'_>,
    wide: CellWide,
    bytes: &mut Vec<u8>,
    text: &mut String,
) -> Result<(), hide_vt::Error> {
    text.clear();
    match wide {
        CellWide::SpacerTail => {}
        CellWide::SpacerHead => text.push(' '),
        CellWide::Narrow | CellWide::Wide => {
            cells.grapheme_text_into(bytes, text)?;
            if text.is_empty() {
                text.push(' ');
            }
            if text.len() > MAX_TEXT {
                let first = text.chars().next().unwrap_or(' ');
                text.clear();
                text.push(first);
            }
        }
    }
    let expected = match wide {
        CellWide::Wide => 2,
        CellWide::Narrow | CellWide::SpacerHead => 1,
        CellWide::SpacerTail => 0,
    };
    let actual = text.width();
    if actual != expected
        && !(wide == CellWide::Narrow && actual == 2)
        && !(wide == CellWide::Wide && is_halfwidth_katakana_voiced(text))
    {
        text.clear();
        text.push_str(match wide {
            CellWide::Wide => "  ",
            CellWide::SpacerTail => "",
            CellWide::Narrow | CellWide::SpacerHead => " ",
        });
    }
    Ok(())
}

/// How many columns the receiver gives `text` (Herdr's `cell_width`).
fn cell_width(text: &str) -> usize {
    if is_halfwidth_katakana_voiced(text) {
        return 2;
    }
    text.width()
}

fn is_halfwidth_katakana_voiced(text: &str) -> bool {
    let mut chars = text.chars();
    let (Some(base), Some(mark), None) = (chars.next(), chars.next(), chars.next()) else {
        return false;
    };
    ('\u{ff66}'..='\u{ff9d}').contains(&base) && matches!(mark, '\u{ff9e}' | '\u{ff9f}')
}

/// A hyperlink with nothing that could end the sequence that carries it, or
/// none when nothing is left or it is past [`MAX_LINK`].
fn sanitized_link(uri: &str) -> Option<String> {
    let sanitized: String = uri.chars().filter(|ch| !ch.is_control()).collect();
    (!sanitized.is_empty() && sanitized.len() <= MAX_LINK).then_some(sanitized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_or_unsafe_link_is_left_out() {
        assert_eq!(
            sanitized_link("https://a\x1b\x07b").as_deref(),
            Some("https://ab")
        );
        assert_eq!(sanitized_link("\x1b"), None);
        assert_eq!(sanitized_link(&"a".repeat(MAX_LINK + 1)), None);
    }

    #[test]
    fn a_default_color_goes_out_as_the_receivers_unless_the_program_changed_it() {
        let white = RgbColor {
            r: 255,
            g: 255,
            b: 255,
        };
        let red = RgbColor { r: 255, g: 0, b: 0 };
        assert_eq!(default_color(white, Some(white), None), None);
        assert_eq!(
            default_color(red, Some(white), None),
            Some(Color::Rgb(255, 0, 0))
        );
        assert_eq!(default_color(white, None, Some(white)), None);
        assert_eq!(
            default_color(red, None, Some(white)),
            Some(Color::Rgb(255, 0, 0))
        );
        assert_eq!(default_color(red, None, None), None);
    }
}
