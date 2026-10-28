//! The Hide global instruction (PRD memory-self-improve D-23, D-29, D-31):
//! one file of the operator's approved standing instructions, in the core's
//! state folder (`hide_kit::layout::global_instruction`), that every session
//! started in a Hide pane receives at its start, in every Project, on every
//! machine, whether or not Memory is on there. It is Hide's own file, never
//! one of an agent's global files the operator owns (D-09, D-19).
//!
//! The operator may edit the file by hand, and may make it a link to a file
//! of their own, which Hide then writes through. Hide writes it only for a
//! fix the operator applied from the 제안 tab, one line at a time
//! ([`apply`]), and takes back only the line that fix added ([`revert`]);
//! each write is the editor's checked save, so an edit saved meanwhile is
//! never overwritten.

use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The file's size cap in bytes (D-23, B4): past it the file is not given at
/// all, and an apply that would cross it is refused.
pub const LIMIT_BYTES: usize = 4 * 1024;
/// Its cap in estimated tokens (B4), the same local estimate Memory's own
/// 1,200 uses, kept apart from Memory's.
pub const LIMIT_TOKENS: usize = 1_600;
/// The longest single line a fix adds, in characters.
pub const LINE_CHARS: usize = 400;

/// The file as it is now.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Instruction {
    pub text: String,
    /// What a preview was made against, so an apply can tell the file moved.
    pub fingerprint: String,
}

/// Why the file cannot be given (B5, B29).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unusable {
    /// Over [`LIMIT_BYTES`] or [`LIMIT_TOKENS`].
    TooLarge,
    /// Not UTF-8, or not readable as a file.
    Unreadable,
}

impl Unusable {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TooLarge => "too_large",
            Self::Unreadable => "unreadable",
        }
    }
}

/// The file's text, or why it cannot be given. A missing file is an empty
/// instruction. Nothing past the cap is read, so a huge file costs one bounded
/// read.
pub fn read(state_dir: &Path) -> Result<Instruction, Unusable> {
    use std::io::Read;
    let file = match target(state_dir).and_then(|path| hide_platform::fs::open_regular(&path)) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(instruction(String::new()));
        }
        // A folder, a pipe or a device is refused without blocking on it.
        Err(_) => return Err(Unusable::Unreadable),
    };
    let mut bytes = Vec::with_capacity(LIMIT_BYTES + 1);
    file.take(LIMIT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Unusable::Unreadable)?;
    if bytes.len() > LIMIT_BYTES {
        return Err(Unusable::TooLarge);
    }
    let text = String::from_utf8(bytes).map_err(|_| Unusable::Unreadable)?;
    if !fits(&text) {
        return Err(Unusable::TooLarge);
    }
    Ok(instruction(text))
}

/// The file the name stands for: the name itself, or what a link there
/// points at, which may not exist yet.
fn target(state_dir: &Path) -> std::io::Result<std::path::PathBuf> {
    let path = hide_kit::layout::global_instruction(state_dir);
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            let link = std::fs::read_link(&path)?;
            Ok(path.parent().unwrap_or(state_dir).join(link))
        }
        _ => Ok(path),
    }
}

fn instruction(text: String) -> Instruction {
    Instruction {
        fingerprint: fingerprint(&text),
        text,
    }
}

fn fits(text: &str) -> bool {
    text.len() <= LIMIT_BYTES && hide_memory::hook::estimated_tokens(text) <= LIMIT_TOKENS
}

fn fingerprint(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

/// The block a session start carries (B1): Hide's own wrapper says the
/// operator approved it, which Memory's untrusted block never does. An empty
/// file gives nothing.
pub fn render(instruction: &Instruction) -> Option<String> {
    let text = instruction.text.trim();
    if text.is_empty() {
        return None;
    }
    Some(format!(
        "<hide-global-instruction trust=\"operator-approved\">\nThe operator approved these instructions for every project and every agent. Follow them unless the operator says otherwise now.\n{text}\n</hide-global-instruction>\n"
    ))
}

/// The block for a session start in `state_dir`, or none; an unusable file
/// is logged and left out whole, never cut (B5, B29).
pub fn for_session(state_dir: &Path) -> Option<String> {
    match read(state_dir) {
        Ok(instruction) => render(&instruction),
        Err(reason) => {
            crate::diagnostic!(serde_json::json!({
                "component": "global_instruction",
                "kind": "global_instruction.not_given",
                "reason": reason.as_str(),
            }));
            None
        }
    }
}

/// What applying one line would make of the file (B6, B8, B10).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Preview {
    pub line: String,
    /// The file the preview was made against.
    pub before: String,
    /// The file the apply leaves, so a retry after a write it never heard
    /// back from reads as done (B8).
    pub after: String,
}

impl Preview {
    /// What the operator was shown, the line and the file it goes into, as
    /// one value a press sends back: a press checked against it writes
    /// neither another line nor into another file (B8).
    pub fn token(&self) -> String {
        format!(
            "{:x}",
            Sha256::digest(format!("{}\n{}", self.before, self.line))
        )
    }
}

/// Why an apply or a revert wrote nothing.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "reason")]
pub enum Refusal {
    /// The file cannot be read or is over its cap as it is (B5).
    Unusable { unusable: Unusable },
    /// The line would take the file past a cap (B10).
    NoRoom,
    /// The line is empty, too long or more than one line.
    BadLine,
    /// The file changed since the preview; here is a new one (B8).
    Changed { preview: Preview },
    /// The line this fix added was edited or removed by hand; the file is
    /// as it is and the line is what the fix added (B9).
    Edited { line: String, text: String },
    /// Writing the file failed (B29).
    WriteFailed,
}

/// The one line a fix adds, cleaned, or [`Refusal::BadLine`].
/// Every session takes the line as the operator's own instruction, so it
/// carries no markup that could end the block it sits in, no character no one
/// would see in the preview, and no secret.
fn clean_line(line: &str) -> Result<String, Refusal> {
    let line = line.trim();
    if line.is_empty()
        || line.chars().count() > LINE_CHARS
        || line.contains(['<', '>'])
        || line
            .chars()
            .any(|character| character.is_control() || hide_memory::is_invisible(character))
        || hide_memory::redact(line).contains_secret_candidate
    {
        return Err(Refusal::BadLine);
    }
    Ok(line.to_owned())
}

/// Whether `candidate`, one line of the file, is `line` as a fix wrote it.
fn same_line(candidate: &str, line: &str) -> bool {
    candidate.trim() == line.trim()
}

/// Whether the file already has `line` as one of its lines, so an apply
/// would only repeat it.
pub fn holds(state_dir: &Path, line: &str) -> Result<bool, Unusable> {
    let current = read(state_dir)?;
    let line = line.trim();
    Ok(current
        .text
        .lines()
        .any(|candidate| same_line(candidate, line)))
}

/// What applying `line` to the file as it is now would write.
pub fn preview(state_dir: &Path, line: &str) -> Result<Preview, Refusal> {
    let line = clean_line(line)?;
    let current = read(state_dir).map_err(|unusable| Refusal::Unusable { unusable })?;
    preview_of(&current.text, line)
}

/// What applying `line` to `current`, a read of the file, would write.
pub fn preview_against(current: &Instruction, line: &str) -> Result<Preview, Refusal> {
    preview_of(&current.text, clean_line(line)?)
}

fn preview_of(text: &str, line: String) -> Result<Preview, Refusal> {
    let mut after = text.to_owned();
    if !after.is_empty() && !after.ends_with('\n') {
        after.push('\n');
    }
    after.push_str(&line);
    after.push('\n');
    if !fits(&after) {
        return Err(Refusal::NoRoom);
    }
    Ok(Preview {
        line,
        before: fingerprint(text),
        after: fingerprint(&after),
    })
}

/// Adds the preview's line when the file is still what the preview was made
/// against; the operator's own lines are never removed. The same apply twice,
/// or again after a write whose answer was lost, adds the line once (B8).
pub fn apply(state_dir: &Path, preview: &Preview) -> Result<(), Refusal> {
    let line = clean_line(&preview.line)?;
    let current = read(state_dir).map_err(|unusable| Refusal::Unusable { unusable })?;
    if current.fingerprint == preview.after {
        return Ok(());
    }
    let fresh = preview_of(&current.text, line)?;
    if current.fingerprint != preview.before {
        return Err(Refusal::Changed { preview: fresh });
    }
    let mut after = current.text.clone();
    if !after.is_empty() && !after.ends_with('\n') {
        after.push('\n');
    }
    after.push_str(&fresh.line);
    after.push('\n');
    match write(state_dir, &current.text, &after) {
        Err(Write::Moved) => {
            let now = read(state_dir).map_err(|unusable| Refusal::Unusable { unusable })?;
            Err(Refusal::Changed {
                preview: preview_of(&now.text, fresh.line)?,
            })
        }
        Err(Write::Failed) => Err(Refusal::WriteFailed),
        Ok(()) => Ok(()),
    }
}

/// Takes out the line one fix added, and only it (D-30, B9): another fix's
/// line and the operator's own edits stay. A line no longer in the file as it
/// was added is refused with the file as it is, and nothing is written.
pub fn revert(state_dir: &Path, line: &str) -> Result<(), Refusal> {
    let line = clean_line(line)?;
    let current = read(state_dir).map_err(|unusable| Refusal::Unusable { unusable })?;
    let lines: Vec<&str> = current.text.split_inclusive('\n').collect();
    let Some(at) = lines
        .iter()
        .rposition(|candidate| same_line(candidate, &line))
    else {
        return Err(Refusal::Edited {
            line,
            text: current.text,
        });
    };
    let after: String = lines
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != at)
        .map(|(_, candidate)| *candidate)
        .collect();
    match write(state_dir, &current.text, &after) {
        Ok(()) => Ok(()),
        // Saved by hand meanwhile: the line is judged again on what is there.
        Err(Write::Moved) => {
            let now = read(state_dir).map_err(|unusable| Refusal::Unusable { unusable })?;
            Err(Refusal::Edited {
                line,
                text: now.text,
            })
        }
        Err(Write::Failed) => Err(Refusal::WriteFailed),
    }
}

/// Why a write left the file as it was.
enum Write {
    /// The file is no longer the one read; someone saved it meanwhile.
    Moved,
    Failed,
}

/// Replaces the file that held `before` with `after`, through a link, by the
/// editor's checked save (`hide_host::save`), which puts back an edit saved
/// between the read and the swap; a file that did not exist is made only if
/// it still does not.
fn write(state_dir: &Path, before: &str, after: &str) -> Result<(), Write> {
    let failed = |error: &dyn std::fmt::Display| {
        crate::diagnostic!(serde_json::json!({
            "component": "global_instruction",
            "kind": "global_instruction.write_failed",
            "error": error.to_string(),
        }));
        Write::Failed
    };
    let path = target(state_dir).map_err(|error| failed(&error.kind()))?;
    let (Some(folder), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(failed(&"the file has no folder"));
    };
    hide_platform::fs::private::create_dir_all(folder).map_err(|error| failed(&error.kind()))?;
    let exists = std::fs::symlink_metadata(&path).is_ok();
    if !exists {
        if !before.is_empty() {
            return Err(Write::Moved);
        }
        let staged = folder.join(format!(".{}.hide-new", name.to_string_lossy()));
        let made = hide_platform::fs::private::create_new_file(&staged)
            .and_then(|mut file| {
                use std::io::Write as _;
                file.write_all(after.as_bytes())?;
                file.sync_all()
            })
            .and_then(|_| hide_platform::fs::atomic::rename_no_replace_path(&staged, &path));
        return match made {
            Ok(()) => Ok(()),
            Err(error) => {
                let _ = std::fs::remove_file(&staged);
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    Err(Write::Moved)
                } else {
                    Err(failed(&error.kind()))
                }
            }
        };
    }
    let dir = cap_std::fs::Dir::open_ambient_dir(folder, cap_std::ambient_authority())
        .map_err(|error| failed(&error.kind()))?;
    match hide_host::save::save(
        &dir,
        Path::new(name),
        after.as_bytes(),
        &hide_host::document::revision_of(before.as_bytes()),
    ) {
        Ok(_) => Ok(()),
        Err(error) if error.code == hide_host::ErrorCode::Conflict => Err(Write::Moved),
        Err(error) => Err(failed(&error.code.as_str())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(state: &Path) -> std::path::PathBuf {
        hide_kit::layout::global_instruction(state)
    }

    #[test]
    fn a_missing_or_blank_file_gives_nothing() {
        let state = tempfile::tempdir().unwrap();
        assert_eq!(for_session(state.path()), None);
        std::fs::write(file(state.path()), "\n  \n").unwrap();
        assert_eq!(for_session(state.path()), None);
    }

    #[test]
    fn a_session_gets_the_whole_file_in_the_operator_approved_block() {
        let state = tempfile::tempdir().unwrap();
        std::fs::write(
            file(state.path()),
            "Answer in Korean.\nNever use the em dash.\n",
        )
        .unwrap();
        let block = for_session(state.path()).unwrap();
        assert!(block.starts_with("<hide-global-instruction trust=\"operator-approved\">\n"));
        assert!(
            block.contains("Answer in Korean.\nNever use the em dash.\n</hide-global-instruction>")
        );
    }

    #[test]
    fn an_oversized_or_non_utf8_file_is_left_out_whole() {
        let state = tempfile::tempdir().unwrap();
        std::fs::write(file(state.path()), "a".repeat(LIMIT_BYTES + 1)).unwrap();
        assert_eq!(read(state.path()), Err(Unusable::TooLarge));
        assert_eq!(for_session(state.path()), None);
        // Under the byte cap but over the token cap: digits cost a token
        // every two characters.
        std::fs::write(file(state.path()), "1".repeat(3_000)).unwrap();
        assert_eq!(read(state.path()), Err(Unusable::TooLarge));
        std::fs::write(file(state.path()), [0xff, 0xfe, b'a']).unwrap();
        assert_eq!(read(state.path()), Err(Unusable::Unreadable));
        assert_eq!(for_session(state.path()), None);
    }

    #[test]
    fn an_apply_adds_its_line_once_and_keeps_the_operators_lines() {
        let state = tempfile::tempdir().unwrap();
        std::fs::write(file(state.path()), "My own line").unwrap();
        let preview = preview(state.path(), "  Answer in Korean.  ").unwrap();
        assert_eq!(preview.line, "Answer in Korean.");
        apply(state.path(), &preview).unwrap();
        // Pressed twice, or retried after a lost answer.
        apply(state.path(), &preview).unwrap();
        assert_eq!(
            std::fs::read_to_string(file(state.path())).unwrap(),
            "My own line\nAnswer in Korean.\n"
        );
    }

    #[test]
    fn an_apply_against_a_file_that_moved_is_refused_with_a_new_preview() {
        let state = tempfile::tempdir().unwrap();
        let stale = preview(state.path(), "Answer in Korean.").unwrap();
        std::fs::write(file(state.path()), "Edited meanwhile\n").unwrap();
        let Err(Refusal::Changed { preview: fresh }) = apply(state.path(), &stale) else {
            panic!("a moved file must refuse the old preview");
        };
        assert_eq!(
            std::fs::read_to_string(file(state.path())).unwrap(),
            "Edited meanwhile\n"
        );
        apply(state.path(), &fresh).unwrap();
        assert_eq!(
            std::fs::read_to_string(file(state.path())).unwrap(),
            "Edited meanwhile\nAnswer in Korean.\n"
        );
    }

    #[test]
    fn an_apply_past_the_cap_writes_nothing() {
        let state = tempfile::tempdir().unwrap();
        let full = format!("{}\n", "word ".repeat(800));
        std::fs::write(file(state.path()), &full).unwrap();
        assert_eq!(
            preview(state.path(), &"x".repeat(LINE_CHARS)),
            Err(Refusal::NoRoom)
        );
        assert_eq!(std::fs::read_to_string(file(state.path())).unwrap(), full);
    }

    #[test]
    fn a_revert_takes_out_only_its_own_line() {
        let state = tempfile::tempdir().unwrap();
        for line in ["First fix.", "Second fix."] {
            let preview = preview(state.path(), line).unwrap();
            apply(state.path(), &preview).unwrap();
        }
        std::fs::write(file(state.path()), "First fix.\nMy edit.\nSecond fix.\n").unwrap();
        revert(state.path(), "First fix.").unwrap();
        assert_eq!(
            std::fs::read_to_string(file(state.path())).unwrap(),
            "My edit.\nSecond fix.\n"
        );
    }

    #[test]
    fn a_revert_of_a_line_edited_by_hand_writes_nothing_and_shows_the_file() {
        let state = tempfile::tempdir().unwrap();
        apply(
            state.path(),
            &preview(state.path(), "Answer in Korean.").unwrap(),
        )
        .unwrap();
        std::fs::write(file(state.path()), "Answer in Korean, mostly.\n").unwrap();
        assert_eq!(
            revert(state.path(), "Answer in Korean."),
            Err(Refusal::Edited {
                line: "Answer in Korean.".into(),
                text: "Answer in Korean, mostly.\n".into(),
            })
        );
        assert_eq!(
            std::fs::read_to_string(file(state.path())).unwrap(),
            "Answer in Korean, mostly.\n"
        );
    }

    #[test]
    fn a_line_that_is_not_one_line_is_refused() {
        let state = tempfile::tempdir().unwrap();
        assert_eq!(preview(state.path(), " "), Err(Refusal::BadLine));
        assert_eq!(preview(state.path(), "a\nb"), Err(Refusal::BadLine));
        assert_eq!(
            preview(state.path(), &"x".repeat(LINE_CHARS + 1)),
            Err(Refusal::BadLine)
        );
    }
}
