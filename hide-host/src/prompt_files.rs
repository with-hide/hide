//! The node's side of a start's first input (`Call::PromptFile`): one
//! owner-only file per start in the state folder, which the start line reads
//! and removes (docs/delivery.md, The input ladder). The core removes the
//! file of a start that typed nothing (`Call::PromptFileRemove`). A line that
//! never ran leaves its file, which is finished once its start's deadline
//! has passed ([`PROMPT_FILE_IN_USE`]): finished files go on every write and
//! when a node starts ([`sweep`]), and with
//! [`PROMPT_FILES_CAP`] files still in use a write is refused, never one in
//! use dropped, whose line would then read nothing.

use std::path::{Path, PathBuf};

use hide_node_link::protocol::{
    PROMPT_FILE_IN_USE, PROMPT_FILE_MAX_BYTES, PROMPT_FILES_CAP, PromptFileKept, valid_prompt_start,
};
use hide_platform::fs::{Access, atomic, private};

/// Writes `text` as the prompt file of `start` under `env`'s state folder
/// and answers its path in the wire spelling with the `hide` beside the node
/// that reads it.
pub fn write(env: &crate::serve::Env, start: &str, text: &str) -> Result<PromptFileKept, String> {
    let exec = env
        .program
        .as_deref()
        .ok_or_else(|| "This node does not know its own program.".to_owned())
        .and_then(crate::profiles::exec_command)?;
    let path = write_file(env.state.as_deref(), start, text)?;
    Ok(PromptFileKept { path, exec })
}

/// Writes `text` as the prompt file of `start` under the state folder
/// `state` and answers its path in the wire spelling.
fn write_file(state: Option<&Path>, start: &str, text: &str) -> Result<String, String> {
    let folder = folder(state)?;
    if !valid_prompt_start(start) {
        return Err("Invalid prompt file name.".to_owned());
    }
    if text.len() > PROMPT_FILE_MAX_BYTES {
        return Err(format!(
            "The first input is longer than {} KiB.",
            PROMPT_FILE_MAX_BYTES / 1024
        ));
    }
    private::create_dir_all(&folder)
        .and_then(|()| private::own_dir(&folder).map(drop))
        .map_err(|error| {
            format!("The prompt folder must be this account's private folder: {error}")
        })?;
    make_room(&folder, start)?;
    let path = folder.join(start);
    atomic::write_file(&path, text.as_bytes(), Access::Private)
        .map_err(|error| format!("The prompt file could not be written: {error}"))?;
    hide_platform::path::to_wire(&path)
        .map_err(|error| format!("The prompt file's path cannot be named: {error}"))
}

/// Removes the prompt file of `start`; one already gone is not an error.
pub fn remove(state: Option<&Path>, start: &str) -> Result<(), String> {
    if !valid_prompt_start(start) {
        return Err("Invalid prompt file name.".to_owned());
    }
    match std::fs::remove_file(folder(state)?.join(start)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("The prompt file could not be removed: {error}")),
    }
}

fn folder(state: Option<&Path>) -> Result<PathBuf, String> {
    state
        .map(hide_kit::layout::prompt_files)
        .ok_or_else(|| "This machine has no state folder for prompt files.".to_owned())
}

/// Removes the finished prompt files under the state folder `state`, as a
/// node does when it starts; a folder not there yet holds none.
pub fn sweep(state: Option<&Path>) {
    let Ok(folder) = folder(state) else { return };
    if let Err(cause) = drop_finished(&folder, std::time::SystemTime::now()) {
        eprintln!(
            "{}",
            serde_json::json!({
                "component": "prompt_files", "kind": "prompt_files.sweep_failed", "cause": cause,
            })
        );
    }
}

/// Drops the finished files, then refuses when the ones still in use leave
/// no room for one more; the file of `start` itself is replaced, so it
/// takes no room.
fn make_room(folder: &Path, start: &str) -> Result<(), String> {
    let in_use = drop_finished(folder, std::time::SystemTime::now())?
        .into_iter()
        .filter(|name| name != start)
        .count();
    if in_use >= PROMPT_FILES_CAP {
        return Err(format!(
            "{in_use} starts on this machine are still waiting to read their first input; try again once they have started."
        ));
    }
    Ok(())
}

/// Removes each file in `folder` last written longer than
/// [`PROMPT_FILE_IN_USE`] before `now` and answers the names of the others.
fn drop_finished(folder: &Path, now: std::time::SystemTime) -> Result<Vec<String>, String> {
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("The prompt folder could not be read: {error}")),
    };
    let mut in_use = Vec::new();
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("The prompt folder could not be read: {error}"))?;
        let modified = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .map_err(|error| format!("A prompt file could not be read: {error}"))?;
        let finished = now
            .duration_since(modified)
            .is_ok_and(|age| age > PROMPT_FILE_IN_USE);
        if !finished {
            in_use.push(entry.file_name().to_string_lossy().into_owned());
            continue;
        }
        match std::fs::remove_file(entry.path()) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "A finished prompt file could not be removed: {error}"
                ));
            }
        }
    }
    Ok(in_use)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sets `path`'s last write `age` before now.
    fn aged(path: &Path, age: std::time::Duration) {
        let file = std::fs::File::options().write(true).open(path).unwrap();
        file.set_modified(std::time::SystemTime::now() - age)
            .unwrap();
    }

    #[test]
    fn a_prompt_file_is_private_and_replaced_by_its_start() {
        let root = tempfile::tempdir().unwrap();
        let state = Some(root.path());
        let text = "한글 첫 지시\u{2028}둘째 줄";
        let path = write_file(state, "start-1", text).unwrap();
        let path = hide_platform::path::from_wire(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        assert!(private::is_private(&path).unwrap());
        write_file(state, "start-1", "again").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "again");
        remove(state, "start-1").unwrap();
        remove(state, "start-1").unwrap();
        assert!(!path.exists());
        assert!(write_file(state, "../out", "x").is_err());
        assert!(write_file(None, "start-1", "x").is_err());
    }

    /// Past the cap, finished files go first; a file still in use is never
    /// dropped, so with the cap in use a new start is refused instead.
    #[test]
    fn the_cap_drops_finished_files_and_refuses_rather_than_drop_one_in_use() {
        let root = tempfile::tempdir().unwrap();
        let state = Some(root.path());
        let folder = hide_kit::layout::prompt_files(root.path());
        let finished = PROMPT_FILE_IN_USE + std::time::Duration::from_secs(1);
        for n in 0..PROMPT_FILES_CAP + 4 {
            let path = write_file(state, &format!("old-{n}"), "x");
            // Each finished once written, so the next write drops it.
            aged(
                &hide_platform::path::from_wire(&path.unwrap()).unwrap(),
                finished,
            );
        }
        for n in 0..PROMPT_FILES_CAP - 1 {
            write_file(state, &format!("live-{n}"), "x").unwrap();
        }
        let names = || {
            let mut names = std::fs::read_dir(&folder)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            names.sort();
            names
        };
        // Every finished file went; the ones in use stay.
        assert!(names().iter().all(|name| name.starts_with("live-")));
        assert_eq!(names().len(), PROMPT_FILES_CAP - 1);
        // One more fits; the next is refused, and nothing in use is dropped.
        write_file(state, "live-last", "x").unwrap();
        let before = names();
        assert!(write_file(state, "one-too-many", "x").is_err());
        assert_eq!(names(), before);
        // Its own file is replaced, so a start writing again still fits.
        write_file(state, "live-0", "again").unwrap();
        // A node's start drops what finished while it was down.
        aged(&folder.join("live-1"), finished);
        sweep(state);
        assert!(!folder.join("live-1").exists());
        assert_eq!(names().len(), PROMPT_FILES_CAP - 1);
    }
}
