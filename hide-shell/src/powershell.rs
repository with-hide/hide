//! How PowerShell reads one command line, into the same [`ShellLine`] a
//! POSIX line gives (`about_Parsing`, `about_Quoting_Rules`,
//! `about_Special_Characters`, `about_Comments`, `about_Redirection` and
//! `about_Pipeline_Chain_Operators` in the PowerShell documentation).
//!
//! What differs from a POSIX shell is read here and nowhere else: `` ` `` is
//! the escape character and `\` a plain one, so `.\out.png` stays a path; a
//! single-quoted string ends only at a quote that is not doubled, the four
//! typographic single quotes counting as quotes; a double-quoted string,
//! opened and closed by `"` or a typographic double quote, doubles its quote
//! or escapes it with `` ` ``; a `` ` `` before a line end, `\n` or `\r\n`,
//! continues the line; `&` before a command is the call
//! operator and after one sends it to the background; `{ }` is a script
//! block, which may run any number of times; `<# #>` is a comment, and a
//! here-string (`@'` ... `'@`) is one word. A line PowerShell refuses to
//! parse (a string, a `$( )`, a block, a here-string or a comment it ends
//! inside, a stray closing bracket, a `<`, which PowerShell reserves) reads
//! as no commands.

use crate::quote::{POWERSHELL_DOUBLE_QUOTES, POWERSHELL_QUOTES};
use crate::{Join, ShellLine};

/// The keywords that open a statement whose condition sits in parentheses
/// and whose body is a block: alone, they are not a command.
const KEYWORDS: [&str; 12] = [
    "if", "elseif", "else", "foreach", "for", "while", "do", "until", "switch", "try", "catch",
    "finally",
];

pub(crate) fn read(text: &str) -> ShellLine {
    let mut reader = Reader {
        chars: text.chars().collect(),
        at: 0,
        line: ShellLine::empty(),
        words: Vec::new(),
        word: String::new(),
        in_word: false,
        target_next: false,
        pending_substitutions: Vec::new(),
        and_or: false,
        operator_open: false,
        blocks: Vec::new(),
        parentheses: 0,
        join: Join::List,
    };
    match reader.run() {
        Ok(()) => reader.line,
        Err(Refused) => ShellLine::empty(),
    }
}

/// PowerShell would refuse to run the line.
struct Refused;

/// The assignment operators (`about_Assignment_Operators`).
const ASSIGNMENTS: [&str; 7] = ["=", "+=", "-=", "*=", "/=", "%=", "??="];

/// Takes the `$name =` in front of a command out of its words: what runs is
/// the command after it, whose output the variable takes (`$r = git push`,
/// `$r=git push`, `$a = $b = git push`). A variable whose name is given in
/// braces or with a scope (`${a}`, `$env:x`) counts; a cast or several
/// targets (`[int]$x =`, `$a, $b =`) are left as they are.
fn strip_assignments(words: &mut Vec<String>) {
    loop {
        let Some(first) = words.first() else {
            return;
        };
        let Some(rest) = variable_end(first).map(|end| &first[end..]) else {
            return;
        };
        if rest.is_empty() {
            match words.get(1) {
                Some(operator) if ASSIGNMENTS.contains(&operator.as_str()) => {
                    words.drain(..2);
                }
                _ => return,
            }
        } else if let Some(operator) = ASSIGNMENTS.iter().find(|op| rest.starts_with(**op)) {
            let value = rest[operator.len()..].to_owned();
            if value.is_empty() {
                words.remove(0);
            } else {
                words[0] = value;
            }
        } else {
            return;
        }
    }
}

/// Where the variable a word starts with ends: `$name`, `$scope:name` or
/// `${any}`, read to its last name character.
fn variable_end(word: &str) -> Option<usize> {
    let rest = word.strip_prefix('$')?;
    if let Some(braced) = rest.strip_prefix('{') {
        return braced.find('}').map(|end| end + 3);
    }
    let name = rest
        .char_indices()
        .find(|(_, c)| !(c.is_alphanumeric() || matches!(c, '_' | ':' | '?')))
        .map_or(rest.len(), |(at, _)| at);
    (name > 0).then_some(name + 1)
}

struct Reader {
    chars: Vec<char>,
    at: usize,
    line: ShellLine,
    words: Vec<String>,
    word: String,
    in_word: bool,
    /// The word after a redirection is the file it writes.
    target_next: bool,
    /// The current command's `$( )` texts, which take its state when it ends.
    pending_substitutions: Vec<String>,
    /// The current list has passed `&&` or `||`.
    and_or: bool,
    /// A `|`, `&&` or `||` still waits for its command past a newline.
    operator_open: bool,
    /// Each open `{ }` block: `and_or` where it opened.
    blocks: Vec<bool>,
    parentheses: usize,
    join: Join,
}

impl Reader {
    fn peek(&self, offset: usize) -> Option<char> {
        self.chars.get(self.at + offset).copied()
    }

    fn run(&mut self) -> Result<(), Refused> {
        while let Some(c) = self.peek(0) {
            match c {
                '`' => {
                    self.at += 1;
                    match self.peek(0) {
                        // A backtick before a line end, `\n` or `\r\n`,
                        // continues the line.
                        Some('\n') => self.at += 1,
                        Some('\r') if self.peek(1) == Some('\n') => self.at += 2,
                        Some(escaped) => {
                            self.push(escaped);
                            self.at += 1;
                        }
                        None => {}
                    }
                }
                c if POWERSHELL_QUOTES.contains(&c) => self.single_quoted()?,
                c if POWERSHELL_DOUBLE_QUOTES.contains(&c) => self.double_quoted()?,
                '@' if matches!(self.peek(1), Some('\'' | '"')) && self.opens_here_string() => {
                    self.here_string()?
                }
                '$' if self.peek(1) == Some('(') => {
                    self.at += 1;
                    self.subexpression()?;
                }
                '$' if self.peek(1) == Some('{') => self.braced_variable()?,
                '#' if !self.in_word => self.comment(),
                '<' if self.peek(1) == Some('#') && !self.in_word => self.block_comment()?,
                '<' => return Err(Refused),
                ' ' | '\t' | '\r' => {
                    self.end_word();
                    self.at += 1;
                }
                '\n' => {
                    self.end_command();
                    if !self.operator_open {
                        self.and_or = false;
                        self.join = Join::List;
                    }
                    self.at += 1;
                }
                ';' => {
                    self.end_command();
                    self.and_or = false;
                    self.join = Join::List;
                    self.at += 1;
                }
                '|' => {
                    self.end_command();
                    self.at += 1;
                    if self.peek(0) == Some('|') {
                        self.at += 1;
                        self.and_or = true;
                        self.line.fallback = true;
                        self.join = Join::Or;
                    } else {
                        self.join = Join::Pipe;
                    }
                    self.operator_open = true;
                }
                '&' if self.peek(1) == Some('&') => {
                    self.end_command();
                    self.at += 2;
                    self.and_or = true;
                    self.join = Join::And;
                    self.operator_open = true;
                }
                '&' => {
                    self.at += 1;
                    // Before a command, `&` calls the word after it; after
                    // one, it runs that command as a background job.
                    if !self.in_word && self.words.is_empty() {
                        continue;
                    }
                    self.end_command();
                    self.join = Join::List;
                }
                '(' => {
                    self.end_command();
                    self.parentheses += 1;
                    self.at += 1;
                }
                ')' => {
                    self.end_command();
                    self.parentheses = self.parentheses.checked_sub(1).ok_or(Refused)?;
                    self.at += 1;
                }
                '{' => {
                    self.end_command();
                    self.blocks.push(std::mem::take(&mut self.and_or));
                    self.at += 1;
                }
                '}' => {
                    self.end_command();
                    self.and_or = self.blocks.pop().ok_or(Refused)?;
                    self.at += 1;
                }
                '>' => self.redirection(),
                other => {
                    self.push(other);
                    self.at += 1;
                }
            }
        }
        self.end_command();
        if !self.blocks.is_empty() || self.parentheses > 0 {
            return Err(Refused);
        }
        Ok(())
    }

    fn push(&mut self, c: char) {
        self.word.push(c);
        self.in_word = true;
    }

    fn end_word(&mut self) {
        if !self.in_word {
            return;
        }
        let word = std::mem::take(&mut self.word);
        self.in_word = false;
        if std::mem::take(&mut self.target_next) {
            let command = self.line.commands.len();
            self.line.writes.push((command, word.clone()));
            self.line.targets.push((command, word));
        } else {
            self.words.push(word);
        }
    }

    fn end_command(&mut self) {
        self.end_word();
        self.target_next = false;
        let conditional = self.and_or || !self.blocks.is_empty();
        let mut words = std::mem::take(&mut self.words);
        strip_assignments(&mut words);
        let keyword =
            words.len() == 1 && KEYWORDS.contains(&words[0].to_ascii_lowercase().as_str());
        if !words.is_empty() && !keyword {
            if words[0].eq_ignore_ascii_case("function") || words[0].eq_ignore_ascii_case("filter")
            {
                self.line.functions = true;
            }
            self.line.conditional.push(conditional);
            self.line.bodies.push(!self.blocks.is_empty());
            self.line
                .joins
                .push(std::mem::replace(&mut self.join, Join::List));
            self.line.scopes.push(Vec::new());
            self.line.commands.push(words);
            self.operator_open = false;
        }
        let pending = std::mem::take(&mut self.pending_substitutions);
        self.line
            .substitutions
            .extend(pending.into_iter().map(|text| (conditional, text)));
    }

    /// A quote PowerShell reads as a single quote opens the string and the
    /// next one that is not doubled closes it.
    fn single_quoted(&mut self) -> Result<(), Refused> {
        self.in_word = true;
        self.at += 1;
        while let Some(c) = self.peek(0) {
            self.at += 1;
            if POWERSHELL_QUOTES.contains(&c) {
                match self.peek(0) {
                    Some(next) if POWERSHELL_QUOTES.contains(&next) => {
                        self.word.push(next);
                        self.at += 1;
                    }
                    _ => return Ok(()),
                }
            } else {
                self.word.push(c);
            }
        }
        Err(Refused)
    }

    fn double_quoted(&mut self) -> Result<(), Refused> {
        self.in_word = true;
        self.at += 1;
        while let Some(c) = self.peek(0) {
            match c {
                '`' => {
                    self.at += 1;
                    if let Some(escaped) = self.peek(0) {
                        self.word.push(escaped);
                        self.at += 1;
                    }
                }
                c if POWERSHELL_DOUBLE_QUOTES.contains(&c)
                    && self
                        .peek(1)
                        .is_some_and(|next| POWERSHELL_DOUBLE_QUOTES.contains(&next)) =>
                {
                    self.word.push(self.chars[self.at + 1]);
                    self.at += 2;
                }
                c if POWERSHELL_DOUBLE_QUOTES.contains(&c) => {
                    self.at += 1;
                    return Ok(());
                }
                '$' if self.peek(1) == Some('(') => {
                    self.at += 1;
                    self.subexpression()?;
                }
                other => {
                    self.word.push(other);
                    self.at += 1;
                }
            }
        }
        Err(Refused)
    }

    /// `@'` or `@"` opens a here-string only when the rest of its line is
    /// blank.
    fn opens_here_string(&self) -> bool {
        self.chars[self.at + 2..]
            .iter()
            .take_while(|c| **c != '\n')
            .all(|c| matches!(c, ' ' | '\t' | '\r'))
    }

    /// A here-string: its lines, up to the line that starts with its closing
    /// quote and `@`, as one word.
    fn here_string(&mut self) -> Result<(), Refused> {
        let quote = self.chars[self.at + 1];
        let Some(newline) = self.chars[self.at..].iter().position(|c| *c == '\n') else {
            return Err(Refused);
        };
        self.at += newline + 1;
        let mut body: Vec<String> = Vec::new();
        while self.at < self.chars.len() {
            let stop = self.chars[self.at..]
                .iter()
                .position(|c| *c == '\n')
                .map_or(self.chars.len(), |offset| self.at + offset);
            let line: String = self.chars[self.at..stop].iter().collect();
            let line = line.strip_suffix('\r').unwrap_or(&line);
            if line.starts_with(quote) && line[quote.len_utf8()..].starts_with('@') {
                self.at += quote.len_utf8() + 1;
                self.in_word = true;
                self.word.push_str(&body.join("\n"));
                return Ok(());
            }
            body.push(line.to_owned());
            self.at = (stop + 1).min(self.chars.len());
        }
        Err(Refused)
    }

    /// `$( )` after its `$`, copied into the word as written, to its
    /// matching parenthesis; a quote inside it keeps its own parentheses out
    /// of the count. Its text is kept as a line the shell runs.
    fn subexpression(&mut self) -> Result<(), Refused> {
        self.word.push('$');
        self.in_word = true;
        let start = self.word.len();
        let mut depth = 0usize;
        while let Some(c) = self.peek(0) {
            self.at += 1;
            self.word.push(c);
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        self.pending_substitutions
                            .push(self.word[start + 1..self.word.len() - 1].to_owned());
                        return Ok(());
                    }
                }
                '`' => {
                    if let Some(escaped) = self.peek(0) {
                        self.word.push(escaped);
                        self.at += 1;
                    }
                }
                '\'' | '"' => {
                    let mut closed = false;
                    while let Some(inner) = self.peek(0) {
                        self.at += 1;
                        self.word.push(inner);
                        if inner == c {
                            closed = true;
                            break;
                        }
                    }
                    if !closed {
                        return Err(Refused);
                    }
                }
                _ => {}
            }
        }
        Err(Refused)
    }

    /// `${name}`, a variable whose name may hold any character but `}`,
    /// kept in the word as written.
    fn braced_variable(&mut self) -> Result<(), Refused> {
        let Some(end) = self.chars[self.at..].iter().position(|c| *c == '}') else {
            return Err(Refused);
        };
        let text: String = self.chars[self.at..=self.at + end].iter().collect();
        self.word.push_str(&text);
        self.in_word = true;
        self.at += end + 1;
        Ok(())
    }

    fn comment(&mut self) {
        while let Some(c) = self.peek(0) {
            if c == '\n' {
                return;
            }
            self.at += 1;
        }
    }

    fn block_comment(&mut self) -> Result<(), Refused> {
        self.at += 2;
        while self.at < self.chars.len() {
            if self.peek(0) == Some('#') && self.peek(1) == Some('>') {
                self.at += 2;
                return Ok(());
            }
            self.at += 1;
        }
        Err(Refused)
    }

    /// `>`, `>>`, `2>`, `*>` and `2>&1`: a stream number before it is not a
    /// word, and the word after it is the file it writes unless it names a
    /// stream (`&1`).
    fn redirection(&mut self) {
        if self.in_word
            && (self.word == "*"
                || (!self.word.is_empty() && self.word.chars().all(|d| d.is_ascii_digit())))
        {
            self.word.clear();
            self.in_word = false;
        } else {
            self.end_word();
        }
        self.at += 1;
        if self.peek(0) == Some('>') {
            self.at += 1;
        }
        if self.peek(0) == Some('&') {
            self.at += 1;
            while self.peek(0).is_some_and(|c| c.is_ascii_digit()) {
                self.at += 1;
            }
            return;
        }
        self.target_next = true;
    }
}
