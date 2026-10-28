//! The filesystem boundary the web shell reads and writes behind (PRD
//! web-shell-pivot-s2 D-09 and B10, web-shell-pivot-s3 D-01 and B11).
//!
//! Two lines live here, and every path a client sends is checked against one
//! of them before the core sees it.
//!
//! The registration flow (`create_workspace`, the folder Add a project
//! picked) lives behind `$HOME`, as it did in S2: a path under home is
//! resolved one component at a time from home, where a symlink's target is
//! tested as written the same way before it is followed, so a symlink planted
//! under home never leads a registration out of it. A path written outside
//! home is read only once the request carries the operator's confirmation of
//! that same path (PRD wsl-device D-02, D-10); until then it is answered
//! `needs_confirmation` before anything is read, so an unconfirmed request
//! never learns whether a path outside home exists. Creating a new folder and
//! cloning stay inside home. `..` and encoded segments are refused on shape
//! or resolve like any other name. The boundary root is read from `HOME` at
//! boot and is not configurable (`practices/env.md`), and nothing remembers a
//! confirmation: the registration it made is the permission, so removing the
//! project removes it.
//!
//! The Explorer's file work (open, reveal, save, create, rename, move,
//! trash, and later the index and the attachment bytes) lives behind the
//! registered checkout roots the core snapshot carries, which is the same
//! line the core draws for a path outside every checkout. The roots come from
//! the snapshot frame hided already decodes for its clients
//! (`rest.navigator.workspaces`, a local workspace whose checkout exists),
//! so the daemon keeps no second copy of the registration contract. The walk
//! below is the one the home line uses with the root set in place of home, so
//! a symlink out of a checkout is refused as `outside_checkout` before its
//! target is read, whether or not that target exists. Roots are kept most
//! specific first, so a checkout nested inside another owns the paths under
//! it.
//!
//! The Explorer's listing (`file_list`) is that same line: the children of a
//! folder under a root, files and hidden names included and `.git` dropped,
//! ordered as the Explorer orders them, with a symlink that leaves the
//! root left out rather than followed.
//!
//! A refused path is answered to the client and logged. Accepted core events
//! retain the checked path for UI identity, while actual file I/O uses opened
//! checkout-root capabilities supplied from this boundary.

use herdr_core::node::NodeId;
use std::collections::VecDeque;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

#[cfg(unix)]
fn open_nonblocking(path: &Path, directory: bool) -> io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut flags = libc::O_NONBLOCK | libc::O_NOCTTY | libc::O_CLOEXEC;
    if directory {
        flags |= libc::O_DIRECTORY;
    }
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(flags)
        .open(path)
}

#[cfg(windows)]
fn open_nonblocking(path: &Path, directory: bool) -> io::Result<fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    // cap-std's rooted traversal requires a root handle that cannot be
    // renamed out from under it on Windows.
    let mut options = fs::OpenOptions::new();
    options.read(true).share_mode(0x00000001 | 0x00000002);
    if directory {
        // FILE_FLAG_BACKUP_SEMANTICS permits opening a directory as a handle.
        options.custom_flags(0x02000000);
    }
    options.open(path)
}

#[cfg(not(any(unix, windows)))]
fn open_nonblocking(path: &Path, _directory: bool) -> io::Result<fs::File> {
    fs::File::open(path)
}

/// The filesystem's identity of a registered root, pinned at registration.
type RootIdentity = hide_platform::fs::identity::FileId;

fn root_identity(opened: &fs::File) -> Option<RootIdentity> {
    hide_platform::fs::identity::file_id_of(opened).ok()
}

/// The path the open root or file is at now.
fn opened_file_path(file: &fs::File) -> io::Result<PathBuf> {
    hide_platform::fs::identity::path_of(file)
}

/// Why a path was not answered or forwarded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Refusal {
    /// The real path is not under the home directory.
    OutsideHome,
    /// The real path is not under a registered checkout root.
    OutsideCheckout,
    /// The path is the home directory itself, or a folder holding it; a
    /// workspace is a subdirectory.
    HomeRoot,
    /// The path is written outside home and the request did not carry the
    /// operator's confirmation of it; nothing about it was read.
    NeedsConfirmation,
    /// Nothing exists at the path.
    NotFound,
    /// The path exists but is not a directory.
    NotADirectory,
    /// The path exists but is not a regular file.
    NotAFile,
    /// Empty, relative, or otherwise not a path this daemon reads.
    InvalidPath,
    /// Something already stands where a new project's folder would be made.
    AlreadyExists,
}

impl Refusal {
    /// The reason code the contract names (`contracts/hided-ws.schema.json`).
    pub fn code(self) -> &'static str {
        match self {
            Self::OutsideHome => "outside_home",
            Self::OutsideCheckout => "outside_checkout",
            Self::HomeRoot => "home_root",
            Self::NeedsConfirmation => "needs_confirmation",
            Self::NotFound => "not_found",
            Self::NotADirectory => "not_a_directory",
            Self::NotAFile => "not_a_file",
            Self::InvalidPath => "invalid_path",
            Self::AlreadyExists => "already_exists",
        }
    }
}

/// Why a `create_workspace` was not forwarded: its path was refused, or the
/// folder belongs to a project outside home other than the folder confirmed,
/// whose root the operator is asked about instead.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Unregistrable {
    Refused(Refusal),
    ConfirmProject(PathBuf),
}

impl From<Refusal> for Unregistrable {
    fn from(refusal: Refusal) -> Self {
        Self::Refused(refusal)
    }
}

/// One child a listing shows: its name, the path a client sends back, and
/// whether it is a directory, which the Explorer draws and opens differently.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub is_directory: bool,
    /// The entry's own inode in a checkout listing, which a trash of the
    /// row confirms; decimal text on the wire (`herdr_core::inode_text`).
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "herdr_core::inode_text::serialize"
    )]
    pub inode: Option<u64>,
}

/// A listing answer: the directory that was listed and its visible children.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct Listing {
    pub root_path: String,
    pub entries: Vec<Entry>,
    /// More than `hide_host::list::LIST_CAP` children existed; the rest were not read.
    pub truncated: bool,
}

/// Symlinks one path may pass through before it is refused as a loop; the
/// kernel's own limit for one lookup is the same order.
const SYMLINK_HOPS: usize = 40;

/// Bytes one `file_bytes` request may name, for the whole file or one range.
/// A viewer reads a document into memory, so the daemon refuses a read past
/// this rather than growing an unbounded allocation (engineering 15).
pub const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;

/// A name a create or a rename may carry: one component, and not one of the
/// shapes that would name a different path. The core refuses the same names
/// from the paths alone, so this is the boundary's own answer to the client
/// rather than the last word on the name.
pub fn valid_name(name: &str) -> bool {
    !name.contains('/')
        && hide_platform::path::RelPath::root()
            .join(name)
            .and_then(|path| path.to_native())
            .is_ok()
}

/// Lexical normalization of an absolute path: `.` dropped, `..` applied to
/// the component before it. Used only on a symlink target joined to a path
/// that holds no symlink, where the lexical parent is the real one.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// One checkout the Explorer's file work is confined to: the pair a client
/// names when it opens or reveals, and the path the core's snapshot carries for
/// that checkout, kept exactly as the snapshot spells it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Root {
    pub workspace_id: String,
    pub checkout_id: String,
    pub path: PathBuf,
}

impl Root {
    /// A checkout root the core opened on this node over its link: it
    /// names no workspace or checkout here, only its path.
    pub fn opened_on_node(path: &str) -> Self {
        Self {
            workspace_id: String::new(),
            checkout_id: String::new(),
            path: PathBuf::from(path),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RegisteredRoot {
    source: Root,
    /// Pinned when the core registered the checkout. A renamed or replaced
    /// root cannot make an outside file appear to be inside that checkout.
    real_path: PathBuf,
    identity: RootIdentity,
}

#[cfg(unix)]
fn open_under_root(root: &RegisteredRoot, path: &Path, directory: bool) -> io::Result<fs::File> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;

    let anchor = open_nonblocking(&root.source.path, true)?;
    if root_identity(&anchor) != Some(root.identity) || opened_file_path(&anchor)? != root.real_path
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "registered checkout root was replaced",
        ));
    }
    let relative = path
        .strip_prefix(&root.source.path)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "path is outside checkout"))?;
    if relative.as_os_str().is_empty() {
        return Ok(anchor);
    }
    let name = CString::new(relative.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid path"))?;
    let mut flags = libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOCTTY | libc::O_CLOEXEC;
    if directory {
        flags |= libc::O_DIRECTORY;
    }
    let fd = unsafe { libc::openat(anchor.as_raw_fd(), name.as_ptr(), flags) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { fs::File::from_raw_fd(fd) })
}

#[cfg(windows)]
fn open_under_root(root: &RegisteredRoot, path: &Path, directory: bool) -> io::Result<fs::File> {
    let anchor = open_nonblocking(&root.source.path, true)?;
    if root_identity(&anchor) != Some(root.identity) || opened_file_path(&anchor)? != root.real_path
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "registered checkout root was replaced",
        ));
    }
    if path == root.source.path {
        return Ok(anchor);
    }
    let opened = open_nonblocking(path, directory)?;
    if !opened_file_path(&opened)?.starts_with(&root.real_path) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "path is outside checkout",
        ));
    }
    Ok(opened)
}

#[cfg(not(any(unix, windows)))]
fn open_under_root(_root: &RegisteredRoot, _path: &Path, _directory: bool) -> io::Result<fs::File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "registered root handles unavailable",
    ))
}

#[derive(Debug)]
pub struct Boundary {
    /// The real home directory every accepted path resolves under.
    home: PathBuf,
    /// Home as `HOME` names it; a client writes paths under this spelling
    /// when home itself sits behind a symlink (`/var` for `/private/var`).
    home_as_given: PathBuf,
    /// The registered checkout roots, most specific first. Empty until the
    /// core's first snapshot arrives, so Explorer work that arrives before it
    /// is refused as `outside_checkout` rather than acted on.
    roots: RwLock<Vec<RegisteredRoot>>,
    /// Checkout roots on SSH devices, as the core's catalog names them. They
    /// are another machine's paths: this boundary only admits the pair a
    /// device listing names, and the device's helper confines the work.
    device_roots: RwLock<Vec<DeviceRoot>>,
    /// The node this boundary guards: a path an event names under this
    /// device id, or under none, is one of this machine's paths.
    node: NodeId,
}

/// A new project's folder as the `$HOME` line resolved it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewProject {
    /// The canonical folder it goes in: home or a directory under it.
    pub parent: PathBuf,
    /// The canonical path the folder takes.
    pub path: PathBuf,
    /// A folder holding nothing but `.git` is already there, and is used.
    pub leftover: bool,
}

/// One checkout on an SSH device: the device and the root path there.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceRoot {
    pub device_id: String,
    pub path: String,
}

impl Boundary {
    /// This machine's boundary, for a test: the node is read from the
    /// operating system, as the daemon reads it.
    #[cfg(test)]
    pub fn new(home: &Path) -> Result<Self, String> {
        Self::for_node(home, NodeId::of_this_machine()?)
    }

    /// Reads the boundary root once. Fails when `$HOME` does not resolve to a
    /// directory: a daemon without a boundary must not serve the registration
    /// flow at all.
    pub fn for_node(home: &Path, node: NodeId) -> Result<Self, String> {
        // The short spelling (no `\\?\` on Windows), which is the one a
        // client sends and the snapshot carries.
        let real = hide_platform::fs::identity::canonical(home)
            .map_err(|error| format!("HOME {} does not resolve: {error}", home.display()))?;
        if !real.is_dir() {
            return Err(format!("HOME {} is not a directory", home.display()));
        }
        Ok(Self {
            home: real,
            home_as_given: home.to_path_buf(),
            roots: RwLock::new(Vec::new()),
            device_roots: RwLock::new(Vec::new()),
            node,
        })
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    /// The machine whose paths this boundary guards.
    pub fn node(&self) -> &NodeId {
        &self.node
    }

    /// Whether an event's `device_id` names this machine: the node id, or
    /// none at all, the protocol's spelling of "the core's own machine".
    pub fn names_this_node(&self, device_id: Option<&str>) -> bool {
        device_id.is_none_or(|device| device.is_empty() || self.node == device)
    }

    /// Replaces the root set wholesale with the checkouts the core's latest
    /// snapshot carries: a registration whose path is not an absolute existing
    /// directory is dropped, and the rest are kept most specific first, so a
    /// checkout nested inside another owns the paths under it.
    ///
    /// The paths are kept exactly as the snapshot spells them. The core
    /// compares the root it is handed with the focused checkout's path byte for
    /// byte, so re-spelling one here would refuse every operation on it.
    pub fn set_roots(&self, roots: Vec<Root>) {
        let previous = self.roots_for_read().clone();
        let mut accepted: Vec<RegisteredRoot> = roots
            .into_iter()
            .filter_map(|root| {
                if !root.path.is_absolute() {
                    return None;
                }
                // A snapshot refresh is not a new registration. Preserve the
                // original physical root if its pathname was replaced.
                let (real_path, identity) = if let Some(registered) =
                    previous.iter().find(|registered| registered.source == root)
                {
                    (registered.real_path.clone(), registered.identity)
                } else {
                    let opened = open_nonblocking(&root.path, true).ok()?;
                    let real_path = opened_file_path(&opened).ok()?;
                    if !opened.metadata().ok()?.is_dir() {
                        return None;
                    }
                    (real_path, root_identity(&opened)?)
                };
                Some(RegisteredRoot {
                    source: root,
                    real_path,
                    identity,
                })
            })
            .collect();
        accepted.sort_by_key(|root| std::cmp::Reverse(root.source.path.components().count()));
        {
            let current = self.roots_for_read();
            if *current == accepted {
                return;
            }
        }
        let count = accepted.len();
        *self.roots_for_write() = accepted;
        eprintln!(
            "{}",
            serde_json::json!({"component": "hided", "kind": "boundary.roots", "roots": count})
        );
    }

    /// Replaces the device checkout roots with the catalog's.
    pub fn set_device_roots(&self, roots: Vec<DeviceRoot>) {
        *self
            .device_roots
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = roots;
    }

    /// Whether `root` is a checkout the catalog carries for `device_id`.
    pub fn is_device_root(&self, device_id: &str, root: &str) -> bool {
        self.device_roots
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .any(|known| known.device_id == device_id && known.path == root)
    }

    /// Whether `path` is a checkout root the catalog carries for `device_id`
    /// or a folder under one, by spelling alone: the device's helper is what
    /// judges the path itself.
    pub fn is_under_device_root(&self, device_id: &str, path: &str) -> bool {
        self.device_roots
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .any(|known| {
                known.device_id == device_id
                    && hide_platform::path::wire_relative(&known.path, path).is_ok()
            })
    }

    fn roots_for_read(&self) -> RwLockReadGuard<'_, Vec<RegisteredRoot>> {
        self.roots
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn roots_for_write(&self) -> RwLockWriteGuard<'_, Vec<RegisteredRoot>> {
        self.roots
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The registered root `raw` names, as the snapshot spells it, or `None`
    /// when no registration carries that path.
    pub fn known_root(&self, raw: &str) -> Option<PathBuf> {
        let wanted = Path::new(raw);
        self.roots_for_read()
            .iter()
            .find(|candidate| candidate.source.path == wanted)
            .filter(|candidate| Self::root_is_current(candidate))
            .map(|candidate| candidate.source.path.clone())
    }

    /// The identity `raw`'s registered root was verified by, the one every
    /// listing of it is checked against, while that root is current.
    pub fn root_identity(&self, raw: &str) -> Option<hide_node_link::RootIdentity> {
        let wanted = Path::new(raw);
        let roots = self.roots_for_read();
        let root = roots
            .iter()
            .find(|candidate| candidate.source.path == wanted)
            .filter(|candidate| Self::root_is_current(candidate))?;
        Some(hide_node_link::RootIdentity {
            device: root.identity.volume(),
            inode: u64::try_from(root.identity.index()).ok()?,
        })
    }

    fn root_is_current(root: &RegisteredRoot) -> bool {
        let Ok(opened) = open_nonblocking(&root.source.path, true) else {
            return false;
        };
        opened_file_path(&opened).ok().as_deref() == Some(root.real_path.as_path())
            && root_identity(&opened) == Some(root.identity)
    }

    /// Keep verified root identities alive for the core. Its file workers
    /// reopen the root against this identity before performing I/O.
    /// A passive pin must not block removal of an otherwise unused worktree.
    pub fn opened_roots(&self) -> Vec<(PathBuf, fs::File)> {
        self.roots_for_read()
            .iter()
            .filter_map(|root| {
                #[cfg(windows)]
                {
                    // Unlike a traversal capability, this handle only pins
                    // identity. open_dir keeps Windows' default delete sharing
                    // so retaining it in FileRoots cannot prevent cleanup.
                    let opened = hide_platform::fs::open_dir(&root.source.path).ok()?;
                    if root_identity(&opened) != Some(root.identity)
                        || opened_file_path(&opened).ok().as_deref()
                            != Some(root.real_path.as_path())
                    {
                        return None;
                    }
                    Some((root.source.path.clone(), opened))
                }
                #[cfg(not(windows))]
                {
                    open_under_root(root, &root.source.path, true)
                        .ok()
                        .map(|opened| (root.source.path.clone(), opened))
                }
            })
            .collect()
    }

    /// An opened directory whose handle, rather than its mutable pathname,
    /// is the source for watch and index work.
    pub fn open_directory(&self, root: &Path, raw: &str) -> Result<(PathBuf, fs::File), Refusal> {
        let registered = self.registered_root(root)?;
        let path = self.resolve_below(root, raw)?;
        let opened =
            open_under_root(&registered, &path, true).map_err(|_| Refusal::OutsideCheckout)?;
        if !opened.metadata().map_err(|_| Refusal::NotFound)?.is_dir() {
            return Err(Refusal::NotADirectory);
        }
        if !opened_file_path(&opened)
            .map_err(|_| Refusal::OutsideCheckout)?
            .starts_with(&registered.real_path)
        {
            return Err(Refusal::OutsideCheckout);
        }
        Ok((path, opened))
    }

    fn registered_root(&self, path: &Path) -> Result<RegisteredRoot, Refusal> {
        self.roots_for_read()
            .iter()
            .find(|candidate| candidate.source.path == path)
            .filter(|candidate| Self::root_is_current(candidate))
            .cloned()
            .ok_or(Refusal::OutsideCheckout)
    }

    /// The path of `raw` when it is written under `root`, which has to be one
    /// of the registered roots (`known_root`).
    ///
    /// The answer keeps the root's own spelling instead of the filesystem's,
    /// because the core compares the root it receives with the focused
    /// checkout's path byte for byte.
    pub fn resolve_below(&self, root: &Path, raw: &str) -> Result<PathBuf, Refusal> {
        self.registered_root(root)?;
        let raw = self.expand(raw)?;
        let path = Path::new(&raw);
        let Ok(rest) = path.strip_prefix(root) else {
            return Err(Refusal::OutsideCheckout);
        };
        self.walk(root, rest, Refusal::OutsideCheckout)
    }

    /// The path of `raw` when it is written under a registered checkout root,
    /// whether it names a file or a directory.
    ///
    /// A path under home that is not under a root is not this line's to answer
    /// and is refused as `outside_checkout`, so the two lines never mix: the
    /// registration flow reads home and this one reads the checkouts.
    pub fn resolve_target(&self, raw: &str) -> Result<PathBuf, Refusal> {
        let raw = self.expand(raw)?;
        let path = Path::new(&raw);
        let Some((root, rest)) = self.strip_root(path) else {
            return Err(Refusal::OutsideCheckout);
        };
        self.registered_root(&root)?;
        self.walk(&root, rest, Refusal::OutsideCheckout)
    }

    /// A native browser may report the physical spelling of a file loaded
    /// through a registered checkout alias. Map only a pinned root's physical
    /// prefix back to its source spelling, then run the ordinary boundary.
    /// Never canonicalize an untrusted path or retry a refused source spelling.
    pub fn resolve_browser_report(&self, raw: &str) -> Result<PathBuf, Refusal> {
        let raw = self.expand(raw)?;
        let path = Path::new(&raw);
        if self.strip_root(path).is_some() {
            return self.resolve_target(&raw);
        }
        let checked = self
            .roots_for_read()
            .iter()
            .filter_map(|root| {
                path.strip_prefix(&root.real_path)
                    .ok()
                    .map(|rest| (root, rest))
            })
            .max_by_key(|(root, _)| root.real_path.components().count())
            .map(|(root, rest)| root.source.path.join(rest))
            .ok_or(Refusal::OutsideCheckout)?;
        self.resolve_target(checked.to_str().ok_or(Refusal::InvalidPath)?)
    }

    /// A regular file under a registered checkout root, with its size. The
    /// file-bytes read is the only caller: a viewer asks for bytes, so a
    /// directory, a symlink to one, and anything else that is not a file is
    /// refused as `not_a_file` rather than read.
    pub fn resolve_file(&self, raw: &str) -> Result<(PathBuf, u64), Refusal> {
        let path = self.resolve_target(raw)?;
        let metadata = fs::metadata(&path).map_err(|_| Refusal::NotFound)?;
        if !metadata.is_file() {
            return Err(Refusal::NotAFile);
        }
        Ok((path, metadata.len()))
    }

    /// Open a byte source and prove the opened handle is still below the
    /// registered root. `resolve_target` alone cannot prove this: an attacker
    /// may swap a checked component for a symlink before `File::open`.
    pub fn open_file(&self, raw: &str) -> Result<(PathBuf, fs::File, u64), Refusal> {
        let expanded = self.expand(raw)?;
        let root = self
            .roots_for_read()
            .iter()
            .find(|candidate| Path::new(&expanded).starts_with(&candidate.source.path))
            .cloned()
            .ok_or(Refusal::OutsideCheckout)?;
        let path = self.resolve_target(raw)?;
        let file = open_under_root(&root, &path, false).map_err(|error| {
            if error.kind() == io::ErrorKind::InvalidData {
                Refusal::OutsideCheckout
            } else {
                Refusal::NotFound
            }
        })?;
        let metadata = file.metadata().map_err(|_| Refusal::NotFound)?;
        if !metadata.is_file() {
            return Err(Refusal::NotAFile);
        }
        let opened = opened_file_path(&file).map_err(|_| Refusal::OutsideCheckout)?;
        if !opened.starts_with(&root.real_path) {
            return Err(Refusal::OutsideCheckout);
        }
        Ok((path, file, metadata.len()))
    }

    /// The path of `raw` when it is written under `root`, the root the event
    /// itself named.
    pub fn resolve_in_root(&self, root: &str, raw: &str) -> Result<PathBuf, Refusal> {
        let Some(known) = self.known_root(root) else {
            return Err(Refusal::OutsideCheckout);
        };
        self.resolve_below(&known, raw)
    }

    /// The path of `raw` for an open or a reveal that names a checkout: it has
    /// to sit under that checkout's root. A pair this daemon holds no root for
    /// is answered against any root, because the core names that case itself
    /// (`reveal.unknown_checkout`) and the client reads one reason either way.
    pub fn resolve_checkout(
        &self,
        workspace_id: &str,
        checkout_id: &str,
        raw: &str,
    ) -> Result<PathBuf, Refusal> {
        let named = self
            .roots_for_read()
            .iter()
            .find(|root| {
                root.source.workspace_id == workspace_id && root.source.checkout_id == checkout_id
            })
            .map(|root| root.source.path.clone());
        match named {
            Some(root) => self.resolve_below(&root, raw),
            None => self.resolve_target(raw),
        }
    }

    /// Whether `raw` resolves under a current registered root. A save keeps
    /// the original spelling because the core compares it with its open tab.
    pub fn is_under_root(&self, raw: &str) -> bool {
        if self.resolve_target(raw).is_ok() {
            return true;
        }
        // An open tab may outlive a file deleted on disk. Let the core report
        // the failed save and preserve its draft, but only when the missing
        // leaf's existing parent is still within the pinned checkout.
        let Ok(expanded) = self.expand(raw) else {
            return false;
        };
        let path = Path::new(&expanded);
        matches!(fs::symlink_metadata(path), Err(error) if error.kind() == io::ErrorKind::NotFound)
            && path
                .parent()
                .is_some_and(|parent| self.resolve_target(&parent.to_string_lossy()).is_ok())
    }

    /// The most specific root `path` is written under, with what follows it.
    /// The comparison is component-wise, so a sibling whose name merely starts
    /// with a root's path (`/repo-other` beside `/repo`) is not inside it.
    fn strip_root<'a>(&self, path: &'a Path) -> Option<(PathBuf, &'a Path)> {
        self.roots_for_read().iter().find_map(|root| {
            path.strip_prefix(&root.source.path)
                .ok()
                .map(|rest| (root.source.path.clone(), rest))
        })
    }

    /// The canonical directory for `raw` when it is home or under home.
    /// `~` and `~/...` name the home directory, so a client can start its
    /// listing without knowing the path; the answer carries the real one.
    /// A path written outside home, or reached through a symlink whose
    /// target is written outside home, is refused before the filesystem is
    /// read there, so the reason never says whether such a path exists.
    pub fn resolve_dir(&self, raw: &str) -> Result<PathBuf, Refusal> {
        let raw = self.expand(raw)?;
        let path = Path::new(&raw);
        let Some(rest) = self.strip_home(path) else {
            return Err(Refusal::OutsideHome);
        };
        let resolved = self.walk(&self.home, rest, Refusal::OutsideHome)?;
        // The walk left no symlink in `resolved`, so this only settles the
        // spelling the OS keeps (letter case on a case-insensitive volume)
        // and cannot fail for a reason the walk has not already answered.
        let real =
            hide_platform::fs::identity::canonical(&resolved).map_err(|_| Refusal::InvalidPath)?;
        if !real.starts_with(&self.home) {
            return Err(Refusal::OutsideHome);
        }
        let metadata = fs::metadata(&real).map_err(|_| Refusal::NotFound)?;
        if !metadata.is_dir() {
            return Err(Refusal::NotADirectory);
        }
        Ok(real)
    }

    /// `~` and `~/...` name the home directory, so a client can start without
    /// knowing the path; any other spelling is used as written. Every shape
    /// this daemon does not read is refused here: empty, a NUL, a relative
    /// path, and a `..` component, which would otherwise name its way out of
    /// the directory it is joined to. A `.` component is normalized away by
    /// the path parser and only survives as the first component of a relative
    /// path, which the absolute check above already refuses; the core reads
    /// the same rule, so the two agree on what a normal path is.
    fn expand(&self, raw: &str) -> Result<String, Refusal> {
        let expanded;
        let raw = if raw == "~" || raw.starts_with("~/") {
            expanded = self
                .home
                .join(raw.trim_start_matches('~').trim_start_matches('/'));
            expanded.to_string_lossy().into_owned()
        } else {
            raw.to_owned()
        };
        let path = Path::new(&raw);
        if raw.is_empty() || raw.contains('\0') || !path.is_absolute() {
            return Err(Refusal::InvalidPath);
        }
        if path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
        {
            return Err(Refusal::InvalidPath);
        }
        Ok(raw)
    }

    /// The part of `path` below home, when `path` is written under home in
    /// either spelling.
    fn strip_home<'a>(&self, path: &'a Path) -> Option<&'a Path> {
        path.strip_prefix(&self.home)
            .or_else(|_| path.strip_prefix(&self.home_as_given))
            .ok()
    }

    /// The part of `path` below `anchor`, accepting the second spelling of
    /// home when the anchor is home (`/var` for `/private/var`), which is the
    /// spelling `HOME` uses.
    fn strip_anchor<'a>(&self, anchor: &Path, path: &'a Path) -> Option<&'a Path> {
        if let Ok(rest) = path.strip_prefix(anchor) {
            return Some(rest);
        }
        if anchor == self.home {
            self.strip_home(path)
        } else {
            None
        }
    }

    /// Resolves `rest` below `anchor` one component at a time. Every name is
    /// read where it sits, so the filesystem is only ever asked about paths
    /// under the anchor; a symlink is replaced by its target as written, and a
    /// target not written under the anchor is refused as `outside` - the
    /// reason the line that asked for the walk names - before anything about it
    /// is read, whether or not it exists.
    fn walk(&self, anchor: &Path, rest: &Path, outside: Refusal) -> Result<PathBuf, Refusal> {
        let mut current = anchor.to_path_buf();
        let mut pending: VecDeque<OsString> = rest
            .components()
            .filter(|component| matches!(component, Component::Normal(_)))
            .map(|component| component.as_os_str().to_owned())
            .collect();
        let mut hops = 0;
        while let Some(name) = pending.pop_front() {
            let next = current.join(&name);
            let metadata = match fs::symlink_metadata(&next) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    return Err(Refusal::NotFound);
                }
                Err(error) if error.kind() == io::ErrorKind::NotADirectory => {
                    return Err(Refusal::NotADirectory);
                }
                // A loop, a denied directory, or another OS refusal under
                // home: none of them describes anything outside it.
                Err(_) => return Err(Refusal::InvalidPath),
            };
            if metadata.file_type().is_symlink() {
                hops += 1;
                if hops > SYMLINK_HOPS {
                    return Err(Refusal::InvalidPath);
                }
                let target = fs::read_link(&next).map_err(|_| Refusal::InvalidPath)?;
                let target = normalize(&current.join(target));
                let Some(below) = self.strip_anchor(anchor, &target) else {
                    return Err(outside);
                };
                // Start over from the anchor with the target's components in
                // front of what is still pending; `current` holds no symlink,
                // so a `..` in the target was resolved on real directories.
                let mut replaced: VecDeque<OsString> = below
                    .components()
                    .filter(|component| matches!(component, Component::Normal(_)))
                    .map(|component| component.as_os_str().to_owned())
                    .collect();
                replaced.append(&mut pending);
                pending = replaced;
                current = anchor.to_path_buf();
                continue;
            }
            if !pending.is_empty() && !metadata.is_dir() {
                return Err(Refusal::NotADirectory);
            }
            current = next;
        }
        Ok(current)
    }

    /// The path a `create_workspace` registers, judged by the one
    /// registration rule a device's helper also answers with
    /// (`hide_host::register::judge`) against this machine's home: a directory
    /// under home that stays under it, or an existing directory written
    /// outside home that `confirmed` names, the operator's confirmation of
    /// that one path as the request spells it, and whose project is that
    /// folder. A folder in a project outside home is answered with the
    /// question about the project's root (`Unregistrable::ConfirmProject`).
    /// Home and a folder holding it are never a workspace (`home_root`), and
    /// a network share, written or reached through a link, is no project's
    /// identity (PRD wsl-device D-15).
    pub fn resolve_registration(
        &self,
        raw: &str,
        confirmed: Option<&str>,
    ) -> Result<PathBuf, Unregistrable> {
        use hide_host::register::Unregistrable as Judged;
        let expanded = self.expand(raw)?;
        // A confirmation that is not a path this daemon reads confirms nothing.
        let confirmed = confirmed.and_then(|raw| self.expand(raw).ok());
        // Below home, the walk refuses a path that leaves home before it
        // reads anything past home, so the reason never says what is there.
        let walk = |rest: &Path| {
            let resolved = self.walk(&self.home, rest, Refusal::OutsideHome).map_err(
                |refusal| match refusal {
                    Refusal::OutsideHome => Judged::OutsideHome,
                    Refusal::NotFound => Judged::NotFound,
                    Refusal::NotADirectory => Judged::NotADirectory,
                    _ => Judged::InvalidPath,
                },
            )?;
            // The walk left no link, so this only settles the spelling the
            // system keeps (letter case on a volume that ignores it).
            hide_platform::fs::identity::canonical(&resolved).map_err(|_| Judged::InvalidPath)
        };
        hide_host::register::judge_with(
            Path::new(&expanded),
            confirmed.as_deref().map(Path::new),
            &self.home,
            walk,
        )
        .map(|judged| judged.real)
        .map_err(|refusal| match refusal {
            Judged::NeedsConfirmation(Some(root)) => Unregistrable::ConfirmProject(root),
            Judged::NeedsConfirmation(None) => Refusal::NeedsConfirmation.into(),
            Judged::InvalidPath => Refusal::InvalidPath.into(),
            Judged::HomeRoot => Refusal::HomeRoot.into(),
            Judged::OutsideHome => Refusal::OutsideHome.into(),
            Judged::NotFound => Refusal::NotFound.into(),
            Judged::NotADirectory => Refusal::NotADirectory.into(),
            // A loop or a denied directory, as a walk under home answers.
            Judged::Unreadable(_) => Refusal::InvalidPath.into(),
        })
    }

    /// Where Add a project's Create new project would make its folder: `name`
    /// inside `parent`, which is home or a directory under it, so the new
    /// folder is always strictly under home. The target is read without
    /// following a symlink at it; a leftover of an earlier attempt (a folder
    /// holding nothing but `.git`) is accepted so a retry converges, and
    /// anything else there is `already_exists`. Answers the canonical parent
    /// and target, and whether the target is such a leftover.
    pub fn resolve_new_project(&self, parent: &str, name: &str) -> Result<NewProject, Refusal> {
        let parent = self.resolve_dir(parent)?;
        if !valid_name(name) {
            return Err(Refusal::InvalidPath);
        }
        let path = parent.join(name);
        let leftover = match hide_host::project::folder(&path) {
            hide_host::project::ProjectFolder::Free => false,
            hide_host::project::ProjectFolder::Leftover => true,
            hide_host::project::ProjectFolder::Taken => return Err(Refusal::AlreadyExists),
        };
        Ok(NewProject {
            parent,
            path,
            leftover,
        })
    }

    /// A path under home as `~` spells it, for a line the operator reads.
    pub fn home_label(&self, path: &Path) -> String {
        match path.strip_prefix(&self.home) {
            Ok(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
            Ok(rest) => format!(
                "~/{}",
                hide_platform::path::RelPath::from_native(rest)
                    .map(hide_platform::path::RelPath::into_string)
                    .unwrap_or_else(|_| rest.display().to_string())
            ),
            Err(_) => path.display().to_string(),
        }
    }

    /// The canonical parent a clone may write `name` into: home or a folder
    /// under it, and a plain folder name nothing holds there yet, not even a
    /// dangling symlink or the `.git`-only leftover Create new project
    /// continues into, since a clone never writes into a folder it did not
    /// make. The folder the clone creates is therefore strictly under home,
    /// which is what `create_workspace` then registers.
    pub fn resolve_clone_target(&self, parent: &str, name: &str) -> Result<PathBuf, Refusal> {
        let real = self.resolve_dir(parent)?;
        if !valid_name(name) {
            return Err(Refusal::InvalidPath);
        }
        match fs::symlink_metadata(real.join(name)) {
            Ok(_) => Err(Refusal::AlreadyExists),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(real),
            Err(_) => Err(Refusal::InvalidPath),
        }
    }

    /// The children of a folder inside a registered checkout: files and
    /// directories, hidden names included, `.git` left out.
    ///
    /// This is the Explorer's line, so the policy is the Explorer's
    /// (`web/src/explorer.ts`): `.git` is the name it hides that the approved
    /// line hides too - the other names it skips are its own build directories,
    /// which the approved line does not name - and the order is its order,
    /// directories first and then the natural one `localizedStandardCompare`
    /// gives, so `file2` sorts before `file10`. The Explorer renders the order
    /// it is handed; nothing sorts these rows again on the client.
    ///
    /// A child that is neither a file nor a directory, and a symlink whose
    /// target leaves `root`, is not a row: the boundary does not describe a
    /// path outside the checkout, not even as a name in a listing.
    pub fn list_children(&self, root: &Path, raw: &str) -> Result<Listing, Refusal> {
        let registered = self.registered_root(root)?;
        let dir = self.resolve_below(root, raw)?;
        let anchor = open_under_root(&registered, &registered.source.path, true)
            .map_err(|_| Refusal::OutsideCheckout)?;
        let relative = dir
            .strip_prefix(&registered.source.path)
            .map_err(|_| Refusal::OutsideCheckout)?;
        let checkout = cap_std::fs::Dir::from_std_file(anchor);
        let listed = hide_host::list::require_directory(&checkout, relative)
            .and_then(|()| hide_host::list::list(&checkout, relative, &registered.real_path))
            .map_err(|error| match error.code {
                hide_host::ErrorCode::NotADirectory => Refusal::NotADirectory,
                hide_host::ErrorCode::OutsideRoot | hide_host::ErrorCode::InvalidPath => {
                    Refusal::OutsideCheckout
                }
                _ => Refusal::NotFound,
            })?;
        Ok(Listing {
            root_path: hide_platform::path::to_wire_lossy(&dir),
            entries: listed
                .entries
                .into_iter()
                .map(|entry| Entry {
                    path: hide_platform::path::to_wire_lossy(&dir.join(&entry.name)),
                    name: entry.name,
                    is_directory: entry.is_directory,
                    inode: Some(entry.inode),
                })
                .collect(),
            truncated: listed.truncated,
        })
    }
}

/// The Windows forms of the confirmed outside-home line (PRD wsl-device B21).
#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    /// [`Boundary::resolve_registration`] as the refusal its frame carries.
    fn workspace(
        boundary: &Boundary,
        raw: &str,
        confirmed: Option<&str>,
    ) -> Result<PathBuf, Refusal> {
        boundary
            .resolve_registration(raw, confirmed)
            .map_err(|refused| match refused {
                Unregistrable::Refused(refusal) => refusal,
                Unregistrable::ConfirmProject(_) => Refusal::NeedsConfirmation,
            })
    }

    #[test]
    fn a_folder_on_a_drive_is_confirmable_and_a_network_share_is_not() {
        let outer = tempfile::tempdir().unwrap();
        let home = outer.path().join("home");
        let outside = outer.path().join("outside");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let boundary = Boundary::new(&home).unwrap();
        let written = hide_platform::path::to_wire_lossy(&outside);
        assert_eq!(
            workspace(&boundary, &written, None),
            Err(Refusal::NeedsConfirmation)
        );
        assert_eq!(
            workspace(&boundary, &written, Some(&written)),
            Ok(hide_platform::fs::identity::canonical(&outside).unwrap())
        );
        for share in [
            "//server/share/app",
            "//wsl$/Ubuntu/home/me/app",
            "//wsl.localhost/Ubuntu/home/me/app",
            "//?/C:/app",
        ] {
            assert_eq!(
                workspace(&boundary, share, Some(share)),
                Err(Refusal::OutsideHome),
                "{share}"
            );
        }
        let drive = &written[..3];
        assert_eq!(
            workspace(&boundary, drive, Some(drive)),
            Err(Refusal::HomeRoot),
            "the drive home is on holds it"
        );
    }

    /// A folder on a drive that is a link to a network share is judged by
    /// where it leads: no confirmation makes the share a project.
    #[test]
    fn a_confirmed_link_to_a_network_share_is_refused_like_the_share() {
        let outer = tempfile::tempdir().unwrap();
        let home = outer.path().join("home");
        let outside = outer.path().join("outside");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(outside.join("shared")).unwrap();
        let boundary = Boundary::new(&home).unwrap();
        // The same folder through the drive's administrative share.
        let local = hide_platform::fs::identity::canonical(&outside.join("shared")).unwrap();
        let local = local.to_string_lossy();
        let share = format!(r"\\localhost\{}${}", &local[..1], &local[2..]);
        let link = outside.join("share-link");
        std::os::windows::fs::symlink_dir(&share, &link).unwrap();
        let written = hide_platform::path::to_wire_lossy(&link);
        assert_eq!(
            boundary.resolve_registration(&written, Some(&written)),
            Err(Unregistrable::Refused(Refusal::OutsideHome)),
            "{written} -> {share}"
        );
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// [`Boundary::resolve_registration`] as the refusal its frame carries.
    fn workspace(
        boundary: &Boundary,
        raw: &str,
        confirmed: Option<&str>,
    ) -> Result<PathBuf, Refusal> {
        boundary
            .resolve_registration(raw, confirmed)
            .map_err(|refused| match refused {
                Unregistrable::Refused(refusal) => refusal,
                Unregistrable::ConfirmProject(_) => Refusal::NeedsConfirmation,
            })
    }
    use std::os::unix::fs::symlink;

    struct Fixture {
        _outer: tempfile::TempDir,
        home: PathBuf,
        outside: PathBuf,
        boundary: Boundary,
    }

    fn fixture() -> Fixture {
        let outer = tempfile::tempdir().unwrap();
        let home = outer.path().join("home");
        let outside = outer.path().join("outside");
        fs::create_dir_all(home.join("projects/alpha")).unwrap();
        fs::create_dir_all(home.join("projects/Beta")).unwrap();
        fs::create_dir_all(home.join("projects/.hidden")).unwrap();
        fs::create_dir_all(outside.join("secret")).unwrap();
        fs::write(home.join("projects/notes.txt"), "x").unwrap();
        symlink(&outside, home.join("projects/escape")).unwrap();
        symlink(home.join("projects/alpha"), home.join("projects/alias")).unwrap();
        symlink(
            home.join("projects/notes.txt"),
            home.join("projects/filelink"),
        )
        .unwrap();
        let boundary = Boundary::new(&home).unwrap();
        Fixture {
            _outer: outer,
            home: boundary.home().to_path_buf(),
            outside,
            boundary,
        }
    }

    fn s(path: &Path) -> String {
        path.display().to_string()
    }

    #[test]
    fn a_checkout_listing_shows_files_and_hidden_names_but_never_git() {
        let f = fixture();
        let root = f.home.join("projects/alpha");
        f.boundary.set_roots(vec![Root {
            workspace_id: "w".to_owned(),
            checkout_id: "c".to_owned(),
            path: root.clone(),
        }]);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join(".config")).unwrap();
        fs::create_dir_all(root.join(".git/objects")).unwrap();
        fs::write(root.join("file2.txt"), "x").unwrap();
        fs::write(root.join("File10.txt"), "x").unwrap();
        fs::write(root.join(".env"), "x").unwrap();
        symlink(root.join("src"), root.join("alias")).unwrap();
        symlink(f.outside.join("secret"), root.join("escape")).unwrap();
        symlink(root.join("nope"), root.join("dangling")).unwrap();
        let listing = f.boundary.list_children(&root, &s(&root)).unwrap();
        let rows: Vec<(&str, bool)> = listing
            .entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.is_directory))
            .collect();
        assert_eq!(
            rows,
            vec![
                (".config", true),
                ("alias", true),
                ("src", true),
                (".env", false),
                ("file2.txt", false),
                ("File10.txt", false),
            ],
            "directories first and then the natural order; .git, the escaping \
             symlink and the dangling one are not rows"
        );
        assert!(!listing.truncated);
        assert_eq!(listing.root_path, s(&root));
        // A row's path keeps the root's own spelling, which is the spelling the
        // core compares the focused checkout against.
        assert_eq!(listing.entries[0].path, s(&root.join(".config")));
        // A folder under the root lists the same way one level down.
        let nested = f
            .boundary
            .list_children(&root, &s(&root.join("src")))
            .unwrap();
        assert!(nested.entries.is_empty());
        assert_eq!(nested.root_path, s(&root.join("src")));
    }

    #[test]
    fn a_checkout_listing_refuses_what_is_not_under_its_root() {
        let f = fixture();
        let root = f.home.join("projects/alpha");
        f.boundary.set_roots(vec![Root {
            workspace_id: "w".to_owned(),
            checkout_id: "c".to_owned(),
            path: root.clone(),
        }]);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("notes.txt"), "x").unwrap();
        // A sibling checkout is not under this root, and neither is home.
        for outside in [f.home.join("projects/Beta"), f.home.clone()] {
            assert_eq!(
                f.boundary.list_children(&root, &s(&outside)),
                Err(Refusal::OutsideCheckout),
                "{outside:?}"
            );
        }
        // A symlink inside the root to a directory outside it is refused
        // rather than read through.
        symlink(&f.outside, root.join("escape")).unwrap();
        assert_eq!(
            f.boundary.list_children(&root, &s(&root.join("escape"))),
            Err(Refusal::OutsideCheckout)
        );
        assert_eq!(
            f.boundary.list_children(&root, &s(&root.join("missing"))),
            Err(Refusal::NotFound)
        );
        assert_eq!(
            f.boundary.list_children(&root, &s(&root.join("notes.txt"))),
            Err(Refusal::NotADirectory)
        );
        assert_eq!(
            f.boundary
                .list_children(&root, &format!("{}/../Beta", s(&root))),
            Err(Refusal::InvalidPath)
        );
        assert!(
            f.boundary
                .list_children(&root, &s(&root.join("src")))
                .is_ok()
        );
    }

    #[test]
    fn a_symlink_out_of_home_is_refused_even_when_named_directly() {
        let f = fixture();
        let escape = s(&f.home.join("projects/escape"));
        assert_eq!(f.boundary.resolve_dir(&escape), Err(Refusal::OutsideHome));
        assert_eq!(
            workspace(&f.boundary, &escape, None),
            Err(Refusal::OutsideHome)
        );
        let deeper = s(&f.home.join("projects/escape/secret"));
        assert_eq!(
            workspace(&f.boundary, &deeper, None),
            Err(Refusal::OutsideHome)
        );
        // Past the escape the reason is the same whether the rest exists,
        // is a file, or is missing: the disk beyond home is not described.
        fs::write(f.outside.join("secret/marker.txt"), "x").unwrap();
        for tail in [
            "nope",
            "secret/nope",
            "secret/marker.txt",
            "secret/marker.txt/child",
        ] {
            let path = s(&f.home.join("projects/escape").join(tail));
            assert_eq!(
                workspace(&f.boundary, &path, None),
                Err(Refusal::OutsideHome),
                "{tail}"
            );
            assert_eq!(
                f.boundary.resolve_dir(&path),
                Err(Refusal::OutsideHome),
                "{tail}"
            );
        }
    }

    #[test]
    fn parent_segments_and_encoded_segments_do_not_leave_home() {
        let f = fixture();
        let dotdot = format!("{}/projects/../../outside", s(&f.home));
        assert_eq!(
            workspace(&f.boundary, &dotdot, None),
            Err(Refusal::InvalidPath)
        );
        let encoded = format!("{}/projects/%2e%2e/%2e%2e/outside", s(&f.home));
        assert_eq!(
            workspace(&f.boundary, &encoded, None),
            Err(Refusal::NotFound)
        );
        assert_eq!(
            workspace(&f.boundary, &s(&f.outside), None),
            Err(Refusal::NeedsConfirmation)
        );
        let sibling_prefix = format!("{}-extra", s(&f.home));
        fs::create_dir_all(&sibling_prefix).unwrap();
        assert_eq!(
            workspace(&f.boundary, &sibling_prefix, None),
            Err(Refusal::NeedsConfirmation),
            "a sibling whose name starts with the home path is outside"
        );
        // A confirmed path outside home is refused on shape like any other.
        let outside_dotdot = format!("{}/secret/../secret", s(&f.outside));
        assert_eq!(
            workspace(&f.boundary, &outside_dotdot, Some(&outside_dotdot)),
            Err(Refusal::InvalidPath)
        );
    }

    #[test]
    fn a_path_outside_home_gets_one_reason_whether_or_not_it_exists() {
        let f = fixture();
        let existing_dir = s(&f.outside.join("secret"));
        let missing_dir = s(&f.outside.join("nope"));
        let existing_file = s(&f.outside.join("secret/marker.txt"));
        fs::write(&existing_file, "x").unwrap();
        for path in [
            &existing_dir,
            &missing_dir,
            &existing_file,
            &"/etc/nope-zzz".to_owned(),
        ] {
            assert_eq!(
                workspace(&f.boundary, path, None),
                Err(Refusal::NeedsConfirmation),
                "{path}: the reason must not tell whether it exists"
            );
            assert_eq!(
                f.boundary.resolve_dir(path),
                Err(Refusal::OutsideHome),
                "{path}"
            );
        }
        // A symlink under home to a target outside home is refused the same
        // way whether its target exists or not, named directly or with a tail.
        symlink(&existing_dir, f.home.join("projects/out-exists")).unwrap();
        symlink(&missing_dir, f.home.join("projects/out-missing")).unwrap();
        symlink("../../outside/secret", f.home.join("projects/out-relative")).unwrap();
        symlink(
            "../../outside/nope",
            f.home.join("projects/out-relative-missing"),
        )
        .unwrap();
        for name in [
            "out-exists",
            "out-missing",
            "out-relative",
            "out-relative-missing",
        ] {
            for tail in ["", "child"] {
                let path = s(&f.home.join("projects").join(name).join(tail));
                assert_eq!(
                    workspace(&f.boundary, &path, None),
                    Err(Refusal::OutsideHome),
                    "{name}/{tail}"
                );
            }
        }
        // A symlink into home from outside is still outside: the path as
        // written decides, not where it leads.
        let inbound = f.outside.join("inbound");
        symlink(f.home.join("projects/alpha"), &inbound).unwrap();
        assert_eq!(
            workspace(&f.boundary, &s(&inbound), None),
            Err(Refusal::NeedsConfirmation)
        );
    }

    /// PRD wsl-device B15, B18, B20: an existing folder written outside home
    /// is registered only with the operator's confirmation of that same path,
    /// and nothing remembers it.
    #[test]
    fn a_folder_outside_home_is_registered_only_with_its_own_confirmation() {
        let f = fixture();
        let secret = s(&f.outside.join("secret"));
        let real = f.outside.join("secret").canonicalize().unwrap();
        assert_eq!(
            workspace(&f.boundary, &secret, Some(&secret)),
            Ok(real.clone())
        );
        // The confirmation is compared as a path, so a trailing slash on
        // either side names the same folder.
        assert_eq!(
            workspace(&f.boundary, &format!("{secret}/"), Some(&secret)),
            Ok(real)
        );
        // A confirmation of one folder does not carry to another, not even a
        // child or a parent of it.
        fs::create_dir_all(f.outside.join("secret/child")).unwrap();
        fs::create_dir_all(f.outside.join("other")).unwrap();
        for (path, confirmed) in [
            (s(&f.outside.join("other")), secret.clone()),
            (s(&f.outside.join("secret/child")), secret.clone()),
            (secret.clone(), s(&f.outside.join("secret/child"))),
            (secret.clone(), "relative".to_owned()),
            (secret.clone(), String::new()),
        ] {
            assert_eq!(
                workspace(&f.boundary, &path, Some(&confirmed)),
                Err(Refusal::NeedsConfirmation),
                "{path} confirmed as {confirmed}"
            );
        }
        // The boundary keeps no grant: the same folder asks again, which is
        // what a removed project's next add meets.
        assert_eq!(
            workspace(&f.boundary, &secret, None),
            Err(Refusal::NeedsConfirmation)
        );
    }

    /// PRD wsl-device B18: a confirmed folder is answered by what is there,
    /// the reasons a path under home gets, and confirming changes nothing
    /// about a symlink that leaves home, home itself or a folder holding it.
    #[test]
    fn a_confirmation_keeps_the_refusals_a_path_gets_today() {
        let f = fixture();
        let file = f.outside.join("secret/marker.txt");
        fs::write(&file, "x").unwrap();
        for (path, reason) in [
            (s(&f.outside.join("nope")), Refusal::NotFound),
            (s(&file), Refusal::NotADirectory),
            (s(&file.join("child")), Refusal::NotADirectory),
        ] {
            assert_eq!(
                workspace(&f.boundary, &path, Some(&path)),
                Err(reason),
                "{path}"
            );
        }
        let escape = s(&f.home.join("projects/escape/secret"));
        assert_eq!(
            workspace(&f.boundary, &escape, Some(&escape)),
            Err(Refusal::OutsideHome),
            "a symlink under home that leaves it is not confirmable"
        );
        let parent = f.home.parent().unwrap().to_path_buf();
        for holder in [s(&f.home), s(&parent), "/".to_owned()] {
            assert_eq!(
                workspace(&f.boundary, &holder, Some(&holder)),
                Err(Refusal::HomeRoot),
                "{holder}"
            );
        }
        // A confirmed outside path that leads to a folder holding home is
        // refused once it is read.
        let up = f.outside.join("up");
        symlink(&parent, &up).unwrap();
        assert_eq!(
            workspace(&f.boundary, &s(&up), Some(&s(&up))),
            Err(Refusal::HomeRoot)
        );
        // A path written outside home that leads back into it is registered
        // as the folder it leads to once confirmed.
        let inbound = f.outside.join("inbound");
        symlink(f.home.join("projects/alpha"), &inbound).unwrap();
        assert_eq!(
            workspace(&f.boundary, &s(&inbound), Some(&s(&inbound))),
            Ok(f.home.join("projects/alpha"))
        );
    }

    /// PRD wsl-device B19: a project registered outside home is a checkout
    /// root like any other; its files are read and nothing beside it is.
    #[test]
    fn a_checkout_outside_home_confines_its_files_like_one_inside() {
        let f = fixture();
        let root = f.outside.join("secret").canonicalize().unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}").unwrap();
        fs::create_dir_all(f.outside.join("beside")).unwrap();
        fs::write(f.outside.join("beside/key"), "k").unwrap();
        symlink(f.outside.join("beside"), root.join("leak")).unwrap();
        f.boundary.set_roots(vec![Root {
            workspace_id: "w".to_owned(),
            checkout_id: "c".to_owned(),
            path: root.clone(),
        }]);
        let main = s(&root.join("src/main.rs"));
        assert!(f.boundary.open_file(&main).is_ok());
        assert!(f.boundary.list_children(&root, &s(&root)).is_ok());
        let beside = f.outside.join("beside").canonicalize().unwrap();
        for path in [s(&root.join("leak/key")), s(&beside.join("key"))] {
            assert_eq!(
                f.boundary.resolve_target(&path),
                Err(Refusal::OutsideCheckout),
                "{path}"
            );
            assert!(f.boundary.open_file(&path).is_err(), "{path}");
        }
    }

    /// A repository at `main` with a linked worktree at `linked`, as Git
    /// writes the two (`hide_project` reads them without a git process).
    fn repository_with_worktree(main: &Path, linked: &Path) {
        let admin = main.join(".git/worktrees/linked");
        fs::create_dir_all(&admin).unwrap();
        fs::write(main.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::create_dir_all(linked).unwrap();
        fs::write(
            linked.join(".git"),
            format!("gitdir: {}\n", admin.display()),
        )
        .unwrap();
        fs::write(admin.join("HEAD"), "ref: refs/heads/feature\n").unwrap();
        fs::write(admin.join("commondir"), "../..\n").unwrap();
        fs::write(
            admin.join("gitdir"),
            format!("{}\n", linked.join(".git").display()),
        )
        .unwrap();
    }

    /// PRD wsl-device B15: a registration adds the project a folder belongs
    /// to, so a folder inside a repository outside home, or a linked worktree
    /// of one, is asked about once, by that repository's root; the
    /// confirmation of the folder alone never covers it, and the root
    /// confirmed is what is registered and whose files are read.
    #[test]
    fn a_folder_in_a_project_outside_home_asks_about_the_project_it_adds() {
        let f = fixture();
        let repo = f.outside.join("repo");
        repository_with_worktree(&repo, &f.outside.join("elsewhere/linked"));
        fs::create_dir_all(repo.join("public")).unwrap();
        fs::write(repo.join("private.key"), "k").unwrap();
        let root = repo.canonicalize().unwrap();
        let ask_root = Err(Unregistrable::ConfirmProject(root.clone()));

        let public = s(&repo.join("public"));
        for confirmed in [None, Some(public.as_str())] {
            assert_eq!(
                f.boundary.resolve_registration(&public, confirmed),
                ask_root,
                "asked once, about the project it adds"
            );
        }
        assert_eq!(
            f.boundary.resolve_registration(&public, Some(&s(&root))),
            Ok(root.join("public")),
            "the root's confirmation adds the folder's project"
        );
        let linked = s(&f.outside.join("elsewhere/linked"));
        assert_eq!(
            f.boundary.resolve_registration(&linked, Some(&linked)),
            ask_root,
            "a linked worktree adds its repository"
        );
        // A worktree under home whose repository is outside asks too, though
        // the folder itself needs no confirmation.
        let inward = f.home.join("projects/inward");
        let other = f.outside.join("other-repo");
        repository_with_worktree(&other, &inward);
        assert_eq!(
            f.boundary.resolve_registration(&s(&inward), None),
            Err(Unregistrable::ConfirmProject(other.canonicalize().unwrap()))
        );

        let root_text = s(&root);
        assert_eq!(
            f.boundary
                .resolve_registration(&root_text, Some(&root_text)),
            Ok(root.clone())
        );
        // What a folder under home belongs to under home asks nothing more.
        assert_eq!(
            f.boundary
                .resolve_registration(&s(&f.home.join("projects/alpha")), None),
            Ok(f.home.join("projects/alpha"))
        );
        f.boundary.set_roots(vec![Root {
            workspace_id: "w".to_owned(),
            checkout_id: "c".to_owned(),
            path: root.clone(),
        }]);
        assert!(f.boundary.open_file(&s(&root.join("private.key"))).is_ok());
        let beside = f.outside.join("secret").canonicalize().unwrap();
        assert_eq!(
            f.boundary.resolve_target(&s(&beside)),
            Err(Refusal::OutsideCheckout)
        );
    }

    #[test]
    fn home_spelled_as_given_resolves_when_home_is_a_symlink() {
        let outer = tempfile::tempdir().unwrap();
        let real_home = outer.path().join("real-home");
        fs::create_dir_all(real_home.join("projects/alpha")).unwrap();
        let link_home = outer.path().join("link-home");
        symlink(&real_home, &link_home).unwrap();
        let boundary = Boundary::new(&link_home).unwrap();
        let canonical = real_home.canonicalize().unwrap();
        assert_eq!(boundary.home(), canonical);
        assert_eq!(
            workspace(&boundary, &s(&link_home.join("projects/alpha")), None),
            Ok(canonical.join("projects/alpha")),
            "the spelling HOME uses is under home"
        );
        assert_eq!(
            workspace(&boundary, &s(&canonical.join("projects/alpha")), None),
            Ok(canonical.join("projects/alpha"))
        );
        assert_eq!(boundary.resolve_dir(&s(&link_home)), Ok(canonical));
    }

    #[test]
    fn files_hidden_dirs_home_itself_and_bad_shapes() {
        let f = fixture();
        let file = s(&f.home.join("projects/notes.txt"));
        assert_eq!(
            workspace(&f.boundary, &file, None),
            Err(Refusal::NotADirectory)
        );
        assert_eq!(f.boundary.resolve_dir(&file), Err(Refusal::NotADirectory));
        let filelink = s(&f.home.join("projects/filelink"));
        assert_eq!(
            workspace(&f.boundary, &filelink, None),
            Err(Refusal::NotADirectory)
        );
        let under_file = s(&f.home.join("projects/notes.txt/child"));
        assert!(matches!(
            workspace(&f.boundary, &under_file, None),
            Err(Refusal::NotFound | Refusal::NotADirectory)
        ));
        let missing = s(&f.home.join("projects/nope"));
        assert_eq!(
            workspace(&f.boundary, &missing, None),
            Err(Refusal::NotFound)
        );
        assert_eq!(
            workspace(&f.boundary, &s(&f.home), None),
            Err(Refusal::HomeRoot)
        );
        assert_eq!(f.boundary.resolve_dir(&s(&f.home)), Ok(f.home.clone()));
        assert_eq!(f.boundary.resolve_dir("~"), Ok(f.home.clone()));
        assert_eq!(
            workspace(&f.boundary, "~/projects/alpha", None).unwrap(),
            f.home.join("projects/alpha")
        );
        assert_eq!(workspace(&f.boundary, "~", None), Err(Refusal::HomeRoot));
        assert_eq!(
            workspace(&f.boundary, "~user/x", None),
            Err(Refusal::InvalidPath)
        );
        assert_eq!(workspace(&f.boundary, "", None), Err(Refusal::InvalidPath));
        assert_eq!(
            workspace(&f.boundary, "projects", None),
            Err(Refusal::InvalidPath)
        );
        assert_eq!(
            workspace(
                &f.boundary,
                &format!("{}\0", s(&f.home.join("projects"))),
                None
            ),
            Err(Refusal::InvalidPath)
        );
        // A hidden directory may be added like any other.
        let hidden = s(&f.home.join("projects/.hidden"));
        assert!(workspace(&f.boundary, &hidden, None).is_ok());
    }

    #[test]
    fn symlinks_inside_home_resolve_and_loops_are_refused() {
        let f = fixture();
        symlink("alpha", f.home.join("projects/relative-alias")).unwrap();
        assert_eq!(
            workspace(
                &f.boundary,
                &s(&f.home.join("projects/relative-alias")),
                None
            ),
            Ok(f.home.join("projects/alpha"))
        );
        symlink("../projects/alpha", f.home.join("projects/dotdot-alias")).unwrap();
        assert_eq!(
            workspace(&f.boundary, &s(&f.home.join("projects/dotdot-alias")), None),
            Ok(f.home.join("projects/alpha"))
        );
        symlink("loop-b", f.home.join("projects/loop-a")).unwrap();
        symlink("loop-a", f.home.join("projects/loop-b")).unwrap();
        assert_eq!(
            workspace(&f.boundary, &s(&f.home.join("projects/loop-a")), None),
            Err(Refusal::InvalidPath)
        );
        symlink("nowhere", f.home.join("projects/dangling-inside")).unwrap();
        assert_eq!(
            workspace(
                &f.boundary,
                &s(&f.home.join("projects/dangling-inside")),
                None
            ),
            Err(Refusal::NotFound),
            "a dangling target under home is a missing path under home"
        );
    }

    #[test]
    fn an_accepted_path_is_the_canonical_one() {
        let f = fixture();
        let alias = s(&f.home.join("projects/alias"));
        assert_eq!(
            workspace(&f.boundary, &alias, None).unwrap(),
            f.home.join("projects/alpha")
        );
        let trailing = format!("{}/", s(&f.home.join("projects/alpha")));
        assert_eq!(
            workspace(&f.boundary, &trailing, None).unwrap(),
            f.home.join("projects/alpha")
        );
    }

    /// A device watch frame is admitted only for a folder under a root the
    /// catalog carries for that same device (S5.5 B2, B43).
    #[test]
    fn a_device_folder_is_under_only_its_own_devices_roots() {
        let home = tempfile::tempdir().unwrap();
        let boundary = Boundary::new(home.path()).unwrap();
        boundary.set_device_roots(vec![DeviceRoot {
            device_id: "mac".to_owned(),
            path: "/r".to_owned(),
        }]);
        assert!(boundary.is_under_device_root("mac", "/r"));
        assert!(boundary.is_under_device_root("mac", "/r/src"));
        assert!(!boundary.is_under_device_root("mac", "/rx/src"));
        assert!(!boundary.is_under_device_root("other", "/r/src"));
    }

    #[test]
    fn a_missing_home_refuses_to_boot() {
        assert!(Boundary::new(Path::new("/nonexistent/home/for/hided")).is_err());
    }

    fn root(workspace_id: &str, checkout_id: &str, path: &Path) -> Root {
        Root {
            workspace_id: workspace_id.to_owned(),
            checkout_id: checkout_id.to_owned(),
            path: path.to_path_buf(),
        }
    }

    /// The fixture with two registered checkouts: alpha, and alpha/nested
    /// inside it, so a path under the nested one has two candidate owners.
    fn rooted() -> Fixture {
        let f = fixture();
        let repo = f.home.join("projects/alpha");
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::create_dir_all(repo.join("nested/deep")).unwrap();
        fs::write(repo.join("src/main.rs"), "fn main() {}").unwrap();
        f.boundary.set_roots(vec![
            root("w1", "c1", &repo),
            root("w1", "c2", &repo.join("nested")),
        ]);
        f
    }

    #[test]
    fn a_path_under_a_root_resolves_in_the_root_spelling() {
        let f = rooted();
        let repo = f.home.join("projects/alpha");
        assert_eq!(f.boundary.resolve_target(&s(&repo)).unwrap(), repo);
        assert_eq!(
            f.boundary
                .resolve_target(&s(&repo.join("src/main.rs")))
                .unwrap(),
            repo.join("src/main.rs")
        );
        assert_eq!(
            f.boundary
                .resolve_target(&format!("{}/", s(&repo.join("src"))))
                .unwrap(),
            repo.join("src"),
            "the spelling that was checked is the one forwarded"
        );
        assert!(f.boundary.is_under_root(&s(&repo.join("src/main.rs"))));
        assert!(f.boundary.is_under_root(&s(&repo.join("src/deleted.rs"))));
        assert!(
            !f.boundary
                .is_under_root(&s(&repo.join("missing/deleted.rs")))
        );
    }

    #[test]
    fn a_file_under_a_root_resolves_with_its_size_and_a_directory_does_not() {
        let f = rooted();
        let repo = f.home.join("projects/alpha");
        let (path, size) = f
            .boundary
            .resolve_file(&s(&repo.join("src/main.rs")))
            .unwrap();
        assert_eq!(path, repo.join("src/main.rs"));
        assert_eq!(size, "fn main() {}".len() as u64);
        assert_eq!(
            f.boundary.resolve_file(&s(&repo.join("src"))),
            Err(Refusal::NotAFile),
            "a directory is not a byte source"
        );
        assert_eq!(
            f.boundary.resolve_file(&s(&repo.join("src/nope"))),
            Err(Refusal::NotFound)
        );
        assert_eq!(
            f.boundary.resolve_file(&s(&f.outside.join("secret"))),
            Err(Refusal::OutsideCheckout),
            "the file line is the checkout line, not the home one"
        );
    }

    #[cfg(unix)]
    #[test]
    fn opened_byte_source_keeps_the_checked_file_after_a_symlink_swap() {
        use std::io::Read;
        use std::os::unix::fs::symlink;

        let f = rooted();
        let repo = f.home.join("projects/alpha");
        let path = repo.join("src/swap.txt");
        fs::write(&path, b"checkout bytes").unwrap();
        let (_, mut opened, _) = f.boundary.open_file(&s(&path)).unwrap();
        fs::remove_file(&path).unwrap();
        fs::write(f.outside.join("secret/outside.txt"), b"outside secret").unwrap();
        symlink(f.outside.join("secret/outside.txt"), &path).unwrap();
        let mut bytes = Vec::new();
        opened.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"checkout bytes");
        assert!(matches!(
            f.boundary.open_file(&s(&path)),
            Err(Refusal::OutsideCheckout)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn a_replaced_checkout_root_cannot_authorize_an_outside_byte_source() {
        use std::os::unix::fs::symlink;

        let f = rooted();
        let repo = f.home.join("projects/alpha");
        fs::write(f.outside.join("secret/outside.txt"), b"outside secret").unwrap();
        fs::rename(&repo, f.home.join("projects/alpha-old")).unwrap();
        symlink(f.outside.join("secret"), &repo).unwrap();
        let unchanged_snapshot = f
            .boundary
            .roots_for_read()
            .iter()
            .map(|root| root.source.clone())
            .collect();
        f.boundary.set_roots(unchanged_snapshot);
        assert!(matches!(
            f.boundary.open_file(&s(&repo.join("outside.txt"))),
            Err(Refusal::OutsideCheckout)
        ));
        assert_eq!(
            f.boundary.list_children(&repo, &s(&repo)),
            Err(Refusal::OutsideCheckout),
            "a replaced root cannot reveal outside child names"
        );
        assert_eq!(
            f.boundary
                .resolve_below(&repo, &s(&repo.join("outside.txt"))),
            Err(Refusal::OutsideCheckout),
            "mutations cannot use the replaced root"
        );
        assert!(!f.boundary.is_under_root(&s(&repo.join("outside.txt"))));
        assert_eq!(f.boundary.known_root(&s(&repo)), None);
        fs::remove_file(&repo).unwrap();
        fs::create_dir(&repo).unwrap();
        fs::write(repo.join("replacement.txt"), "new directory").unwrap();
        assert_eq!(
            f.boundary.list_children(&repo, &s(&repo)),
            Err(Refusal::OutsideCheckout),
            "a new directory at the same spelling is not the registered inode"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_byte_source_is_refused_without_waiting_for_a_writer() {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        use std::sync::mpsc;
        use std::time::Duration;

        let f = rooted();
        let path = f.home.join("projects/alpha/pipe");
        let name = CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let raw = s(&path);
        let (sender, receiver) = mpsc::channel();
        let work = std::thread::spawn(move || {
            let result = f.boundary.open_file(&raw).map(|_| ());
            sender.send(result).unwrap();
        });
        let result = receiver.recv_timeout(Duration::from_secs(2));
        if result.is_err() {
            // If the assertion catches a blocking open, let its reader exit
            // before failing the test rather than stranding a test child.
            let _ = fs::OpenOptions::new().read(true).write(true).open(&path);
        }
        work.join().unwrap();
        assert_eq!(result.unwrap(), Err(Refusal::NotAFile));
    }

    #[cfg(unix)]
    #[test]
    fn listing_during_root_replacement_never_returns_outside_names() {
        use std::sync::Barrier;

        let f = rooted();
        let repo = f.home.join("projects/alpha");
        let moved = f.home.join("projects/alpha-moved");
        fs::write(repo.join("inside.txt"), "inside").unwrap();
        fs::write(f.outside.join("secret/outside-secret.txt"), "secret").unwrap();
        let start = Barrier::new(2);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                start.wait();
                for _ in 0..100 {
                    fs::rename(&repo, &moved).unwrap();
                    symlink(f.outside.join("secret"), &repo).unwrap();
                    fs::remove_file(&repo).unwrap();
                    fs::rename(&moved, &repo).unwrap();
                }
            });
            start.wait();
            for _ in 0..1000 {
                if let Ok(listing) = f.boundary.list_children(&repo, &s(&repo)) {
                    assert!(
                        listing
                            .entries
                            .iter()
                            .all(|entry| entry.name != "outside-secret.txt"),
                        "an opened listing must stay within its registered checkout"
                    );
                }
            }
        });
    }

    #[test]
    fn a_path_outside_every_root_is_refused_as_outside_checkout() {
        let f = rooted();
        let repo = f.home.join("projects/alpha");
        fs::create_dir_all(f.home.join("projects/alpha-other")).unwrap();
        for path in [
            s(&f.home),
            s(&f.home.join("projects")),
            s(&f.home.join("projects/notes.txt")),
            s(&f.home.join("projects/alpha-other")),
            s(&f.outside),
            "/etc/nope-zzz".to_owned(),
        ] {
            assert_eq!(
                f.boundary.resolve_target(&path),
                Err(Refusal::OutsideCheckout),
                "{path}: a path under home that is not under a checkout is not this line's"
            );
            assert!(!f.boundary.is_under_root(&path), "{path}");
        }
        assert_eq!(
            f.boundary.resolve_target(&s(&repo.join("src/nope"))),
            Err(Refusal::NotFound),
            "inside a root, a missing name is missing rather than outside"
        );
    }

    #[test]
    fn parent_segments_never_leave_a_root() {
        let f = rooted();
        let repo = f.home.join("projects/alpha");
        for shape in [
            format!("{}/src/../nested", s(&repo)),
            format!("{}/../../outside", s(&repo)),
        ] {
            assert_eq!(
                f.boundary.resolve_target(&shape),
                Err(Refusal::InvalidPath),
                "{shape}"
            );
        }
        // A `.` component is not a way out of a root and not a reason to
        // refuse one either: the parser drops it, in this daemon and in the
        // core's own `path_inside_root`.
        assert_eq!(
            f.boundary.resolve_target(&format!("{}/./src", s(&repo))),
            Ok(repo.join("src"))
        );
    }

    #[test]
    fn a_symlink_out_of_a_root_is_refused_before_its_target_is_read() {
        let f = rooted();
        let repo = f.home.join("projects/alpha");
        symlink(&f.outside, repo.join("escape")).unwrap();
        symlink("../../outside/nope", repo.join("escape-missing")).unwrap();
        symlink("src", repo.join("alias")).unwrap();
        assert_eq!(
            f.boundary.resolve_target(&s(&repo.join("alias"))).unwrap(),
            repo.join("src"),
            "a link that stays inside the root resolves"
        );
        for name in ["escape", "escape-missing"] {
            for tail in ["", "secret", "secret/marker.txt"] {
                let path = s(&repo.join(name).join(tail));
                assert_eq!(
                    f.boundary.resolve_target(&path),
                    Err(Refusal::OutsideCheckout),
                    "{name}/{tail}: the reason is the same whether the target exists"
                );
            }
        }
        // A link that leaves the nested checkout but stays inside the outer
        // one belongs to the nested checkout, which owns the path it sits in.
        fs::create_dir_all(repo.join("other")).unwrap();
        symlink("../other", repo.join("nested/out-of-nested")).unwrap();
        assert_eq!(
            f.boundary
                .resolve_target(&s(&repo.join("nested/out-of-nested"))),
            Err(Refusal::OutsideCheckout)
        );
        f.boundary.set_roots(vec![root("w1", "c1", &repo)]);
        assert_eq!(
            f.boundary
                .resolve_target(&s(&repo.join("nested/out-of-nested")))
                .unwrap(),
            repo.join("other"),
            "with only the outer root registered, the same link resolves"
        );
    }

    #[test]
    fn a_named_root_has_to_be_one_of_the_registered_ones() {
        let f = rooted();
        let repo = f.home.join("projects/alpha");
        let nested = repo.join("nested");
        assert_eq!(f.boundary.known_root(&s(&repo)), Some(repo.clone()));
        assert_eq!(f.boundary.known_root(&s(&nested)), Some(nested.clone()));
        assert_eq!(
            f.boundary
                .resolve_in_root(&s(&repo), &s(&nested.join("deep")))
                .unwrap(),
            nested.join("deep"),
            "an outer root still owns a nested path"
        );
        assert_eq!(
            f.boundary
                .resolve_in_root(&s(&nested), &s(&nested.join("deep")))
                .unwrap(),
            nested.join("deep")
        );
        assert_eq!(
            f.boundary
                .resolve_in_root(&s(&f.home.join("projects")), &s(&nested.join("deep"))),
            Err(Refusal::OutsideCheckout),
            "a root the event named has to be registered"
        );
        assert_eq!(
            f.boundary
                .resolve_in_root(&s(&repo), &s(&f.home.join("projects/notes.txt"))),
            Err(Refusal::OutsideCheckout)
        );
    }

    #[test]
    fn the_root_set_is_replaced_wholesale() {
        let f = rooted();
        let repo = f.home.join("projects/alpha");
        f.boundary
            .set_roots(vec![root("w9", "c9", &f.home.join("projects/ghost"))]);
        assert_eq!(
            f.boundary.known_root(&s(&repo)),
            None,
            "a root that is not an existing directory is dropped"
        );
        assert_eq!(
            f.boundary.resolve_target(&s(&repo.join("src/main.rs"))),
            Err(Refusal::OutsideCheckout)
        );
        f.boundary.set_roots(Vec::new());
        assert_eq!(
            f.boundary.resolve_target(&s(&repo)),
            Err(Refusal::OutsideCheckout)
        );
    }

    #[test]
    fn a_checkout_pair_owns_the_paths_under_its_root() {
        let f = rooted();
        let repo = f.home.join("projects/alpha");
        let nested = repo.join("nested");
        assert_eq!(
            f.boundary
                .resolve_checkout("w1", "c2", &s(&nested.join("deep")))
                .unwrap(),
            nested.join("deep")
        );
        assert_eq!(
            f.boundary
                .resolve_checkout("w1", "c2", &s(&repo.join("src/main.rs"))),
            Err(Refusal::OutsideCheckout),
            "the named checkout does not own a path outside its root"
        );
        assert_eq!(
            f.boundary
                .resolve_checkout("w1", "c1", &s(&nested.join("deep")))
                .unwrap(),
            nested.join("deep"),
            "the outer checkout owns the nested path too"
        );
        assert_eq!(
            f.boundary
                .resolve_checkout("w9", "c9", &s(&repo.join("src/main.rs")))
                .unwrap(),
            repo.join("src/main.rs"),
            "a pair with no root falls back to any root; the core names the unknown checkout"
        );
        assert_eq!(
            f.boundary
                .resolve_checkout("w9", "c9", &s(&f.home.join("projects/notes.txt"))),
            Err(Refusal::OutsideCheckout)
        );
    }

    #[test]
    fn a_save_is_checked_without_being_respelled() {
        let f = rooted();
        let repo = f.home.join("projects/alpha");
        assert!(f.boundary.is_under_root(&s(&repo)));
        assert!(f.boundary.is_under_root(&s(&repo.join("src/main.rs"))));
        assert!(
            !f.boundary
                .is_under_root(&s(&f.home.join("projects/notes.txt")))
        );
        assert!(
            !f.boundary.is_under_root(&s(&repo.join("src/../main.rs"))),
            "a path that names its way around is not a path this line reads"
        );
        assert!(!f.boundary.is_under_root("/etc/passwd"));
        assert!(!f.boundary.is_under_root("relative"));
        assert!(!f.boundary.is_under_root(""));
        symlink(f.outside.join("secret"), repo.join("save-escape")).unwrap();
        assert!(!f.boundary.is_under_root(&s(&repo.join("save-escape"))));
    }

    #[test]
    fn a_home_relative_path_is_the_home_directory_on_both_lines() {
        let f = rooted();
        let repo = f.home.join("projects/alpha");
        assert_eq!(f.boundary.resolve_target("~/projects/alpha").unwrap(), repo);
        assert_eq!(
            f.boundary
                .resolve_target("~/projects/alpha/src/main.rs")
                .unwrap(),
            repo.join("src/main.rs")
        );
        assert_eq!(
            f.boundary.resolve_target("~"),
            Err(Refusal::OutsideCheckout),
            "home is not a checkout unless one is registered at it"
        );
        assert_eq!(f.boundary.resolve_dir("~").unwrap(), f.home);
        assert_eq!(
            f.boundary.resolve_target("~user/projects/alpha"),
            Err(Refusal::InvalidPath)
        );
    }

    #[test]
    fn names_are_gated_by_shape() {
        for name in ["main.rs", "a b", ".hidden", "한글.txt"] {
            assert!(valid_name(name), "{name}");
        }
        for name in ["", ".", "..", "a/b", "\0"] {
            assert!(!valid_name(name), "{name:?}");
        }
    }

    #[test]
    fn a_clone_target_is_a_free_plain_name_in_a_folder_under_home() {
        let f = fixture();
        let projects = f.home.join("projects");
        let real = projects.canonicalize().unwrap();
        assert_eq!(
            f.boundary.resolve_clone_target(&s(&projects), "gamma"),
            Ok(real.clone())
        );
        assert_eq!(
            f.boundary.resolve_clone_target("~/projects", "gamma"),
            Ok(real)
        );
        // Home itself is a parent a clone may use; the folder is below it.
        assert_eq!(
            f.boundary.resolve_clone_target("~", "gamma"),
            Ok(f.home.canonicalize().unwrap())
        );
        assert_eq!(
            f.boundary.resolve_clone_target(&s(&projects), "alpha"),
            Err(Refusal::AlreadyExists)
        );
        symlink(f.outside.join("nowhere"), projects.join("dangling")).unwrap();
        assert_eq!(
            f.boundary.resolve_clone_target(&s(&projects), "dangling"),
            Err(Refusal::AlreadyExists)
        );
        for name in ["", ".", "..", "a/b", "../escape", "a\0b"] {
            assert_eq!(
                f.boundary.resolve_clone_target(&s(&projects), name),
                Err(Refusal::InvalidPath),
                "{name:?}"
            );
        }
        assert_eq!(
            f.boundary.resolve_clone_target(&s(&f.outside), "gamma"),
            Err(Refusal::OutsideHome)
        );
        assert_eq!(
            f.boundary
                .resolve_clone_target(&s(&f.home.join("missing")), "gamma"),
            Err(Refusal::NotFound)
        );
    }
}

#[cfg(all(test, windows))]
mod windows_boundary_tests {
    use super::*;

    #[test]
    fn retained_root_identity_pins_allow_cleanup_and_refuse_a_replacement() {
        let sandbox = tempfile::tempdir().unwrap();
        let home = sandbox.path().join("home");
        let root = home.join("checkout");
        let moved = home.join("removed");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("inside.txt"), "inside").unwrap();
        let boundary = Boundary::new(&home).unwrap();
        boundary.set_roots(vec![Root {
            workspace_id: "w".to_owned(),
            checkout_id: "c".to_owned(),
            path: root.clone(),
        }]);
        let opened = boundary.opened_roots();
        assert_eq!(opened.len(), 1);
        let retained = hide_node::hold_roots(opened);
        fs::rename(&root, &moved).unwrap();
        fs::remove_dir_all(&moved).unwrap();
        assert!(!root.exists());
        assert!(!moved.exists());

        fs::create_dir(&root).unwrap();
        fs::write(root.join("outside.txt"), "outside").unwrap();
        assert!(boundary.known_root(&root.display().to_string()).is_none());
        assert!(boundary.opened_roots().is_empty());
        assert!(
            boundary
                .open_file(&root.join("outside.txt").display().to_string())
                .is_err()
        );
        drop(retained);
    }

    #[test]
    fn opened_directory_rows_and_root_identity_follow_the_handle() {
        let sandbox = tempfile::tempdir().unwrap();
        let home = sandbox.path().join("home");
        let root = home.join("checkout");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("inside.txt"), "inside").unwrap();
        let boundary = Boundary::new(&home).unwrap();
        boundary.set_roots(vec![Root {
            workspace_id: "w".to_owned(),
            checkout_id: "c".to_owned(),
            path: root.clone(),
        }]);
        let (_, opened) = boundary
            .open_directory(&root, &root.display().to_string())
            .unwrap();
        assert_eq!(
            root_identity(&opened),
            root_identity(&open_nonblocking(&root, true).unwrap())
        );
        assert!(
            fs::rename(&root, home.join("moved")).is_err(),
            "an opened root denies delete sharing"
        );
        drop(opened);
        fs::rename(&root, home.join("moved")).unwrap();
        fs::create_dir(&root).unwrap();
        assert!(boundary.known_root(&root.display().to_string()).is_none());
    }
}

#[cfg(test)]
mod identity_tests {
    use super::{Boundary, Root};

    /// The boundary pins a local root by the identity it read when the root
    /// was registered, and the node refuses a root whose identity differs, so
    /// on every system both must read one directory as the same identity or
    /// every local read would be refused as replaced.
    #[test]
    fn root_identity_is_the_one_the_node_reads() {
        let sandbox = tempfile::tempdir().unwrap();
        let home = sandbox.path().join("home");
        std::fs::create_dir_all(home.join("checkout")).unwrap();
        let boundary = Boundary::new(&home).unwrap();
        let root = boundary.home().join("checkout");
        boundary.set_roots(vec![Root {
            workspace_id: "w".to_owned(),
            checkout_id: "c".to_owned(),
            path: root.clone(),
        }]);

        let pinned = boundary
            .root_identity(&root.to_string_lossy())
            .expect("a registered root has an identity");
        let opened = hide_host::root::Root::open_pinned(&root, pinned)
            .expect("the node opens the root the boundary pinned");
        assert_eq!(opened.identity(), pinned);
    }
}

#[cfg(test)]
mod wire_tests {
    use super::Entry;

    /// A browser reads a JSON number as a double, which rounds an id past
    /// 2^53 (an NTFS file id once its sequence number reaches 32), and a
    /// trash that sends the rounded id back is refused as a change.
    #[test]
    fn an_inode_past_what_a_browser_number_holds_reaches_the_shell_exact() {
        let inode = (1_u64 << 53) + 1;
        let entry = Entry {
            name: "dest".to_owned(),
            path: "/checkout/dest".to_owned(),
            is_directory: true,
            inode: Some(inode),
        };
        let sent = serde_json::to_value(&entry).unwrap();
        assert_eq!(sent["inode"], serde_json::json!("9007199254740993"));
    }
}
