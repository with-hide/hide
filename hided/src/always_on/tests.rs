use super::*;
use hide_peer::protocol::PeerKey;
use hide_peer::store::{Grants, Via};

const NOW: u64 = 1_800_000_000_000;

fn guest() -> PeerKey {
    PeerKey::from_bytes([7; 32])
}

/// launchd's stand-in: the session answers, `print` answers a job loaded
/// once a list is bootstrapped and until a bootout, and each bootout is
/// counted.
#[cfg(unix)]
const LAUNCHCTL: &str = "#!/bin/sh\ncase \"$1\" in\n  print) [ \"$2\" = fixture ] && exit 0; [ -f \"$HOME/loaded\" ] && exit 0; exit 113 ;;\n  bootstrap) touch \"$HOME/loaded\" ;;\n  bootout) rm -f \"$HOME/loaded\"; echo bootout >> \"$HOME/bootouts\" ;;\nesac\nexit 0\n";

struct Machine {
    home: tempfile::TempDir,
    state: PathBuf,
    agents: UserAgents,
}

impl Machine {
    /// launchd's stand-in on Unix; on Windows Task Scheduler's, whose
    /// `schtasks.exe` is a copy of this test program: given `schtasks`'
    /// arguments it runs no test and exits 0, a task created and run.
    fn new() -> Self {
        let home = tempfile::tempdir().unwrap();
        let state = home.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        #[cfg(unix)]
        let agents = {
            use std::os::unix::fs::PermissionsExt;
            let launchctl = home.path().join("launchctl");
            std::fs::write(&launchctl, LAUNCHCTL).unwrap();
            std::fs::set_permissions(&launchctl, std::fs::Permissions::from_mode(0o755)).unwrap();
            UserAgents::fixture(home.path())
        };
        #[cfg(windows)]
        let agents = {
            std::fs::copy(
                std::env::current_exe().unwrap(),
                home.path().join("schtasks.exe"),
            )
            .unwrap();
            UserAgents::fixture_of(hide_platform::user_agents::Kind::TaskScheduler, home.path())
        };
        Self {
            home,
            state,
            agents,
        }
    }

    /// Records this test process as the core running on the folder, as the
    /// daemon's state names it.
    fn run_here(&self) {
        let pid = std::process::id();
        crate::state_file::write_state(
            &self.state,
            &crate::state_file::DaemonState {
                pid,
                port: 1,
                token: "t".to_owned(),
                started_at: String::new(),
                pid_started: Some(hide_platform::process::start_time(pid).unwrap()),
            },
        )
        .unwrap();
    }

    #[cfg(unix)]
    fn bootouts(&self) -> usize {
        std::fs::read_to_string(self.home.path().join("bootouts"))
            .map_or(0, |calls| calls.lines().count())
    }

    fn pair(&self) -> String {
        let mut grants = Grants::open(hide_kit::layout::peer_grants(&self.state)).unwrap();
        let grant = grants
            .grant_pending(guest(), "d1", "Laptop", Via::Link, NOW)
            .unwrap();
        grants.confirm(&guest(), &grant.id, NOW).unwrap();
        grant.id
    }

    fn unpair(&self) {
        let mut grants = Grants::open(hide_kit::layout::peer_grants(&self.state)).unwrap();
        grants.revoke(&guest(), None, NOW).unwrap();
    }

    /// Reconciles as a core that runs `program`, recording each install of
    /// a build in the install root.
    fn reconcile(&self, under_item: bool, installs: &std::cell::Cell<usize>) -> Outcome {
        let program = self.home.path().join("app/hided");
        let asker = Asker {
            program: &program,
            under_item,
        };
        reconcile(
            &self.agents,
            self.home.path(),
            &self.state,
            &asker,
            |root| {
                installs.set(installs.get() + 1);
                Ok(root.join("0123456789abcdef").join("hided"))
            },
        )
        .unwrap()
    }

    fn installed(&self) -> bool {
        crate::login_item::installed(&self.agents, self.home.path(), &self.state).unwrap()
    }

    #[cfg(unix)]
    fn loaded(&self) -> bool {
        self.home.path().join("loaded").exists()
    }

    fn kept(&self) -> bool {
        hide_kit::layout::keep_alive(&self.state).exists()
    }
}

/// B30 / c2: nothing is put in before the first pairing; the first pairing
/// puts in the login item, which runs a build in the install root with the
/// marker, once; the last removal takes the item and the marker out.
#[cfg(unix)]
#[test]
fn the_first_pairing_puts_the_login_item_in_and_the_last_removal_takes_it_out() {
    let machine = Machine::new();
    let installs = std::cell::Cell::new(0);
    assert_eq!(machine.reconcile(false, &installs), Outcome::Nothing);
    assert!(!machine.installed() && !machine.kept());

    machine.pair();
    let outcome = machine.reconcile(false, &installs);
    assert_eq!(
        outcome,
        Outcome::Installed {
            program: hide_kit::layout::core_root(machine.home.path())
                .join("0123456789abcdef")
                .join("hided"),
            warning: None,
        }
    );
    assert!(machine.installed() && machine.loaded() && machine.kept());
    let list = std::fs::read_to_string(machine.home.path().join("Library/LaunchAgents").join(
        format!(
            "{}.plist",
            hide_kit::layout::core_login_item(machine.home.path(), &machine.state)
        ),
    ))
    .unwrap();
    assert!(
        list.contains(&format!(
            "<string>core-login</string><string>--state-dir</string><string>{}</string>",
            machine.state.display()
        )),
        "{list}"
    );
    assert!(!list.contains("HIDE_KEEP_ALIVE"), "{list}");

    assert_eq!(machine.reconcile(false, &installs), Outcome::Kept);
    assert_eq!(installs.get(), 1, "a paired machine's build is put in once");

    machine.unpair();
    assert_eq!(machine.reconcile(false, &installs), Outcome::Removed);
    assert!(!machine.installed() && !machine.loaded() && !machine.kept());
}

/// The core the item runs, asking after the last removal, only forgets the
/// item: unloading it would stop the asking core midway. Without the
/// marker the item starts nothing of it again.
#[cfg(unix)]
#[test]
fn the_core_the_item_runs_forgets_its_item_and_keeps_running() {
    let machine = Machine::new();
    let installs = std::cell::Cell::new(0);
    machine.pair();
    machine.reconcile(false, &installs);
    machine.unpair();
    assert_eq!(machine.reconcile(true, &installs), Outcome::Removed);
    assert!(!machine.installed() && !machine.kept());
    assert!(machine.loaded(), "the asking core's job was unloaded");
}

/// The first pairing's item is put in by the core running on the folder,
/// which nothing stops: the item's core only waits on the folder's lock
/// (S1). On Windows the item's start once ended the running core by its
/// pid, which here is this test's own process.
#[test]
fn the_core_that_puts_its_item_in_keeps_running() {
    let machine = Machine::new();
    machine.run_here();
    let installs = std::cell::Cell::new(0);
    machine.pair();
    assert!(matches!(
        machine.reconcile(false, &installs),
        Outcome::Installed { .. }
    ));
    assert!(machine.installed() && machine.kept());
    #[cfg(unix)]
    assert_eq!(machine.bootouts(), 0, "the asking core's job was unloaded");
    assert_eq!(machine.reconcile(false, &installs), Outcome::Kept);
}

/// The core the item runs forgets its item at the last removal and keeps
/// running; paired again in the same run, it puts the item back over its
/// own run without unloading it (S1).
#[test]
fn the_core_the_item_runs_paired_again_adopts_its_item() {
    let machine = Machine::new();
    machine.run_here();
    let installs = std::cell::Cell::new(0);
    machine.pair();
    machine.reconcile(false, &installs);
    machine.unpair();
    assert_eq!(machine.reconcile(true, &installs), Outcome::Removed);
    assert!(!machine.installed() && !machine.kept());

    machine.pair();
    assert!(matches!(
        machine.reconcile(true, &installs),
        Outcome::Installed { .. }
    ));
    assert!(machine.installed() && machine.kept());
    #[cfg(unix)]
    {
        assert!(machine.loaded());
        assert_eq!(machine.bootouts(), 0, "the asking core's job was unloaded");
    }
}

/// An item whose definition is there but that the system no longer holds
/// (Hide turned off in the login items, a failed load) is loaded again, so
/// a paired machine's item is kept only while it can run the core (O2).
#[cfg(unix)]
#[test]
fn an_item_the_system_dropped_is_loaded_again() {
    let machine = Machine::new();
    let installs = std::cell::Cell::new(0);
    machine.pair();
    machine.reconcile(false, &installs);
    std::fs::remove_file(machine.home.path().join("loaded")).unwrap();
    assert_eq!(machine.reconcile(false, &installs), Outcome::Kept);
    assert!(machine.loaded());
}

/// A system with no login items puts nothing in; a paired machine there is
/// told why.
#[test]
fn a_system_with_no_login_items_puts_nothing_in() {
    let home = tempfile::tempdir().unwrap();
    let state = home.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let mut grants = Grants::open(hide_kit::layout::peer_grants(&state)).unwrap();
    let grant = grants
        .grant_pending(guest(), "d1", "Laptop", Via::Link, NOW)
        .unwrap();
    grants.confirm(&guest(), &grant.id, NOW).unwrap();
    let program = home.path().join("hided");
    let asker = Asker {
        program: &program,
        under_item: false,
    };
    let outcome = reconcile(&UserAgents::none(), home.path(), &state, &asker, |_| {
        panic!("a build was put in")
    })
    .unwrap();
    assert_eq!(
        outcome,
        Outcome::Unsupported {
            reason: "this system has no login agents".to_owned()
        }
    );
}
