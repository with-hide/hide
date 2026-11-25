//! The node's side of a start's first input (`Call::PromptFile`): one
//! owner-only file per start in the state folder, which the start line reads
//! and removes (docs/delivery.md, The input ladder). The core removes the
//! file of a start that typed nothing (`Call::PromptFileRemove`); a line
//! that never ran leaves its file, and past
//! [`PROMPT_FILES_CAP`](hide_node_link::protocol::PROMPT_FILES_CAP) the
//! oldest go before a new one is written.

use std::path::{Path, PathBuf};

use hide_node_link::protocol::{PROMPT_FILE_MAX_BYTES, PROMPT_FILES_CAP, valid_prompt_start};
use hide_platform::fs::{Access, atomic, private};

/// Writes `text` as the prompt file of `start` under the state folder
/// `state` and answers its path in the wire spelling.
pub fn write(state: Option<&Path>, start: &str, text: &str) -> Result<String, String> {
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

/// Removes the oldest files until one more fits under the cap; the file of
/// `start` itself is replaced, so it takes no room.
fn make_room(folder: &Path, start: &str) -> Result<(), String> {
    let mut kept = std::fs::read_dir(folder)
        .map_err(|error| format!("The prompt folder could not be read: {error}"))?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name() != start)
        .filter_map(|entry| {
            let modified = entry.metadata().and_then(|meta| meta.modified()).ok()?;
            Some((modified, entry.path()))
        })
        .collect::<Vec<_>>();
    kept.sort();
    let over = (kept.len() + 1).saturating_sub(PROMPT_FILES_CAP);
    for (_, path) in kept.into_iter().take(over) {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("An old prompt file could not be removed: {error}")),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prompt_file_is_private_replaced_by_its_start_and_capped_oldest_first() {
        let state = tempfile::tempdir().unwrap();
        let state = Some(state.path());
        let text = "한글 첫 지시\u{2028}둘째 줄";
        let path = write(state, "start-1", text).unwrap();
        let path = hide_platform::path::from_wire(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        assert!(private::is_private(&path).unwrap());
        write(state, "start-1", "again").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "again");
        for n in 2..=PROMPT_FILES_CAP + 3 {
            write(state, &format!("start-{n}"), "x").unwrap();
        }
        let folder = hide_kit::layout::prompt_files(state.unwrap());
        assert_eq!(
            std::fs::read_dir(&folder).unwrap().count(),
            PROMPT_FILES_CAP
        );
        assert!(
            folder
                .join(format!("start-{}", PROMPT_FILES_CAP + 3))
                .exists()
        );
        remove(state, "start-2").unwrap();
        remove(state, "start-2").unwrap();
        assert!(write(state, "../out", "x").is_err());
        assert!(write(None, "start-1", "x").is_err());
    }
}
