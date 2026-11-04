//! How a path is spelled between machines, and how this machine reads it.
//!
//! A path leaves the machine that owns it in one spelling, the one Git's
//! index, LSP and VS Code's remote URIs use: UTF-8 with `/` between names.
//! A path below a root (a checkout, a History scope) is a [`RelPath`]: no
//! leading `/`, no empty, `.` or `..` name, so it can only name something
//! under the root it is joined to. An absolute path keeps its root as the
//! owning system writes it and only its separators change: `/repo/src` on
//! macOS and Linux, `C:/repo/src` and `//server/share/src` on Windows. On
//! macOS and Linux both spellings are the native ones, so nothing changes
//! there.
//!
//! Only the machine that owns a path turns it into a native one or makes
//! one from a native one: [`to_wire`] and [`relative`] on the way out,
//! [`from_wire`] and [`RelPath::to_native`] on the way in, each the one
//! place its direction happens. Any machine may relate wire spellings to
//! each other by their names alone ([`wire_join`], [`wire_relative`]),
//! which is correct whatever system owns them because the spelling is the
//! same; what it may not do is ask its own filesystem about another
//! machine's path or read it by its own system's rules.

use std::ffi::OsStr;
use std::fmt;
use std::path::{Component, Path, PathBuf};

/// Why a path cannot be spelled or read as asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathError {
    /// An absolute path where a path below a root was asked for.
    Absolute,
    /// A path below a root, or a bare name, where an absolute path was
    /// asked for.
    NotAbsolute,
    /// An empty, `.` or `..` name, which could name something other than a
    /// child of the root.
    NotNormal,
    /// Not under the root it was related to.
    Outside,
    /// On another drive or share than the root.
    OtherVolume,
    /// A name that is not UTF-8, which the wire cannot carry.
    NotUtf8,
    /// A name this system cannot hold as written: a NUL anywhere, and on
    /// Windows a `\`, `:` or other reserved character, a trailing dot or
    /// space, or a device name such as `NUL`.
    Unrepresentable(String),
}

impl fmt::Display for PathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Absolute => formatter.write_str("the path is absolute"),
            Self::NotAbsolute => formatter.write_str("the path is not absolute"),
            Self::NotNormal => formatter.write_str("the path has an empty, `.` or `..` name"),
            Self::Outside => formatter.write_str("the path is not inside its root"),
            Self::OtherVolume => formatter.write_str("the path is on another drive than its root"),
            Self::NotUtf8 => formatter.write_str("the path is not UTF-8"),
            Self::Unrepresentable(name) => {
                write!(formatter, "this system cannot hold the name {name:?}")
            }
        }
    }
}

impl std::error::Error for PathError {}

/// A path below a root, as every machine spells it. The empty path is the
/// root itself.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RelPath(String);

impl RelPath {
    /// The root itself.
    pub fn root() -> Self {
        Self(String::new())
    }

    /// A wire spelling read as a path below a root.
    pub fn parse(wire: &str) -> Result<Self, PathError> {
        if wire.is_empty() {
            return Ok(Self::root());
        }
        if wire.starts_with('/') {
            return Err(PathError::Absolute);
        }
        for name in wire.split('/') {
            check_name(name)?;
        }
        Ok(Self(wire.to_owned()))
    }

    /// One child name below this path. A name holding a `/` is refused
    /// rather than read as more than one.
    pub fn join(&self, name: &str) -> Result<Self, PathError> {
        if name.contains('/') {
            return Err(PathError::NotNormal);
        }
        check_name(name)?;
        Ok(Self(if self.0.is_empty() {
            name.to_owned()
        } else {
            format!("{}/{name}", self.0)
        }))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// The names from the root down; none for the root itself.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.0.split('/').filter(|name| !name.is_empty())
    }

    /// The last name; `None` for the root.
    pub fn file_name(&self) -> Option<&str> {
        self.names().last()
    }

    /// The folder holding this path; `None` for the root.
    pub fn parent(&self) -> Option<Self> {
        if self.0.is_empty() {
            return None;
        }
        Some(Self(
            self.0
                .rsplit_once('/')
                .map_or_else(String::new, |(parent, _)| parent.to_owned()),
        ))
    }

    /// Whether `self` is `ancestor` or below it.
    pub fn starts_with(&self, ancestor: &Self) -> bool {
        ancestor.0.is_empty()
            || self.0 == ancestor.0
            || self
                .0
                .strip_prefix(&ancestor.0)
                .is_some_and(|rest| rest.starts_with('/'))
    }

    /// What follows `ancestor` below it; the root when they are equal, and
    /// `None` when `self` is not below it.
    pub fn strip_prefix(&self, ancestor: &Self) -> Option<Self> {
        if ancestor.0.is_empty() {
            return Some(self.clone());
        }
        if self.0 == ancestor.0 {
            return Some(Self::root());
        }
        let rest = self.0.strip_prefix(&ancestor.0)?.strip_prefix('/')?;
        Some(Self(rest.to_owned()))
    }

    /// A relative path this machine produced (a walk below a root), in the
    /// wire spelling.
    pub fn from_native(relative: &Path) -> Result<Self, PathError> {
        let mut names = Vec::new();
        for component in relative.components() {
            match component {
                Component::Normal(name) => names.push(utf8(name)?),
                Component::CurDir | Component::ParentDir => return Err(PathError::NotNormal),
                Component::RootDir | Component::Prefix(_) => return Err(PathError::Absolute),
            }
        }
        let wire = names.join("/");
        // A name that holds the other system's separator stays one name on
        // this one; the parse refuses only what no system can hold.
        Self::parse(&wire)
    }

    /// This path as this machine names it below a root, to join to the
    /// root's native path or to open from its handle. A name this system
    /// cannot hold as written is refused, never read as something else.
    pub fn to_native(&self) -> Result<PathBuf, PathError> {
        let mut native = PathBuf::new();
        for name in self.names() {
            sys::check_native_name(name)?;
            native.push(name);
        }
        Ok(native)
    }
}

impl fmt::Display for RelPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

fn check_name(name: &str) -> Result<(), PathError> {
    if name.is_empty() || name == "." || name == ".." {
        return Err(PathError::NotNormal);
    }
    if name.contains('\0') {
        return Err(PathError::Unrepresentable(name.to_owned()));
    }
    Ok(())
}

fn utf8(name: &OsStr) -> Result<&str, PathError> {
    name.to_str().ok_or(PathError::NotUtf8)
}

/// An absolute path on this machine in the wire spelling. On Windows the
/// `\\?\` prefix goes where the short spelling means the same path, and a
/// drive letter is written upper case, so one folder has one spelling.
pub fn to_wire(path: &Path) -> Result<String, PathError> {
    if !path.is_absolute() {
        return Err(PathError::NotAbsolute);
    }
    sys::to_wire(path.to_str().ok_or(PathError::NotUtf8)?)
}

/// [`to_wire`] for a path that was always sent and is shown or compared
/// rather than read back: one with no wire spelling (not UTF-8, or a
/// Windows name Win32 would rewrite) keeps the system's own spelling with
/// replacement characters, which is what it carried before and on macOS and
/// Linux is the same text. Any later relation to it fails on its own.
pub fn to_wire_lossy(path: &Path) -> String {
    to_wire(path).unwrap_or_else(|_| path.to_string_lossy().into_owned())
}

/// The native path an absolute wire spelling names on this machine. Only
/// the machine that owns the path may ask.
pub fn from_wire(wire: &str) -> Result<PathBuf, PathError> {
    if wire.contains('\0') {
        return Err(PathError::Unrepresentable(wire.to_owned()));
    }
    sys::from_wire(wire).map(PathBuf::from)
}

/// `path` below `root`, both absolute paths on this machine. Names compare
/// as the root's volume compares them: a name that differs from the root's
/// only in case is the root's on a volume that ignores case (asked of the
/// root folder only when the spellings differ), and is outside it on one
/// that does not. On Windows a `\\?\` prefix on either side is read as the
/// path it names, and a path on another drive is [`PathError::OtherVolume`].
pub fn relative(root: &Path, path: &Path) -> Result<RelPath, PathError> {
    if !root.is_absolute() || !path.is_absolute() {
        return Err(PathError::NotAbsolute);
    }
    let mut below = path.components();
    let mut folded = false;
    for expected in root.components() {
        let Some(found) = below.next() else {
            return Err(PathError::Outside);
        };
        match (expected, found) {
            (Component::Prefix(expected), Component::Prefix(found)) => {
                if !sys::same_volume(expected.kind(), found.kind()) {
                    return Err(PathError::OtherVolume);
                }
            }
            (Component::RootDir, Component::RootDir) => {}
            (Component::Normal(expected), Component::Normal(found)) if expected == found => {}
            (Component::Normal(expected), Component::Normal(found)) => {
                match (expected.to_str(), found.to_str()) {
                    (Some(expected), Some(found))
                        if expected.to_lowercase() == found.to_lowercase() =>
                    {
                        folded = true;
                    }
                    _ => return Err(PathError::Outside),
                }
            }
            (_, Component::ParentDir | Component::CurDir) => return Err(PathError::NotNormal),
            _ => return Err(PathError::Outside),
        }
    }
    if folded && crate::fs::identity::case_sensitive(root).unwrap_or(true) {
        return Err(PathError::Outside);
    }
    RelPath::from_native(below.as_path())
}

/// Whether a wire spelling is absolute: `/...`, or a Windows drive or
/// share (`C:/...`, `//server/share`). It reads the spelling only, so any
/// machine may ask it of any machine's path.
pub fn is_wire_absolute(path: &str) -> bool {
    path.starts_with('/') || windows::drive_rest(path).is_some()
}

/// The Windows path a POSIX shell word names on Windows, where Git Bash
/// (MSYS2) and Cygwin mount each drive at `/c` and `/cygdrive/c`:
/// `/c/work/x` is `C:\work\x`. `None` for a word that names no drive mount.
/// It reads the spelling only, so every system checks it; a Windows caller
/// applies it to the words a shell line names.
pub fn shell_drive_path(word: &str) -> Option<String> {
    let mounted = word
        .strip_prefix("/cygdrive")
        .filter(|rest| rest.starts_with('/'))
        .unwrap_or(word);
    let rest = mounted.strip_prefix('/')?;
    let mut letters = rest.chars();
    let drive = letters.next().filter(char::is_ascii_alphabetic)?;
    let below = letters.as_str();
    if !(below.is_empty() || below.starts_with('/')) {
        return None;
    }
    Some(format!(
        "{}:\\{}",
        drive.to_ascii_uppercase(),
        below.trim_start_matches('/').replace('/', "\\")
    ))
}

/// The last name of a wire spelling, absolute or relative, by names alone;
/// the whole spelling when it has no `/` after its first letter.
pub fn wire_name(path: &str) -> &str {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
}

/// The folder above a wire spelling, by names alone, so any machine may ask
/// it of any machine's path: a root keeps its `/` (`/`, a drive's `C:/`),
/// and a root, a share's `//server/share` among them, has none.
pub fn wire_parent(path: &str) -> Option<&str> {
    let trimmed = path.trim_end_matches('/');
    if windows::share_rest(trimmed) == Some("") {
        return None;
    }
    let cut = trimmed.rfind('/')?;
    let (parent, root) = (&trimmed[..cut], &trimmed[..=cut]);
    if !is_wire_absolute(parent) && is_wire_absolute(root) {
        Some(root)
    } else {
        Some(parent)
    }
}

/// `relative` below the wire spelling `root`, by names alone: any machine
/// may ask, whatever system owns the root.
pub fn wire_join(root: &str, relative: &RelPath) -> String {
    if relative.is_root() {
        return root.to_owned();
    }
    format!("{}/{relative}", root.trim_end_matches('/'))
}

/// The wire spelling `path` below the wire spelling `root`, by names alone:
/// a sibling whose name only starts with the root's (`/repo-other` beside
/// `/repo`) is outside it. Names compare exactly, since only the machine
/// that owns the paths knows whether its volume ignores case.
pub fn wire_relative(root: &str, path: &str) -> Result<RelPath, PathError> {
    let root = root.trim_end_matches('/');
    let rest = path.strip_prefix(root).ok_or(PathError::Outside)?;
    if rest.is_empty() {
        return Ok(RelPath::root());
    }
    let rest = rest.strip_prefix('/').ok_or(PathError::Outside)?;
    if rest.is_empty() {
        return Ok(RelPath::root());
    }
    RelPath::parse(rest)
}

#[cfg(unix)]
mod sys {
    use std::path::Prefix;

    use super::PathError;

    pub(super) fn to_wire(native: &str) -> Result<String, PathError> {
        Ok(native.to_owned())
    }

    pub(super) fn from_wire(wire: &str) -> Result<String, PathError> {
        if !wire.starts_with('/') {
            return Err(PathError::NotAbsolute);
        }
        Ok(wire.to_owned())
    }

    pub(super) fn same_volume(_: Prefix<'_>, _: Prefix<'_>) -> bool {
        true
    }

    pub(super) fn check_native_name(_: &str) -> Result<(), PathError> {
        Ok(())
    }
}

#[cfg(windows)]
mod sys {
    use std::path::Prefix;

    use super::PathError;

    pub(super) fn to_wire(native: &str) -> Result<String, PathError> {
        super::windows::to_wire(native)
    }

    pub(super) fn from_wire(wire: &str) -> Result<String, PathError> {
        super::windows::from_wire(wire)
    }

    /// Whether two prefixes name one drive or share, `\\?\C:` and `C:`
    /// included.
    pub(super) fn same_volume(left: Prefix<'_>, right: Prefix<'_>) -> bool {
        fn volume(prefix: Prefix<'_>) -> Option<String> {
            match prefix {
                Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => {
                    Some(format!("{}:", char::from(drive).to_ascii_uppercase()))
                }
                Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => Some(format!(
                    r"\\{}\{}",
                    server.to_str()?.to_lowercase(),
                    share.to_str()?.to_lowercase()
                )),
                Prefix::Verbatim(_) | Prefix::DeviceNS(_) => None,
            }
        }
        matches!((volume(left), volume(right)), (Some(left), Some(right)) if left == right)
    }

    pub(super) fn check_native_name(name: &str) -> Result<(), PathError> {
        super::windows::check_name(name)
    }
}

/// The Windows spellings, as text so that every system checks them.
#[cfg_attr(not(windows), allow(dead_code))]
mod windows {
    use super::PathError;
    use crate::fs::identity::{short_verbatim, win32_rewrites};

    pub(super) fn to_wire(native: &str) -> Result<String, PathError> {
        let unprefixed;
        let native = if native.starts_with(r"\\?\") {
            unprefixed = short_verbatim(native)
                .ok_or_else(|| PathError::Unrepresentable(native.to_owned()))?;
            unprefixed.as_str()
        } else {
            native
        };
        if native.starts_with(r"\\.\") || native.starts_with(r"\??\") {
            return Err(PathError::Unrepresentable(native.to_owned()));
        }
        let wire = native.replace('\\', "/");
        if let Some(rest) = drive_rest(&wire) {
            let drive = wire[..1].to_ascii_uppercase();
            return Ok(format!("{drive}:{rest}"));
        }
        if share_rest(&wire).is_some() {
            return Ok(wire);
        }
        Err(PathError::NotAbsolute)
    }

    pub(super) fn from_wire(wire: &str) -> Result<String, PathError> {
        if drive_rest(wire).is_none() && share_rest(wire).is_none() {
            return Err(PathError::NotAbsolute);
        }
        Ok(wire.replace('/', "\\"))
    }

    /// What follows `X:` in `X:/...`.
    pub(super) fn drive_rest(wire: &str) -> Option<&str> {
        let mut letters = wire.chars();
        let drive = letters.next()?;
        (drive.is_ascii_alphabetic() && letters.next() == Some(':'))
            .then(|| &wire[2..])
            .filter(|rest| rest.starts_with('/'))
    }

    /// What follows `//server/share` in `//server/share/...`.
    pub(super) fn share_rest(wire: &str) -> Option<&str> {
        let body = wire.strip_prefix("//")?;
        let mut names = body.splitn(3, '/');
        let server = names.next()?;
        let share = names.next()?;
        if server.is_empty() || share.is_empty() || server == "?" || server == "." {
            return None;
        }
        Some(names.next().unwrap_or_default())
    }

    pub(super) fn check_name(name: &str) -> Result<(), PathError> {
        const RESERVED: [char; 9] = ['<', '>', ':', '"', '\\', '|', '?', '*', '/'];
        if name
            .chars()
            .any(|letter| letter < ' ' || RESERVED.contains(&letter))
            || win32_rewrites(name)
        {
            return Err(PathError::Unrepresentable(name.to_owned()));
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_windows_path_is_spelled_with_slashes_and_its_drive_upper_case() {
            assert_eq!(to_wire(r"C:\repo\src\a.rs").unwrap(), "C:/repo/src/a.rs");
            assert_eq!(to_wire(r"c:\repo").unwrap(), "C:/repo");
            assert_eq!(to_wire(r"C:\").unwrap(), "C:/");
            assert_eq!(to_wire(r"\\?\C:\repo\a").unwrap(), "C:/repo/a");
            assert_eq!(
                to_wire(r"\\server\share\repo").unwrap(),
                "//server/share/repo"
            );
            assert_eq!(
                to_wire(r"\\?\UNC\server\share\repo").unwrap(),
                "//server/share/repo"
            );
            // A device name is matched with ASCII case folding and Unicode
            // white space trimmed, so these are names, not devices.
            assert_eq!(
                to_wire("\\\\?\\C:\\a\\con\u{131}n$").unwrap(),
                "C:/a/con\u{131}n$"
            );
            assert_eq!(
                to_wire("\\\\?\\C:\\a\\lpt1\u{feff}").unwrap(),
                "C:/a/lpt1\u{feff}"
            );
            let long = format!(r"\\?\C:\{}", "a".repeat(300));
            assert_eq!(to_wire(&long).unwrap(), format!("C:/{}", "a".repeat(300)));
        }

        #[test]
        fn a_windows_path_with_no_short_spelling_is_refused() {
            for native in [
                r"\\?\C:\repo\name.",
                r"\\?\C:\repo\NUL",
                "\\\\?\\C:\\repo\\nul\u{85}",
                r"\\?\Volume{1234}\a",
                r"\\.\pipe\x",
                r"repo\a",
                r"C:repo",
            ] {
                assert!(to_wire(native).is_err(), "{native}");
            }
        }

        #[test]
        fn a_windows_wire_path_reads_back_as_the_native_one() {
            assert_eq!(from_wire("C:/repo/src/a.rs").unwrap(), r"C:\repo\src\a.rs");
            assert_eq!(from_wire("//server/share/a").unwrap(), r"\\server\share\a");
            assert_eq!(from_wire("/repo"), Err(PathError::NotAbsolute));
            assert_eq!(from_wire("repo/a"), Err(PathError::NotAbsolute));
        }

        #[test]
        fn names_windows_cannot_hold_are_refused() {
            for name in [
                "a:b", r"a\b", "NUL", "com1.txt", "name.", "name ", "a*b", "a\u{1}",
            ] {
                assert!(check_name(name).is_err(), "{name}");
            }
            assert!(check_name("main.rs").is_ok());
            assert!(check_name("한글 이름.md").is_ok());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relative_wire_path_holds_only_names_below_its_root() {
        for wire in ["/etc/passwd", "../x", "a/../b", "./a", "a//b", "a/", "a\0b"] {
            assert!(RelPath::parse(wire).is_err(), "{wire:?}");
        }
        assert!(RelPath::parse("").unwrap().is_root());
        assert_eq!(RelPath::parse("a/b.txt").unwrap().as_str(), "a/b.txt");
        // A `\` is a name's own letter on the wire; only Windows refuses it,
        // when it reads the path as its own.
        assert_eq!(RelPath::parse(r"a\b").unwrap().names().count(), 1);
    }

    #[test]
    fn a_relative_path_knows_its_parent_name_and_ancestors() {
        let path = RelPath::parse("src/lib/a.rs").unwrap();
        assert_eq!(path.file_name(), Some("a.rs"));
        assert_eq!(path.parent().unwrap().as_str(), "src/lib");
        assert_eq!(RelPath::parse("a").unwrap().parent(), Some(RelPath::root()));
        assert_eq!(RelPath::root().parent(), None);
        assert!(path.starts_with(&RelPath::parse("src").unwrap()));
        assert!(!path.starts_with(&RelPath::parse("sr").unwrap()));
        assert!(path.starts_with(&RelPath::root()));
        assert_eq!(
            RelPath::root().join("src").unwrap().join("a.rs").unwrap(),
            RelPath::parse("src/a.rs").unwrap()
        );
        assert!(RelPath::root().join("a/b").is_err());
        assert!(RelPath::root().join("..").is_err());
        let src = RelPath::parse("src").unwrap();
        assert_eq!(path.strip_prefix(&src).unwrap().as_str(), "lib/a.rs");
        assert!(src.strip_prefix(&src).unwrap().is_root());
        assert_eq!(path.strip_prefix(&RelPath::parse("sr").unwrap()), None);
    }

    #[test]
    fn wire_spellings_relate_by_whole_names() {
        assert_eq!(
            wire_relative("/repo", "/repo/src/a.rs").unwrap().as_str(),
            "src/a.rs"
        );
        assert_eq!(
            wire_relative("/repo/", "/repo/src").unwrap().as_str(),
            "src"
        );
        assert!(wire_relative("/repo", "/repo").unwrap().is_root());
        assert_eq!(
            wire_relative("/repo", "/repo-other/a"),
            Err(PathError::Outside)
        );
        assert_eq!(
            wire_relative("/repo", "/repo/../etc"),
            Err(PathError::NotNormal)
        );
        assert_eq!(wire_relative("C:/", "C:/a/b").unwrap().as_str(), "a/b");
        assert_eq!(
            wire_relative("C:/repo", "C:/Repo/a"),
            Err(PathError::Outside)
        );
        let src = RelPath::parse("src/a.rs").unwrap();
        assert_eq!(wire_join("/repo", &src), "/repo/src/a.rs");
        assert_eq!(wire_join("/repo/", &src), "/repo/src/a.rs");
        assert_eq!(wire_join("C:/", &src), "C:/src/a.rs");
        assert_eq!(wire_join("/repo", &RelPath::root()), "/repo");
        assert_eq!(wire_name("C:/repo/a.rs"), "a.rs");
        assert_eq!(wire_name("/repo/src/"), "src");
        assert_eq!(wire_name("/"), "/");
        for absolute in ["/repo", "C:/repo", "c:/", "//server/share/a"] {
            assert!(is_wire_absolute(absolute), "{absolute}");
        }
        for relative in ["repo", "C:repo", "src/a", ""] {
            assert!(!is_wire_absolute(relative), "{relative}");
        }
    }
}
