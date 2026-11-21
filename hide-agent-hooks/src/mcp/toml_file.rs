//! A TOML file the operator owns, read and written the way the JSON hook
//! files are (`crate::install::HookDocument`): parsed with `toml_edit`,
//! which keeps every byte of the document Hide does not change, written
//! where a link leads, keeping the file's permissions, and only while the
//! file still holds what Hide read.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use toml_edit::DocumentMut;

use crate::install::{InstallFailure, MAX_DOCUMENT_BYTES, read_source};

pub(super) struct TomlFile {
    pub(super) document: DocumentMut,
    /// What the file held when it was read; `None` for a file not there.
    source: Option<String>,
    /// Where the file really is, through any link.
    target: PathBuf,
}

impl TomlFile {
    /// The file at `path`, or an empty document when there is none.
    pub(super) fn read(path: &Path) -> Result<Self, InstallFailure> {
        let unreadable = |detail: String| InstallFailure::Unreadable {
            path: path.display().to_string(),
            detail,
        };
        let target = match hide_platform::fs::identity::canonical(path) {
            Ok(target) => target,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Ok(Self {
                    document: DocumentMut::new(),
                    source: None,
                    target: path.to_path_buf(),
                });
            }
            Err(error) => return Err(unreadable(error.to_string())),
        };
        let source = read_source(&target)
            .map_err(|error| unreadable(error.to_string()))?
            .ok_or_else(|| {
                unreadable("the file disappeared while Hide was reading it; retry".to_owned())
            })?;
        let document = source.parse::<DocumentMut>().map_err(|error| {
            // toml_edit's message quotes the line it stopped at, which can
            // hold a key, so only where it stopped is said.
            let line = error
                .span()
                .map(|span| source[..span.start.min(source.len())].matches('\n').count() + 1);
            InstallFailure::UnexpectedShape {
                path: path.display().to_string(),
                detail: match line {
                    Some(line) => {
                        format!("it is not valid TOML (line {line}); Hide left it untouched")
                    }
                    None => "it is not valid TOML; Hide left it untouched".to_owned(),
                },
            }
        })?;
        Ok(Self {
            document,
            source: Some(source),
            target,
        })
    }

    /// Writes the document through a file beside the target, refusing when
    /// the file changed since it was read.
    pub(super) fn write(&self, path: &Path) -> Result<(), InstallFailure> {
        let failure = |detail: String| InstallFailure::NotWritable {
            path: path.display().to_string(),
            detail,
        };
        let rendered = self.document.to_string();
        if rendered.len() as u64 > MAX_DOCUMENT_BYTES {
            return Err(failure(
                "the edited file exceeds the 16 MiB configuration limit".to_owned(),
            ));
        }
        let now = match hide_platform::fs::identity::canonical(path) {
            Ok(real) => real,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                if std::fs::symlink_metadata(path)
                    .is_ok_and(|metadata| metadata.file_type().is_symlink())
                {
                    return Err(failure(
                        "the file is a link with no existing target; repair it and retry"
                            .to_owned(),
                    ));
                }
                path.to_path_buf()
            }
            Err(error) => return Err(failure(error.to_string())),
        };
        if now != self.target {
            return Err(failure(
                "the file's link changed while Hide was editing it; retry".to_owned(),
            ));
        }
        let current = read_source(&self.target).map_err(|error| failure(error.to_string()))?;
        let unchanged = match (&self.source, &current) {
            (Some(before), Some(current)) => before == current,
            (None, current) => current.as_ref().is_none_or(|text| text.trim().is_empty()),
            _ => false,
        };
        if !unchanged {
            return Err(failure(
                "the file changed while Hide was editing it; retry".to_owned(),
            ));
        }
        if let Some(parent) = self.target.parent() {
            std::fs::create_dir_all(parent).map_err(|error| failure(error.to_string()))?;
        }
        hide_platform::fs::atomic::write_file(
            &self.target,
            rendered.as_bytes(),
            hide_platform::fs::Access::KeepOrPrivate,
        )
        .map(drop)
        .map_err(|error| failure(error.to_string()))
    }
}
