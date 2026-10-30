//! How a shell reads one command line: its simple commands as words with
//! quoting removed, split at `&&`, `||`, `;`, `|`, `&`, a newline and a
//! parenthesis, with redirections, heredoc bodies and comments set aside.
//!
//! The hooks read an agent's shell call through it before the call runs
//! (`hide-agent-hooks`: the spawn guard, the guardrails, the observation and
//! the plan record), and the session reader reads the calls a session file
//! recorded through it (`hide-session`, the pull requests a call names). It
//! reads text only: it starts nothing, opens nothing and keeps no state.

/// The simple commands of a shell line, each as its words with quoting
/// removed. A substitution (`$(...)`, `` `...` ``) stays inside its word as
/// the text it was written with: it is not parsed.
pub fn simple_commands(text: &str) -> Vec<Vec<String>> {
    Lexer::new(text).run().commands
}

/// A shell line as [`simple_commands`] reads it, with the file each
/// redirection names (`< f`, `> f`, `2>> f`, `&> f`) kept beside the index of
/// the command it belongs to: `cat < .env` names `.env` as much as
/// `cat .env` does.
pub struct ShellLine {
    pub commands: Vec<Vec<String>>,
    pub targets: Vec<(usize, String)>,
    /// For each command, whether running the line can skip it: it follows
    /// `&&` or `||` in its list, sits in an `if`, loop or `case` body, or in
    /// a function.
    pub conditional: Vec<bool>,
    /// The text of each `$(...)` command substitution, which the shell runs
    /// as a line of its own, and whether its command is conditional.
    pub substitutions: Vec<(bool, String)>,
    /// The line holds an `||`: a command that runs only when another failed.
    pub fallback: bool,
}

pub fn shell_line(text: &str) -> ShellLine {
    Lexer::new(text).run()
}

struct Lexer {
    chars: Vec<char>,
    at: usize,
    commands: Vec<Vec<String>>,
    words: Vec<String>,
    word: String,
    in_word: bool,
    /// The word after a redirection operator is its target, not an argument.
    drop_next: bool,
    /// Whether that word names a file (`<`, `>`), not a here-string's text.
    next_is_file: bool,
    targets: Vec<(usize, String)>,
    /// The word after `<<` names the heredoc's end; `true` strips leading tabs.
    heredoc_next: Option<bool>,
    heredocs: Vec<(String, bool)>,
    /// A quote, backtick or `$(` that the text ended inside. A shell reads such
    /// a line as a syntax error and runs none of it, so nothing in it is a launch.
    unclosed: bool,
    substitutions: Vec<(bool, String)>,
    /// The current command's substitutions, which take its state when it ends.
    pending_substitutions: Vec<String>,
    conditional: Vec<bool>,
    /// An `||` has been read.
    fallback: bool,
    /// The current list has passed `&&` or `||`.
    and_or: bool,
    /// An `&&`, `||` or `|` still waits for its command, which may follow a
    /// newline.
    operator_open: bool,
    /// Open `if`, loop and `case` bodies.
    compound: usize,
    /// A function was just named; the group that follows is its body,
    /// which runs only when called.
    function_next: bool,
    /// Each open `{` group: `and_or` where it opened, and whether it is a
    /// function's body.
    braces: Vec<(bool, bool)>,
    /// Each open `(` subshell: `and_or` where it opened, and whether it is
    /// a function's body.
    subshells: Vec<(bool, bool)>,
}

impl Lexer {
    fn new(text: &str) -> Self {
        Self {
            chars: text.chars().collect(),
            at: 0,
            commands: Vec::new(),
            words: Vec::new(),
            word: String::new(),
            in_word: false,
            drop_next: false,
            next_is_file: false,
            targets: Vec::new(),
            heredoc_next: None,
            heredocs: Vec::new(),
            unclosed: false,
            substitutions: Vec::new(),
            pending_substitutions: Vec::new(),
            conditional: Vec::new(),
            fallback: false,
            and_or: false,
            operator_open: false,
            compound: 0,
            function_next: false,
            braces: Vec::new(),
            subshells: Vec::new(),
        }
    }

    fn is_conditional(&self) -> bool {
        self.and_or
            || self.compound > 0
            || self
                .braces
                .iter()
                .chain(&self.subshells)
                .any(|&(and_or, body)| and_or || body)
    }

    fn peek(&self, offset: usize) -> Option<char> {
        self.chars.get(self.at + offset).copied()
    }

    fn run(mut self) -> ShellLine {
        while let Some(c) = self.peek(0) {
            match c {
                '\\' => self.backslash(),
                '\'' => self.single_quoted(),
                '"' => self.double_quoted(),
                '`' => self.backtick(),
                '$' if self.peek(1) == Some('(') => {
                    self.word.push('$');
                    self.in_word = true;
                    self.at += 1;
                    self.parenthesized();
                }
                ' ' | '\t' => {
                    self.end_word();
                    self.at += 1;
                }
                '\n' => {
                    self.end_command();
                    self.and_or &= self.operator_open;
                    self.at += 1;
                    self.heredoc_bodies();
                }
                '#' if !self.in_word => {
                    self.comment();
                }
                // `name()` names a function; the group after it is its body.
                '(' if self.peek(1) == Some(')') => {
                    self.end_command();
                    self.function_next = true;
                    self.at += 2;
                }
                ';' | '|' | '(' | ')' => {
                    self.end_command();
                    self.at += 1;
                    // `&&`-style doubling of the same character is one operator.
                    let doubled = matches!(c, ';' | '|') && self.peek(0) == Some(c);
                    // `|&` is a pipe that carries stderr too, not a background job.
                    if doubled || (c == '|' && self.peek(0) == Some('&')) {
                        self.at += 1;
                    }
                    self.operator_open = c == '|';
                    match c {
                        ';' => self.and_or = false,
                        '|' if doubled => {
                            self.and_or = true;
                            self.fallback = true;
                        }
                        '(' => {
                            let body = std::mem::take(&mut self.function_next);
                            self.subshells
                                .push((std::mem::take(&mut self.and_or), body));
                        }
                        ')' => {
                            if let Some((outer, _)) = self.subshells.pop() {
                                self.and_or = outer;
                            }
                        }
                        _ => {}
                    }
                }
                '&' => {
                    self.ampersand();
                }
                '<' | '>' => {
                    self.redirection(c);
                }
                other => {
                    self.word.push(other);
                    self.in_word = true;
                    self.at += 1;
                }
            }
        }
        self.end_command();
        if self.unclosed {
            return ShellLine {
                commands: Vec::new(),
                targets: Vec::new(),
                conditional: Vec::new(),
                substitutions: Vec::new(),
                fallback: false,
            };
        }
        ShellLine {
            commands: self.commands,
            targets: self.targets,
            conditional: self.conditional,
            substitutions: self.substitutions,
            fallback: self.fallback,
        }
    }

    fn end_word(&mut self) {
        if !self.in_word {
            return;
        }
        let word = std::mem::take(&mut self.word);
        self.in_word = false;
        if let Some(strip) = self.heredoc_next.take() {
            self.heredocs.push((word, strip));
        } else if self.drop_next {
            self.drop_next = false;
            if std::mem::take(&mut self.next_is_file) {
                self.targets.push((self.commands.len(), word));
            }
        } else {
            self.words.push(word);
        }
    }

    fn end_command(&mut self) {
        self.end_word();
        self.drop_next = false;
        self.next_is_file = false;
        self.heredoc_next = None;
        let mut conditional = self.is_conditional();
        if !self.words.is_empty() {
            let words = std::mem::take(&mut self.words);
            // Words that open or close a body, past the keywords before them.
            // A group opens before its first command; an `if` or loop body
            // opens after the command that tests it.
            let mut opens = 0;
            let mut at = 0;
            while let Some(word) = words.get(at) {
                match word.as_str() {
                    "if" | "while" | "until" | "for" | "select" | "case" => opens += 1,
                    "fi" | "done" | "esac" => self.compound = self.compound.saturating_sub(1),
                    "function" => {
                        self.function_next = true;
                        // `function name {`: the name, then the group.
                        at += 2;
                        continue;
                    }
                    "{" => {
                        let body = std::mem::take(&mut self.function_next);
                        self.braces.push((std::mem::take(&mut self.and_or), body));
                    }
                    "}" => {
                        if let Some((outer, _)) = self.braces.pop() {
                            self.and_or = outer;
                        }
                    }
                    "then" | "else" | "elif" | "do" | "!" | "time" => {}
                    _ => break,
                }
                at += 1;
            }
            conditional = self.is_conditional();
            self.conditional.push(conditional);
            self.compound += opens;
            self.commands.push(words);
            self.operator_open = false;
        }
        let pending = std::mem::take(&mut self.pending_substitutions);
        self.substitutions
            .extend(pending.into_iter().map(|text| (conditional, text)));
    }

    fn backslash(&mut self) {
        self.at += 1;
        match self.peek(0) {
            // A backslash before a newline joins the lines.
            Some('\n') => self.at += 1,
            Some(escaped) => {
                self.word.push(escaped);
                self.in_word = true;
                self.at += 1;
            }
            None => {}
        }
    }

    fn single_quoted(&mut self) {
        self.in_word = true;
        self.at += 1;
        while let Some(c) = self.peek(0) {
            self.at += 1;
            if c == '\'' {
                return;
            }
            self.word.push(c);
        }
        self.unclosed = true;
    }

    fn double_quoted(&mut self) {
        self.in_word = true;
        self.at += 1;
        while let Some(c) = self.peek(0) {
            match c {
                '"' => {
                    self.at += 1;
                    return;
                }
                '\\' => {
                    self.at += 1;
                    match self.peek(0) {
                        Some('\n') => self.at += 1,
                        Some(escaped @ ('"' | '\\' | '$' | '`')) => {
                            self.word.push(escaped);
                            self.at += 1;
                        }
                        _ => self.word.push('\\'),
                    }
                }
                '$' if self.peek(1) == Some('(') => {
                    self.word.push('$');
                    self.at += 1;
                    self.parenthesized();
                }
                '`' => self.backtick(),
                other => {
                    self.word.push(other);
                    self.at += 1;
                }
            }
        }
        self.unclosed = true;
    }

    /// `` `...` ``, copied into the word as written.
    fn backtick(&mut self) {
        self.in_word = true;
        self.word.push('`');
        self.at += 1;
        while let Some(c) = self.peek(0) {
            self.at += 1;
            self.word.push(c);
            match c {
                '`' => return,
                '\\' => {
                    if let Some(escaped) = self.peek(0) {
                        self.word.push(escaped);
                        self.at += 1;
                    }
                }
                _ => {}
            }
        }
        self.unclosed = true;
    }

    /// `(...)` after a `$`, copied into the word as written, to its matching
    /// parenthesis; quotes inside it keep their own parentheses out of the count.
    fn parenthesized(&mut self) {
        let start = self.word.len();
        let mut depth = 0usize;
        // The heredocs this substitution opened: their bodies are text, so a
        // quote or a parenthesis in one does not count.
        let mut heredocs: Vec<(String, bool)> = Vec::new();
        while let Some(c) = self.peek(0) {
            self.at += 1;
            self.word.push(c);
            match c {
                // `<<<` is a here-string: no body follows.
                '<' if self.peek(0) == Some('<') && self.peek(1) == Some('<') => {
                    self.word.push_str("<<");
                    self.at += 2;
                }
                '<' if self.peek(0) == Some('<') => {
                    if let Some(heredoc) = self.nested_heredoc_start() {
                        heredocs.push(heredoc);
                    }
                }
                '\n' => self.skip_nested_heredoc_bodies(&mut heredocs),
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        // Between this `(` and its `)`.
                        self.pending_substitutions
                            .push(self.word[start + 1..self.word.len() - 1].to_owned());
                        return;
                    }
                }
                '\\' => {
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
                        if inner == '\\'
                            && c == '"'
                            && let Some(escaped) = self.peek(0)
                        {
                            self.word.push(escaped);
                            self.at += 1;
                        }
                    }
                    if !closed {
                        self.unclosed = true;
                    }
                }
                _ => {}
            }
        }
        self.unclosed = true;
    }

    /// After the first `<` of a `<<` inside `$(...)`: copies the operator and
    /// the delimiter word into the substitution's text and returns the
    /// delimiter, with whether leading tabs are stripped (`<<-`).
    fn nested_heredoc_start(&mut self) -> Option<(String, bool)> {
        self.word.push('<');
        self.at += 1;
        let strip = self.peek(0) == Some('-');
        if strip {
            self.word.push('-');
            self.at += 1;
        }
        while let Some(c @ (' ' | '\t')) = self.peek(0) {
            self.word.push(c);
            self.at += 1;
        }
        let mut delimiter = String::new();
        let quote = self.peek(0).filter(|c| matches!(c, '\'' | '"'));
        if let Some(quote) = quote {
            self.word.push(quote);
            self.at += 1;
            while let Some(c) = self.peek(0) {
                self.word.push(c);
                self.at += 1;
                if c == quote {
                    break;
                }
                delimiter.push(c);
            }
        } else {
            // The delimiter is the word a shell reads: quotes and backslashes in
            // it are removed (`\EOF`, `E"OF"`).
            while let Some(c) = self.peek(0) {
                if c.is_whitespace() || matches!(c, ')' | ';' | '|' | '&' | '<' | '>') {
                    break;
                }
                self.word.push(c);
                self.at += 1;
                match c {
                    '\\' => {
                        if let Some(escaped) = self.peek(0) {
                            self.word.push(escaped);
                            self.at += 1;
                            delimiter.push(escaped);
                        }
                    }
                    '\'' | '"' => {}
                    other => delimiter.push(other),
                }
            }
        }
        (!delimiter.is_empty()).then_some((delimiter, strip))
    }

    /// After a newline inside `$(...)`: copies the body of each heredoc the
    /// line opened, to its delimiter line, into the substitution's text.
    fn skip_nested_heredoc_bodies(&mut self, heredocs: &mut Vec<(String, bool)>) {
        for (end, strip) in std::mem::take(heredocs) {
            while self.at < self.chars.len() {
                let start = self.at;
                let stop = self.chars[start..]
                    .iter()
                    .position(|c| *c == '\n')
                    .map_or(self.chars.len(), |offset| start + offset);
                let next = (stop + 1).min(self.chars.len());
                let line: String = self.chars[start..stop].iter().collect();
                self.word.extend(self.chars[start..next].iter());
                self.at = next;
                let line = if strip {
                    line.trim_start_matches('\t')
                } else {
                    line.as_str()
                };
                if line == end {
                    break;
                }
            }
        }
    }

    fn comment(&mut self) {
        while let Some(c) = self.peek(0) {
            if c == '\n' {
                return;
            }
            self.at += 1;
        }
    }

    fn ampersand(&mut self) {
        if self.peek(1) == Some('>') {
            // `&>` and `&>>` send both streams to the next word.
            self.end_word();
            self.at += 2;
            if self.peek(0) == Some('>') {
                self.at += 1;
            }
            self.drop_next = true;
            self.next_is_file = true;
        } else {
            self.end_command();
            self.at += 1;
            self.and_or = self.peek(0) == Some('&');
            self.operator_open = self.and_or;
            if self.and_or {
                self.at += 1;
            }
        }
    }

    /// `<`, `>` and their forms. A number just before it is the descriptor, not
    /// an argument; the word after it is the target (or, for `<<`, the heredoc's
    /// end, whose body follows the line).
    fn redirection(&mut self, c: char) {
        if self.in_word && !self.word.is_empty() && self.word.chars().all(|d| d.is_ascii_digit()) {
            self.word.clear();
            self.in_word = false;
        } else {
            self.end_word();
        }
        self.at += 1;
        if c == '<' && self.peek(0) == Some('<') {
            self.at += 1;
            match self.peek(0) {
                // `<<<` is a here-string: one word, not a body.
                Some('<') => {
                    self.at += 1;
                    self.drop_next = true;
                    self.next_is_file = false;
                }
                Some('-') => {
                    self.at += 1;
                    self.heredoc_next = Some(true);
                }
                _ => self.heredoc_next = Some(false),
            }
            return;
        }
        // `>&2` duplicates a descriptor and names no file.
        let duplicate = self.peek(0) == Some('&');
        if matches!(self.peek(0), Some('>' | '&' | '|')) || (c == '<' && self.peek(0) == Some('>'))
        {
            self.at += 1;
        }
        self.drop_next = true;
        self.next_is_file = !duplicate;
    }

    /// After a newline, skips the body of each heredoc the line opened.
    fn heredoc_bodies(&mut self) {
        for (end, strip) in std::mem::take(&mut self.heredocs) {
            while self.at < self.chars.len() {
                let start = self.at;
                let stop = self.chars[start..]
                    .iter()
                    .position(|c| *c == '\n')
                    .map_or(self.chars.len(), |offset| start + offset);
                self.at = (stop + 1).min(self.chars.len());
                let line: String = self.chars[start..stop].iter().collect();
                let line = if strip {
                    line.trim_start_matches('\t')
                } else {
                    line.as_str()
                };
                if line == end {
                    break;
                }
            }
        }
    }
}

pub fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && name.chars().enumerate().all(|(index, c)| {
                c == '_' || c.is_ascii_alphabetic() || (index > 0 && c.is_ascii_digit())
            })
    })
}

/// Words a shell puts before a command without changing which command runs:
/// the keywords that open a compound command's body, and the wrappers that
/// run the next word as it is.
pub fn is_prefix(word: &str, compound: bool) -> bool {
    matches!(word, "time" | "exec" | "command" | "nohup" | "env")
        || (compound
            && matches!(
                word,
                "if" | "then" | "elif" | "else" | "while" | "until" | "do" | "{" | "!"
            ))
}
