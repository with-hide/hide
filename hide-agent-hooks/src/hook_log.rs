//! The hooks' bounded log, `~/.hide/agent-hooks/spawn-guard.log`: one
//! structured line for each refusal a guard makes (the guardrails, the plan
//! record, the global-instruction guard, the opener guard), each failed
//! report and each daemon the hooks could not ask. A line names the pane, the
//! runtime and the cause, never the command, a path or a prompt. The file
//! keeps the name of the first guard that wrote it, so a machine has one log
//! however many builds wrote to it.

use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use hide_platform::fs::{self, private};

/// The log's file name in `~/.hide/agent-hooks`.
const LOG_NAME: &str = "spawn-guard.log";
pub(crate) const LOG_LIMIT: u64 = 256 * 1024;
const LOG_LOCK_WAIT: Duration = Duration::from_millis(100);

pub(crate) fn log_path(home: &Path) -> PathBuf {
    home.join(".hide").join("agent-hooks").join(LOG_NAME)
}

pub(crate) fn append_line(path: &Path, line: &str) -> std::io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("log parent unavailable"))?;
    private::create_dir_all(parent)?;
    let lock = private::open_own_file(&parent.join("spawn-guard.lock"), true)?;
    let fs::lock::Waited::Locked(_lock) =
        fs::lock::lock_file(lock, fs::lock::Mode::Exclusive, LOG_LOCK_WAIT, &|| false)?
    else {
        return Err(std::io::Error::other("log busy"));
    };
    let mut file = private::open_own_file(path, true)?;
    if file.metadata()?.len() >= LOG_LIMIT {
        drop(file);
        std::fs::rename(path, path.with_extension("log.1"))?;
        file = private::open_own_file(path, true)?;
    }
    file.seek(SeekFrom::End(0))?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    file.flush()
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_private_and_the_log_keeps_one_older_generation_past_its_limit() {
        let home = tempfile::tempdir().unwrap();
        let path = log_path(home.path());
        append_line(&path, r#"{"kind":"first"}"#).unwrap();
        assert!(private::is_private(&path).unwrap());
        let big = "x".repeat(LOG_LIMIT as usize);
        std::fs::write(&path, &big).unwrap();
        append_line(&path, r#"{"kind":"second"}"#).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\"kind\":\"second\"}\n"
        );
        assert_eq!(
            std::fs::read_to_string(path.with_extension("log.1")).unwrap(),
            big
        );
    }
}
