//! Programs a test runs against a product deadline, ready before the
//! deadline starts (issues 813 and 824). Built only into tests: hide-platform's
//! integration tests, and by `#[path]` hided's integration tests,
//! hide-agent-hooks' unit and integration tests and hide-kit's unit tests, so
//! a change here plans every crate that runs it.
//!
//! macOS checks a file the first time it is started, once per file, and the
//! checks of every process on the machine wait in one line: on 2026-10-09 at
//! load 7, a new script's first start took about 90 ms and its second 6 ms,
//! twenty new scripts started together finished one after another with the
//! last at 1.8 s, and a hard link to a file already started began at once. A
//! test that copied the helper or wrote its stand-in afresh queued behind
//! every new file the machine's other builds and tests started, inside the
//! hook's budget or a 5 s `codex` or `launchctl` deadline, and under load the
//! wait outgrew them. Linux has its own refusal: a file another thread's fork
//! still holds open for writing does not start (`ETXTBSY`).
//!
//! So a stand-in is one file per body, kept beside the test binaries of the
//! build, written once and never changed, and started once per process
//! before a test is given it; a test's folder holds a hard link to it, which
//! is the same file and so shares its check. A folder on another filesystem
//! gets a copy, started once the same way.
//! Native macOS executables instead get a private byte copy, ad-hoc signed
//! and started before the test's deadline. Signing a hard link would change
//! cargo's output; a rejected native first start must not count as ready.

use std::collections::HashSet;
use std::io::ErrorKind;
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant};

use hide_platform::fs::identity::FileId;
use hide_platform::process::OwnedChild;

/// How long a program's first start may take. A hang guard, not a deadline:
/// the wait is the machine's line of first-start checks, which grows with
/// every other build and test running beside this one, and it is paid once
/// per file per process, before any deadline a test measures starts. It is
/// nextest's slow-test period (`.config/nextest.toml`).
const READY_GUARD: Duration = Duration::from_secs(60);

/// The most stand-ins the build keeps. A body is fixed text, so the store
/// grows only when a body is edited, by one file per edit until the build
/// folder is cleaned; a body that named a test's own folder would add a file
/// on every run, and this cap fails that test instead.
#[cfg(unix)]
const STORE_CAP: usize = 64;

/// The files this process has already started once.
static READY: LazyLock<Mutex<HashSet<FileId>>> = LazyLock::new(Mutex::default);

/// Puts the stand-in program whose text is `body` at `at`, replacing what is
/// there, ready to run. A stand-in finds its test's files from `HOME`, which
/// the program under test hands on to it, never from a path written into its
/// body.
#[cfg(unix)]
pub fn program(at: &Path, body: &str) {
    place(&stand_in(body), at);
}

/// The stand-in program whose text is `body`, ready to run.
#[cfg(unix)]
fn stand_in(body: &str) -> PathBuf {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    // Beside the test binaries, so it goes with the build folder.
    let store = std::env::current_exe().unwrap().with_file_name("stand-ins");
    std::fs::create_dir_all(&store).unwrap();
    // FNV-1a, so a body keeps its name across toolchains; a name two bodies
    // shared would fail the comparison below rather than run the wrong one.
    let name = body.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    });
    let path = store.join(format!("{name:016x}"));
    if !path.exists() {
        let kept = std::fs::read_dir(&store)
            .unwrap()
            .filter(|entry| {
                !entry
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with('.')
            })
            .count();
        assert!(
            kept < STORE_CAP,
            "{} holds {kept} stand-ins: a stand-in's body must not name a test's folder, \
             and after many edited bodies the folder can be removed",
            store.display()
        );
        let mut file = tempfile::NamedTempFile::new_in(&store).unwrap();
        file.write_all(body.as_bytes()).unwrap();
        // Read-only, so a test that wrote to its link would fail rather than
        // change the program every other test runs.
        file.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o555))
            .unwrap();
        match file.persist_noclobber(&path) {
            Ok(_) => {}
            // Another test process kept the same body first.
            Err(error) if error.error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => panic!("{} could not be kept: {}", path.display(), error.error),
        }
    }
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        body,
        "{} is not the stand-in its name says",
        path.display()
    );
    ready(&path);
    path
}

/// Puts `program`, ready to run, at `at`, replacing what is there: as a hard
/// link, or as a copy started once when `at` is on another filesystem.
/// Native macOS executables are copied and signed without changing `program`.
#[cfg(unix)]
pub fn place(program: &Path, at: &Path) {
    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    if let Err(error) = std::fs::remove_file(at) {
        assert_eq!(error.kind(), ErrorKind::NotFound, "{}", at.display());
    }
    #[cfg(target_os = "macos")]
    if is_mach_o(program) {
        std::io::copy(
            &mut std::fs::File::open(program).unwrap(),
            &mut std::fs::File::create(at).unwrap(),
        )
        .unwrap();
        let mut command = Command::new("/usr/bin/codesign");
        command
            .args(["--force", "--sign", "-"])
            .arg(at)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let output = OwnedChild::spawn(&mut command)
            .unwrap()
            .capture_until(Instant::now() + READY_GUARD, 64 * 1024)
            .unwrap();
        assert!(
            output.status.success(),
            "{} could not be signed: {:?}, {}",
            at.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        std::fs::set_permissions(at, std::fs::metadata(program).unwrap().permissions()).unwrap();
        // This file belongs to its fixture and is removed with it. Do not
        // remember its identity after deletion or grow the shared-file cache.
        start_once(at, &["--help"]);
        return;
    }
    match std::fs::hard_link(program, at) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::CrossesDevices => {
            std::fs::copy(program, at).unwrap();
            ready(at);
        }
        Err(error) => panic!("{} could not be linked: {error}", at.display()),
    }
}

#[cfg(target_os = "macos")]
fn is_mach_o(program: &Path) -> bool {
    use std::io::Read;
    let mut magic = [0; 4];
    if std::fs::File::open(program)
        .unwrap()
        .read(&mut magic)
        .unwrap()
        != magic.len()
    {
        return false;
    }
    // Mach-O's 32/64-bit and universal headers, in either byte order.
    matches!(
        u32::from_be_bytes(magic),
        0xfeed_face
            | 0xcefa_edfe
            | 0xfeed_facf
            | 0xcffa_edfe
            | 0xcafe_babe
            | 0xbeba_feca
            | 0xcafe_babf
            | 0xbfba_feca
    )
}

/// Starts `program` once in this process, unless it already was, and waits
/// for it to end. It runs with `--help`, a `HOME` in a folder of its own and,
/// on Unix, no other variable but `PATH`, so a stand-in that writes under or
/// beside its `HOME`, or under a folder a variable names (`CODEX_HOME`),
/// writes into that folder or nowhere. A program checked into the repository
/// is a new file in every checkout, so it is made ready the same way; its
/// interpreter may refuse a `HOME` it does not know, which costs nothing,
/// because the check is of the file and is paid as it starts.
pub fn ready(program: &Path) {
    ready_with_args(program, &["--help"]);
}

/// Like `ready`, with the program's known one-shot arguments instead of
/// `--help`. The invocation must exit without starting a resident service;
/// for example, hided uses `node --version` because `--help` starts its daemon.
pub fn ready_with_args(program: &Path, args: &[&str]) {
    let id = hide_platform::fs::identity::file_id(program).unwrap();
    let mut started = READY.lock().unwrap_or_else(PoisonError::into_inner);
    if started.contains(&id) {
        return;
    }
    start_once(program, args);
    started.insert(id);
}

#[allow(clippy::disallowed_methods)] // env_clear: a Unix stand-in's first start gets no variable but PATH and its home
fn start_once(program: &Path, args: &[&str]) {
    let scratch = tempfile::tempdir().unwrap();
    let deadline = Instant::now() + READY_GUARD;
    loop {
        let mut command = Command::new(program);
        #[cfg(unix)]
        command
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default());
        command
            .args(args)
            .env(
                hide_platform::host::HOME_VARIABLE,
                scratch.path().join("home"),
            )
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        match OwnedChild::spawn(&mut command) {
            Ok(mut child) => {
                let output = child.capture_until(deadline, 1).unwrap_or_else(|error| {
                    panic!(
                        "{} did not end its first start with {args:?}, which must be a one-shot \
                         invocation: {error:?}",
                        program.display()
                    )
                });
                // A CLI or interpreter may reject --help normally. A signal
                // before main is not a completed first start.
                assert!(
                    output.status.code().is_some(),
                    "{} first start was killed: {:?}",
                    program.display(),
                    output.status
                );
                break;
            }
            Err(error)
                if error.kind() == ErrorKind::ExecutableFileBusy && Instant::now() < deadline =>
            {
                std::thread::yield_now();
            }
            Err(error) => panic!("{} could not start: {error}", program.display()),
        }
    }
}
