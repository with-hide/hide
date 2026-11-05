// Partly derived from Herdr v0.9.5 (https://github.com/herdrdev/herdr,
// commit 2ecf19a), the shell quoting in src/platform/{macos,linux,mod}.rs,
// extracted from herdr-core/src/engine/own/shell_line.rs.
// Copyright 2025 herdr contributors. Licensed under the Apache License,
// Version 2.0; see NOTICE at the repository root.
// Changed by Hide: shared single-quote escaping, with PowerShell's four
// typographic quote characters handled alongside the ASCII apostrophe.

//! How a word is written so that a shell reads it back unchanged: the one
//! single-quoting rule each shell family gets, shared by the hooks' installed
//! commands (`hide-agent-hooks`), the core's own engine's start and resume
//! lines (`herdr-core`, `engine/own/shell_line.rs`) and the notice a node's
//! pane prints before its shell (`hide-node`). Whether a word may stay bare is
//! each caller's own rule; the escaping inside the quotes is decided here.

/// The characters PowerShell reads as a single quote: the ASCII one and the
/// four typographic ones (U+2018 to U+201B), each of which ends a
/// single-quoted string unless doubled.
pub const POWERSHELL_QUOTES: [char; 5] = ['\'', '\u{2018}', '\u{2019}', '\u{201A}', '\u{201B}'];

impl crate::Syntax {
    /// `value` as one single-quoted word this shell reads back unchanged.
    /// A POSIX shell's `'` closes the string, is written escaped and opens it
    /// again; in PowerShell every character it reads as a single quote is
    /// doubled.
    pub fn single_quoted(self, value: &str) -> String {
        match self {
            Self::Posix => format!("'{}'", value.replace('\'', "'\\''")),
            Self::PowerShell => {
                let mut quoted = String::with_capacity(value.len() + 2);
                quoted.push('\'');
                for character in value.chars() {
                    if POWERSHELL_QUOTES.contains(&character) {
                        quoted.push(character);
                    }
                    quoted.push(character);
                }
                quoted.push('\'');
                quoted
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::Syntax;

    #[test]
    fn a_posix_word_keeps_its_apostrophe_outside_the_quotes() {
        assert_eq!(Syntax::Posix.single_quoted("it's"), "'it'\\''s'");
        assert_eq!(Syntax::Posix.single_quoted(""), "''");
    }

    #[test]
    fn a_powershell_string_doubles_every_quote_powershell_reads() {
        assert_eq!(Syntax::PowerShell.single_quoted("it's"), "'it''s'");
        assert_eq!(
            Syntax::PowerShell.single_quoted("it\u{2019}s"),
            "'it\u{2019}\u{2019}s'"
        );
        assert_eq!(
            Syntax::PowerShell.single_quoted("\u{2018}a\u{201B}"),
            "'\u{2018}\u{2018}a\u{201B}\u{201B}'"
        );
    }
}
