//! How a shell reads one command line: its simple commands as words with
//! quoting removed, split at `&&`, `||`, `;`, `|`, `&`, a newline and a
//! parenthesis, with redirections, heredoc bodies and comments set aside;
//! the commands a heredoc body substitutes when its end word is unquoted
//! are kept, since the shell runs them.
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
    /// as a line of its own, and whether its command is conditional. A
    /// heredoc body whose end word is unquoted substitutes too.
    pub substitutions: Vec<(bool, String)>,
    /// The line holds an `||`: a command that runs only when another failed.
    pub fallback: bool,
    /// The text of each `` `...` `` command substitution read outside single
    /// quotes, which the shell runs as a line of its own.
    pub backquoted: Vec<String>,
    /// For each command, how it follows the command before it.
    pub joins: Vec<Join>,
    /// For each command, the `( ... )` subshells it runs in, outermost
    /// first, each numbered in the order it opened: what a command changes
    /// in its shell (`cd`) ends with its subshell.
    pub scopes: Vec<Vec<usize>>,
    /// For each command, whether it sits in an `if`, loop or `case` body or
    /// in a function, which may run it more than once, or after the rest.
    pub bodies: Vec<bool>,
    /// The line defines a function, which a later word may call.
    pub functions: bool,
}

/// How a command follows the one before it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Join {
    /// It starts the line or follows `;`, `&` or a newline: it runs
    /// whatever the command before it did.
    List,
    /// After `&&`: it runs only when the command before it succeeded.
    And,
    /// After `||`: it runs only when the command before it failed.
    Or,
    /// After `|`: it reads the output of the command before it.
    Pipe,
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
    /// Each heredoc the line opened: its end word, whether it strips leading
    /// tabs, and whether its end word was quoted, which keeps its body text.
    heredocs: Vec<(String, bool, bool)>,
    /// The current word holds a quote or a backslash.
    word_quoted: bool,
    /// A quote, backtick or `$(` that the text ended inside. A shell reads such
    /// a line as a syntax error and runs none of it, so nothing in it is a launch.
    unclosed: bool,
    substitutions: Vec<(bool, String)>,
    /// The current command's substitutions, which take its state when it ends.
    pending_substitutions: Vec<String>,
    backquoted: Vec<String>,
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
    /// Each open `(` subshell: `and_or` where it opened, whether it is a
    /// function's body, and its number.
    subshells: Vec<(bool, bool, usize)>,
    /// How many subshells have opened.
    opened: usize,
    /// How the next command follows the one before it.
    join: Join,
    joins: Vec<Join>,
    scopes: Vec<Vec<usize>>,
    bodies: Vec<bool>,
    functions: bool,
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
            word_quoted: false,
            unclosed: false,
            substitutions: Vec::new(),
            pending_substitutions: Vec::new(),
            backquoted: Vec::new(),
            conditional: Vec::new(),
            fallback: false,
            and_or: false,
            operator_open: false,
            compound: 0,
            function_next: false,
            braces: Vec::new(),
            subshells: Vec::new(),
            opened: 0,
            join: Join::List,
            joins: Vec::new(),
            scopes: Vec::new(),
            bodies: Vec::new(),
            functions: false,
        }
    }

    fn is_conditional(&self) -> bool {
        self.and_or
            || self.compound > 0
            || self
                .braces
                .iter()
                .map(|&(and_or, body)| (and_or, body))
                .chain(
                    self.subshells
                        .iter()
                        .map(|&(and_or, body, _)| (and_or, body)),
                )
                .any(|(and_or, body)| and_or || body)
    }

    /// Whether a command here runs in an `if`, loop or `case` body or in a
    /// function.
    fn in_body(&self) -> bool {
        self.compound > 0
            || self.braces.iter().any(|&(_, body)| body)
            || self.subshells.iter().any(|&(_, body, _)| body)
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
                    if !self.operator_open {
                        self.join = Join::List;
                    }
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
                    self.functions = true;
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
                        ';' => {
                            self.and_or = false;
                            self.join = Join::List;
                        }
                        '|' if doubled => {
                            self.and_or = true;
                            self.fallback = true;
                            self.join = Join::Or;
                        }
                        '|' => self.join = Join::Pipe,
                        // The subshell's first command follows what came
                        // before the subshell.
                        '(' => {
                            let body = std::mem::take(&mut self.function_next);
                            self.opened += 1;
                            self.subshells.push((
                                std::mem::take(&mut self.and_or),
                                body,
                                self.opened,
                            ));
                        }
                        ')' => {
                            if let Some((outer, _, _)) = self.subshells.pop() {
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
                backquoted: Vec::new(),
                joins: Vec::new(),
                scopes: Vec::new(),
                bodies: Vec::new(),
                functions: false,
            };
        }
        ShellLine {
            commands: self.commands,
            targets: self.targets,
            conditional: self.conditional,
            substitutions: self.substitutions,
            fallback: self.fallback,
            backquoted: self.backquoted,
            joins: self.joins,
            scopes: self.scopes,
            bodies: self.bodies,
            functions: self.functions,
        }
    }

    fn end_word(&mut self) {
        if !self.in_word {
            return;
        }
        let word = std::mem::take(&mut self.word);
        self.in_word = false;
        let quoted = std::mem::take(&mut self.word_quoted);
        if let Some(strip) = self.heredoc_next.take() {
            self.heredocs.push((word, strip, quoted));
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
                        self.functions = true;
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
            self.bodies.push(self.in_body());
            self.joins
                .push(std::mem::replace(&mut self.join, Join::List));
            self.scopes.push(
                self.subshells
                    .iter()
                    .map(|&(_, _, number)| number)
                    .collect(),
            );
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
                self.word_quoted = true;
                self.word.push(escaped);
                self.in_word = true;
                self.at += 1;
            }
            None => {}
        }
    }

    fn single_quoted(&mut self) {
        self.in_word = true;
        self.word_quoted = true;
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
        self.word_quoted = true;
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

    /// `` `...` ``, copied into the word as written; its text, with the
    /// backslash before `` ` ``, `\\` and `$` removed as the shell removes it,
    /// is kept in `backquoted`.
    fn backtick(&mut self) {
        self.in_word = true;
        self.word.push('`');
        self.at += 1;
        let mut text = String::new();
        while let Some(c) = self.peek(0) {
            self.at += 1;
            self.word.push(c);
            match c {
                '`' => {
                    self.backquoted.push(text);
                    return;
                }
                '\\' => {
                    if let Some(escaped) = self.peek(0) {
                        self.word.push(escaped);
                        self.at += 1;
                        if !matches!(escaped, '`' | '\\' | '$') {
                            text.push('\\');
                        }
                        text.push(escaped);
                    }
                }
                other => text.push(other),
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
            self.join = if self.and_or { Join::And } else { Join::List };
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

    /// After a newline, skips the body of each heredoc the line opened,
    /// keeping what a body with an unquoted end word substitutes.
    fn heredoc_bodies(&mut self) {
        for (end, strip, quoted) in std::mem::take(&mut self.heredocs) {
            let mut body = String::new();
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
                body.push_str(line);
                body.push('\n');
            }
            if !quoted {
                self.expand_body(&body);
            }
        }
    }

    /// The substitutions a heredoc body runs, read as the shell expands it:
    /// like text in double quotes, with no quote that ends it.
    fn expand_body(&mut self, body: &str) {
        let mut inner = Lexer::new(body);
        while let Some(c) = inner.peek(0) {
            match c {
                '\\' => inner.at += 2,
                '$' if inner.peek(1) == Some('(') => {
                    inner.word.push('$');
                    inner.at += 1;
                    inner.parenthesized();
                }
                '`' => inner.backtick(),
                _ => inner.at += 1,
            }
        }
        self.unclosed |= inner.unclosed;
        let conditional = self.conditional.last().copied().unwrap_or(false);
        self.substitutions.extend(
            inner
                .pending_substitutions
                .into_iter()
                .map(|text| (conditional, text)),
        );
        self.backquoted.extend(inner.backquoted);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_backquoted_command_is_kept_and_a_single_quoted_backtick_is_not() {
        let line = shell_line("echo `rm -rf \\`x\\` build` \"$(date) `whoami`\" 'say `hi`'");
        assert_eq!(line.backquoted, ["rm -rf `x` build", "whoami"]);
        assert_eq!(line.substitutions, [(false, "date".to_owned())]);
        assert!(shell_line("echo `unclosed").backquoted.is_empty());
    }

    #[test]
    fn each_command_keeps_how_it_follows_the_last_and_the_subshells_it_runs_in() {
        let line = shell_line(
            "cd /a && (cd /b; ls) || rm x | wc -l\nf() { cd /c; }\nwhile true; do cd ..; done",
        );
        assert_eq!(
            line.joins,
            [
                Join::List,
                Join::And,
                Join::List,
                Join::Or,
                Join::Pipe,
                Join::List,
                Join::List,
                Join::List,
                Join::List,
                Join::List,
                Join::List,
            ]
        );
        let outside: Vec<usize> = Vec::new();
        assert_eq!(line.scopes[..3], [outside.clone(), vec![1], vec![1]]);
        assert!(line.scopes[3..].iter().all(Vec::is_empty));
        // `f`'s body and the loop's body.
        assert_eq!(
            line.bodies,
            [
                false, false, false, false, false, false, true, false, false, true, false
            ]
        );
        assert!(line.functions);
        assert!(!shell_line("cd /a && ls").functions);
    }

    #[test]
    fn a_heredoc_body_substitutes_only_when_its_end_word_is_unquoted() {
        let line =
            shell_line("cat <<EOF\n$(date) `whoami` \\$(not)\nEOF\ncat <<'EOF'\n$(kept)\nEOF\n");
        assert_eq!(line.substitutions, [(false, "date".to_owned())]);
        assert_eq!(line.backquoted, ["whoami"]);
    }
}
