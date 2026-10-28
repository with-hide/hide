//! Bounded, explicit terminal file ingress. No provider draft or submission state.
//! The picked files are read by the node that holds them
//! (`hide_host::attachments`) and, for a device's pane, staged on that
//! device through its node link, whichever way the link opened (PRD
//! wsl-device B10); the clipboard image waits in a folder beside the core's
//! state file, where the screen's hided staged it, and the core's own node
//! removes it.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) use hide_node_link::attachments::{
    AttachmentFile, COMMIT_GRACE, MAX_FILES, MAX_PATH_BYTES, check_cancelled, valid_request_id,
};

pub(crate) fn clipboard_path(state_path: &Path, request_id: &str) -> PathBuf {
    state_path
        .with_file_name(hide_node_link::attachments::CLIPBOARD_FOLDER)
        .join(hide_node_link::attachments::clipboard_file_name(request_id))
}

/// Reads the picked files on `node`, which asks after each file whether to
/// go on, so a cancel stops the read between two files.
pub(crate) fn read_sources(
    node: &dyn crate::node_access::NodeLink,
    paths: &[String],
    cancelled: &AtomicBool,
) -> Result<Vec<AttachmentFile>, String> {
    check_cancelled(cancelled)?;
    let call = hide_node_link::protocol::Call::ReadAttachments {
        paths: paths.to_vec(),
    };
    let read: Vec<hide_node_link::attachments::ReadFile> =
        crate::node_access::call_as_with_progress(
            node,
            call,
            READ_TIMEOUT,
            |_: serde_json::Value| !cancelled.load(Ordering::Acquire),
        )
        .map_err(|error| match error {
            crate::node_access::LinkError::Refused(refusal) => refusal.message,
            other => format!("The selected files could not be read: {other}"),
        })?;
    check_cancelled(cancelled)?;
    read.into_iter()
        .map(|file| {
            Ok(AttachmentFile {
                bytes: file.bytes()?,
                path: file.path,
                name: file.name,
            })
        })
        .collect()
}

/// Reading 40 MiB from a local disk takes well under a minute.
const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Stages `files` on the machine `link` reaches, one file per call so a
/// cancel stops the transfer between two files, and answers each one's path
/// there. A transfer that fails takes what it staged; one that was
/// cancelled leaves that to the cancellation, which removes the request.
pub(crate) fn stage(
    link: &dyn crate::node_access::NodeLink,
    request_id: &str,
    files: &[AttachmentFile],
    cancelled: &AtomicBool,
) -> Result<Vec<String>, String> {
    let mut paths = Vec::with_capacity(files.len());
    for (index, file) in files.iter().enumerate() {
        check_cancelled(cancelled)?;
        let call = hide_node_link::protocol::Call::StageAttachment {
            request: request_id.to_owned(),
            index,
            count: files.len(),
            file: hide_node_link::feedback::File::new(file.name.clone(), &file.bytes),
        };
        match crate::node_access::call_as::<String>(link, call, STAGE_TIMEOUT) {
            Ok(path) => paths.push(path),
            Err(error) => {
                if index > 0 {
                    remove_staged(link, request_id);
                }
                return Err(match error {
                    crate::node_access::LinkError::Refused(refusal) => refusal.message,
                    other => format!("The file could not reach the device: {other}"),
                });
            }
        }
    }
    check_cancelled(cancelled)?;
    Ok(paths)
}

/// Asks the machine `link` reaches to remove what `request_id` staged; a
/// failure is logged, and the node removes it 24 hours on all the same.
pub(crate) fn remove_staged(link: &dyn crate::node_access::NodeLink, request_id: &str) {
    let call = hide_node_link::protocol::Call::RemoveAttachments {
        request: request_id.to_owned(),
    };
    if let Err(error) = crate::node_access::call_as::<()>(link, call, REMOVE_TIMEOUT) {
        crate::diagnostic!(
            serde_json::json!({"kind":"terminal.attachment.cleanup_deferred", "request_id":request_id, "error":error.to_string()})
        );
    }
}

/// One 20 MiB file across a device's link.
const STAGE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// The text a terminal receives for the staged files. A destination on a
/// device is that device's path, spelled with `/` whatever system the core
/// runs on; a local one is this machine's own.
pub(crate) fn paste_bytes(
    paths: &[String],
    bracketed: bool,
    on_device: bool,
) -> Result<Vec<u8>, String> {
    if paths.is_empty() || paths.len() > MAX_FILES {
        return Err("Choose between 1 and 8 regular files.".to_owned());
    }
    let mut quoted = Vec::with_capacity(paths.len());
    for path in paths {
        if path.len() > MAX_PATH_BYTES
            || !(if on_device {
                hide_platform::path::is_wire_absolute(path)
            } else {
                Path::new(path).is_absolute()
            })
            || path.chars().any(char::is_control)
        {
            return Err("The attachment destination is not a safe absolute path.".to_owned());
        }
        let mut value = String::from("\"");
        for character in path.chars() {
            if "\\\"$`".contains(character) {
                value.push('\\');
            }
            value.push(character);
        }
        value.push('"');
        quoted.push(if bracketed {
            format!("\u{1b}[200~{value}\u{1b}[201~")
        } else {
            value
        });
    }
    let mut text = quoted.join(" ");
    if !bracketed {
        text.push(' ');
    }
    Ok(text.into_bytes())
}

/// Asks `node`, the core's own, to remove the clipboard image a paste left;
/// a failure is logged, since the screen's hided ages the folder out too.
pub(crate) fn remove_clipboard(
    node: &dyn crate::node_access::NodeLink,
    state_path: &Path,
    request_id: &str,
) {
    let path = clipboard_path(state_path, request_id);
    let call = hide_node_link::protocol::Call::RemoveClipboard {
        path: path.to_string_lossy().into_owned(),
    };
    if let Err(error) = crate::node_access::call_as::<()>(node, call, REMOVE_TIMEOUT) {
        crate::diagnostic!(
            serde_json::json!({"kind":"terminal.attachment.clipboard_cleanup_failed", "request_id":request_id, "error":error.to_string()})
        );
    }
}

/// Removing one file from a local disk.
const REMOVE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths_are_quoted_in_file_order_without_submission() {
        let paths = vec![
            "/tmp/한글's.png".to_owned(),
            "/tmp/a $HOME `echo` ".to_owned(),
        ];
        assert_eq!(
            String::from_utf8(paste_bytes(&paths, true, true).unwrap()).unwrap(),
            "\u{1b}[200~\"/tmp/한글's.png\"\u{1b}[201~ \u{1b}[200~\"/tmp/a \\$HOME \\`echo\\` \"\u{1b}[201~"
        );
        assert_eq!(
            paste_bytes(&["/tmp/image.png".to_owned()], false, true).unwrap(),
            b"\"/tmp/image.png\" "
        );
        assert!(paste_bytes(&["/tmp/a\ncommand".to_owned()], true, true).is_err());
    }

    /// A device's destination is judged by its `/` spelling and a local one
    /// by this machine's rules, so a core on Windows still pastes a device's
    /// staged file.
    #[test]
    fn a_destination_is_absolute_by_the_machine_that_holds_it() {
        let local = std::env::temp_dir().join("image.png");
        let local = local.to_str().unwrap().to_owned();
        assert!(paste_bytes(&[local], false, false).is_ok());
        assert!(paste_bytes(&["image.png".to_owned()], false, false).is_err());
        assert!(paste_bytes(&["/home/example/image.png".to_owned()], false, true).is_ok());
        assert!(paste_bytes(&["image.png".to_owned()], false, true).is_err());
    }

    #[test]
    fn picked_files_are_read_on_the_node_with_its_refusal_in_the_operators_words() {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("한글.png");
        std::fs::write(&file, b"explicit bytes").unwrap();
        let node = hide_node::Local::of_process();
        let paths = vec![file.to_string_lossy().into_owned()];
        let read = read_sources(&node, &paths, &AtomicBool::new(false)).unwrap();
        assert_eq!(read[0].bytes, b"explicit bytes");
        assert_eq!(read[0].name, "한글.png");
        let refused = read_sources(
            &node,
            &[folder.path().to_string_lossy().into_owned()],
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(refused.starts_with("Folders, symbolic links"), "{refused}");
        assert!(read_sources(&node, &paths, &AtomicBool::new(true)).is_err());
    }
}
