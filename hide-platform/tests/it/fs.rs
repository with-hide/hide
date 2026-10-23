//! The contract of `hide_platform::fs`, stated as what a caller observes.
//! The same file runs on macOS, Linux and Windows.
//!
//! One test below is a second process: the test binary runs itself with
//! `HIDE_PLATFORM_FS_ROLE` set, and the test that reads the variable plays
//! the role; without the variable it does nothing.

use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use hide_platform::fs::Access;
use hide_platform::fs::atomic::{self, DurableAccess, EXCHANGE_IS_ATOMIC};
use hide_platform::fs::identity::{self, FileId};
use hide_platform::fs::link;
use hide_platform::fs::lock::{self, Mode, Waited};
use hide_platform::fs::permissions::{self, Permissions};
use hide_platform::fs::private;
use hide_platform::fs::space;

const ROLE: &str = "HIDE_PLATFORM_FS_ROLE";
const HELD: &str = "HIDE_PLATFORM_FS_HELD";

fn folder() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[test]
fn an_existing_private_anchor_establishes_sibling_home_and_state_chains() {
    let outer = folder();
    let anchor = outer.path().join("anchor");
    private::create_dir(&anchor).unwrap();
    let before = identity::file_id(&anchor).unwrap();
    let canonical = private::establish_dir_durability(&anchor).unwrap();
    assert_eq!(identity::file_id(&canonical).unwrap(), before);
    assert!(names(&anchor).is_empty());
    let home = canonical.join("home");
    let state = canonical.join("state");
    private::create_dir_all_durable(&home, &canonical).unwrap();
    private::create_dir_all_durable(&state, &canonical).unwrap();
    for path in [&home, &state] {
        assert!(private::is_private(path).unwrap());
        assert!(names(path).is_empty());
    }
    assert_eq!(
        private::establish_dir_durability(&anchor).unwrap(),
        canonical
    );
    assert_eq!(names(&anchor), ["home", "state"]);
}

#[test]
fn an_existing_anchor_refuses_links_files_and_modifiable_leaves_without_mutation() {
    let outer = folder();
    let anchor = outer.path().join("anchor");
    private::create_dir(&anchor).unwrap();
    let alias = outer.path().join("alias");
    link::create_link(&anchor, &alias).unwrap();
    let file = outer.path().join("file");
    fs::write(&file, b"kept").unwrap();
    for path in [&alias, &file] {
        assert!(private::establish_dir_durability(path).is_err());
    }
    widen(&anchor, true);
    assert!(private::establish_dir_durability(&anchor).is_err());
    assert!(alias.is_dir());
    assert_eq!(read(&file), "kept");
    assert!(names(&anchor).is_empty());
}

/// Windows flushes a folder only through a handle that may add an entry to
/// it. An ancestor the account neither owns nor may add an entry to holds no
/// entry the account made, so the barrier passes over it (`C:\Users` for a
/// standard account); a folder the account owns is flushed whatever its
/// access list says, and a refused flush there is the answer. Unix syncs
/// every folder it can open, so both chains stand there.
#[test]
fn an_ancestor_is_passed_over_only_when_the_account_neither_owns_nor_may_add_to_it() {
    let outer = folder();

    // Handing a folder to the system takes an administrator, which the
    // Windows runner's account is.
    #[cfg(windows)]
    {
        let given = outer.path().join("given");
        fs::create_dir(&given).unwrap();
        let anchor = given.join("anchor");
        private::create_dir(&anchor).unwrap();
        icacls(&given, &["/setowner", "*S-1-5-18"]);
        read_only(&given);
        assert!(!private::owned_by_current_user(&given).unwrap());
        assert!(!may_add_to(&given), "{}", access_list(&given));

        let canonical = private::establish_dir_durability(&anchor).unwrap();

        assert_eq!(
            identity::file_id(&canonical).unwrap(),
            identity::file_id(&anchor).unwrap()
        );
        assert!(names(&anchor).is_empty());
    }

    let closed = outer.path().join("closed");
    fs::create_dir(&closed).unwrap();
    let anchor = closed.join("anchor");
    private::create_dir(&anchor).unwrap();
    let _closed = Closed::own(&closed);
    #[cfg(windows)]
    assert!(!may_add_to(&closed), "{}", access_list(&closed));

    let synced = atomic::sync_dir(&open_dir(&closed));
    let established = private::establish_dir_durability(&anchor);
    if cfg!(windows) {
        assert_eq!(synced.unwrap_err().kind(), ErrorKind::PermissionDenied);
        // `closed` is flushed to keep `anchor` in it, so `anchor`'s entry is
        // the one left uncertain.
        let anchor = fs::canonicalize(&anchor).unwrap();
        assert!(
            matches!(
                &established,
                Err(private::DirectoryDurabilityError::Uncertain { path, source, .. })
                    if *path == anchor && source.kind() == ErrorKind::PermissionDenied
            ),
            "{established:?}"
        );
    } else {
        synced.unwrap();
        assert_eq!(
            identity::file_id(&established.unwrap()).unwrap(),
            identity::file_id(&anchor).unwrap()
        );
    }
    assert!(names(&anchor).is_empty());
}

/// Windows flushes an ancestor only to keep the chain's next folder in it,
/// and only when the account made that folder: one the system made, as
/// `Users` in a drive's root, is not flushed for, since a drive root's flush
/// writes out all the drive holds unwritten. The account owns `closed` but
/// may not flush it: with the system's folder next on the chain, the chain
/// stands; with the account's own folder there, the refused flush is the
/// answer.
#[cfg(windows)]
#[test]
fn an_ancestor_is_flushed_only_to_keep_a_folder_the_account_made() {
    let outer = folder();
    let closed = outer.path().join("closed");
    fs::create_dir(&closed).unwrap();
    let system = closed.join("system");
    fs::create_dir(&system).unwrap();
    let anchor = system.join("anchor");
    private::create_dir(&anchor).unwrap();
    // Every signed-in account may change it, and it stays that way when its
    // owner is the system and `closed` above it is closed.
    set_access(&system, "D:P(A;OICI;FA;;;AU)");
    icacls(&system, &["/setowner", "*S-1-5-18"]);
    assert!(!private::owned_by_current_user(&system).unwrap());
    let _closed = Closed::own(&closed);
    assert!(!may_add_to(&closed), "{}", access_list(&closed));

    let canonical = private::establish_dir_durability(&anchor).unwrap();

    assert_eq!(
        identity::file_id(&canonical).unwrap(),
        identity::file_id(&anchor).unwrap()
    );

    // The same folder made the account's own: `closed` is flushed to keep
    // it, the account may not flush `closed`, and `system`'s entry is the
    // one left uncertain.
    let user = std::env::var("USERNAME").unwrap();
    icacls(&system, &["/setowner", &user]);
    assert!(private::owned_by_current_user(&system).unwrap());
    let system = fs::canonicalize(&system).unwrap();
    let established = private::establish_dir_durability(&anchor);
    assert!(
        matches!(
            &established,
            Err(private::DirectoryDurabilityError::Uncertain { path, source, .. })
                if *path == system && source.kind() == ErrorKind::PermissionDenied
        ),
        "{established:?}"
    );
    assert!(names(&anchor).is_empty());
}

/// A standard account is how most people run Hide, and the Windows runner's
/// account is an administrator, so the chain is made again on a thread that
/// acts with the account's token filtered the way UAC filters a standard
/// session. From the volume's root down to new folders in the profile, every
/// ancestor that account may add to is flushed and every other passed over.
#[cfg(windows)]
#[test]
fn a_standard_account_makes_and_establishes_a_durable_chain() {
    let outer = folder();
    let deepest = outer.path().join("state").join("ledger");

    let standard = Standard::act();
    assert!(!standard.is_administrator());
    let made = private::create_dir_all_durable(&deepest, outer.path());
    let established = private::establish_dir_durability(&deepest);
    drop(standard);

    made.unwrap();
    assert_eq!(
        identity::file_id(&established.unwrap()).unwrap(),
        identity::file_id(&deepest).unwrap()
    );
}

/// The calling thread acting with the account's `LUA_TOKEN`, the filtered
/// token of a standard session: the Administrators group only denies and
/// the administrator's privileges are gone. Dropping it ends the act on
/// every exit, a failed assertion included; it never leaves its thread.
#[cfg(windows)]
struct Standard(std::marker::PhantomData<*const ()>);

#[cfg(windows)]
impl Standard {
    fn act() -> Self {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::Security::{
            CreateRestrictedToken, ImpersonateLoggedOnUser, LUA_TOKEN, TOKEN_ASSIGN_PRIMARY,
            TOKEN_DUPLICATE, TOKEN_QUERY,
        };
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
        let mut process = std::ptr::null_mut();
        // SAFETY: the current-process pseudo handle is always valid.
        let opened = unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_ASSIGN_PRIMARY | TOKEN_DUPLICATE | TOKEN_QUERY,
                &mut process,
            )
        };
        assert_ne!(opened, 0, "{}", std::io::Error::last_os_error());
        let mut filtered = std::ptr::null_mut();
        // SAFETY: `process` is an open token; nothing else is disabled,
        // deleted or restricted, so every list is empty.
        let made = unsafe {
            CreateRestrictedToken(
                process,
                LUA_TOKEN,
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
                &mut filtered,
            )
        };
        let made_error = std::io::Error::last_os_error();
        // SAFETY: the handle is open and owned here.
        unsafe { CloseHandle(process) };
        assert_ne!(made, 0, "{made_error}");
        // SAFETY: `filtered` is an open primary token; the thread keeps its
        // own reference, so this one is closed either way.
        let acting = unsafe { ImpersonateLoggedOnUser(filtered) };
        let acting_error = std::io::Error::last_os_error();
        unsafe { CloseHandle(filtered) };
        assert_ne!(acting, 0, "{acting_error}");
        Self(std::marker::PhantomData)
    }

    /// Whether the token this thread acts with is in the Administrators
    /// group, which is what the act has to have taken away.
    fn is_administrator(&self) -> bool {
        use windows_sys::Win32::Security::{
            CheckTokenMembership, CreateWellKnownSid, SECURITY_MAX_SID_SIZE,
            WinBuiltinAdministratorsSid,
        };
        let mut sid = [0u8; SECURITY_MAX_SID_SIZE as usize];
        let mut size = sid.len() as u32;
        // SAFETY: `sid` holds `size` writable bytes.
        let made = unsafe {
            CreateWellKnownSid(
                WinBuiltinAdministratorsSid,
                std::ptr::null_mut(),
                sid.as_mut_ptr().cast(),
                &mut size,
            )
        };
        assert_ne!(made, 0, "{}", std::io::Error::last_os_error());
        let mut member = 0;
        // SAFETY: a null token checks the thread's impersonation token, and
        // `sid` is the SID just made.
        let checked = unsafe {
            CheckTokenMembership(std::ptr::null_mut(), sid.as_mut_ptr().cast(), &mut member)
        };
        assert_ne!(checked, 0, "{}", std::io::Error::last_os_error());
        member != 0
    }
}

#[cfg(windows)]
impl Drop for Standard {
    fn drop(&mut self) {
        use windows_sys::Win32::Security::RevertToSelf;
        // SAFETY: ends this thread's impersonation, which `act` began.
        if unsafe { RevertToSelf() } == 0 && !thread::panicking() {
            panic!("{}", std::io::Error::last_os_error());
        }
    }
}

/// A folder of the account's that it can read and enter but add nothing
/// to, made private again when the test is done with it so it can go.
struct Closed(PathBuf);

impl Closed {
    fn own(path: &Path) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o500)).unwrap();
        }
        // The owner's implied right to change the access list puts it back.
        #[cfg(windows)]
        read_only(path);
        Self(path.to_path_buf())
    }
}

impl Drop for Closed {
    fn drop(&mut self) {
        private::restrict_to_owner(&self.0).unwrap();
    }
}

/// Replaces the access list of `folder` with one that lets signed-in
/// accounts read and enter it and nobody add to it, and inherits nothing.
/// `icacls /inheritance:r` in one call with a grant kept what was inherited.
#[cfg(windows)]
fn read_only(folder: &Path) {
    set_access(folder, "D:P(A;;FRFX;;;AU)");
}

/// Replaces the access list of `folder` with the one an access-list string
/// spells, as given.
#[cfg(windows)]
fn set_access(folder: &Path, list: &str) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{ERROR_SUCCESS, LocalFree};
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1, SE_FILE_OBJECT,
        SetNamedSecurityInfoW,
    };
    use windows_sys::Win32::Security::{
        DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl, PROTECTED_DACL_SECURITY_INFORMATION,
    };
    let text: Vec<u16> = list.encode_utf16().chain([0]).collect();
    let name: Vec<u16> = folder.as_os_str().encode_wide().chain([0]).collect();
    let mut descriptor = std::ptr::null_mut();
    // SAFETY: `text` is NUL-terminated and `descriptor` receives a
    // LocalAlloc buffer freed below.
    let made = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            text.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    };
    assert_ne!(made, 0, "{}", std::io::Error::last_os_error());
    let (mut present, mut defaulted) = (0, 0);
    let mut acl = std::ptr::null_mut();
    // SAFETY: `descriptor` is the one just made, and the list it returns
    // lies inside it.
    let read =
        unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut acl, &mut defaulted) };
    let read_error = std::io::Error::last_os_error();
    // SAFETY: `name` is NUL-terminated and `acl` lives until the free below.
    let status = unsafe {
        SetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            acl,
            std::ptr::null(),
        )
    };
    // SAFETY: the buffer came from the conversion above and nothing uses it
    // after this.
    unsafe { LocalFree(descriptor) };
    assert!(read != 0 && present != 0, "{read_error}");
    assert_eq!(
        status,
        ERROR_SUCCESS,
        "{}",
        std::io::Error::from_raw_os_error(status as i32)
    );
}

/// Whether the system lets the account open `folder` to add a file or a
/// folder to it. It asks without backup intent, as the barrier does: the
/// runner's account holds the backup and restore privileges, and those open
/// anything for a caller that asks with it.
#[cfg(windows)]
fn may_add_to(folder: &Path) -> bool {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
    use windows_sys::Wdk::Storage::FileSystem::{
        FILE_DIRECTORY_FILE, FILE_OPEN, FILE_SYNCHRONOUS_IO_NONALERT, NtCreateFile,
    };
    use windows_sys::Win32::Foundation::{RtlNtStatusToDosError, UNICODE_STRING};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        SYNCHRONIZE,
    };
    use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;
    let read = open_dir(folder);
    // The folder itself: an empty name relative to its open handle.
    let name = UNICODE_STRING {
        Length: 0,
        MaximumLength: 0,
        Buffer: std::ptr::null_mut(),
    };
    let attributes = OBJECT_ATTRIBUTES {
        Length: std::mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: read.as_raw_handle(),
        ObjectName: &name,
        ..Default::default()
    };
    [FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY]
        .into_iter()
        .any(|access| {
            let mut handle = std::ptr::null_mut();
            let mut status_block = IO_STATUS_BLOCK::default();
            // SAFETY: the root handle is open for the call and every pointer
            // is to a local that outlives it.
            let status = unsafe {
                NtCreateFile(
                    &mut handle,
                    access | SYNCHRONIZE,
                    &attributes,
                    &mut status_block,
                    std::ptr::null(),
                    0,
                    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                    FILE_OPEN,
                    FILE_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT,
                    std::ptr::null(),
                    0,
                )
            };
            if status >= 0 {
                // SAFETY: the call returned a new handle that nothing else owns.
                drop(unsafe { OwnedHandle::from_raw_handle(handle) });
                return true;
            }
            // SAFETY: the conversion reads only its argument.
            let error =
                std::io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) } as i32);
            assert_eq!(
                error.kind(),
                ErrorKind::PermissionDenied,
                "opening {} to add to it: {error}",
                folder.display()
            );
            false
        })
}

/// The access list of `path` as `icacls` prints it, for a failure message.
#[cfg(windows)]
fn access_list(path: &Path) -> String {
    let output = Command::new("icacls").arg(path).output().unwrap();
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[cfg(windows)]
fn icacls(path: &Path, arguments: &[&str]) {
    let status = Command::new("icacls")
        .arg(path)
        .args(arguments)
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "icacls {arguments:?} failed: {status}");
}

/// An open folder, the way a caller that holds folders (not names) has one.
fn open_dir(path: &Path) -> File {
    hide_platform::fs::open_dir(path).unwrap()
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

fn names(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

/// Gives everyone read access (`write` false) or write access to `path`,
/// the way an operator or another program might.
fn widen(path: &Path, write: bool) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let directory = fs::metadata(path).unwrap().is_dir();
        let mode = match (write, directory) {
            (true, true) => 0o777,
            (true, false) => 0o666,
            (false, true) => 0o755,
            (false, false) => 0o644,
        };
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }
    #[cfg(windows)]
    {
        let grant = if write {
            "Everyone:(M)"
        } else {
            "Everyone:(R)"
        };
        let status = Command::new("icacls")
            .arg(path)
            .args(["/grant", grant])
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
    }
}

// ---- private ------------------------------------------------------------

#[test]
fn inspection_accepts_protected_directories_and_refuses_modifiable_entries_without_changes() {
    let outer = folder();
    let directory = outer.path().join("state");
    private::create_dir(&directory).unwrap();
    let kept = directory.join("keep");
    fs::write(&kept, b"preserved state").unwrap();
    let before = identity::file_id(&directory).unwrap();
    assert_eq!(
        private::inspect_directory_entry(&directory).unwrap(),
        private::InspectionDirectory::Real
    );
    assert!(private::inspect_directory_entry(&kept).is_err());
    assert_eq!(identity::file_id(&directory).unwrap(), before);
    assert_eq!(read(&kept), "preserved state");
    widen(&directory, true);
    assert!(private::inspect_directory_entry(&directory).is_err());
    assert_eq!(identity::file_id(&directory).unwrap(), before);
    assert_eq!(read(&kept), "preserved state");
    assert_eq!(names(&directory), ["keep"]);
}

#[cfg(unix)]
#[test]
fn inspection_returns_only_system_owned_aliases_without_following_their_targets() {
    use std::os::unix::fs::MetadataExt;

    let outer = folder();
    let target = outer.path().join("absent-target");
    let alias = outer.path().join("alias");
    link::create_link(&target, &alias).unwrap();
    let before = fs::read_link(&alias).unwrap();
    let result = private::inspect_directory_entry(&alias);
    if fs::symlink_metadata(&alias).unwrap().uid() == 0 {
        assert_eq!(
            result.unwrap(),
            private::InspectionDirectory::SystemAlias(target.clone())
        );
    } else {
        assert!(result.is_err());
    }
    assert_eq!(fs::read_link(&alias).unwrap(), before);
    assert!(!target.exists());
    assert_eq!(names(outer.path()), ["alias"]);
}

#[cfg(unix)]
#[test]
fn inspection_accepts_sticky_system_directories_and_reports_system_alias_targets() {
    use std::os::unix::fs::MetadataExt;

    let metadata = fs::symlink_metadata("/tmp").unwrap();
    assert_eq!(metadata.uid(), 0);
    if metadata.file_type().is_symlink() {
        assert_eq!(
            private::inspect_directory_entry(Path::new("/tmp")).unwrap(),
            private::InspectionDirectory::SystemAlias(fs::read_link("/tmp").unwrap())
        );
    } else {
        assert_eq!(
            private::inspect_directory_entry(Path::new("/tmp")).unwrap(),
            private::InspectionDirectory::Real
        );
    }
    assert_eq!(fs::symlink_metadata("/tmp").unwrap().uid(), metadata.uid());
}

/// The sticky rule: others may add entries of their own to a system folder
/// above Hide's state, as anyone may to `/tmp` and every signed-in account
/// may to a Windows drive's root, but a folder that lets them delete,
/// rename or re-permission what is in it is refused. A drive's root also
/// hands every signed-in account the right to change a folder made in it,
/// so such a folder is refused, while Hide's own private folder there,
/// which inherits nothing, is not.
#[test]
fn inspection_lets_others_add_to_a_system_folder_but_not_delete_from_it() {
    use private::InspectionDirectory::Real;

    #[cfg(unix)]
    {
        // `/private/tmp` on macOS, where `/tmp` is the system's alias.
        let sticky = fs::canonicalize("/tmp").unwrap();
        assert_eq!(private::inspect_directory_entry(&sticky).unwrap(), Real);
    }
    #[cfg(windows)]
    {
        let root = PathBuf::from(format!("{}\\", std::env::var("SystemDrive").unwrap()));
        assert_eq!(
            private::inspect_directory_entry(&root).unwrap(),
            Real,
            "{}",
            access_list(&root)
        );

        // A drive's root in small: the system's, every signed-in account
        // may read it and add a folder, and what is made in it inherits
        // their right to change it. Handing a folder to the system takes an
        // administrator, which the Windows runner's account is.
        let outer = folder();
        let added = outer.path().join("added");
        fs::create_dir(&added).unwrap();
        icacls(&added, &["/setowner", "*S-1-5-18"]);
        set_access(
            &added,
            "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;;FRFX;;;AU)(A;;0x4;;;AU)(A;OICIIO;0x1301bf;;;AU)",
        );
        let deleted = outer.path().join("deleted");
        fs::create_dir(&deleted).unwrap();
        icacls(&deleted, &["/setowner", "*S-1-5-18"]);
        set_access(
            &deleted,
            "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;;FRFX;;;AU)(A;;0x44;;;AU)",
        );
        let handed = added.join("handed");
        fs::create_dir(&handed).unwrap();
        let own = added.join("own");
        private::create_dir(&own).unwrap();

        assert_eq!(
            private::inspect_directory_entry(&added).unwrap(),
            Real,
            "{}",
            access_list(&added)
        );
        assert!(
            private::inspect_directory_entry(&deleted).is_err(),
            "{}",
            access_list(&deleted)
        );
        assert!(
            private::inspect_directory_entry(&handed).is_err(),
            "{}",
            access_list(&handed)
        );
        assert_eq!(
            private::inspect_directory_entry(&own).unwrap(),
            Real,
            "{}",
            access_list(&own)
        );
    }
}

#[test]
fn a_private_folder_is_the_current_accounts_alone() {
    let outer = folder();
    let made = outer.path().join("state");
    private::create_dir(&made).unwrap();
    assert!(made.is_dir());
    assert!(private::is_private(&made).unwrap());
    assert!(!private::others_can_modify(&made).unwrap());
    assert!(private::owned_by_current_user(&made).unwrap());
    // Making it again says it was already there, so a caller can tell a
    // folder it made from one it found.
    let error = private::create_dir(&made).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::AlreadyExists);
}

#[test]
fn private_folders_are_made_down_to_the_last_and_an_existing_one_is_left_alone() {
    let outer = folder();
    let deep = outer.path().join("state").join("hide").join("attachments");
    private::create_dir_all(&deep).unwrap();
    for made in [
        outer.path().join("state"),
        outer.path().join("state").join("hide"),
        deep.clone(),
    ] {
        assert!(private::is_private(&made).unwrap(), "{}", made.display());
    }
    private::create_dir_all(&deep).unwrap();
    let shared = outer.path().join("shared");
    fs::create_dir(&shared).unwrap();
    widen(&shared, false);
    private::create_dir_all(&shared).unwrap();
    assert!(!private::is_private(&shared).unwrap());
    fs::write(outer.path().join("file"), "").unwrap();
    assert!(private::create_dir_all(&outer.path().join("file")).is_err());
}

#[test]
fn an_owned_folder_is_made_private_whatever_an_earlier_program_left() {
    let outer = folder();
    let state = outer.path().join("hide").join("state");
    assert_eq!(private::own_dir(&state).unwrap(), private::OwnDir::Made);
    for made in [outer.path().join("hide"), state.clone()] {
        assert!(private::is_private(&made).unwrap(), "{}", made.display());
    }
    assert_eq!(private::own_dir(&state).unwrap(), private::OwnDir::Private);
    // A folder an earlier build made the way `fs::create_dir_all` makes one,
    // and one others can even write to, is the account's own and is closed.
    for write in [false, true] {
        widen(&state, write);
        fs::write(state.join("kept"), "kept").unwrap();
        assert_eq!(
            private::own_dir(&state).unwrap(),
            private::OwnDir::Restricted
        );
        assert!(private::is_private(&state).unwrap());
        assert!(!private::others_can_modify(&state).unwrap());
        assert_eq!(read(&state.join("kept")), "kept");
    }
}

/// Its mode changes only through an open descriptor, which a folder closed
/// to its own owner does not give: it is refused with the fix, unchanged.
#[cfg(unix)]
#[test]
fn an_owned_folder_closed_to_its_owner_is_refused_with_its_fix() {
    use std::os::unix::fs::PermissionsExt;

    let outer = folder();
    let closed = outer.path().join("state");
    fs::create_dir(&closed).unwrap();
    fs::set_permissions(&closed, fs::Permissions::from_mode(0o000)).unwrap();
    let refused = private::own_dir(&closed);
    let mode = fs::metadata(&closed).unwrap().permissions().mode() & 0o777;
    // Opened again so the folder can be removed with its owner.
    fs::set_permissions(&closed, fs::Permissions::from_mode(0o700)).unwrap();
    let error = refused.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::PermissionDenied);
    assert!(
        error.to_string().starts_with(&closed.display().to_string())
            && error.to_string().contains("`chmod 700`"),
        "{error}"
    );
    assert_eq!(mode, 0o000, "the refused folder was changed");
}

#[test]
fn a_link_or_a_file_where_an_owned_folder_belongs_is_refused_and_left_as_it_is() {
    let outer = folder();
    let target = outer.path().join("target");
    fs::create_dir(&target).unwrap();
    widen(&target, false);
    let planted = outer.path().join("link");
    link::create_link(&target, &planted).unwrap();
    let error = private::own_dir(&planted).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::PermissionDenied);
    assert!(
        error.to_string().ends_with("is a link or not a folder"),
        "{error}"
    );
    assert!(
        fs::symlink_metadata(&planted)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(!private::is_private(&target).unwrap());
    let file = outer.path().join("file");
    fs::write(&file, "kept").unwrap();
    assert_eq!(
        private::own_dir(&file).unwrap_err().kind(),
        ErrorKind::PermissionDenied
    );
    assert_eq!(read(&file), "kept");
}

#[test]
fn durable_bootstrap_refuses_escape_and_over_budget_paths_before_creation() {
    use private::DirectoryDurabilityError;

    let outer = folder();
    let mut too_deep = outer.path().to_path_buf();
    for _ in 0..65 {
        too_deep.push("child");
    }
    for invalid in [
        too_deep,
        outer.path().join("first").join("..").join("escape"),
        outer.path().parent().unwrap().join("outside"),
    ] {
        assert!(matches!(
            private::create_dir_all_durable(&invalid, outer.path()),
            Err(DirectoryDurabilityError::BeforeCreate { source })
                if source.kind() == ErrorKind::InvalidInput
        ));
        assert!(names(outer.path()).is_empty());
    }
    assert!(matches!(
        private::create_dir_all_durable(Path::new("relative/child"), Path::new("relative")),
        Err(DirectoryDurabilityError::BeforeCreate { source })
            if source.kind() == ErrorKind::InvalidInput
    ));
}

#[test]
fn durable_bootstrap_creates_a_private_chain_up_to_its_bound_and_rechecks_it() {
    let outer = folder();
    let mut deepest = outer.path().to_path_buf();
    for _ in 0..64 {
        deepest.push("d");
    }
    // The supplied ancestor boundary is the caller's precondition. These
    // outcomes establish checked OS operations, not physical power loss.
    private::create_dir_all_durable(&deepest, outer.path()).unwrap();
    let mut checked = deepest.as_path();
    while checked != outer.path() {
        assert!(private::is_private(checked).unwrap());
        assert!(private::owned_by_current_user(checked).unwrap());
        checked = checked.parent().unwrap();
    }
    private::create_dir_all_durable(&deepest, outer.path()).unwrap();
    private::create_dir_all_durable(outer.path(), outer.path()).unwrap();
}

#[test]
fn durable_bootstrap_preserves_existing_permissions_and_refuses_untrusted_directories() {
    use private::DirectoryDurabilityError;

    let outer = folder();
    let shared_read = outer.path().join("readable");
    private::create_dir(&shared_read).unwrap();
    widen(&shared_read, false);
    let child = shared_read.join("private");
    private::create_dir_all_durable(&child, outer.path()).unwrap();
    assert!(!private::is_private(&shared_read).unwrap());
    assert!(private::is_private(&child).unwrap());
    let writable = outer.path().join("writable");
    private::create_dir(&writable).unwrap();
    widen(&writable, true);
    assert!(matches!(
        private::create_dir_all_durable(&writable.join("refused"), outer.path()),
        Err(DirectoryDurabilityError::Uncertain { path, source, .. })
            if path == writable && source.kind() == ErrorKind::PermissionDenied
    ));
    assert!(names(&writable).is_empty());
    assert!(private::others_can_modify(&writable).unwrap());
}

#[test]
fn durable_bootstrap_never_follows_a_link_or_removes_existing_entries_on_failure() {
    use private::DirectoryDurabilityError;

    let outer = folder();
    let target = outer.path().join("target");
    private::create_dir(&target).unwrap();
    let planted = outer.path().join("link");
    link::create_link(&target, &planted).unwrap();
    assert!(matches!(
        private::create_dir_all_durable(&planted.join("refused"), outer.path()),
        Err(DirectoryDurabilityError::Uncertain { path, source, .. })
            if path == planted && source.kind() == ErrorKind::PermissionDenied
    ));
    assert!(names(&target).is_empty());
    assert!(
        fs::symlink_metadata(&planted)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let file = outer.path().join("file");
    fs::write(&file, "kept").unwrap();
    assert!(matches!(
        private::create_dir_all_durable(&file.join("refused"), outer.path()),
        Err(DirectoryDurabilityError::Uncertain { source, .. })
            if source.kind() == ErrorKind::PermissionDenied
    ));
    assert_eq!(read(&file), "kept");
    assert!(matches!(
        private::create_dir_all_durable(&planted, &planted),
        Err(DirectoryDurabilityError::BeforeCreate { source })
            if source.kind() == ErrorKind::PermissionDenied
    ));
}

#[test]
fn a_private_file_is_the_current_accounts_alone_and_is_not_made_twice() {
    let outer = folder();
    let made = outer.path().join("secret");
    {
        use std::io::Write;
        let mut file = private::create_new_file(&made).unwrap();
        file.write_all(b"token").unwrap();
    }
    assert_eq!(read(&made), "token");
    assert!(private::is_private(&made).unwrap());
    assert!(private::owned_by_current_user(&made).unwrap());
    assert_eq!(
        private::create_new_file(&made).unwrap_err().kind(),
        ErrorKind::AlreadyExists
    );
}

#[test]
fn opening_a_private_file_keeps_what_it_holds() {
    let outer = folder();
    let path = outer.path().join(".lock");
    drop(private::open_or_create_file(&path).unwrap());
    assert!(private::is_private(&path).unwrap());
    fs::write(&path, "kept").unwrap();
    drop(private::open_or_create_file(&path).unwrap());
    assert_eq!(read(&path), "kept");
}

#[test]
fn a_folder_others_can_read_is_not_private_and_one_they_can_write_can_be_modified() {
    let outer = folder();
    let made = outer.path().join("shared");
    private::create_dir(&made).unwrap();
    widen(&made, false);
    assert!(!private::is_private(&made).unwrap());
    assert!(!private::others_can_modify(&made).unwrap());
    widen(&made, true);
    assert!(!private::is_private(&made).unwrap());
    assert!(private::others_can_modify(&made).unwrap());
    // Restricting it puts it back.
    private::restrict_to_owner(&made).unwrap();
    assert!(private::is_private(&made).unwrap());
    assert!(!private::others_can_modify(&made).unwrap());
}

#[test]
fn restricting_a_file_makes_it_private() {
    let outer = folder();
    let path = outer.path().join("a.txt");
    fs::write(&path, "x").unwrap();
    widen(&path, true);
    assert!(private::others_can_modify(&path).unwrap());
    private::restrict_to_owner(&path).unwrap();
    assert!(private::is_private(&path).unwrap());
    assert_eq!(read(&path), "x");
}

#[test]
fn ownership_is_answered_for_a_name_and_for_an_open_file_alike() {
    let outer = folder();
    let path = outer.path().join("mine.txt");
    fs::write(&path, "x").unwrap();
    let file = File::open(&path).unwrap();
    assert!(private::owned_by_current_user(&path).unwrap());
    assert!(private::handle_owned_by_current_user(&file).unwrap());
    let dir = open_dir(outer.path());
    assert!(private::handle_owned_by_current_user(&dir).unwrap());
    assert_eq!(
        private::owned_by_current_user(&outer.path().join("missing"))
            .unwrap_err()
            .kind(),
        ErrorKind::NotFound
    );
}

#[test]
fn a_file_made_private_is_readable_by_its_owner_and_runs_when_asked_to() {
    let outer = folder();
    let script = outer.path().join("run.cmd");
    atomic::write_file(&script, b"echo", Access::PrivateExecutable).unwrap();
    assert!(permissions::is_executable(&script).unwrap());
    assert!(private::is_private(&script).unwrap());
    let notes = outer.path().join("notes.txt");
    atomic::write_file(&notes, b"echo", Access::Private).unwrap();
    assert!(!permissions::is_executable(&notes).unwrap());
    assert!(private::is_private(&notes).unwrap());
    assert_eq!(read(&script), "echo");
}

#[test]
fn a_folder_of_ones_own_takes_new_files_and_a_file_is_not_a_folder() {
    let outer = folder();
    assert!(permissions::account_can_add_to(outer.path()).unwrap());
    let file = outer.path().join("plain");
    std::fs::write(&file, b"x").unwrap();
    assert!(permissions::account_can_add_to(&file).is_err());
    assert_eq!(
        permissions::account_can_add_to(&outer.path().join("missing"))
            .unwrap_err()
            .kind(),
        ErrorKind::NotFound
    );
    assert_eq!(std::fs::read_dir(outer.path()).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn a_folder_without_write_permission_takes_no_new_file() {
    use std::os::unix::fs::PermissionsExt;
    let outer = folder();
    let locked = outer.path().join("locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
    let answer = permissions::account_can_add_to(&locked).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    // root writes anywhere, which is the kernel's answer too.
    assert_eq!(answer, unsafe { libc::geteuid() } == 0);
}

// ---- lock ---------------------------------------------------------------

fn never() -> bool {
    false
}

/// Sets the flag when it goes out of scope, so a reader thread that waits for
/// it ends when the writer beside it panics, instead of the test hanging.
struct SetOnDrop<'a>(&'a AtomicBool);

impl Drop for SetOnDrop<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// A child that does not outlive the test that started it.
struct Killed(std::process::Child);

impl Drop for Killed {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn locked(waited: Waited) -> lock::Lock {
    match waited {
        Waited::Locked(held) => held,
        other => panic!("expected the lock, got {other:?}"),
    }
}

#[test]
fn an_exclusive_lock_on_a_folder_keeps_every_other_lock_out_until_it_is_dropped() {
    let outer = folder();
    let first = open_dir(outer.path());
    let second = open_dir(outer.path());
    let held = locked(lock::lock_dir(&first, Mode::Exclusive, Duration::ZERO, &never).unwrap());
    for mode in [Mode::Exclusive, Mode::Shared] {
        assert!(matches!(
            lock::lock_dir(&second, mode, Duration::ZERO, &never).unwrap(),
            Waited::TimedOut
        ));
    }
    drop(held);
    locked(lock::lock_dir(&second, Mode::Exclusive, Duration::ZERO, &never).unwrap());
}

#[test]
fn shared_locks_on_a_folder_admit_each_other_and_keep_an_exclusive_one_out() {
    let outer = folder();
    let (one, two, three) = (
        open_dir(outer.path()),
        open_dir(outer.path()),
        open_dir(outer.path()),
    );
    let first = locked(lock::lock_dir(&one, Mode::Shared, Duration::ZERO, &never).unwrap());
    let second = locked(lock::lock_dir(&two, Mode::Shared, Duration::ZERO, &never).unwrap());
    assert!(matches!(
        lock::lock_dir(&three, Mode::Exclusive, Duration::ZERO, &never).unwrap(),
        Waited::TimedOut
    ));
    drop((first, second));
    locked(lock::lock_dir(&three, Mode::Exclusive, Duration::ZERO, &never).unwrap());
}

#[test]
fn a_lock_on_a_file_is_taken_through_the_file_that_stays_open() {
    let outer = folder();
    let path = outer.path().join(".lock");
    let open = || {
        fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .unwrap()
    };
    let held = lock::lock_file_blocking(open(), Mode::Exclusive).unwrap();
    assert!(matches!(
        lock::lock_file(open(), Mode::Exclusive, Duration::ZERO, &never).unwrap(),
        Waited::TimedOut
    ));
    drop(held);
    locked(lock::lock_file(open(), Mode::Exclusive, Duration::ZERO, &never).unwrap());
}

/// A child another thread starts while the lock is held has a copy of its
/// descriptor until it starts its program. Here the child stops in that
/// moment, after the fork and before its program, until the test lets it go;
/// the lock dropped meanwhile is free at once to a single try (issue 820).
#[cfg(unix)]
#[test]
fn a_dropped_lock_is_free_while_a_child_started_meanwhile_still_holds_its_descriptor() {
    use std::io::Write;
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;

    let outer = folder();
    let path = outer.path().join(".lock");
    let open = || {
        fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .unwrap()
    };
    let held = locked(lock::lock_file(open(), Mode::Exclusive, Duration::ZERO, &never).unwrap());
    let (mut forked, forked_writer) = std::io::pipe().unwrap();
    let (go_reader, mut go) = std::io::pipe().unwrap();
    let (forked_fd, go_fd) = (forked_writer.as_raw_fd(), go_reader.as_raw_fd());
    let mut command = Command::new("/bin/sh");
    command.args(["-c", "exit 0"]);
    // SAFETY: the closure runs in the forked child before its program and
    // calls only `write` and `read`, which are async-signal-safe.
    unsafe {
        command.pre_exec(move || {
            let mut byte = [1u8];
            libc::write(forked_fd, byte.as_ptr().cast(), 1);
            libc::read(go_fd, byte.as_mut_ptr().cast(), 1);
            Ok(())
        });
    }
    let _ends = (forked_writer, go_reader);
    thread::scope(|scope| {
        // `spawn` returns only once the child has started its program, so it
        // runs on a thread of its own, as a concurrent start does.
        let starter = scope.spawn(move || command.spawn().unwrap());
        let mut byte = [0u8];
        forked.read_exact(&mut byte).unwrap();
        drop(held);
        let free = lock::lock_file(open(), Mode::Exclusive, Duration::ZERO, &never).unwrap();
        go.write_all(&[1]).unwrap();
        starter.join().unwrap().wait().unwrap();
        locked(free);
    });
}

/// A wait asks `cancelled` after every try that found the lock held, so its
/// first question is the waiter's own report that it is waiting.
#[test]
fn a_wait_ends_when_the_holder_lets_go_and_not_before() {
    let outer = folder();
    let first = open_dir(outer.path());
    let second = open_dir(outer.path());
    let held = locked(lock::lock_dir(&first, Mode::Exclusive, Duration::ZERO, &never).unwrap());
    let (waiting, heard) = std::sync::mpsc::channel();
    thread::scope(|scope| {
        let waiter = scope.spawn(|| {
            lock::lock_dir(&second, Mode::Exclusive, Duration::from_secs(20), &|| {
                let _ = waiting.send(());
                false
            })
            .unwrap()
        });
        heard
            .recv_timeout(Duration::from_secs(20))
            .expect("the waiter never found the lock held");
        assert!(
            !waiter.is_finished(),
            "the waiter stopped while it was held"
        );
        drop(held);
        locked(waiter.join().unwrap());
    });
}

#[test]
fn a_wait_that_is_not_answered_times_out_after_the_time_it_was_given() {
    let outer = folder();
    let first = open_dir(outer.path());
    let second = open_dir(outer.path());
    let _held = locked(lock::lock_dir(&first, Mode::Exclusive, Duration::ZERO, &never).unwrap());
    let started = Instant::now();
    assert!(matches!(
        lock::lock_dir(&second, Mode::Exclusive, Duration::from_millis(300), &never).unwrap(),
        Waited::TimedOut
    ));
    assert!(started.elapsed() >= Duration::from_millis(300));
}

/// The caller answers "go on" to the first question and "stop" to the second,
/// so the wait has gone round once before it is cancelled.
#[test]
fn a_wait_stops_when_the_caller_cancels_it() {
    let outer = folder();
    let first = open_dir(outer.path());
    let second = open_dir(outer.path());
    let _held = locked(lock::lock_dir(&first, Mode::Exclusive, Duration::ZERO, &never).unwrap());
    let asked = std::sync::atomic::AtomicUsize::new(0);
    let waited = lock::lock_dir(&second, Mode::Exclusive, Duration::from_secs(30), &|| {
        asked.fetch_add(1, Ordering::Relaxed) == 1
    })
    .unwrap();
    assert!(matches!(waited, Waited::Cancelled));
    assert_eq!(asked.load(Ordering::Relaxed), 2);
}

/// A folder opened the way `cap-std` opens one on Linux, with `O_PATH`: the
/// kernel refuses to lock or flush such a descriptor, so a lock through it has
/// to reopen the folder. That was the bug that refused every save on Linux.
#[cfg(target_os = "linux")]
#[test]
fn a_folder_handle_that_cannot_itself_be_locked_is_locked_through_a_fresh_one() {
    use std::os::fd::{FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    let outer = folder();
    let path = std::ffi::CString::new(outer.path().as_os_str().as_bytes()).unwrap();
    // SAFETY: a NUL-terminated path and flags; the result is checked.
    let fd = unsafe { libc_open_path(path.as_ptr()) };
    assert!(fd >= 0);
    // SAFETY: `open` returned a new descriptor that nothing else owns.
    let path_only = unsafe { OwnedFd::from_raw_fd(fd) };
    let held = locked(lock::lock_dir(&path_only, Mode::Exclusive, Duration::ZERO, &never).unwrap());
    let other = open_dir(outer.path());
    assert!(matches!(
        lock::lock_dir(&other, Mode::Exclusive, Duration::ZERO, &never).unwrap(),
        Waited::TimedOut
    ));
    drop(held);
    atomic::sync_dir(&path_only).unwrap();
}

#[cfg(target_os = "linux")]
unsafe extern "C" {
    #[link_name = "open"]
    fn libc_open(path: *const std::ffi::c_char, flags: i32, ...) -> i32;
}

#[cfg(target_os = "linux")]
unsafe fn libc_open_path(path: *const std::ffi::c_char) -> i32 {
    const O_DIRECTORY: i32 = 0o200000;
    const O_PATH: i32 = 0o10000000;
    const O_CLOEXEC: i32 = 0o2000000;
    // SAFETY: forwarded from the caller.
    unsafe { libc_open(path, O_DIRECTORY | O_PATH | O_CLOEXEC) }
}

#[test]
#[allow(clippy::disallowed_methods)] // a child process the test kills later: it sleeps to stay alive
fn lock_role() {
    if std::env::var(ROLE).as_deref() != Ok("lock") {
        return;
    }
    let held = std::env::var(HELD).expect("the role names what to hold");
    let path = PathBuf::from(held);
    let waited = if path.is_dir() {
        lock::lock_dir(&open_dir(&path), Mode::Exclusive, Duration::ZERO, &never)
    } else {
        lock::lock_file(
            File::options().write(true).open(&path).unwrap(),
            Mode::Exclusive,
            Duration::ZERO,
            &never,
        )
    };
    let _held = locked(waited.unwrap());
    println!("READY");
    thread::sleep(Duration::from_secs(60));
}

fn hold_in_another_process(path: &Path) -> Killed {
    let mut child = Killed(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "fs::lock_role",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(ROLE, "lock")
            .env(HELD, path)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = child.0.stdout.take().unwrap();
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        if line.contains("READY") {
            // The reader is dropped; the child's later output has nowhere to
            // go and it prints nothing more.
            return child;
        }
    }
    panic!("the holder never took the lock");
}

#[test]
fn a_lock_held_by_another_process_ends_with_that_process() {
    let outer = folder();
    let file = outer.path().join(".lock");
    fs::write(&file, "").unwrap();
    for target in [outer.path().to_path_buf(), file] {
        let mut holder = hold_in_another_process(&target);
        let attempt = |wait: Duration| {
            if target.is_dir() {
                lock::lock_dir(&open_dir(&target), Mode::Exclusive, wait, &never)
            } else {
                lock::lock_file(
                    File::options().write(true).open(&target).unwrap(),
                    Mode::Exclusive,
                    wait,
                    &never,
                )
            }
            .unwrap()
        };
        assert!(matches!(attempt(Duration::ZERO), Waited::TimedOut));
        holder.0.kill().unwrap();
        holder.0.wait().unwrap();
        // The system releases it as the process goes; allow it a moment.
        locked(attempt(Duration::from_secs(10)));
    }
}

// ---- atomic -------------------------------------------------------------

#[test]
fn a_durable_private_write_acknowledges_whole_bytes_and_the_installed_identity() {
    let outer = folder();
    let path = outer.path().join("ledger.json");
    // This contract observes the file operation's OS acknowledgement, not
    // power-loss persistence of the test's newly created ancestor chain.
    let first = atomic::write_file_durable(&path, b"first intent", DurableAccess::Private).unwrap();
    assert_eq!(read(&path), "first intent");
    assert_eq!(first, identity::file_id(&path).unwrap());
    assert!(private::is_private(&path).unwrap());
    assert!(private::owned_by_current_user(&path).unwrap());

    let second =
        atomic::write_file_durable(&path, b"second intent", DurableAccess::Private).unwrap();
    assert_eq!(read(&path), "second intent");
    assert_eq!(second, identity::file_id(&path).unwrap());
    assert_ne!(first, second);
    assert!(private::is_private(&path).unwrap());
    assert_eq!(names(outer.path()), ["ledger.json"]);
}

#[test]
fn a_durable_write_with_no_parent_fails_before_replacement_without_making_state() {
    let outer = folder();
    let old = outer.path().join("kept");
    fs::write(&old, b"old").unwrap();
    let path = outer.path().join("missing").join("ledger.json");

    let error = atomic::write_file_durable(&path, b"new", DurableAccess::Private).unwrap_err();

    match error {
        atomic::DurableWriteError::BeforeReplace { source, cleanup } => {
            assert_eq!(source.kind(), ErrorKind::NotFound);
            assert!(cleanup.is_none());
        }
        other => panic!("expected failure before replacement: {other}"),
    }
    assert!(!path.exists());
    assert_eq!(read(&old), "old");
    assert_eq!(names(outer.path()), ["kept"]);
}

#[test]
fn a_real_failed_durable_replacement_reports_uncertainty_and_cleans_only_its_temp() {
    let outer = folder();
    let target = outer.path().join("directory");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("keep"), b"existing child").unwrap();

    let error = atomic::write_file_durable(&target, b"new", DurableAccess::Private).unwrap_err();

    match error {
        atomic::DurableWriteError::ReplacementUncertain { source, cleanup } => {
            assert!(source.raw_os_error().is_some());
            assert!(cleanup.is_none());
        }
        other => panic!("expected uncertain replacement: {other}"),
    }
    assert_eq!(read(&target.join("keep")), "existing child");
    assert_eq!(names(outer.path()), ["directory"]);
}

/// A durable write never keeps the access of the file it replaces: what it
/// installs is the account's alone, and a program is runnable by its owner
/// on Unix (Windows runs a file by its extension).
#[test]
fn a_durable_program_replaces_a_shared_file_with_a_private_one() {
    let outer = folder();
    let program = outer.path().join("program");
    fs::write(&program, b"old").unwrap();
    widen(&program, false);
    assert!(!private::is_private(&program).unwrap());

    let installed = atomic::write_file_durable(
        &program,
        b"private program",
        DurableAccess::PrivateExecutable,
    )
    .unwrap();

    assert_eq!(read(&program), "private program");
    assert_eq!(identity::file_id(&program).unwrap(), installed);
    assert!(private::is_private(&program).unwrap());
    #[cfg(unix)]
    assert_eq!(
        Permissions::of(&File::open(&program).unwrap())
            .unwrap()
            .unix_mode(),
        Some(0o700)
    );
    assert_eq!(names(outer.path()), ["program"]);
}

#[test]
fn a_written_file_replaces_the_old_one_whole_and_leaves_nothing_beside_it() {
    let outer = folder();
    let path = outer.path().join("settings.json");
    fs::write(&path, "old").unwrap();
    let id = atomic::write_file(&path, b"new contents", Access::KeepOrPrivate).unwrap();
    assert_eq!(read(&path), "new contents");
    assert_eq!(id, identity::file_id(&path).unwrap());
    assert_eq!(names(outer.path()), ["settings.json"]);
}

#[test]
fn a_new_file_is_private_and_a_kept_one_keeps_what_it_had() {
    let outer = folder();
    let fresh = outer.path().join("fresh");
    atomic::write_file(&fresh, b"a", Access::KeepOrPrivate).unwrap();
    assert!(private::is_private(&fresh).unwrap());

    let existing = outer.path().join("existing");
    fs::write(&existing, "old").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&existing, fs::Permissions::from_mode(0o640)).unwrap();
    }
    let before = Permissions::of(&File::open(&existing).unwrap()).unwrap();
    atomic::write_file(&existing, b"new", Access::KeepOrPrivate).unwrap();
    let after = Permissions::of(&File::open(&existing).unwrap()).unwrap();
    assert_eq!(before, after);
    #[cfg(unix)]
    assert_eq!(after.unix_mode(), Some(0o640));
    assert!(after.owner_can_write());
}

#[test]
fn a_write_that_cannot_be_made_leaves_the_old_file_and_no_temporary() {
    let outer = folder();
    let path = outer.path().join("kept");
    fs::write(&path, "old").unwrap();
    let impossible = outer.path().join("missing-folder").join("kept");
    assert_eq!(
        atomic::write_file(&impossible, b"x", Access::Private)
            .unwrap_err()
            .kind(),
        ErrorKind::NotFound
    );
    assert_eq!(read(&path), "old");
    assert_eq!(names(outer.path()), ["kept"]);
}

#[test]
fn a_reader_sees_one_whole_version_or_the_other_and_never_a_part() {
    let outer = folder();
    let path = outer.path().join("document");
    let (short, long) = ("a".repeat(10), "b".repeat(2_000_000));
    atomic::write_file(&path, short.as_bytes(), Access::Private).unwrap();
    let done = AtomicBool::new(false);
    thread::scope(|scope| {
        let _release = SetOnDrop(&done);
        scope.spawn(|| {
            let mut seen = 0;
            while !done.load(Ordering::Relaxed) || seen < 20 {
                // Windows refuses a read for an instant while a replacement
                // lands; a read that is refused saw nothing, so it is not a
                // part either.
                let mut text = String::new();
                if let Ok(mut file) = File::open(&path)
                    && file.read_to_string(&mut text).is_ok()
                {
                    assert!(
                        text == short || text == long,
                        "a partial file of {} bytes",
                        text.len()
                    );
                    seen += 1;
                }
            }
        });
        for round in 0..40 {
            let contents = if round % 2 == 0 { &long } else { &short };
            atomic::write_file(&path, contents.as_bytes(), Access::Private).unwrap();
        }
        done.store(true, Ordering::Relaxed);
    });
}

#[test]
fn replacing_a_file_by_another_returns_the_identity_of_the_one_that_arrived() {
    let outer = folder();
    let from = outer.path().join("from");
    let to = outer.path().join("to");
    fs::write(&from, "new").unwrap();
    fs::write(&to, "old").unwrap();
    let sent = identity::file_id(&from).unwrap();
    let id = atomic::replace_file(&from, &to).unwrap();
    assert_eq!(id, sent);
    assert_eq!(identity::file_id(&to).unwrap(), sent);
    assert_eq!(read(&to), "new");
    assert!(!from.exists());
}

#[test]
fn two_names_are_exchanged_and_both_still_exist() {
    let outer = folder();
    fs::write(outer.path().join("left"), "L").unwrap();
    fs::write(outer.path().join("right"), "R").unwrap();
    let left_id = identity::file_id(&outer.path().join("left")).unwrap();
    let dir = open_dir(outer.path());
    atomic::exchange(&dir, OsStr::new("left"), OsStr::new("right")).unwrap();
    assert_eq!(read(&outer.path().join("left")), "R");
    assert_eq!(read(&outer.path().join("right")), "L");
    assert_eq!(
        identity::file_id(&outer.path().join("right")).unwrap(),
        left_id
    );
    // Exchanged again, they are as they were: the swap back a save relies on.
    atomic::exchange(&dir, OsStr::new("left"), OsStr::new("right")).unwrap();
    assert_eq!(read(&outer.path().join("left")), "L");
    assert_eq!(read(&outer.path().join("right")), "R");
    assert_eq!(names(outer.path()), ["left", "right"]);
    assert_eq!(
        EXCHANGE_IS_ATOMIC,
        cfg!(any(target_os = "macos", target_os = "linux"))
    );
}

#[test]
fn an_exchange_with_a_name_that_is_not_there_changes_nothing() {
    let outer = folder();
    fs::write(outer.path().join("left"), "L").unwrap();
    let dir = open_dir(outer.path());
    assert!(atomic::exchange(&dir, OsStr::new("left"), OsStr::new("absent")).is_err());
    assert_eq!(read(&outer.path().join("left")), "L");
    assert_eq!(names(outer.path()), ["left"]);
}

#[test]
fn a_rename_that_may_not_replace_refuses_a_name_that_is_taken() {
    let outer = folder();
    fs::write(outer.path().join("from"), "F").unwrap();
    fs::write(outer.path().join("taken"), "T").unwrap();
    let dir = open_dir(outer.path());
    let error =
        atomic::rename_no_replace(&dir, OsStr::new("from"), &dir, OsStr::new("taken")).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::AlreadyExists);
    assert_eq!(read(&outer.path().join("from")), "F");
    assert_eq!(read(&outer.path().join("taken")), "T");
    atomic::rename_no_replace(&dir, OsStr::new("from"), &dir, OsStr::new("free")).unwrap();
    assert_eq!(read(&outer.path().join("free")), "F");
    assert!(!outer.path().join("from").exists());
}

#[test]
fn a_rename_that_may_not_replace_moves_a_folder_between_folders() {
    let outer = folder();
    fs::create_dir_all(outer.path().join("a/item")).unwrap();
    fs::write(outer.path().join("a/item/inside"), "x").unwrap();
    fs::create_dir(outer.path().join("b")).unwrap();
    let (from, to) = (
        open_dir(&outer.path().join("a")),
        open_dir(&outer.path().join("b")),
    );
    atomic::rename_no_replace(&from, OsStr::new("item"), &to, OsStr::new("item")).unwrap();
    assert_eq!(read(&outer.path().join("b/item/inside")), "x");
    assert!(!outer.path().join("a/item").exists());
}

#[test]
fn only_a_folder_opens_as_one() {
    let outer = folder();
    let file = outer.path().join("file");
    fs::write(&file, "x").unwrap();
    assert!(hide_platform::fs::open_dir(&file).is_err());
    assert_eq!(
        hide_platform::fs::open_dir(&outer.path().join("absent"))
            .unwrap_err()
            .kind(),
        ErrorKind::NotFound
    );
}

#[test]
fn a_rename_by_path_refuses_even_an_empty_folder_that_is_there() {
    let outer = folder();
    fs::create_dir_all(outer.path().join("clone/inside")).unwrap();
    fs::create_dir(outer.path().join("taken")).unwrap();
    let error =
        atomic::rename_no_replace_path(&outer.path().join("clone"), &outer.path().join("taken"))
            .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::AlreadyExists);
    assert!(outer.path().join("clone/inside").is_dir());
    atomic::rename_no_replace_path(&outer.path().join("clone"), &outer.path().join("free"))
        .unwrap();
    assert!(outer.path().join("free/inside").is_dir());
    assert!(!outer.path().join("clone").exists());
}

#[test]
fn a_folders_entries_can_be_made_durable() {
    let outer = folder();
    atomic::sync_dir(&open_dir(outer.path())).unwrap();
}

// ---- link ---------------------------------------------------------------

/// A link to a file needs a privilege on a Windows account that has neither it
/// nor Developer Mode; that answer is part of the contract, and a test that
/// meets it has nothing to check.
fn file_link(target: &Path, at: &Path) -> bool {
    match link::create_link(target, at) {
        Ok(()) => true,
        Err(error) if link::needs_privilege(&error) => {
            if !cfg!(windows) {
                panic!("only Windows needs a privilege to link: {error}");
            }
            false
        }
        Err(error) => panic!("{error}"),
    }
}

#[test]
fn a_link_to_a_folder_leads_there_is_known_to_lead_there_and_goes_without_the_folder() {
    let outer = folder();
    let target = outer.path().join("version-1");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("hide"), "binary").unwrap();
    let current = outer.path().join("current");
    link::create_link(Path::new("version-1"), &current).unwrap();
    assert_eq!(read(&current.join("hide")), "binary");
    assert!(link::is_link_to(&current, Path::new("version-1")));
    assert!(!link::is_link_to(&current, Path::new("version-2")));
    assert!(!link::is_link_to(&target, Path::new("version-1")));
    link::remove_link(&current).unwrap();
    assert!(!current.exists());
    assert_eq!(read(&target.join("hide")), "binary");
}

#[test]
fn a_link_to_a_file_leads_there_or_asks_for_the_privilege() {
    let outer = folder();
    fs::write(outer.path().join("AGENTS.md"), "rules").unwrap();
    let at = outer.path().join("CLAUDE.md");
    if file_link(Path::new("AGENTS.md"), &at) {
        assert_eq!(read(&at), "rules");
        assert!(link::is_link_to(&at, Path::new("AGENTS.md")));
        link::remove_link(&at).unwrap();
        assert!(outer.path().join("AGENTS.md").exists());
    }
}

#[test]
fn replacing_a_link_points_the_same_name_at_the_new_target_with_nothing_left_beside_it() {
    let outer = folder();
    for version in ["version-1", "version-2"] {
        fs::create_dir(outer.path().join(version)).unwrap();
        fs::write(outer.path().join(version).join("hide"), version).unwrap();
    }
    let current = outer.path().join("current");
    link::replace_link(Path::new("version-1"), &current).unwrap();
    assert_eq!(read(&current.join("hide")), "version-1");
    link::replace_link(Path::new("version-2"), &current).unwrap();
    assert_eq!(read(&current.join("hide")), "version-2");
    assert!(link::is_link_to(&current, Path::new("version-2")));
    assert_eq!(names(outer.path()), ["current", "version-1", "version-2"]);
}

/// Swaps `current` between two versions 40 times while a reader loops on
/// `read`, which answers the version it found; a read can be refused while
/// the swap lands on Windows, and what it must never be is "not found".
fn replace_while_reading(read: impl Fn(&Path) -> std::io::Result<String> + Sync) {
    let outer = folder();
    for version in ["version-1", "version-2"] {
        fs::create_dir(outer.path().join(version)).unwrap();
        fs::write(outer.path().join(version).join("hide"), version).unwrap();
    }
    let current = outer.path().join("current");
    link::replace_link(Path::new("version-1"), &current).unwrap();
    let done = AtomicBool::new(false);
    thread::scope(|scope| {
        let _release = SetOnDrop(&done);
        scope.spawn(|| {
            let mut seen = 0;
            while !done.load(Ordering::Relaxed) || seen < 20 {
                match read(&current) {
                    Ok(text) => {
                        assert!(text == "version-1" || text == "version-2", "{text:?}");
                        seen += 1;
                    }
                    Err(error) => assert_ne!(error.kind(), ErrorKind::NotFound, "{error}"),
                }
            }
        });
        for round in 0..40 {
            let version = if round % 2 == 0 {
                "version-2"
            } else {
                "version-1"
            };
            link::replace_link(Path::new(version), &current).unwrap();
        }
        done.store(true, Ordering::Relaxed);
    });
}

#[test]
fn a_link_that_is_replaced_is_never_missing_while_a_reader_looks_at_it() {
    // A junction reads back as the absolute path it leads to.
    replace_while_reading(|current| {
        fs::read_link(current).map(|to| {
            to.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        })
    });
}

/// Linux does not keep this one: a path walk that follows the link while it
/// is renamed over can find nothing and answer "not found".
#[cfg(not(target_os = "linux"))]
#[test]
fn a_link_that_is_replaced_never_leads_nowhere_while_a_reader_follows_it() {
    replace_while_reading(|current| fs::read_to_string(current.join("hide")));
}

#[test]
fn a_dangling_link_is_still_a_link_and_a_plain_file_is_not() {
    let outer = folder();
    let plain = outer.path().join("plain");
    fs::write(&plain, "x").unwrap();
    assert!(!link::is_link_to(&plain, Path::new("plain")));
    assert!(!link::is_link_to(
        &outer.path().join("absent"),
        Path::new("plain")
    ));
    let dangling = outer.path().join("dangling");
    if file_link(Path::new("nowhere"), &dangling) {
        assert!(link::is_link_to(&dangling, Path::new("nowhere")));
        assert!(!dangling.exists());
        link::remove_link(&dangling).unwrap();
    }
}

#[test]
fn a_link_made_where_a_name_is_taken_is_refused() {
    let outer = folder();
    fs::create_dir(outer.path().join("target")).unwrap();
    fs::write(outer.path().join("taken"), "x").unwrap();
    let error = link::create_link(Path::new("target"), &outer.path().join("taken")).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::AlreadyExists);
}

// ---- identity -----------------------------------------------------------

#[test]
fn two_names_of_one_file_are_the_same_file_and_two_files_are_not() {
    let outer = folder();
    let one = outer.path().join("one");
    fs::write(&one, "x").unwrap();
    let two = outer.path().join("two");
    fs::hard_link(&one, &two).unwrap();
    let other = outer.path().join("other");
    fs::write(&other, "x").unwrap();
    assert!(identity::same_file(&one, &two).unwrap());
    assert!(!identity::same_file(&one, &other).unwrap());
    assert_eq!(
        identity::same_file(&one, &outer.path().join("absent"))
            .unwrap_err()
            .kind(),
        ErrorKind::NotFound
    );
}

#[test]
fn the_id_of_an_open_file_is_the_id_of_its_name_and_survives_a_rename() {
    let outer = folder();
    let path = outer.path().join("a");
    fs::write(&path, "x").unwrap();
    let file = File::open(&path).unwrap();
    let by_name = identity::file_id(&path).unwrap();
    assert_eq!(identity::file_id_of(&file).unwrap(), by_name);
    let renamed = outer.path().join("b");
    drop(file);
    fs::rename(&path, &renamed).unwrap();
    assert_eq!(identity::file_id(&renamed).unwrap(), by_name);
    let dir = open_dir(outer.path());
    assert_eq!(
        identity::file_id_of(&dir).unwrap(),
        identity::file_id(outer.path()).unwrap()
    );
    let ids: Vec<FileId> = vec![by_name, identity::file_id(outer.path()).unwrap()];
    assert_ne!(ids[0], ids[1]);
    assert_eq!(ids[0].volume(), ids[1].volume());
}

#[test]
fn a_link_has_an_id_of_its_own_and_one_of_what_it_leads_to() {
    let outer = folder();
    let target = outer.path().join("target");
    fs::create_dir(&target).unwrap();
    let at = outer.path().join("alias");
    link::create_link(Path::new("target"), &at).unwrap();
    assert_eq!(
        identity::file_id(&at).unwrap(),
        identity::file_id(&target).unwrap()
    );
    assert_ne!(
        identity::file_id_nofollow(&at).unwrap(),
        identity::file_id(&target).unwrap()
    );
    assert!(identity::same_file(&at, &target).unwrap());
}

#[test]
fn an_entry_of_an_open_folder_has_the_id_of_its_name_and_a_link_has_its_own() {
    let outer = folder();
    fs::create_dir(outer.path().join("target")).unwrap();
    fs::write(outer.path().join("a.txt"), "x").unwrap();
    let dir = open_dir(outer.path());
    let file = identity::entry_id(&dir, OsStr::new("a.txt")).unwrap();
    assert_eq!(
        file,
        identity::file_id(&outer.path().join("a.txt")).unwrap()
    );
    link::create_link(Path::new("target"), &outer.path().join("alias")).unwrap();
    let alias = identity::entry_id(&dir, OsStr::new("alias")).unwrap();
    assert_eq!(
        alias,
        identity::file_id_nofollow(&outer.path().join("alias")).unwrap()
    );
    assert_ne!(
        alias,
        identity::entry_id(&dir, OsStr::new("target")).unwrap()
    );
    // Replaced by another file under the same name, the entry is another one.
    fs::rename(outer.path().join("a.txt"), outer.path().join("old.txt")).unwrap();
    fs::write(outer.path().join("a.txt"), "y").unwrap();
    assert_ne!(identity::entry_id(&dir, OsStr::new("a.txt")).unwrap(), file);
    assert_eq!(
        identity::entry_id(&dir, OsStr::new("old.txt")).unwrap(),
        file
    );
    assert_eq!(
        identity::entry_id(&dir, OsStr::new("absent"))
            .unwrap_err()
            .kind(),
        ErrorKind::NotFound
    );
}

#[test]
fn a_files_name_count_grows_with_every_hard_link() {
    let outer = folder();
    let path = outer.path().join("a");
    fs::write(&path, "x").unwrap();
    let file = File::open(&path).unwrap();
    assert_eq!(identity::link_count(&file).unwrap(), 1);
    fs::hard_link(&path, outer.path().join("b")).unwrap();
    assert_eq!(identity::link_count(&file).unwrap(), 2);
}

#[test]
fn spellings_of_one_place_have_one_canonical_form_without_the_system_prefix() {
    let outer = folder();
    let real = outer.path().join("real");
    fs::create_dir(&real).unwrap();
    fs::write(real.join("file"), "x").unwrap();
    let alias = outer.path().join("alias");
    link::create_link(Path::new("real"), &alias).unwrap();
    let through_link = identity::canonical(&alias.join("file")).unwrap();
    let direct = identity::canonical(&real.join("file")).unwrap();
    assert_eq!(through_link, direct);
    assert!(direct.is_absolute());
    assert!(
        !direct.to_string_lossy().starts_with(r"\\?\"),
        "{}",
        direct.display()
    );
    assert!(identity::canonical(&outer.path().join("absent")).is_err());
    // Going up and down again names the same place.
    let roundabout = real.join("..").join("real").join("file");
    assert_eq!(identity::canonical(&roundabout).unwrap(), direct);
}

#[cfg(target_os = "macos")]
#[test]
fn tmp_and_private_tmp_are_one_place() {
    assert_eq!(
        identity::canonical(Path::new("/tmp")).unwrap(),
        identity::canonical(Path::new("/private/tmp")).unwrap()
    );
    assert!(identity::same_file(Path::new("/tmp"), Path::new("/private/tmp")).unwrap());
}

/// What the filesystem really does, asked the long way.
fn really_case_sensitive(folder: &Path) -> bool {
    fs::write(folder.join("probe-a"), "").unwrap();
    let sensitive = !folder.join("PROBE-A").exists();
    fs::remove_file(folder.join("probe-a")).unwrap();
    sensitive
}

#[test]
fn a_folder_is_known_to_tell_case_apart_or_not_by_asking_the_folder() {
    let outer = folder();
    let named = outer.path().join("Named");
    fs::create_dir(&named).unwrap();
    // A folder that holds an entry with a cased name is judged by that entry,
    // writing nothing; one that holds none by making a file in it.
    let holding = outer.path().join("Holding");
    fs::create_dir(&holding).unwrap();
    fs::write(holding.join("Readme.md"), "").unwrap();
    for under_test in [&named, &holding] {
        assert_eq!(
            identity::case_sensitive(under_test).unwrap(),
            really_case_sensitive(under_test)
        );
    }
    assert_eq!(names(&holding), ["Readme.md"]);
    assert!(fs::read_dir(&named).unwrap().next().is_none());
    // Asked to write nothing, the empty folder has no answer.
    assert_eq!(
        identity::case_sensitive_by_entries(&holding).unwrap(),
        Some(really_case_sensitive(&holding))
    );
    assert_eq!(identity::case_sensitive_by_entries(&named).unwrap(), None);
    assert!(fs::read_dir(&named).unwrap().next().is_none());
}

// ---- own and regular files -----------------------------------------------

#[test]
fn an_own_file_is_made_private_keeps_what_it_holds_and_must_exist_to_be_read() {
    let outer = folder();
    let path = outer.path().join("generator.lock");
    assert_eq!(
        private::open_own_file(&path, false).unwrap_err().kind(),
        ErrorKind::NotFound
    );
    {
        use std::io::Write;
        let mut file = private::open_own_file(&path, true).unwrap();
        file.write_all(b"held").unwrap();
    }
    assert!(private::is_private(&path).unwrap());
    private::open_own_file(&path, true).unwrap();
    let mut held = String::new();
    private::open_own_file(&path, false)
        .unwrap()
        .read_to_string(&mut held)
        .unwrap();
    assert_eq!(held, "held");
}

#[test]
fn a_link_planted_at_an_own_files_name_is_neither_followed_nor_accepted() {
    let outer = folder();
    let target = outer.path().join("operator.json");
    fs::write(&target, "kept").unwrap();
    let at = outer.path().join("generator.lock");
    if file_link(&target, &at) {
        assert!(private::open_own_file(&at, true).is_err());
        assert!(private::open_own_file(&at, false).is_err());
        assert_eq!(read(&target), "kept");
    }
    let folder_at = outer.path().join("folder.lock");
    link::create_link(outer.path(), &folder_at).unwrap();
    assert!(private::open_own_file(&folder_at, true).is_err());
    // A hard link is a second name of the target itself.
    let hard = outer.path().join("hard.lock");
    fs::hard_link(&target, &hard).unwrap();
    assert!(private::open_own_file(&hard, true).is_err());
    assert!(private::open_own_file(&hard, false).is_err());
    assert_eq!(read(&target), "kept");
}

#[test]
fn directory_entry_open_refuses_files_links_and_pipes() {
    let outer = folder();
    let directory = outer.path().join("directory");
    fs::create_dir(&directory).unwrap();
    hide_platform::fs::open_dir_nofollow(&directory).unwrap();
    let file = outer.path().join("file");
    fs::write(&file, "kept").unwrap();
    assert!(hide_platform::fs::open_dir_nofollow(&file).is_err());
    let at = outer.path().join("link");
    link::create_link(&directory, &at).unwrap();
    assert!(hide_platform::fs::open_dir_nofollow(&at).is_err());
    #[cfg(unix)]
    {
        let pipe = outer.path().join("pipe");
        assert!(
            Command::new("mkfifo")
                .arg(&pipe)
                .status()
                .unwrap()
                .success()
        );
        assert!(hide_platform::fs::open_dir_nofollow(&pipe).is_err());
    }
}

#[test]
fn only_a_regular_file_opens_as_one_and_a_link_at_its_name_is_not_followed() {
    let outer = folder();
    let file = outer.path().join("image.png");
    fs::write(&file, "pixels").unwrap();
    let mut contents = String::new();
    hide_platform::fs::open_regular(&file)
        .unwrap()
        .read_to_string(&mut contents)
        .unwrap();
    assert_eq!(contents, "pixels");
    assert_eq!(
        hide_platform::fs::open_regular(outer.path())
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidInput
    );
    let at = outer.path().join("link.png");
    if file_link(&file, &at) {
        assert!(hide_platform::fs::open_regular(&at).is_err());
    }
    #[cfg(unix)]
    {
        // A pipe would block a plain open until a writer came.
        let pipe = outer.path().join("pipe");
        assert!(
            Command::new("mkfifo")
                .arg(&pipe)
                .status()
                .unwrap()
                .success()
        );
        assert_eq!(
            hide_platform::fs::open_regular(&pipe).unwrap_err().kind(),
            ErrorKind::InvalidInput
        );
    }
}

#[test]
fn a_stamp_holds_through_a_read_and_moves_with_a_write() {
    let outer = folder();
    let path = outer.path().join("attachment");
    fs::write(&path, "first").unwrap();
    let mut file = File::open(&path).unwrap();
    let before = identity::stamp_of(&file).unwrap();
    let mut contents = String::new();
    file.read_to_string(&mut contents).unwrap();
    assert_eq!(identity::stamp_of(&file).unwrap(), before);
    fs::write(&path, "second, longer").unwrap();
    assert_ne!(identity::stamp_of(&file).unwrap(), before);
}

// ---- space ---------------------------------------------------------------

#[test]
fn a_file_takes_at_least_its_bytes_and_a_second_name_is_the_same_file() {
    let outer = folder();
    let file = outer.path().join("data");
    fs::write(&file, vec![7u8; 65536]).unwrap();
    let usage = space::usage_nofollow(&file).unwrap();
    assert!(usage.allocated >= 65536, "{usage:?}");
    assert_eq!(usage.links, 1);
    assert!(!usage.is_dir);
    let alias = outer.path().join("alias");
    fs::hard_link(&file, &alias).unwrap();
    let second = space::usage_nofollow(&alias).unwrap();
    assert_eq!(second.id, usage.id);
    assert_eq!(second.links, 2);
    assert!(space::usage_nofollow(outer.path()).unwrap().is_dir);
}

#[test]
fn a_link_to_a_folder_is_measured_as_itself_and_is_not_a_folder() {
    let outer = folder();
    let target = outer.path().join("target");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("big"), vec![1u8; 65536]).unwrap();
    let at = outer.path().join("link");
    link::create_link(Path::new("target"), &at).unwrap();
    let usage = space::usage_nofollow(&at).unwrap();
    assert!(!usage.is_dir, "{usage:?}");
    assert_ne!(usage.id, space::usage_nofollow(&target).unwrap().id);
}

#[test]
fn the_volume_a_folder_is_on_has_room_and_a_missing_one_has_no_answer() {
    let outer = folder();
    let free = space::free_bytes(outer.path()).unwrap();
    assert!(free > 0);
    assert!(space::total_bytes(outer.path()).unwrap() >= free);
    assert!(space::free_bytes(&outer.path().join("missing")).is_err());
    assert!(space::total_bytes(&outer.path().join("missing")).is_err());
}

#[test]
fn an_open_file_knows_the_path_it_is_at_even_after_a_rename() {
    let outer = folder();
    let first = outer.path().join("first.txt");
    fs::write(&first, "contents").unwrap();
    let file = File::open(&first).unwrap();
    let folder_handle = hide_platform::fs::open_dir(outer.path()).unwrap();
    let folder_path = identity::path_of(&folder_handle).unwrap();
    assert!(identity::same_file(&folder_path, outer.path()).unwrap());
    // A file's answer is its folder's answer and its name, so a boundary can
    // compare the two by prefix.
    assert_eq!(
        identity::path_of(&file).unwrap(),
        folder_path.join("first.txt")
    );
    let second = outer.path().join("second.txt");
    // Windows renames an open file only when it was opened sharing delete,
    // which `File::open` does.
    fs::rename(&first, &second).unwrap();
    assert_eq!(
        identity::path_of(&file).unwrap(),
        folder_path.join("second.txt")
    );
}

#[test]
fn open_file_permissions_are_judged_on_the_descriptor() {
    let home = folder();
    let path = home.path().join("record.json");
    let file = private::create_new_file(&path).unwrap();
    assert!(!private::handle_others_can_modify(&file).unwrap());
    let original = home.path().join("original.json");
    fs::rename(&path, &original).unwrap();
    fs::write(&path, b"replacement").unwrap();
    widen(&path, true);
    assert!(private::others_can_modify(&path).unwrap());
    assert!(!private::handle_others_can_modify(&file).unwrap());
    widen(&original, false);
    assert!(!private::handle_others_can_modify(&file).unwrap());
    widen(&original, true);
    assert!(private::handle_others_can_modify(&file).unwrap());
    assert!(private::handle_owned_by_current_user(&file).unwrap());
}

/// A folder that is open, or has a file open inside it, is listed with the
/// process that holds it, which is what the record of a refused rename needs
/// to tell this process from another one. Only Windows has the question.
#[cfg(windows)]
#[test]
fn the_process_holding_a_folder_or_a_file_in_it_is_listed_with_its_id() {
    use hide_platform::fs::holders::holders_of;
    let sandbox = tempfile::tempdir().unwrap();
    let folder = sandbox.path().join("held");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("inside.txt"), "inside").unwrap();
    let mine = std::process::id();
    let listed = |folder: &Path| {
        holders_of(folder, 64)
            .unwrap()
            .iter()
            .any(|holder| holder.pid == mine)
    };

    assert!(!listed(&folder), "nothing is open yet");
    let file = File::open(folder.join("inside.txt")).unwrap();
    assert!(listed(&folder), "a file open inside the folder is a holder");
    drop(file);
    assert!(!listed(&folder), "a closed file holds nothing");
    let handle = hide_platform::fs::open_dir(&folder).unwrap();
    assert!(listed(&folder), "the folder itself open is a holder");
    drop(handle);
    assert!(!listed(&folder));
}

#[test]
#[allow(clippy::disallowed_methods)] // a child process the test kills later: it sleeps to stay alive
fn idle_role() {
    if std::env::var(ROLE).as_deref() != Ok("idle") {
        return;
    }
    println!("READY");
    thread::sleep(Duration::from_secs(60));
}

/// Waits until nothing holds a file under `folder`: Windows unmaps a program
/// and a scanner lets a new file go a moment later, with no event to wait on.
#[cfg(windows)]
#[allow(clippy::disallowed_methods)] // the kernel's release has no event; this polls it, bounded
fn until_free(folder: &Path) {
    use hide_platform::fs::in_use::first_in_use;
    let deadline = Instant::now() + Duration::from_secs(10);
    while let Some(held) = first_in_use(folder).unwrap() {
        assert!(
            Instant::now() < deadline,
            "{} is still held",
            held.display()
        );
        thread::sleep(Duration::from_millis(20));
    }
}

/// A folder whose files nothing holds is free to remove, on every system.
#[test]
fn a_folder_nothing_holds_is_not_in_use() {
    use hide_platform::fs::in_use::first_in_use;
    let sandbox = folder();
    fs::create_dir(sandbox.path().join("inner")).unwrap();
    fs::write(sandbox.path().join("inner").join("file.txt"), "file").unwrap();
    assert_eq!(first_in_use(sandbox.path()).unwrap(), None);
    assert!(first_in_use(&sandbox.path().join("missing")).is_err());
}

/// A program running from a folder holds it in use until it ends, though
/// Windows maps the image without an open handle; a file opened without
/// write sharing holds it too. The runtime copy's prune decides on this,
/// since a rename of the folder does not see the program.
#[cfg(windows)]
#[test]
fn a_program_running_from_a_folder_or_a_file_open_in_it_holds_it_in_use() {
    use hide_platform::fs::in_use::first_in_use;
    use std::os::windows::fs::OpenOptionsExt;
    let sandbox = folder();
    let program = sandbox.path().join("nested").join("program.exe");
    fs::create_dir(program.parent().unwrap()).unwrap();
    fs::copy(std::env::current_exe().unwrap(), &program).unwrap();
    // A scanner may hold a new program for a moment after it is written.
    until_free(sandbox.path());

    let mut running = Killed(
        Command::new(&program)
            .args([
                "--exact",
                "fs::idle_role",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(ROLE, "idle")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = running.0.stdout.take().unwrap();
    let ready = BufReader::new(stdout)
        .lines()
        .map_while(Result::ok)
        .any(|line| line.contains("READY"));
    assert!(ready, "the program never started");
    assert_eq!(first_in_use(sandbox.path()).unwrap(), Some(program.clone()));
    // Why a rename is no test of use: the running program's folder moves.
    let moved = sandbox.path().join("moved");
    fs::rename(program.parent().unwrap(), &moved)
        .expect("Windows renames a running program's folder");
    fs::rename(&moved, program.parent().unwrap()).unwrap();
    drop(running);
    // The image is unmapped a moment after the process has ended.
    until_free(sandbox.path());

    let file = sandbox.path().join("open.txt");
    fs::write(&file, "open").unwrap();
    // FILE_SHARE_READ only.
    let open = File::options()
        .read(true)
        .share_mode(1)
        .open(&file)
        .unwrap();
    assert_eq!(first_in_use(sandbox.path()).unwrap(), Some(file));
    drop(open);
    assert_eq!(first_in_use(sandbox.path()).unwrap(), None);
}

/// A file another handle holds refuses a reader on Windows for as long as it
/// is held, and the refusal says so: an open the holder did not share, and a
/// read of a range it locked. Elsewhere the holder refuses neither. On every
/// system an error that never lets go, a missing file or Windows' access
/// denied, is not one.
#[test]
fn a_file_another_handle_holds_refuses_a_reader_only_on_windows_and_only_while_held() {
    use hide_platform::fs::in_use::held_by_another;
    let sandbox = folder();
    let path = sandbox.path().join("held.db");
    fs::write(&path, "held").unwrap();
    let read = || -> std::io::Result<String> {
        let mut text = String::new();
        File::open(&path)?.read_to_string(&mut text)?;
        Ok(text)
    };
    let refused_while_held = || {
        let answered = read();
        if cfg!(windows) {
            let error = answered.expect_err("a held file refuses a reader on Windows");
            assert!(held_by_another(&error), "{error:?}");
        } else {
            assert_eq!(answered.unwrap(), "held");
        }
    };
    let writer = || File::options().read(true).write(true).clone();

    #[cfg(windows)]
    let unshared = {
        use std::os::windows::fs::OpenOptionsExt;
        writer().share_mode(0).open(&path).unwrap()
    };
    #[cfg(not(windows))]
    let unshared = writer().open(&path).unwrap();
    refused_while_held();
    drop(unshared);
    assert_eq!(read().unwrap(), "held");

    let locked = lock::lock_file_blocking(writer().open(&path).unwrap(), Mode::Exclusive).unwrap();
    refused_while_held();
    drop(locked);
    assert_eq!(read().unwrap(), "held");

    let missing = File::open(sandbox.path().join("missing")).unwrap_err();
    assert!(!held_by_another(&missing));
    // ERROR_ACCESS_DENIED on Windows.
    assert!(!held_by_another(&std::io::Error::from_raw_os_error(5)));
}
