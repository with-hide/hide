//! Files and folders only the current account can use, and the checks that
//! tell whether a folder can be trusted with code the account runs.
//!
//! "Private" is mode 0600 (0700 for a folder) on Unix. On Windows it is an
//! access list that names only the current account and does not inherit, set
//! when the file is created so there is no moment it is wider.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use super::Handle;

/// Makes the folder `path`, private. The parent must exist and `path` must
/// not (`AlreadyExists` otherwise), so the caller learns whether the folder
/// it is about to trust is one it made.
pub fn create_dir(path: &Path) -> io::Result<()> {
    sys::create_dir(path)
}

/// Makes the folder `path` and any missing folder above it, each private, the
/// way `fs::create_dir_all` makes them open. A folder that is already there
/// is left as it is, so the caller checks one it did not make before
/// trusting it.
pub fn create_dir_all(path: &Path) -> io::Result<()> {
    if path.is_dir() {
        return Ok(());
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        create_dir_all(parent)?;
    }
    match sys::create_dir(path) {
        // Another process made it between the look and the make.
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists && path.is_dir() => Ok(()),
        made => made,
    }
}

/// What [`own_dir`] found at the folder it answered for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OwnDir {
    /// It was missing; it and every missing folder above it are made private.
    Made,
    /// It was already a private folder of this account.
    Private,
    /// It was this account's and others could reach it; it is private now.
    Restricted,
}

/// Makes `path` a private folder of this account, whatever an earlier
/// program left there: a missing one is made as [`create_dir_all`] makes it,
/// and an existing folder this account owns that others can reach, such as
/// one `fs::create_dir_all` made under the process's umask, is restricted to
/// its owner. A link, anything but a folder, and a folder another account
/// owns are refused (`PermissionDenied`, naming the path) and left as they
/// are, never followed or changed. The folder is opened as the entry itself
/// and judged and changed through that handle, so a link swapped in at the
/// name after the look cannot redirect the change. Only the last name is
/// judged: the folders above it are the caller's to trust.
pub fn own_dir(path: &Path) -> io::Result<OwnDir> {
    // Every failure names the folder, so a caller can show it as it is.
    let at =
        |error: io::Error| io::Error::new(error.kind(), format!("{}: {error}", path.display()));
    let made = match fs::symlink_metadata(path) {
        Ok(_) => false,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            create_dir_all(path).map_err(at)?;
            true
        }
        Err(error) => return Err(at(error)),
    };
    let refused = |why: &str| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} {why}", path.display()),
        )
    };
    // Looked at again when it was just made: another process may have made
    // it first, the way an earlier build did.
    let metadata = fs::symlink_metadata(path).map_err(at)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(refused("is a link or not a folder"));
    }
    let folder = match sys::open_entry_dir(path) {
        Ok(folder) => folder,
        // Swapped for a link or a file since the look above.
        Err(error) if error.kind() == io::ErrorKind::InvalidInput => {
            return Err(refused("is a link or not a folder"));
        }
        // A folder this account may not open: another account's, or its
        // own closed even to it, which only its owner can open again.
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            return Err(match owned_by_current_user(path) {
                Ok(false) => refused("belongs to another account"),
                Ok(true) => refused(sys::CLOSED_TO_ITS_OWNER),
                Err(_) => refused("cannot be read by this account"),
            });
        }
        Err(error) => return Err(at(error)),
    };
    if !handle_owned_by_current_user(&folder).map_err(at)? {
        return Err(refused("belongs to another account"));
    }
    if !sys::handle_access_of_others(&folder, false).map_err(at)? {
        return Ok(if made { OwnDir::Made } else { OwnDir::Private });
    }
    sys::restrict_dir_handle(&folder).map_err(at)?;
    if sys::handle_access_of_others(&folder, false).map_err(at)? {
        return Err(refused("could not be made private"));
    }
    Ok(OwnDir::Restricted)
}

/// A bootstrap failure, with the last directory whose entries were synced.
/// An uncertain component is retained, including one this call created;
/// recovery revalidates and syncs the chain rather than assuming rollback.
#[derive(Debug)]
pub enum DirectoryDurabilityError {
    /// Validation failed before any directory creation was attempted.
    BeforeCreate { source: io::Error },
    /// A component was attempted but its durability was not established.
    Uncertain {
        completed: PathBuf,
        path: PathBuf,
        source: io::Error,
    },
}

impl std::fmt::Display for DirectoryDurabilityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BeforeCreate { source } => {
                write!(formatter, "before directory creation: {source}")
            }
            Self::Uncertain {
                completed,
                path,
                source,
            } => write!(
                formatter,
                "directory durability unconfirmed at {} after {}: {source}",
                path.display(),
                completed.display(),
            ),
        }
    }
}

impl std::error::Error for DirectoryDurabilityError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(match self {
            Self::BeforeCreate { source } | Self::Uncertain { source, .. } => source,
        })
    }
}

/// Establishes an existing directory's ancestor barriers, returning its
/// canonical spelling for use as a [`create_dir_all_durable`] anchor.
/// The leaf must be a real directory owned by this account that other
/// accounts cannot modify. The caller must trust and keep its ancestor
/// namespace stable; system aliases in that namespace are canonicalized.
/// Shared system ancestors are not made private or treated as account-owned.
///
/// The existing chain is synced from the filesystem root ([`sync_dir`] on
/// each folder), with at most 64 components below the root. No directory is
/// created or removed, including on failure. On Windows an ancestor is
/// flushed only for its entry on the chain, and only when that entry is
/// this account's and the account owns or may add an entry to the ancestor
/// (`sys::flushes_entry`): `C:\` is passed over for its `Users`, which
/// the system made, and `C:\Users` for a standard account, which may not
/// flush it. The leaf is always flushed, and a failed flush is an error.
///
/// [`sync_dir`]: super::atomic::sync_dir
pub fn establish_dir_durability(path: &Path) -> Result<PathBuf, DirectoryDurabilityError> {
    let before = |source| DirectoryDurabilityError::BeforeCreate { source };
    let leaf = trusted_dir(path).map_err(before)?;
    let canonical = fs::canonicalize(path).map_err(before)?;
    // `/` on Unix; the volume and its root, `\\?\C:\`, on Windows.
    let root: PathBuf = canonical
        .components()
        .take_while(|part| matches!(part, Component::Prefix(_) | Component::RootDir))
        .collect();
    let parts: Vec<_> = canonical
        .components()
        .skip_while(|part| matches!(part, Component::Prefix(_) | Component::RootDir))
        .collect();
    if parts.len() > 64 {
        return Err(before(io::Error::new(
            io::ErrorKind::InvalidInput,
            "durable directory chain exceeds 64 components",
        )));
    }
    let canonical_leaf = trusted_dir(&canonical).map_err(before)?;
    if super::identity::file_id_of(&leaf).map_err(before)?
        != super::identity::file_id_of(&canonical_leaf).map_err(before)?
    {
        return Err(before(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "directory identity changed during validation",
        )));
    }
    let mut completed = root;
    let mut parent = super::open_dir(&completed).map_err(before)?;
    chain_barrier(&parent, !parts.is_empty()).map_err(|source| {
        DirectoryDurabilityError::Uncertain {
            completed: completed.clone(),
            path: completed.clone(),
            source,
        }
    })?;
    for (index, part) in parts.iter().enumerate() {
        let child_path = completed.join(part.as_os_str());
        let step: io::Result<fs::File> = (|| {
            let child = super::open_dir(&child_path)?;
            #[cfg(test)]
            bootstrap_faults::fail(bootstrap_faults::Point::ChildSync)?;
            chain_barrier(&child, index + 1 < parts.len())?;
            #[cfg(test)]
            bootstrap_faults::fail(bootstrap_faults::Point::ParentSync)?;
            entry_barrier(&parent, &child)?;
            Ok(child)
        })();
        parent = step.map_err(|source| DirectoryDurabilityError::Uncertain {
            completed: completed.clone(),
            path: child_path.clone(),
            source,
        })?;
        completed = child_path;
    }
    let changed = |source| DirectoryDurabilityError::Uncertain {
        completed: completed.clone(),
        path: canonical.clone(),
        source,
    };
    if super::identity::file_id_of(&parent).map_err(changed)?
        != super::identity::file_id_of(&leaf).map_err(changed)?
    {
        return Err(DirectoryDurabilityError::Uncertain {
            completed,
            path: canonical,
            source: io::Error::new(
                io::ErrorKind::PermissionDenied,
                "directory identity changed during barriers",
            ),
        });
    }
    Ok(canonical)
}

/// One folder's own barrier on an existing chain: the leaf's always, an
/// ancestor's only where [`sys::flushes_ancestor`] says so.
fn chain_barrier(dir: &fs::File, ancestor: bool) -> io::Result<()> {
    if ancestor && !sys::flushes_ancestor(dir)? {
        return Ok(());
    }
    super::atomic::sync_dir(dir)
}

/// The barrier that keeps `entry`, the chain's next folder, in `folder`:
/// passed over only when [`sys::flushes_entry`] says, which is read from
/// the two folders before any flush, never from a refused one.
fn entry_barrier(folder: &fs::File, entry: &fs::File) -> io::Result<()> {
    if !sys::flushes_entry(folder, entry)? {
        return Ok(());
    }
    super::atomic::sync_dir(folder)
}

/// Creates at most 64 private descendant directories and checks their
/// persistence barriers. `durable_ancestor` must be an absolute, existing,
/// trusted directory whose ancestor durability the caller already knows.
/// Neither a successful read nor `AlreadyExists` establishes that fact.
/// The caller must keep the entire namespace stable, with no competing writer.
///
/// Each new or existing child and its containing directory are synced
/// ([`super::atomic::sync_dir`]) before proceeding; every one of them is this
/// account's own, so on Windows too a folder it cannot flush is a failure,
/// never passed over. Links and directories another account can modify are
/// refused. Existing permissions are left unchanged, so a caller requiring
/// privacy also checks any existing directory's access.
/// Failures retain the partial chain and identify its uncertain component;
/// a later explicit recovery checks all existing levels again.
/// This is not a physical power-loss promise.
pub fn create_dir_all_durable(
    path: &Path,
    durable_ancestor: &Path,
) -> Result<(), DirectoryDurabilityError> {
    let before = |source| DirectoryDurabilityError::BeforeCreate { source };
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid durable directory chain",
        )
    };
    if !durable_ancestor.is_absolute()
        || durable_ancestor
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return Err(before(invalid()));
    }
    let relative = path
        .strip_prefix(durable_ancestor)
        .map_err(|_| before(invalid()))?;
    for (count, part) in relative.components().enumerate() {
        if !matches!(part, Component::Normal(_)) || count == 64 {
            return Err(before(invalid()));
        }
    }
    let mut parent = trusted_dir(durable_ancestor).map_err(before)?;
    let mut completed = durable_ancestor.to_path_buf();
    for part in relative.components() {
        let child_path = completed.join(part.as_os_str());
        let step: io::Result<fs::File> = (|| {
            #[cfg(test)]
            bootstrap_faults::fail(bootstrap_faults::Point::Create)?;
            match create_dir(&child_path) {
                Ok(()) => {
                    #[cfg(test)]
                    bootstrap_faults::fail(bootstrap_faults::Point::Created)?;
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
            let child = trusted_dir(&child_path)?;
            #[cfg(test)]
            bootstrap_faults::fail(bootstrap_faults::Point::ChildSync)?;
            super::atomic::sync_dir(&child)?;
            #[cfg(test)]
            bootstrap_faults::fail(bootstrap_faults::Point::ParentSync)?;
            super::atomic::sync_dir(&parent)?;
            Ok(child)
        })();
        parent = step.map_err(|source| DirectoryDurabilityError::Uncertain {
            completed: completed.clone(),
            path: child_path.clone(),
            source,
        })?;
        completed = child_path;
    }
    Ok(())
}

fn trusted_dir(path: &Path) -> io::Result<fs::File> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "not a real trusted directory",
        ));
    }
    let directory = super::open_dir(path)?;
    if !handle_owned_by_current_user(&directory)? || others_can_modify(path)? {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "directory can be modified by another account",
        ));
    }
    Ok(directory)
}

// Only the external syscall boundary is faulted; the public bootstrap,
// directory validation and sync helpers remain real.
#[cfg(test)]
mod bootstrap_faults {
    use std::cell::Cell;
    use std::io;

    #[derive(Clone, Copy, PartialEq)]
    pub(super) enum Point {
        Create,
        Created,
        ChildSync,
        ParentSync,
    }

    thread_local! {
        static FAULT: Cell<Option<(Point, usize)>> = const { Cell::new(None) };
    }

    #[cfg(unix)]
    pub(super) const IO_ERROR: i32 = libc::EIO;
    #[cfg(windows)]
    pub(super) const IO_ERROR: i32 = windows_sys::Win32::Foundation::ERROR_WRITE_FAULT as i32;

    pub(super) struct Scope(Option<(Point, usize)>);

    pub(super) fn install(point: Point, skip: usize) -> Scope {
        Scope(FAULT.replace(Some((point, skip))))
    }

    impl Drop for Scope {
        fn drop(&mut self) {
            FAULT.set(self.0);
        }
    }

    pub(super) fn fail(point: Point) -> io::Result<()> {
        if let Some((wanted, skip)) = FAULT.get()
            && wanted == point
        {
            if skip == 0 {
                return Err(io::Error::from_raw_os_error(IO_ERROR));
            }
            FAULT.set(Some((wanted, skip - 1)));
        }
        Ok(())
    }
}

#[cfg(test)]
mod bootstrap_tests {
    use super::bootstrap_faults::{self, IO_ERROR, Point};
    use super::*;

    #[test]
    fn an_existing_anchor_failure_retains_the_chain_and_recovery_rechecks_it() {
        for point in [Point::ChildSync, Point::ParentSync] {
            let root = tempfile::tempdir().unwrap();
            let anchor = root.path().join("anchor");
            create_dir(&anchor).unwrap();
            let identity = super::super::identity::file_id(&anchor).unwrap();
            let fault = bootstrap_faults::install(point, 0);
            assert!(matches!(
                establish_dir_durability(&anchor),
                Err(DirectoryDurabilityError::Uncertain { source, .. })
                    if source.raw_os_error() == Some(IO_ERROR)
            ));
            assert_eq!(super::super::identity::file_id(&anchor).unwrap(), identity);
            drop(fault);
            let established = establish_dir_durability(&anchor).unwrap();
            assert_eq!(established, fs::canonicalize(&anchor).unwrap());
            let fault = bootstrap_faults::install(point, 0);
            assert!(matches!(
                establish_dir_durability(&anchor),
                Err(DirectoryDurabilityError::Uncertain { source, .. })
                    if source.raw_os_error() == Some(IO_ERROR)
            ));
            drop(fault);
            establish_dir_durability(&anchor).unwrap();
        }
    }

    #[test]
    fn failed_creation_retains_the_completed_chain_and_reports_the_os_cause() {
        for point in [Point::Create, Point::Created] {
            let root = tempfile::tempdir().unwrap();
            let first = root.path().join("first");
            let second = first.join("second");
            let end = second.join("last");
            let fault = bootstrap_faults::install(point, 1);
            let error = create_dir_all_durable(&end, root.path()).unwrap_err();
            match error {
                DirectoryDurabilityError::Uncertain {
                    completed,
                    path,
                    source,
                } => {
                    assert_eq!(completed, first);
                    assert_eq!(path, second);
                    assert_eq!(source.raw_os_error(), Some(IO_ERROR));
                }
                other => panic!("unexpected outcome: {other:?}"),
            }
            assert!(first.is_dir());
            assert_eq!(second.is_dir(), point == Point::Created);
            assert!(!end.exists());
            drop(fault);
            create_dir_all_durable(&end, root.path()).unwrap();
            assert!(is_private(&end).unwrap());
        }
    }

    #[test]
    fn failed_barriers_retain_real_directories_and_recovery_checks_existing_levels() {
        for point in [Point::ChildSync, Point::ParentSync] {
            let root = tempfile::tempdir().unwrap();
            let first = root.path().join("first");
            let second = first.join("second");
            let end = second.join("last");
            let fault = bootstrap_faults::install(point, 1);
            let error = create_dir_all_durable(&end, root.path()).unwrap_err();
            match error {
                DirectoryDurabilityError::Uncertain {
                    completed,
                    path,
                    source,
                } => {
                    assert_eq!(completed, first);
                    assert_eq!(path, second);
                    assert_eq!(source.raw_os_error(), Some(IO_ERROR));
                }
                other => panic!("unexpected outcome: {other:?}"),
            }
            assert!(first.is_dir());
            assert!(second.is_dir());
            assert!(is_private(&second).unwrap());
            assert!(!end.exists());
            drop(fault);
            // Existing directories must not let recovery bypass a barrier.
            let fault = bootstrap_faults::install(point, 0);
            assert!(matches!(
                create_dir_all_durable(&end, root.path()),
                Err(DirectoryDurabilityError::Uncertain { path, source, .. })
                    if path == first && source.raw_os_error() == Some(IO_ERROR)
            ));
            assert!(!end.exists());
            drop(fault);
            create_dir_all_durable(&end, root.path()).unwrap();
            assert!(is_private(&end).unwrap());
        }
    }
}

/// Makes the file `path`, private and open for writing. It must not exist.
pub fn create_new_file(path: &Path) -> io::Result<fs::File> {
    sys::open_file(path, true)
}

/// Opens the file `path` for writing without truncating it, making it
/// private when it does not exist. An existing file keeps what it has.
pub fn open_or_create_file(path: &Path) -> io::Result<fs::File> {
    sys::open_file(path, false)
}

/// Opens the account's own file `path` for reading, and for writing too when
/// `create` asks for it to be made, private, if it is not there. The name
/// itself is opened, never what a symbolic link at it leads to, and only a
/// regular file the current account owns and that has no other name is
/// accepted (`PermissionDenied` otherwise): the file may sit in a folder
/// other accounts write, where a planted link would have its target written
/// or read, or a planted pipe would block.
pub fn open_own_file(path: &Path, create: bool) -> io::Result<fs::File> {
    let file = sys::open_own(path, create)?;
    // A second name would be another account's hard link to one of this
    // account's files, planted where the caller looks.
    if !file.metadata()?.is_file()
        || !handle_owned_by_current_user(&file)?
        || super::identity::link_count(&file)? != 1
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the file is not the account's own regular file with one name",
        ));
    }
    Ok(file)
}

/// Makes an existing file or folder private: 0700 for a folder or a file its
/// owner can run, 0600 for any other file, on Unix. A folder's children made
/// after this are private too on Windows, which inherits the list; Unix has no
/// inheritance, and a new child takes the process's umask.
pub fn restrict_to_owner(path: &Path) -> io::Result<()> {
    sys::restrict_to_owner(path)
}

/// Whether nobody but the current account (and, on Windows, the system and
/// the administrators, who can take any file anyway) has any access to the
/// file or folder at `path`.
pub fn is_private(path: &Path) -> io::Result<bool> {
    sys::access_of_others(path, false).map(|others| !others)
}

/// Whether anybody but the current account (and the system on Windows) can
/// change the file or folder at `path`: write to it, delete it, or change who
/// may. A folder that answers yes cannot hold code the account runs, because
/// another account could replace that code.
pub fn others_can_modify(path: &Path) -> io::Result<bool> {
    sys::access_of_others(path, true)
}

/// Whether another account can change an open file or folder. The descriptor
/// is judged directly, so replacing its path cannot change this answer.
pub fn handle_others_can_modify(handle: &impl Handle) -> io::Result<bool> {
    sys::handle_access_of_others(handle, true)
}

/// Whether the file or folder at `path` belongs to the current account.
pub fn owned_by_current_user(path: &Path) -> io::Result<bool> {
    sys::owned_by_current_user(path)
}

/// Whether the open file or folder belongs to the current account.
pub fn handle_owned_by_current_user(handle: &impl Handle) -> io::Result<bool> {
    sys::handle_owned_by_current_user(handle)
}

/// One authenticated directory entry. The caller still validates every
/// containing directory and, for an alias, its returned target namespace.
#[derive(Debug, Eq, PartialEq)]
pub enum InspectionDirectory {
    /// A real directory protected from replacement by another account.
    Real,
    /// An alias owned by the system, inspected as the alias itself.
    SystemAlias(PathBuf),
}

/// Inspects one entry without following it. Real directories must belong
/// to this account or the system and deny other-account modification; a
/// system directory others may only add entries to protects the names in it
/// as well (the sticky rule: a root-owned sticky folder on Unix, on Windows a
/// system-owned folder that grants others only `FILE_ADD_FILE` and
/// `FILE_ADD_SUBDIRECTORY`, as a drive's root does).
/// A system-owned alias returns its raw target so the caller can inspect
/// that namespace before following it. Account-owned links are refused.
/// This creates nothing and leaves permissions unchanged.
pub fn inspect_directory_entry(path: &Path) -> io::Result<InspectionDirectory> {
    sys::inspect_directory_entry(path)
}

#[cfg(unix)]
mod sys {
    use std::fs;
    use std::io;
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::path::Path;

    use super::{Handle, InspectionDirectory};

    pub(super) fn create_dir(path: &Path) -> io::Result<()> {
        fs::DirBuilder::new().mode(0o700).create(path)
    }

    pub(super) fn open_file(path: &Path, new: bool) -> io::Result<fs::File> {
        let mut options = fs::OpenOptions::new();
        options.write(true).mode(0o600);
        if new {
            options.create_new(true);
        } else {
            options.create(true).truncate(false);
        }
        options.open(path)
    }

    pub(super) fn open_own(path: &Path, create: bool) -> io::Result<fs::File> {
        let mut options = fs::OpenOptions::new();
        options
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        if create {
            options.write(true).create(true).truncate(false).mode(0o600);
        }
        options.open(path)
    }

    /// Why [`super::own_dir`] refuses a folder of this account it cannot
    /// open, and the fix.
    pub(super) const CLOSED_TO_ITS_OWNER: &str =
        "is this account's but closed to it; `chmod 700` it so Hide can use it";

    /// The folder entry at `path` itself, open for reading; a link or
    /// anything but a folder there is `InvalidInput`.
    pub(super) fn open_entry_dir(path: &Path) -> io::Result<fs::File> {
        let mut options = fs::OpenOptions::new();
        options
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_NONBLOCK);
        options
            .open(path)
            .map_err(|error| match error.raw_os_error() {
                Some(libc::ELOOP | libc::ENOTDIR) => {
                    io::Error::new(io::ErrorKind::InvalidInput, "the entry is not a folder")
                }
                _ => error,
            })
    }

    /// `fchmod` on the open folder: only the folder it names changes.
    pub(super) fn restrict_dir_handle(folder: &fs::File) -> io::Result<()> {
        folder.set_permissions(fs::Permissions::from_mode(0o700))
    }

    /// Unix syncs every folder of a chain it can open, whoever may write
    /// to it or made the entry the chain goes through.
    pub(super) fn flushes_ancestor(_: &fs::File) -> io::Result<bool> {
        Ok(true)
    }

    pub(super) fn flushes_entry(_: &fs::File, _: &fs::File) -> io::Result<bool> {
        Ok(true)
    }

    pub(super) fn restrict_to_owner(path: &Path) -> io::Result<()> {
        let metadata = fs::metadata(path)?;
        // A file that ran for its owner still does.
        let mode = if metadata.is_dir() || metadata.mode() & 0o100 != 0 {
            0o700
        } else {
            0o600
        };
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
    }

    /// `changes` asks about write access, otherwise about any access.
    pub(super) fn access_of_others(path: &Path, changes: bool) -> io::Result<bool> {
        let watched = if changes { 0o022 } else { 0o077 };
        Ok(fs::metadata(path)?.mode() & watched != 0)
    }

    pub(super) fn handle_access_of_others(handle: &impl Handle, changes: bool) -> io::Result<bool> {
        let watched = if changes { 0o022 } else { 0o077 };
        Ok(crate::fs::duplicate(handle)?.metadata()?.mode() & watched != 0)
    }

    fn current_user() -> u32 {
        // SAFETY: geteuid has no preconditions and cannot fail.
        unsafe { libc::geteuid() }
    }

    pub(super) fn owned_by_current_user(path: &Path) -> io::Result<bool> {
        Ok(fs::metadata(path)?.uid() == current_user())
    }

    pub(super) fn handle_owned_by_current_user(handle: &impl Handle) -> io::Result<bool> {
        let metadata = crate::fs::duplicate(handle)?.metadata()?;
        Ok(metadata.uid() == current_user())
    }

    pub(super) fn inspect_directory_entry(path: &Path) -> io::Result<InspectionDirectory> {
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() && metadata.uid() == 0 {
            return fs::read_link(path).map(InspectionDirectory::SystemAlias);
        }
        let shared = metadata.mode() & 0o022 != 0;
        let system = metadata.uid() == 0 && (!shared || metadata.mode() & 0o1000 != 0);
        if metadata.is_dir() && (system || (metadata.uid() == current_user() && !shared)) {
            return Ok(InspectionDirectory::Real);
        }
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "directory entry is not an owned real directory safe to inspect",
        ))
    }
}

#[cfg(windows)]
mod sys {
    use std::ffi::c_void;
    use std::fs;
    use std::io;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use std::path::Path;
    use std::ptr::{null, null_mut};

    use widestring::U16CString;
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_NO_TOKEN, ERROR_SUCCESS, GENERIC_READ, GENERIC_WRITE, HANDLE,
        INVALID_HANDLE_VALUE, LUID, LocalFree,
    };
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        ConvertStringSidToSidW, GetNamedSecurityInfoW, GetSecurityInfo, SDDL_REVISION_1,
        SE_FILE_OBJECT, SetNamedSecurityInfoW, SetSecurityInfo,
    };
    use windows_sys::Win32::Security::{
        ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_SIZE_INFORMATION, AccessCheck, AclSizeInformation,
        CreateWellKnownSid, DACL_SECURITY_INFORMATION, DuplicateToken, EqualSid, GENERIC_MAPPING,
        GROUP_SECURITY_INFORMATION, GetAce, GetAclInformation, GetSecurityDescriptorDacl,
        GetTokenInformation, INHERIT_ONLY_ACE, LUID_AND_ATTRIBUTES, OWNER_SECURITY_INFORMATION,
        PRIVILEGE_SET, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
        SECURITY_ATTRIBUTES, SECURITY_MAX_SID_SIZE, SecurityImpersonation, TOKEN_DUPLICATE,
        TOKEN_OWNER, TOKEN_QUERY, TOKEN_USER, TokenOwner, TokenUser, WinBuiltinAdministratorsSid,
        WinLocalSystemSid,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CREATE_NEW, CreateDirectoryW, CreateFileW, FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY,
        FILE_ALL_ACCESS, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_EXECUTE,
        FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, OPEN_ALWAYS, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::SystemServices::MAXIMUM_ALLOWED;
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetCurrentThread, OpenProcessToken, OpenThreadToken,
    };

    use super::{Handle, InspectionDirectory};
    use crate::fs::wide;

    /// Access that changes a file: write, append, delete, delete a child,
    /// change the access list or the owner, and the generic forms of those.
    const CHANGE: u32 = 0x0000_0002
        | 0x0000_0004
        | 0x0000_0010
        | 0x0000_0040
        | 0x0000_0100
        | 0x0001_0000
        | 0x0004_0000
        | 0x0008_0000
        | 0x4000_0000
        | 0x1000_0000;

    /// What others may do to a system folder without touching what is in
    /// it: add a file or a folder of their own. Every signed-in account may
    /// do that to a drive's root, as anyone may to `/tmp` on Unix, and only
    /// deleting, renaming or re-permissioning a folder or an entry in it
    /// (`DELETE`, `FILE_DELETE_CHILD`, `WRITE_DAC`, `WRITE_OWNER`) or
    /// rewriting it takes away what a caller relies on: the sticky rule.
    const ADD_ENTRY: u32 = FILE_ADD_FILE | FILE_ADD_SUBDIRECTORY;

    /// The ACE type of an entry that allows access.
    const ACCESS_ALLOWED: u8 = 0;
    /// The ACE types of a denial: plain, object, callback, callback object.
    const DENIALS: [u8; 4] = [1, 6, 0x0A, 0x0C];

    /// A kernel handle closed on drop.
    struct Closing(HANDLE);

    impl Drop for Closing {
        fn drop(&mut self) {
            // SAFETY: the handle is open and owned by this value.
            unsafe { CloseHandle(self.0) };
        }
    }

    /// A buffer the system allocated with `LocalAlloc`, freed on drop.
    struct Local(*mut c_void);

    impl Drop for Local {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: the pointer came from an API that allocates with
                // LocalAlloc and asks the caller to free it, and is freed once.
                unsafe { LocalFree(self.0) };
            }
        }
    }

    /// A SID owned as bytes, so it can be compared and printed.
    struct Sid(Vec<u8>);

    impl Sid {
        fn as_psid(&self) -> PSID {
            self.0.as_ptr().cast_mut().cast()
        }

        fn well_known(kind: i32) -> io::Result<Self> {
            let mut bytes = vec![0u8; SECURITY_MAX_SID_SIZE as usize];
            let mut size = bytes.len() as u32;
            // SAFETY: `bytes` holds `size` writable bytes.
            if unsafe { CreateWellKnownSid(kind, null_mut(), bytes.as_mut_ptr().cast(), &mut size) }
                == 0
            {
                return Err(io::Error::last_os_error());
            }
            bytes.truncate(size as usize);
            Ok(Self(bytes))
        }

        fn same_as(&self, other: PSID) -> bool {
            // SAFETY: both pointers are valid SIDs for the duration of the call.
            unsafe { EqualSid(self.as_psid(), other) != 0 }
        }

        /// The SID an `S-1-...` string names.
        fn parse(text: &str) -> io::Result<Self> {
            let text = U16CString::from_str(text)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
            let mut sid = null_mut();
            // SAFETY: `text` is NUL-terminated and `sid` receives a LocalAlloc
            // SID freed below.
            if unsafe { ConvertStringSidToSidW(text.as_ptr(), &mut sid) } == 0 {
                return Err(io::Error::last_os_error());
            }
            let sid = Local(sid);
            // SAFETY: the call returned a valid SID, whose length its header
            // gives.
            let length = unsafe { windows_sys::Win32::Security::GetLengthSid(sid.0) } as usize;
            // SAFETY: as above, `length` bytes are readable at the SID.
            Ok(Self(
                unsafe { std::slice::from_raw_parts(sid.0.cast::<u8>(), length) }.to_vec(),
            ))
        }

        /// `S-1-5-21-...`, the form an access-list string names an account by.
        fn text(&self) -> io::Result<String> {
            let mut text = null_mut();
            // SAFETY: the SID is valid, and `text` receives a LocalAlloc string.
            if unsafe { ConvertSidToStringSidW(self.as_psid(), &mut text) } == 0 {
                return Err(io::Error::last_os_error());
            }
            let text = Local(text.cast());
            // SAFETY: the call returned a NUL-terminated wide string.
            let wide = unsafe { U16CString::from_ptr_str(text.0.cast::<u16>()) };
            Ok(wide.to_string_lossy())
        }
    }

    /// The account this thread acts as, and the owner a file it creates gets
    /// (the Administrators group for an elevated process).
    struct Account {
        user: Sid,
        owner: Sid,
    }

    fn token_sid(token: HANDLE, class: i32) -> io::Result<Sid> {
        let mut size = 0u32;
        // SAFETY: a size query with no buffer; the call fails and sets `size`.
        unsafe { GetTokenInformation(token, class, null_mut(), 0, &mut size) };
        // A `u64` buffer keeps the structure the call writes aligned.
        let mut buffer = vec![0u64; (size as usize).div_ceil(8).max(1)];
        // SAFETY: `buffer` holds at least `size` writable bytes.
        if unsafe { GetTokenInformation(token, class, buffer.as_mut_ptr().cast(), size, &mut size) }
            == 0
        {
            return Err(io::Error::last_os_error());
        }
        // TOKEN_USER and TOKEN_OWNER both start with the pointer to the SID.
        // SAFETY: the call filled the structure for this class, and the SID it
        // points at lies inside `buffer`.
        let sid: PSID = if class == TokenUser {
            unsafe { (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid }
        } else {
            unsafe { (*buffer.as_ptr().cast::<TOKEN_OWNER>()).Owner }
        };
        // SAFETY: `sid` is a valid SID inside `buffer`; its length is read from
        // the SID's own header and copied out.
        let length = unsafe { windows_sys::Win32::Security::GetLengthSid(sid) } as usize;
        // SAFETY: as above, `length` bytes are readable at `sid`.
        Ok(Sid(unsafe {
            std::slice::from_raw_parts(sid.cast::<u8>(), length)
        }
        .to_vec()))
    }

    /// The token this thread acts with: the one it impersonates, which is what
    /// the system checks its opens against, or else the process's.
    fn acting_token(access: u32) -> io::Result<Closing> {
        let mut token = null_mut();
        // SAFETY: the current-thread pseudo handle is always valid; the token
        // is opened with the process's own rights.
        if unsafe { OpenThreadToken(GetCurrentThread(), access, 1, &mut token) } != 0 {
            return Ok(Closing(token));
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_NO_TOKEN as i32) {
            return Err(error);
        }
        // SAFETY: the current-process pseudo handle is always valid.
        if unsafe { OpenProcessToken(GetCurrentProcess(), access, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Closing(token))
    }

    fn account() -> io::Result<Account> {
        let token = acting_token(TOKEN_QUERY)?;
        Ok(Account {
            user: token_sid(token.0, TokenUser)?,
            owner: token_sid(token.0, TokenOwner)?,
        })
    }

    /// A security descriptor made from an access-list string, and the
    /// attributes that hand it to a create call.
    struct Descriptor {
        _local: Local,
        attributes: SECURITY_ATTRIBUTES,
    }

    /// An access list that gives the current account everything and nobody
    /// else anything, with inheritance cut off. A folder's children inherit
    /// it.
    fn private_descriptor(folder: bool) -> io::Result<Descriptor> {
        let user = account()?.user.text()?;
        let inherit = if folder { "OICI" } else { "" };
        let text = U16CString::from_str(format!("D:P(A;{inherit};FA;;;{user})"))
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: `text` is NUL-terminated and `descriptor` receives a
        // LocalAlloc buffer.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                text.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Descriptor {
            _local: Local(descriptor),
            attributes: SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            },
        })
    }

    pub(super) fn create_dir(path: &Path) -> io::Result<()> {
        let descriptor = private_descriptor(true)?;
        let name = wide(path)?;
        // SAFETY: `name` is NUL-terminated and the attributes outlive the call.
        if unsafe { CreateDirectoryW(name.as_ptr(), &descriptor.attributes) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub(super) fn open_file(path: &Path, new: bool) -> io::Result<fs::File> {
        let descriptor = private_descriptor(false)?;
        create_file(
            path,
            GENERIC_WRITE,
            Some(&descriptor),
            if new { CREATE_NEW } else { OPEN_ALWAYS },
            FILE_ATTRIBUTE_NORMAL,
        )
    }

    pub(super) fn open_own(path: &Path, create: bool) -> io::Result<fs::File> {
        // A link at the name is opened as itself, and refused by the caller
        // as a file that is not regular.
        if create {
            let descriptor = private_descriptor(false)?;
            create_file(
                path,
                GENERIC_READ | GENERIC_WRITE,
                Some(&descriptor),
                OPEN_ALWAYS,
                FILE_FLAG_OPEN_REPARSE_POINT,
            )
        } else {
            create_file(
                path,
                GENERIC_READ,
                None,
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT,
            )
        }
    }

    /// `CreateFileW`, sharing everything, with the private descriptor for a
    /// file it makes.
    fn create_file(
        path: &Path,
        access: u32,
        descriptor: Option<&Descriptor>,
        disposition: u32,
        flags: u32,
    ) -> io::Result<fs::File> {
        let name = wide(path)?;
        let attributes = descriptor.map_or(null(), |descriptor| &descriptor.attributes);
        // SAFETY: `name` is NUL-terminated and the attributes, when given,
        // outlive the call.
        let handle = unsafe {
            CreateFileW(
                name.as_ptr(),
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                attributes,
                disposition,
                flags,
                null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the call returned a new handle that nothing else owns.
        Ok(unsafe { fs::File::from_raw_handle(handle) })
    }

    /// The private access list, in the descriptor that holds it.
    fn private_acl(folder: bool) -> io::Result<(Descriptor, *mut ACL)> {
        let descriptor = private_descriptor(folder)?;
        let mut present = 0;
        let mut defaulted = 0;
        let mut acl: *mut ACL = null_mut();
        // SAFETY: the descriptor is valid and the out parameters are writable.
        if unsafe {
            GetSecurityDescriptorDacl(
                descriptor.attributes.lpSecurityDescriptor,
                &mut present,
                &mut acl,
                &mut defaulted,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if present == 0 {
            // A descriptor without an access list would be set as one that
            // gives everybody everything.
            return Err(io::Error::other("the private access list has no entries"));
        }
        Ok((descriptor, acl))
    }

    pub(super) fn restrict_to_owner(path: &Path) -> io::Result<()> {
        let (_descriptor, acl) = private_acl(fs::metadata(path)?.is_dir())?;
        let name = wide(path)?;
        // SAFETY: `name` is NUL-terminated and `acl` lives in the descriptor
        // that outlives the call.
        let status = unsafe {
            SetNamedSecurityInfoW(
                name.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                acl,
                null(),
            )
        };
        if status != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        Ok(())
    }

    /// Why [`super::own_dir`] refuses a folder of this account it cannot
    /// open, and the fix.
    pub(super) const CLOSED_TO_ITS_OWNER: &str = "is this account's but closed to it; give the account full control of it so Hide can use it";

    /// The folder entry at `path` itself, a link or junction opened as the
    /// link; anything but a real folder is `InvalidInput`. Asked for the
    /// right to change its access list too, which only its owner has
    /// without an entry granting it; another account's folder is opened to
    /// be read only, so the caller can say whose it is.
    pub(super) fn open_entry_dir(path: &Path) -> io::Result<fs::File> {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES, READ_CONTROL, WRITE_DAC,
        };
        let open = |access: u32| {
            fs::OpenOptions::new()
                .access_mode(access)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
                .open(path)
        };
        let folder = match open(READ_CONTROL | WRITE_DAC | FILE_READ_ATTRIBUTES) {
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                open(READ_CONTROL | FILE_READ_ATTRIBUTES)?
            }
            opened => opened?,
        };
        // A link or junction opened as itself is not a folder here.
        if !folder.metadata()?.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the entry is not a folder",
            ));
        }
        Ok(folder)
    }

    /// The private access list set on the open folder, protected from
    /// what its parent would pass down.
    pub(super) fn restrict_dir_handle(folder: &fs::File) -> io::Result<()> {
        let (_descriptor, acl) = private_acl(true)?;
        // SAFETY: the handle is open for the borrow of `folder` and `acl`
        // lives in the descriptor that outlives the call.
        let status = unsafe {
            SetSecurityInfo(
                folder.as_raw_handle(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                acl,
                null(),
            )
        };
        if status != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        Ok(())
    }

    /// An ancestor is flushed for its entry on the chain alone
    /// ([`flushes_entry`]), by the step that opens that entry.
    pub(super) fn flushes_ancestor(_: &fs::File) -> io::Result<bool> {
        Ok(false)
    }

    /// Whether `folder` is flushed to keep `entry`, the chain's next folder
    /// in it: only when `entry`'s owner is this account (its user, or the
    /// owner its token gives what it makes), since a folder another owner
    /// holds, such as the system's `Users` in a drive's root, cannot be one
    /// this account or an earlier run made, and a drive root's flush writes
    /// out all the drive holds unwritten, seconds after an install; and only
    /// when the account may flush `folder` ([`may_flush`]). An owner that
    /// cannot be read is flushed for, as every entry was before (operator
    /// decision on #1515: the safe side when the fact is missing).
    pub(super) fn flushes_entry(folder: &fs::File, entry: &fs::File) -> io::Result<bool> {
        let made_here = handle_owned_by_current_user(entry).unwrap_or(true);
        Ok(made_here && may_flush(folder)?)
    }

    /// Whether this account owns the open folder or may add a file or a
    /// folder to it, which a flush of its entries needs: an ancestor that
    /// holds neither holds no entry the account made. The right is what the
    /// system's own access check answers for the process's token against
    /// the folder's owner, group and access list (`AccessCheck`, asked for
    /// the most the folder allows).
    fn may_flush(folder: &fs::File) -> io::Result<bool> {
        if handle_owned_by_current_user(folder)? {
            return Ok(true);
        }
        let mut descriptor = null_mut();
        // SAFETY: the handle is open for the borrow of `folder`, and
        // `descriptor` receives a LocalAlloc buffer `Local` frees.
        let status = unsafe {
            GetSecurityInfo(
                folder.as_raw_handle(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
                &mut descriptor,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        let descriptor = Local(descriptor);
        let acting = acting_token(TOKEN_QUERY | TOKEN_DUPLICATE)?;
        // The access check takes an impersonation token.
        let mut token = null_mut();
        // SAFETY: `acting` is an open token; `token` receives a new one.
        if unsafe { DuplicateToken(acting.0, SecurityImpersonation, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = Closing(token);
        let mapping = GENERIC_MAPPING {
            GenericRead: FILE_GENERIC_READ,
            GenericWrite: FILE_GENERIC_WRITE,
            GenericExecute: FILE_GENERIC_EXECUTE,
            GenericAll: FILE_ALL_ACCESS,
        };
        let mut privileges = PRIVILEGE_SET {
            PrivilegeCount: 0,
            Control: 0,
            Privilege: [LUID_AND_ATTRIBUTES {
                Luid: LUID {
                    LowPart: 0,
                    HighPart: 0,
                },
                Attributes: 0,
            }],
        };
        let mut length = size_of::<PRIVILEGE_SET>() as u32;
        let mut granted = 0u32;
        let mut allowed = 0;
        // SAFETY: the descriptor holds the owner, group and access list the
        // check needs, the token is an impersonation token, and every out
        // parameter is writable for the length given.
        if unsafe {
            AccessCheck(
                descriptor.0,
                token.0,
                MAXIMUM_ALLOWED,
                &mapping,
                &mut privileges,
                &mut length,
                &mut granted,
                &mut allowed,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(allowed != 0 && granted & (FILE_ADD_FILE | FILE_ADD_SUBDIRECTORY) != 0)
    }

    /// The security a path or an open file carries, and who owns it.
    struct Security {
        _descriptor: Local,
        owner: PSID,
        acl: *mut ACL,
    }

    fn security_of_path(path: &Path) -> io::Result<Security> {
        let name = wide(path)?;
        let mut owner = null_mut();
        let mut acl = null_mut();
        let mut descriptor = null_mut();
        // SAFETY: `name` is NUL-terminated and the out parameters are writable.
        let status = unsafe {
            GetNamedSecurityInfoW(
                name.as_ptr(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut owner,
                null_mut(),
                &mut acl,
                null_mut(),
                &mut descriptor,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        Ok(Security {
            _descriptor: Local(descriptor),
            owner,
            acl,
        })
    }

    fn security_of_handle(handle: &impl Handle) -> io::Result<Security> {
        let raw = handle.as_handle().as_raw_handle();
        let mut owner = null_mut();
        let mut acl = null_mut();
        let mut descriptor = null_mut();
        // SAFETY: `raw` is an open handle borrowed for the call and the out
        // parameters are writable.
        let status = unsafe {
            GetSecurityInfo(
                raw,
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut owner,
                null_mut(),
                &mut acl,
                null_mut(),
                &mut descriptor,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        Ok(Security {
            _descriptor: Local(descriptor),
            owner,
            acl,
        })
    }

    fn owned(security: &Security) -> io::Result<bool> {
        let account = account()?;
        Ok(account.user.same_as(security.owner) || account.owner.same_as(security.owner))
    }

    pub(super) fn owned_by_current_user(path: &Path) -> io::Result<bool> {
        owned(&security_of_path(path)?)
    }

    pub(super) fn handle_owned_by_current_user(handle: &impl Handle) -> io::Result<bool> {
        owned(&security_of_handle(handle)?)
    }

    pub(super) fn inspect_directory_entry(path: &Path) -> io::Result<InspectionDirectory> {
        use std::os::windows::fs::MetadataExt;
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES,
            READ_CONTROL,
        };

        // Security inspection needs READ_CONTROL, unlike identity-only
        // queries. Open the entry itself, including a directory or alias.
        let entry = fs::OpenOptions::new()
            .access_mode(READ_CONTROL | FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;
        let metadata = entry.metadata()?;
        let security = security_of_handle(&entry)?;
        let system = [
            Sid::well_known(WinLocalSystemSid)?,
            Sid::well_known(WinBuiltinAdministratorsSid)?,
            // TrustedInstaller, which owns a system drive's root.
            Sid::parse("S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464")?,
        ]
        .iter()
        .any(|sid| sid.same_as(security.owner));
        let alias = metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
        // Others may add their own entries to a system folder (ADD_ENTRY);
        // a folder that inherited a drive root's grant of change to every
        // signed-in account still lets them delete, and stays refused.
        let changes = if system && metadata.is_dir() && !alias {
            CHANGE & !ADD_ENTRY
        } else {
            CHANGE
        };
        if !access_of_security(&security, changes)? {
            if alias {
                if system && metadata.file_type().is_symlink() {
                    return fs::read_link(path).map(InspectionDirectory::SystemAlias);
                }
            } else if metadata.is_dir() && (system || owned(&security)?) {
                return Ok(InspectionDirectory::Real);
            }
        }
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "directory entry is not an owned real directory safe to inspect",
        ))
    }

    /// Whether an access list the current account does not own grants `others`
    /// any access (`changes` false) or any access that changes the file.
    pub(super) fn access_of_others(path: &Path, changes: bool) -> io::Result<bool> {
        access_of_security(&security_of_path(path)?, watched(changes))
    }

    pub(super) fn handle_access_of_others(handle: &impl Handle, changes: bool) -> io::Result<bool> {
        access_of_security(&security_of_handle(handle)?, watched(changes))
    }

    fn watched(changes: bool) -> u32 {
        if changes { CHANGE } else { u32::MAX }
    }

    /// Whether anyone but this account and the system is allowed any of the
    /// `watched` access.
    fn access_of_security(security: &Security, watched: u32) -> io::Result<bool> {
        let account = account()?;
        let trusted = [
            account.user,
            account.owner,
            Sid::well_known(WinLocalSystemSid)?,
            Sid::well_known(WinBuiltinAdministratorsSid)?,
        ];
        if security.acl.is_null() {
            // No access list at all means everybody has every access.
            return Ok(true);
        }
        let mut size = ACL_SIZE_INFORMATION {
            AceCount: 0,
            AclBytesInUse: 0,
            AclBytesFree: 0,
        };
        // SAFETY: the list is valid and `size` is the structure the class names.
        if unsafe {
            GetAclInformation(
                security.acl,
                (&mut size as *mut ACL_SIZE_INFORMATION).cast(),
                size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        for index in 0..size.AceCount {
            let mut entry: *mut c_void = null_mut();
            // SAFETY: `index` is below the entry count of a valid list.
            if unsafe { GetAce(security.acl, index, &mut entry) } == 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: every entry starts with its header.
            let header = unsafe { &*entry.cast::<ACE_HEADER>() };
            if u32::from(header.AceFlags) & INHERIT_ONLY_ACE != 0 {
                // It only applies to the children of a folder.
                continue;
            }
            // Denials take access away and need no check; an entry that is
            // neither a plain allowance nor a denial (an object, callback or
            // compound allowance) is counted as access, since not knowing is
            // not the same as nobody having it.
            if header.AceType != ACCESS_ALLOWED {
                if DENIALS.contains(&header.AceType) {
                    continue;
                }
                return Ok(true);
            }
            // SAFETY: an allow entry has this layout; the SID starts at
            // `SidStart` and runs to the end of the entry.
            let allowed = unsafe { &*entry.cast::<ACCESS_ALLOWED_ACE>() };
            let sid: PSID = (&allowed.SidStart as *const u32).cast_mut().cast();
            if trusted.iter().any(|known| known.same_as(sid)) {
                continue;
            }
            if allowed.Mask & watched != 0 {
                return Ok(true);
            }
        }
        Ok(false)
    }
}
