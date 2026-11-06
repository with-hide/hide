//! The keys Hide presses in a pane by its own names (`hide pane send
//! --key`). They are Hide's, never the engine's: each engine names them in
//! its own terms, so another engine presses the same
//! keys for the same names.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One key, by the name `hide pane send --key` takes. Every one of them is
/// a non-printing key, so naming it in a log never records what was typed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum PaneKey {
    Esc,
    Enter,
    Tab,
    ShiftTab,
    Up,
    Down,
    Left,
    Right,
    Backspace,
    CtrlC,
}

impl PaneKey {
    /// Every key, in the order `hide pane send --help` lists them.
    pub const ALL: [PaneKey; 10] = [
        Self::Esc,
        Self::Enter,
        Self::Tab,
        Self::ShiftTab,
        Self::Up,
        Self::Down,
        Self::Left,
        Self::Right,
        Self::Backspace,
        Self::CtrlC,
    ];

    /// The name `--key` takes.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Esc => "esc",
            Self::Enter => "enter",
            Self::Tab => "tab",
            Self::ShiftTab => "shift-tab",
            Self::Up => "up",
            Self::Down => "down",
            Self::Left => "left",
            Self::Right => "right",
            Self::Backspace => "backspace",
            Self::CtrlC => "ctrl-c",
        }
    }

    /// Every key's name, in the order of [`Self::ALL`].
    pub const NAMES: [&'static str; 10] = {
        let mut names = [""; 10];
        let mut index = 0;
        while index < Self::ALL.len() {
            names[index] = Self::ALL[index].name();
            index += 1;
        }
        names
    };

    /// The key `name` names, exactly as [`Self::name`] spells it.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|key| key.name() == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_is_found_by_its_name_and_serializes_as_it() {
        for key in PaneKey::ALL {
            assert_eq!(PaneKey::parse(key.name()), Some(key));
            assert_eq!(serde_json::to_value(key).unwrap(), key.name());
        }
        assert_eq!(PaneKey::parse("ESC"), None);
        assert_eq!(PaneKey::parse("escape"), None);
        assert_eq!(PaneKey::parse("ctrl+c"), None);
    }
}
