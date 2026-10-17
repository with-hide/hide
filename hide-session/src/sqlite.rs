//! URI policy for native databases: never create or write a WAL index.
use rusqlite::{Connection, OpenFlags, ffi};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock, Weak};

type ReadGate = (std::path::PathBuf, Weak<Mutex<()>>);
static READ_GATES: Mutex<Vec<ReadGate>> = Mutex::new(Vec::new());

/// Closing any descriptor for an inode drops this process's POSIX locks on
/// it. Keep all OpenCode guard handles inside the same per-database gate as
/// SQLite, including admission and destruction. Refuse contention rather
/// than making a session worker wait behind another read.
/// The canonical namespace, not its inode, survives provider replacement.
pub(crate) fn read_gate(path: &Path) -> Result<Arc<Mutex<()>>, &'static str> {
    let mut gates = READ_GATES.lock().map_err(|_| "opencode_db_unreadable")?;
    gates.retain(|(_, gate)| gate.strong_count() != 0);
    if let Some(gate) = gates
        .iter()
        .find_map(|(key, gate)| (key == path).then(|| gate.upgrade()).flatten())
    {
        return Ok(gate);
    }
    if gates.len() >= 64 {
        return Err("opencode_db_busy");
    }
    let gate = Arc::new(Mutex::new(()));
    gates.push((path.to_path_buf(), Arc::downgrade(&gate)));
    Ok(gate)
}

static READ_VFS: OnceLock<Result<(), ()>> = OnceLock::new();
static BASE_VFS: OnceLock<usize> = OnceLock::new();
const VFS_NAME: &str = "hide-session-readonly";

/// SQLite asks to CREATE a WAL even for a read-only main database. Intercept
/// that request at the VFS boundary: a concurrent provider unlink must not
/// turn a read into creation. SHM separately uses readonly_shm in the URI.
pub(crate) fn open_readonly(uri: &str, flags: OpenFlags) -> rusqlite::Result<Connection> {
    let registered = READ_VFS.get_or_init(|| {
        // SAFETY: SQLite owns its default VFS for process lifetime. The copied
        // descriptor is registered once, never as default, and retained forever.
        unsafe {
            let base = ffi::sqlite3_vfs_find(std::ptr::null());
            if base.is_null() || (*base).xOpen.is_none() {
                return Err(());
            }
            BASE_VFS.set(base as usize).map_err(|_| ())?;
            let mut read = Box::new(std::ptr::read(base));
            read.zName = c"hide-session-readonly".as_ptr();
            read.pNext = std::ptr::null_mut();
            read.xOpen = Some(open_existing);
            read.xDelete = Some(refuse_delete);
            let read = Box::into_raw(read);
            if ffi::sqlite3_vfs_register(read, 0) != ffi::SQLITE_OK {
                return Err(());
            }
        }
        Ok(())
    });
    if registered.is_err() {
        return Err(rusqlite::Error::InvalidParameterName(
            "native read VFS unavailable".into(),
        ));
    }
    let connection = Connection::open_with_flags_and_vfs(uri, flags, VFS_NAME)?;
    // No writable temporary database or mmap of a file a writer may truncate.
    connection
        .execute_batch("PRAGMA query_only=ON; PRAGMA temp_store=MEMORY; PRAGMA mmap_size=0;")?;
    Ok(connection)
}

unsafe extern "C" fn open_existing(
    _vfs: *mut ffi::sqlite3_vfs,
    name: *const std::ffi::c_char,
    file: *mut ffi::sqlite3_file,
    flags: std::ffi::c_int,
    out: *mut std::ffi::c_int,
) -> std::ffi::c_int {
    if file.is_null() {
        return ffi::SQLITE_CANTOPEN;
    }
    // SAFETY: SQLite supplies writable file storage; NULL methods is its
    // required failed-open state. No allocation or panicking call crosses FFI.
    unsafe {
        (*file).pMethods = std::ptr::null();
    }
    if name.is_null() || flags & (ffi::SQLITE_OPEN_MAIN_DB | ffi::SQLITE_OPEN_WAL) == 0 {
        return ffi::SQLITE_READONLY;
    }
    let Some(base) = BASE_VFS.get() else {
        return ffi::SQLITE_CANTOPEN;
    };
    let base = *base as *mut ffi::sqlite3_vfs;
    // SAFETY: BASE_VFS was taken from SQLite and retained before registration.
    let Some(open) = (unsafe { (*base).xOpen }) else {
        return ffi::SQLITE_CANTOPEN;
    };
    let flags = (flags
        & !(ffi::SQLITE_OPEN_READWRITE | ffi::SQLITE_OPEN_CREATE | ffi::SQLITE_OPEN_DELETEONCLOSE))
        | ffi::SQLITE_OPEN_READONLY
        | ffi::SQLITE_OPEN_NOFOLLOW;
    // Delegate with the original descriptor, preserving URI and NOFOLLOW.
    unsafe { open(base, name, file, flags, out) }
}

unsafe extern "C" fn refuse_delete(
    _vfs: *mut ffi::sqlite3_vfs,
    _name: *const std::ffi::c_char,
    _sync: std::ffi::c_int,
) -> std::ffi::c_int {
    ffi::SQLITE_READONLY
}

/// The reasons one provider's database read fails with.
pub(crate) struct Reasons {
    pub(crate) busy: &'static str,
    pub(crate) unreadable: &'static str,
    pub(crate) changed: &'static str,
    pub(crate) work: &'static str,
}

const BUSY_WAIT: std::time::Duration = std::time::Duration::from_millis(50);

/// One bounded read of the provider database at `path` under `folder` in
/// `home`, buffered until its source and companions are proven unchanged.
/// The VFS forbids writes even if the provider removes a WAL between
/// admission and SQLite opening it. A checkpointed main is read without
/// asking SQLite to create companions; a live WAL retains locking.
pub(crate) fn with_read<T>(
    home: &Path,
    folder: &str,
    path: &Path,
    reasons: &Reasons,
    steps: usize,
    read: impl FnOnce(&Connection) -> T,
) -> Result<T, String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            "session_file_missing".to_owned()
        } else {
            reasons.unreadable.to_owned()
        }
    })?;
    if metadata.file_type().is_symlink() {
        return Err("label_session_linked".to_owned());
    }
    if !metadata.is_file() {
        return Err("label_session_not_regular".to_owned());
    }
    // Resolve OS aliases such as macOS /var before SQLite's NOFOLLOW check.
    // The final database symlink was refused above.
    let path = crate::native_file::checked_path_under(home, folder, path)
        .map_err(|_| reasons.unreadable.to_owned())?;
    let gate = read_gate(&path).map_err(str::to_owned)?;
    let _read_guard = gate.try_lock().map_err(|error| match error {
        std::sync::TryLockError::WouldBlock => reasons.busy.to_owned(),
        std::sync::TryLockError::Poisoned(_) => reasons.unreadable.to_owned(),
    })?;
    // Drop order matters: Source's raw handles close before this gate opens.
    let source = Source::admit(home, &path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::WouldBlock {
            reasons.busy.to_owned()
        } else {
            reasons.unreadable.to_owned()
        }
    })?;
    let failed = |error: &rusqlite::Error| refusal(reasons, error);
    let connection = open_readonly(
        &read_uri(&path, !source.checkpointed_wal).map_err(str::to_owned)?,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::from_bits_retain(ffi::SQLITE_OPEN_NOFOLLOW),
    )
    .map_err(|error| failed(&error))?;
    connection
        .busy_timeout(BUSY_WAIT)
        .map_err(|error| failed(&error))?;
    work_budget(&connection, steps);
    connection
        .execute_batch("BEGIN")
        .map_err(|error| failed(&error))?;
    let result = read(&connection);
    connection
        .execute_batch("ROLLBACK")
        .map_err(|error| failed(&error))?;
    connection.close().map_err(|(_, error)| failed(&error))?;
    source.validate().map_err(|_| reasons.changed.to_owned())?;
    Ok(result)
}

/// Gives `connection` a fresh cap of `steps` SQL work units.
pub(crate) fn work_budget(connection: &Connection, steps: usize) {
    let mut sql_steps = 0;
    connection.progress_handler(
        1_000,
        Some(move || {
            sql_steps += 1_000;
            sql_steps >= steps
        }),
    );
}

pub(crate) fn refusal(reasons: &Reasons, error: &rusqlite::Error) -> String {
    match error.sqlite_error_code() {
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked) => {
            reasons.busy.to_owned()
        }
        Some(rusqlite::ErrorCode::OperationInterrupted) => reasons.work.to_owned(),
        _ => reasons.unreadable.to_owned(),
    }
}

type Mark = (
    hide_platform::fs::identity::FileId,
    hide_platform::fs::identity::Stamp,
);
fn mark(file: &std::fs::File) -> std::io::Result<Mark> {
    Ok((
        hide_platform::fs::identity::file_id_of(file)?,
        hide_platform::fs::identity::stamp_of(file)?,
    ))
}

/// Retained handles and names fence one bounded native-database read. No
/// result escapes until every file, companion and ancestor still agrees.
pub(crate) struct Source {
    root: std::path::PathBuf,
    path: std::path::PathBuf,
    files: Vec<(std::path::PathBuf, Option<(std::fs::File, Mark)>)>,
    folders: Vec<(std::path::PathBuf, std::fs::File, Mark)>,
    pub(crate) checkpointed_wal: bool,
}

impl Source {
    pub(crate) fn admit(root: &Path, path: &Path) -> std::io::Result<Self> {
        let root = hide_platform::fs::identity::canonical(root)?;
        let path = hide_platform::fs::identity::canonical(path)?;
        if !path.starts_with(&root) {
            return Err(changed());
        }
        let mut files = Vec::with_capacity(4);
        let mut wal = false;
        let mut wal_mode = false;
        let companion = |suffix: &str| {
            let mut name = path.as_os_str().to_os_string();
            name.push(suffix);
            std::path::PathBuf::from(name)
        };
        for (index, name) in [
            path.clone(),
            companion("-wal"),
            companion("-shm"),
            companion("-journal"),
        ]
        .into_iter()
        .enumerate()
        {
            let held = match std::fs::symlink_metadata(&name) {
                Ok(_) => {
                    if index == 3 {
                        return Err(std::io::ErrorKind::WouldBlock.into());
                    }
                    let file = crate::open_session_file_nofollow(&name)?;
                    if hide_platform::fs::identity::link_count(&file)? != 1 {
                        return Err(changed());
                    }
                    let stamp = mark(&file)?;
                    if index == 0 {
                        use std::io::Read;
                        let mut header = [0; 20];
                        (&file).read_exact(&mut header)?;
                        wal_mode = header[18] == 2 && header[19] == 2;
                    }
                    if index == 1 {
                        wal = file.metadata()?.len() != 0;
                    }
                    Some((file, stamp))
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound && index != 0 => None,
                Err(error) => return Err(error),
            };
            files.push((name, held));
        }
        if wal && files[2].1.is_none() {
            return Err(std::io::ErrorKind::WouldBlock.into());
        }
        let mut folders = Vec::new();
        for folder in path
            .ancestors()
            .skip(1)
            .take_while(|folder| folder.starts_with(&root))
        {
            if folders.len() >= 64 {
                return Err(changed());
            }
            let handle = hide_platform::fs::open_dir_nofollow(folder)?;
            let stamp = mark(&handle)?;
            folders.push((folder.to_path_buf(), handle, stamp));
        }
        let source = Self {
            root,
            path,
            files,
            folders,
            checkpointed_wal: wal_mode && !wal,
        };
        source.validate()?;
        Ok(source)
    }

    pub(crate) fn validate(&self) -> std::io::Result<()> {
        if hide_platform::fs::identity::canonical(&self.path)? != self.path
            || !self.path.starts_with(&self.root)
        {
            return Err(changed());
        }
        for (path, held) in &self.files {
            match held {
                Some((handle, before)) => {
                    let current = crate::open_session_file_nofollow(path)?;
                    if mark(handle)? != *before || mark(&current)? != *before {
                        return Err(changed());
                    }
                }
                None => match std::fs::symlink_metadata(path) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    _ => return Err(changed()),
                },
            }
        }
        for (path, handle, before) in &self.folders {
            let current = hide_platform::fs::open_dir_nofollow(path)?;
            if mark(handle)? != *before || mark(&current)? != *before {
                return Err(changed());
            }
        }
        Ok(())
    }
}

fn changed() -> std::io::Error {
    std::io::Error::other("native database changed or requires recovery")
}

pub(crate) fn read_uri(path: &Path, wal: bool) -> Result<String, &'static str> {
    use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
    const URI_PATH: &AsciiSet = &CONTROLS.add(b'%').add(b'?').add(b'#').add(b' ');
    let spelling = hide_platform::path::to_wire(path).map_err(|_| "database_path_unconfirmed")?;
    if spelling.starts_with("//") {
        return Err("database_path_unconfirmed");
    }
    let leading = if spelling.starts_with('/') {
        "file:"
    } else {
        "file:/"
    };
    let policy = if wal { "readonly_shm=1" } else { "immutable=1" };
    Ok(format!(
        "{leading}{}?mode=ro&{policy}",
        utf8_percent_encode(&spelling, URI_PATH)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checkpointed() -> (tempfile::TempDir, std::path::PathBuf) {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(".local/share/opencode/opencode.db");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let writer = Connection::open(&path).unwrap();
        writer.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE sample(value); INSERT INTO sample VALUES (1);").unwrap();
        drop(writer);
        (home, path)
    }

    #[test]
    fn read_vfs_cannot_recreate_a_wal_removed_before_sqlite_opens_it() {
        let (_home, path) = checkpointed();
        let before = std::fs::read(&path).unwrap();
        let uri = read_uri(&path, true).unwrap();
        // This is the state at xOpen when a writer removes its WAL after
        // admission. Even without a post-read fence, creation is impossible.
        let result = open_readonly(
            &uri,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        )
        .and_then(|db| db.query_row("SELECT value FROM sample", [], |row| row.get::<_, i64>(0)));
        assert!(result.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
    }

    #[test]
    fn a_checkpointed_read_discards_output_if_a_writer_changes_its_source() {
        let (home, path) = checkpointed();
        let result = crate::opencode::with_unchecked_read(home.path(), |db| {
            let value: i64 = db
                .query_row("SELECT value FROM sample", [], |row| row.get(0))
                .unwrap();
            let writer = Connection::open(&path).unwrap();
            writer.execute("UPDATE sample SET value=2", []).unwrap();
            drop(writer);
            value
        });
        assert_eq!(result, Err("opencode_db_changed".to_owned()));
    }

    #[test]
    fn a_checkpointed_read_keeps_empty_sidecars_and_refuses_a_journal() {
        let (home, path) = checkpointed();
        let wal = path.with_file_name("opencode.db-wal");
        let shm = path.with_file_name("opencode.db-shm");
        std::fs::write(&wal, []).unwrap();
        std::fs::write(&shm, [0; 32768]).unwrap();
        let result = crate::opencode::with_unchecked_read(home.path(), |db| {
            db.query_row("SELECT value FROM sample", [], |row| row.get::<_, i64>(0))
        });
        assert_eq!(result.unwrap().unwrap(), 1);
        assert_eq!(std::fs::read(&wal).unwrap(), Vec::<u8>::new());
        assert_eq!(std::fs::read(&shm).unwrap(), vec![0; 32768]);
        let journal = path.with_file_name("opencode.db-journal");
        std::fs::write(&journal, b"unfinished native transaction").unwrap();
        assert!(
            crate::opencode::with_unchecked_read(home.path(), |_| panic!(
                "journal must be refused before reading"
            ))
            .is_err()
        );
        assert_eq!(
            std::fs::read(&journal).unwrap(),
            b"unfinished native transaction"
        );
    }

    #[test]
    fn guarded_reads_refuse_same_database_contention_and_release_on_error() {
        let (home, _path) = checkpointed();
        let (other, _other_path) = checkpointed();
        let read_value = |db: &Connection| {
            db.query_row("SELECT value FROM sample", [], |row| row.get::<_, i64>(0))
        };
        let result = crate::opencode::with_unchecked_read(home.path(), |db| {
            std::thread::scope(|scope| {
                let competing = scope.spawn(|| {
                    crate::opencode::with_unchecked_read(home.path(), |_| {
                        panic!("a competing connection must not open")
                    })
                });
                assert_eq!(
                    competing.join().unwrap(),
                    Err("opencode_db_busy".to_owned())
                );
                assert_eq!(
                    crate::opencode::with_unchecked_read(other.path(), read_value)
                        .unwrap()
                        .unwrap(),
                    1
                );
            });
            assert_eq!(read_value(db).unwrap(), 1);
            db.execute_batch("SELECT absent_column FROM sample")
        });
        assert!(result.unwrap().is_err());
        assert_eq!(
            crate::opencode::with_unchecked_read(home.path(), read_value)
                .unwrap()
                .unwrap(),
            1
        );
    }

    #[test]
    fn replacing_the_database_keeps_its_guard_until_the_previous_read_ends() {
        let (home, path) = checkpointed();
        let result = crate::opencode::with_unchecked_read(home.path(), |_| {
            let previous = path.with_file_name("previous.db");
            std::fs::rename(&path, &previous).unwrap();
            std::fs::copy(&previous, &path).unwrap();
            assert_eq!(
                crate::opencode::with_unchecked_read(home.path(), |_| panic!(
                    "replacement bypassed gate"
                )),
                Err("opencode_db_busy".to_owned())
            );
        });
        assert_eq!(result, Err("opencode_db_changed".to_owned()));
        assert!(crate::opencode::with_unchecked_read(home.path(), |_| ()).is_ok());
    }
}
