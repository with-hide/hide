//! The state route captured by the kit, independent of an agent's shell.
//!
//! A hook may lose Hide's environment while keeping the shell's XDG state
//! home. The installed command therefore carries the kit's state folder.
//! An explicit pane route still wins; the captured route replaces only the
//! environment-based default, never the daemon's caller authentication.

use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

const ARGUMENT: &str = "--state-dir-b64";
const STATE_VARIABLE: &str = "HIDE_STATE_DIR";
const ENCODED_LIMIT: usize = 32 * 1024;

/// The installed helper and the daemon state folder its kit serves.
#[derive(Clone, Copy, Debug)]
pub struct HookTarget<'a> {
    helper: &'a Path,
    state: Option<&'a Path>,
}

impl<'a> HookTarget<'a> {
    pub fn new(helper: &'a Path, state: &'a Path) -> Result<Self, &'static str> {
        let text = state
            .to_str()
            .ok_or("the hook state folder must be UTF-8")?;
        if !state.is_absolute() || text.contains('\0') {
            return Err("the hook state folder must be an absolute path without NUL bytes");
        }
        if base64::encoded_len(text.len(), false).is_none_or(|length| length > ENCODED_LIMIT) {
            return Err("the hook state folder exceeds the encoded route limit");
        }
        Ok(Self {
            helper,
            state: Some(state),
        })
    }

    /// Shell-safe on POSIX and PowerShell, including paths containing quotes,
    /// dollars or spaces. These are arguments, not shell environment syntax.
    pub(crate) fn arguments(self) -> String {
        self.state.map_or_else(String::new, |state| {
            format!(
                " {ARGUMENT} {}",
                URL_SAFE_NO_PAD.encode(
                    state
                        .to_str()
                        .expect("validated hook state folder")
                        .as_bytes()
                )
            )
        })
    }
}

impl Deref for HookTarget<'_> {
    type Target = Path;

    fn deref(&self) -> &Path {
        self.helper
    }
}

// Unit tests retain the pre-route byte contracts. Production callers must
// construct a validated target, so no installer can silently omit its route.
#[cfg(test)]
impl<'a> From<&'a Path> for HookTarget<'a> {
    fn from(helper: &'a Path) -> Self {
        Self {
            helper,
            state: None,
        }
    }
}

#[cfg(test)]
impl<'a> From<&'a PathBuf> for HookTarget<'a> {
    fn from(helper: &'a PathBuf) -> Self {
        helper.as_path().into()
    }
}

fn parse(arguments: impl Iterator<Item = String>) -> Result<Option<PathBuf>, ()> {
    let mut arguments = arguments;
    let mut route = None;
    while let Some(argument) = arguments.next() {
        if argument != ARGUMENT {
            continue;
        }
        if route.is_some() {
            return Err(());
        }
        let encoded = arguments.next().ok_or(())?;
        if encoded.len() > ENCODED_LIMIT {
            return Err(());
        }
        let bytes = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| ())?;
        let path = PathBuf::from(String::from_utf8(bytes).map_err(|_| ())?);
        HookTarget::new(Path::new(""), &path).map_err(|_| ())?;
        route = Some(path);
    }
    Ok(route)
}

/// Give every sibling CLI call the same captured route. An invalid installed
/// route fails the hook rather than silently contacting another daemon.
pub fn apply(command: &mut Command) -> Result<(), &'static str> {
    if std::env::var_os(STATE_VARIABLE).is_some_and(|value| !value.is_empty()) {
        return Ok(());
    }
    let route = installed().as_ref().map_err(|_| "record")?;
    if let Some(route) = route {
        command.env(STATE_VARIABLE, route);
    }
    Ok(())
}

/// The route the kit installed this hook with, read from its arguments once.
fn installed() -> &'static Result<Option<PathBuf>, ()> {
    static ROUTE: OnceLock<Result<Option<PathBuf>, ()>> = OnceLock::new();
    ROUTE.get_or_init(|| parse(std::env::args().skip(1)))
}

/// The state folder the kit installed this hook for, when it named one.
pub(crate) fn installed_state_dir() -> Option<PathBuf> {
    installed().as_ref().ok().cloned().flatten()
}

/// Read a generated shell command for install diagnosis only. Execution
/// parses argv instead; the full command must still match the generator.
pub(crate) fn in_command(command: &str) -> Result<Option<PathBuf>, ()> {
    parse(
        command
            .split_whitespace()
            .map(|word| word.trim_end_matches(';').to_owned()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(values: &[&str]) -> impl Iterator<Item = String> {
        values.iter().map(|value| (*value).to_owned())
    }

    #[test]
    fn rejects_a_missing_malformed_relative_or_duplicate_route() {
        for values in [
            vec![ARGUMENT],
            vec![ARGUMENT, "!"],
            vec![ARGUMENT, "cmVsYXRpdmU"],
            vec![ARGUMENT, "L29uZQ", ARGUMENT, "L3R3bw"],
        ] {
            assert_eq!(parse(arguments(&values)), Err(()));
        }
    }

    #[test]
    fn an_old_command_keeps_its_environment_route() {
        assert_eq!(parse(arguments(&["hook", "--runtime", "grok"])), Ok(None));
    }

    #[test]
    fn installation_rejects_relative_nul_and_oversized_state_folders() {
        let home = tempfile::tempdir().unwrap();
        let helper = home.path().join("helper");
        for state in [
            PathBuf::from("relative"),
            home.path().join("bad\0route"),
            home.path().join("x".repeat(ENCODED_LIMIT)),
        ] {
            assert!(HookTarget::new(&helper, &state).is_err(), "{state:?}");
        }
        assert!(HookTarget::new(&helper, home.path()).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn installation_rejects_non_utf8_instead_of_capturing_another_path() {
        use std::os::unix::ffi::OsStringExt;
        let state = PathBuf::from(std::ffi::OsString::from_vec(b"/state/\xff".to_vec()));
        assert!(HookTarget::new(Path::new("/helper"), &state).is_err());
    }
}
