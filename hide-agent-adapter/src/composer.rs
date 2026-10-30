//! Where each agent draws its composer and what an empty one looks like, and
//! the one parser that judges a styled screen against that declaration
//! (`docs/delivery.md`, Reading the composer).
//!
//! "Empty" is only ever a positive signature: nothing at all, or the agent's
//! known placeholder in its known style. Anything else, including text the
//! declaration cannot place, is content or not found, so a misread screen
//! holds a letter rather than ringing over the operator's words.

/// How an agent's composer sits on the screen and what it may show when it
/// holds nothing of the operator's.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Composer {
    pub region: Region,
    /// What the composer's first line begins with, after its border and
    /// indentation: any one of these, since an agent may draw another glyph
    /// on another terminal. A region whose first line begins with none of
    /// them is not the composer; none declared means the region alone
    /// places it.
    pub marker: &'static [&'static str],
    /// Lines of the agent's own chrome at the end of the region, such as
    /// OpenCode's agent and model line, which are never content.
    pub trailer: usize,
    /// What may stand in an empty composer besides spaces. Every line that
    /// is not blank must be one of these for the composer to read empty.
    pub empty: &'static [Placeholder],
}

/// The lines of the screen that make up the composer, found from the bottom
/// so a transcript above cannot be taken for it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Region {
    /// The lines between the last line drawn only of `─` and the rule above
    /// it, which may carry a label: Claude Code draws a named session's name
    /// in its top rule, as `──── name ─`, and drops the `─` before a name that
    /// does not fit (` long na… ─`). A label is only text between a space
    /// and a closing run of `─` whose every `─` is drawn in the bottom rule's
    /// own pen, so typed text that ends in `─` is never taken for one.
    BetweenRules,
    /// The lines between the last line beginning with `close` and the line
    /// beginning with `open` above it, each stripped of the `side` border on
    /// both ends.
    Box { open: char, close: char, side: char },
    /// The lines beginning with `side` directly above the last line beginning
    /// with `close`, each stripped of that border.
    Bar { side: char, close: char },
    /// From the last line beginning with a marker to the first blank line.
    ToBlank,
    /// The last line beginning with a marker, alone.
    Line,
}

/// A placeholder the agent draws in an empty composer: a line whose visible
/// text begins with `text`, every character of it drawn in `style`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Placeholder {
    pub text: &'static str,
    pub style: Style,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Style {
    /// Any colour or weight: the text alone is the signature.
    Any,
    /// The terminal's dim attribute (SGR 2), never merely a faint colour.
    Dim,
    /// The terminal's italic attribute (SGR 3).
    Italic,
    /// This exact 24-bit foreground colour.
    Rgb(u8, u8, u8),
}

/// A foreground colour the terminal was told to draw with.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Colour {
    /// One of the terminal's palette entries: 0-15 for the basic and bright
    /// colours, up to 255 for the 256-colour palette.
    Index(u8),
    Rgb(u8, u8, u8),
}

/// What a screen says about the composer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Verdict {
    /// The composer shows nothing of the operator's.
    Empty,
    /// The composer shows text, a paste or anything the declaration does not
    /// name as empty.
    Holds,
    /// No composer as declared is on the screen.
    NotFound,
}

impl Verdict {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Holds => "not_empty",
            Self::NotFound => "not_found",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Pen {
    dim: bool,
    italic: bool,
    /// `None` is the terminal's default foreground.
    fg: Option<Colour>,
}

/// One screen line as characters with the pen each was drawn with. Any
/// whitespace, such as the no-break space Claude Code draws after its
/// marker, is blank.
type Cells = Vec<(char, Pen)>;

impl Composer {
    /// Judges a screen Herdr read with styling (`pane.read`, format `ansi`,
    /// rows separated by newlines).
    pub fn judge(&self, screen: &str) -> Verdict {
        let lines: Vec<Cells> = screen.split('\n').map(cells).collect();
        let Some(mut region) = self.region(&lines) else {
            return Verdict::NotFound;
        };
        if !self.marker.is_empty() {
            let Some(first) = region.first_mut() else {
                return Verdict::NotFound;
            };
            let start = first.iter().take_while(|(c, _)| c.is_whitespace()).count();
            let text: String = first[start..].iter().map(|(c, _)| *c).collect();
            let Some(marker) = self.marker.iter().find(|marker| text.starts_with(**marker)) else {
                return Verdict::NotFound;
            };
            first.drain(..start + marker.chars().count());
        }
        for _ in 0..self.trailer {
            let Some(last) = region.iter().rposition(|line| !blank(line)) else {
                return Verdict::NotFound;
            };
            region.truncate(last);
        }
        if region
            .iter()
            .filter(|line| !blank(line))
            .all(|line| self.empty.iter().any(|shown| shown.matches(line)))
        {
            Verdict::Empty
        } else {
            Verdict::Holds
        }
    }

    fn region(&self, lines: &[Cells]) -> Option<Vec<Cells>> {
        let marked = |line: &Cells| {
            let text = visible(line);
            let text = text.trim_start();
            self.marker.iter().any(|marker| text.starts_with(marker))
        };
        match self.region {
            Region::BetweenRules => {
                let bottom = lines.iter().rposition(rule)?;
                let pen = lines[bottom].iter().find(|(c, _)| *c == '─')?.1;
                let top = lines[..bottom]
                    .iter()
                    .rposition(|line| rule(line) || labelled_rule(line, pen))?;
                Some(lines[top + 1..bottom].to_vec())
            }
            Region::Box { open, close, side } => {
                let bottom = lines.iter().rposition(|line| starts_with(line, close))?;
                let top = lines[..bottom]
                    .iter()
                    .rposition(|line| starts_with(line, open))?;
                lines[top + 1..bottom]
                    .iter()
                    .map(|line| strip_side(line, side, true))
                    .collect()
            }
            Region::Bar { side, close } => {
                let bottom = lines.iter().rposition(|line| starts_with(line, close))?;
                let top = lines[..bottom]
                    .iter()
                    .rposition(|line| !starts_with(line, side))
                    .map_or(0, |index| index + 1);
                (top < bottom).then_some(())?;
                lines[top..bottom]
                    .iter()
                    .map(|line| strip_side(line, side, false))
                    .collect()
            }
            Region::ToBlank => {
                let top = lines.iter().rposition(marked)?;
                let end = lines[top + 1..]
                    .iter()
                    .position(blank)
                    .map_or(lines.len(), |offset| top + 1 + offset);
                Some(lines[top..end].to_vec())
            }
            Region::Line => {
                let line = lines.iter().rposition(marked)?;
                Some(vec![lines[line].clone()])
            }
        }
    }
}

/// Whether a row of a screen Herdr read with styling (as [`Composer::judge`]
/// reads one) shows `text`, whatever its pen.
pub fn screen_shows(screen: &str, text: &str) -> bool {
    screen
        .split('\n')
        .any(|row| visible(&cells(row)).contains(text))
}

impl Placeholder {
    fn matches(&self, line: &Cells) -> bool {
        let start = line
            .iter()
            .position(|(c, _)| !c.is_whitespace())
            .unwrap_or(line.len());
        let text: String = line[start..].iter().map(|(c, _)| *c).collect();
        text.trim_end().starts_with(self.text)
            && line[start..]
                .iter()
                .filter(|(c, _)| !c.is_whitespace())
                .all(|(_, pen)| match self.style {
                    Style::Any => true,
                    Style::Dim => pen.dim,
                    Style::Italic => pen.italic,
                    Style::Rgb(r, g, b) => pen.fg == Some(Colour::Rgb(r, g, b)),
                })
    }
}

fn visible(line: &Cells) -> String {
    line.iter().map(|(c, _)| *c).collect()
}

fn blank(line: &Cells) -> bool {
    line.iter().all(|(c, _)| c.is_whitespace())
}

fn starts_with(line: &Cells, first: char) -> bool {
    line.iter()
        .find(|(c, _)| !c.is_whitespace())
        .is_some_and(|(c, _)| *c == first)
}

/// A line drawn only of `─`, as a rule around a composer is. A short run is
/// part of a picture, not a rule.
fn rule(line: &Cells) -> bool {
    let text = visible(line);
    let text = text.trim();
    text.chars().count() >= 8 && text.chars().all(|c| c == '─')
}

/// A rule with a label in it, as Claude Code draws a named session's top
/// rule: an optional run of `─` and a space, the label, a space and a
/// closing run of `─`, every `─` of both runs drawn in `pen`, the rule's own. The
/// label's pen is not part of it: after `/color` Claude Code draws the name
/// on the rule's colour instead of in it.
fn labelled_rule(line: &Cells, pen: Pen) -> bool {
    let start = line
        .iter()
        .position(|(c, _)| !c.is_whitespace())
        .unwrap_or(line.len());
    let end = line
        .iter()
        .rposition(|(c, _)| !c.is_whitespace())
        .map_or(start, |index| index + 1);
    let text = &line[start.min(end)..end];
    let is_rule = |(c, cell): &(char, Pen)| *c == '─' && *cell == pen;
    let opening = text.iter().take_while(|cell| is_rule(cell)).count();
    let closing = text.iter().rev().take_while(|cell| is_rule(cell)).count();
    if closing == 0 || opening + closing >= text.len() {
        return false;
    }
    // The label sits between spaces, and the space before it is the line's
    // own indentation once the opening run is dropped.
    let label = &text[opening..text.len() - closing];
    label.last().is_some_and(|(c, _)| *c == ' ')
        && (opening == 0 || label.first().is_some_and(|(c, _)| *c == ' '))
        && label.iter().any(|(c, _)| !c.is_whitespace())
}

/// The line inside a `side` border: everything after the first `side`, and
/// before the last one when the border closes on the right too.
fn strip_side(line: &Cells, side: char, closed: bool) -> Option<Cells> {
    let open = line.iter().position(|(c, _)| *c == side)?;
    let mut inner: Cells = line[open + 1..].to_vec();
    if closed {
        let close = inner.iter().rposition(|(c, _)| *c == side)?;
        inner.truncate(close);
    }
    Some(inner)
}

/// One row of a styled screen as characters with their pen. Only SGR
/// sequences change the pen; every other escape is dropped, and a carriage
/// return ends nothing, since Herdr separates rows with newlines.
fn cells(row: &str) -> Cells {
    let mut pen = Pen::default();
    let mut out = Vec::new();
    let mut chars = row.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.next() {
                Some('[') => {
                    let mut parameters = String::new();
                    let mut last = None;
                    for next in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&next) {
                            last = Some(next);
                            break;
                        }
                        parameters.push(next);
                    }
                    if last == Some('m') {
                        apply(&mut pen, &parameters);
                    }
                }
                Some(']') => {
                    while let Some(next) = chars.next() {
                        if next == '\u{7}' {
                            break;
                        }
                        if next == '\u{1b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\r' => {}
            c if c.is_control() => {}
            c => out.push((c, pen)),
        }
    }
    out
}

fn apply(pen: &mut Pen, parameters: &str) {
    let codes: Vec<u16> = parameters
        .split([';', ':'])
        .map(|code| code.parse().unwrap_or(0))
        .collect();
    let mut index = 0;
    while index < codes.len() {
        match codes[index] {
            0 => *pen = Pen::default(),
            2 => pen.dim = true,
            3 => pen.italic = true,
            22 => pen.dim = false,
            23 => pen.italic = false,
            code @ 30..=37 => pen.fg = Some(Colour::Index(code as u8 - 30)),
            code @ 90..=97 => pen.fg = Some(Colour::Index(code as u8 - 90 + 8)),
            39 => pen.fg = None,
            38 => match codes.get(index + 1) {
                Some(2) => {
                    let channel = |offset: usize| {
                        codes
                            .get(index + offset)
                            .and_then(|value| u8::try_from(*value).ok())
                    };
                    pen.fg = channel(2)
                        .zip(channel(3))
                        .zip(channel(4))
                        .map(|((r, g), b)| Colour::Rgb(r, g, b));
                    index += 4;
                }
                Some(5) => {
                    pen.fg = codes
                        .get(index + 2)
                        .and_then(|value| u8::try_from(*value).ok())
                        .map(Colour::Index);
                    index += 2;
                }
                _ => {}
            },
            48 | 58 => match codes.get(index + 1) {
                Some(2) => index += 4,
                Some(5) => index += 2,
                _ => {}
            },
            _ => {}
        }
        index += 1;
    }
}
