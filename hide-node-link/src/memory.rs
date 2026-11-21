//! The memory pressure of a node's machine, as its operating system judges
//! it (`hide_platform::memory::pressure`), which agent sleep reads to put the
//! longest-idle agent there to sleep.

use serde::{Deserialize, Serialize};

/// [`crate::protocol::Call::MemoryPressure`]'s answer: the level, and where
/// the node read it. A machine that cannot say answers the call with an
/// error, never `Normal`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PressureAnswer {
    pub level: MemoryPressure,
    pub source: PressureSource,
}

/// Where a node read its level: from its operating system, or from the file
/// a test-only variable names, so a run record says which it acted on.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PressureSource {
    System,
    TestFile,
}

impl PressureSource {
    /// The word the wire carries, for a diagnostic.
    pub fn code(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::TestFile => "test_file",
        }
    }
}

/// How pressed a machine's memory is, as its operating system judges it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryPressure {
    Normal,
    Warning,
    Critical,
}

impl MemoryPressure {
    /// The word the wire carries, for a diagnostic.
    pub fn code(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Warning => "warning",
            Self::Critical => "critical",
        }
    }

    /// The level a word names, as [`Self::code`] spells it.
    pub fn from_code(code: &str) -> Option<Self> {
        [Self::Normal, Self::Warning, Self::Critical]
            .into_iter()
            .find(|level| level.code() == code)
    }
}

impl From<hide_platform::memory::Pressure> for MemoryPressure {
    fn from(pressure: hide_platform::memory::Pressure) -> Self {
        match pressure {
            hide_platform::memory::Pressure::Normal => Self::Normal,
            hide_platform::memory::Pressure::Warning => Self::Warning,
            hide_platform::memory::Pressure::Critical => Self::Critical,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MemoryPressure, PressureAnswer, PressureSource};

    #[test]
    fn each_level_crosses_the_wire_as_its_word() {
        for level in [
            MemoryPressure::Normal,
            MemoryPressure::Warning,
            MemoryPressure::Critical,
        ] {
            let wire = serde_json::to_value(level).unwrap();
            assert_eq!(wire, serde_json::json!(level.code()));
            assert_eq!(MemoryPressure::from_code(level.code()), Some(level));
            assert_eq!(
                serde_json::from_value::<MemoryPressure>(wire).unwrap(),
                level
            );
        }
        assert_eq!(MemoryPressure::from_code("low"), None);
        for source in [PressureSource::System, PressureSource::TestFile] {
            let answer = PressureAnswer {
                level: MemoryPressure::Warning,
                source,
            };
            let wire = serde_json::to_value(answer).unwrap();
            assert_eq!(
                wire,
                serde_json::json!({"level": "warning", "source": source.code()})
            );
            assert_eq!(
                serde_json::from_value::<PressureAnswer>(wire).unwrap(),
                answer
            );
        }
    }
}
