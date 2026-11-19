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
