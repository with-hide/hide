//! The machine's memory pressure, as the node answers it
//! (`Call::MemoryPressure`), for the core's agent sleep.

use hide_node_link::memory::{MemoryPressure, PressureAnswer, PressureSource};

use crate::error::{ErrorCode, HostError, HostResult};

/// Test-only: the path of a file holding `normal`, `warning` or `critical`,
/// answered in place of the system's level. A file, read at every answer, so
/// an isolated candidate's pressure can change while it runs; an unreadable
/// file or another word is a failed read, as a system that cannot say is.
/// The registry entry is `hided::env::HIDE_TEST_MEMORY_PRESSURE_FILE`.
pub const TEST_PRESSURE_FILE: &str = "HIDE_TEST_MEMORY_PRESSURE_FILE";

/// The level now, and where it was read: the test file's when one is named,
/// else the system's.
pub fn pressure() -> HostResult<PressureAnswer> {
    match std::env::var_os(TEST_PRESSURE_FILE) {
        Some(path) => from_file(std::path::Path::new(&path)).map(|level| PressureAnswer {
            level,
            source: PressureSource::TestFile,
        }),
        None => hide_platform::memory::pressure()
            .map(|level| PressureAnswer {
                level: MemoryPressure::from(level),
                source: PressureSource::System,
            })
            .map_err(|error| {
                let code = if error.kind() == std::io::ErrorKind::Unsupported {
                    ErrorCode::Unsupported
                } else {
                    ErrorCode::Io
                };
                HostError::new(code, format!("memory pressure could not be read: {error}"))
            }),
    }
}

fn from_file(path: &std::path::Path) -> HostResult<MemoryPressure> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        HostError::io(
            &error,
            format!("{TEST_PRESSURE_FILE} names {}: {error}", path.display()),
        )
    })?;
    MemoryPressure::from_code(text.trim()).ok_or_else(|| {
        HostError::new(
            ErrorCode::Io,
            format!(
                "{TEST_PRESSURE_FILE} holds {:?}, not normal, warning or critical",
                text.trim()
            ),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_test_file_answers_its_level_and_anything_else_is_a_failed_read() {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("pressure");
        std::fs::write(&file, "warning\n").unwrap();
        assert_eq!(from_file(&file).unwrap(), MemoryPressure::Warning);
        std::fs::write(&file, "critical").unwrap();
        assert_eq!(from_file(&file).unwrap(), MemoryPressure::Critical);
        std::fs::write(&file, "low").unwrap();
        assert!(from_file(&file).is_err());
        assert!(from_file(&folder.path().join("missing")).is_err());
    }
}
