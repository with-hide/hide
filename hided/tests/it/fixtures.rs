//! The HOME of a daemon a test starts, on every system: the one place a
//! test makes one, so no test daemon runs under a HOME the kit reads as the
//! account's own (docs/TESTING.md, A daemon's HOME is a fixture's).

/// A folder for the HOMEs of the daemons a test starts: in the system's
/// temporary folder as the account's own home names it, the only place a
/// marked HOME is read as a fixture's (`hide_kit::layout::fixture_home`).
/// `TMPDIR` is not that place on a Mac, so `tempfile::tempdir()` is not
/// this.
pub fn fixture_root(prefix: &str) -> tempfile::TempDir {
    let temporary = if cfg!(windows) {
        hide_platform::user_agents::account_home()
            .expect("the account's home")
            .join("AppData")
            .join("Local")
            .join("Temp")
    } else {
        std::path::PathBuf::from("/tmp")
    };
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(temporary)
        .expect("a folder in the system's temporary folder")
}

/// Makes `home` the HOME of a daemon a test starts, marked as a fixture's.
/// The account is the one that runs the test whatever HOME says, and a
/// daemon under an unmarked HOME reaches what is that account's: a paired
/// core loads a login item into it (PRD device-connect-iroh D-41), which a
/// test that fails partway leaves there. A HOME the kit does not read as a
/// fixture's fails the test here, before any daemon starts under it.
pub fn fixture_home(home: &std::path::Path) {
    std::fs::create_dir_all(home).expect("the fixture HOME");
    std::fs::write(home.join(hide_kit::layout::FIXTURE_HOME_MARKER), "")
        .expect("the fixture HOME's marker");
    assert!(
        hide_kit::layout::fixture_home(home),
        "{} is not a fixture HOME (make its folder with `fixtures::fixture_root`): a daemon \
         under it would reach the login items of the account that runs this test",
        home.display()
    );
}

/// The lock a test holds on its fixture HOME's owner file
/// (`hided::env::FIXTURE_OWNER_FILE`) while it runs. Every `hided` under
/// that HOME ends once the lock is let go, which the system does with the
/// test's process however it ends, so a test a runner kills leaves none of
/// its daemons running, the ones it holds no handle to included (a core a
/// core update started; with-hide/hide#3119). Dropping it lets go too.
pub struct FixtureOwner {
    _lock: hide_platform::fs::lock::Lock,
}

/// Makes `home` a fixture HOME ([`fixture_home`]) this test owns
/// ([`FixtureOwner`]); taken before the first daemon starts under it.
pub fn own_fixture_home(home: &std::path::Path) -> FixtureOwner {
    fixture_home(home);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(home.join(hided::env::FIXTURE_OWNER_FILE))
        .expect("the fixture HOME's owner file");
    let lock =
        hide_platform::fs::lock::lock_file_blocking(file, hide_platform::fs::lock::Mode::Exclusive)
            .expect("the fixture HOME's owner lock");
    FixtureOwner { _lock: lock }
}
