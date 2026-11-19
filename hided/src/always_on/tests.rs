use super::*;
use hide_peer::protocol::PeerKey;
use hide_peer::store::{Grants, Via};

const NOW: u64 = 1_800_000_000_000;

fn guest() -> PeerKey {
    PeerKey::from_bytes([7; 32])
}

/// launchd's stand-in: the session answers, and `print` answers a job
/// loaded once a list is bootstrapped and until a bootout.
#[cfg(unix)]
const LAUNCHCTL: &str = "#!/bin/sh\ncase \"$1\" in\n  print) [ \"$2\" = fixture ] && exit 0; [ -f \"$HOME/loaded\" ] && exit 0; exit 113 ;;\n  bootstrap) touch \"$HOME/loaded\" ;;\n  bootout) rm -f \"$HOME/loaded\" ;;\nesac\nexit 0\n";

#[cfg(unix)]
struct Machine {
    home: tempfile::TempDir,
    state: PathBuf,
    agents: UserAgents,
}

#[cfg(unix)]
impl Machine {
    fn new() -> Self {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().unwrap();
        let state = home.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        let launchctl = home.path().join("launchctl");
        std::fs::write(&launchctl, LAUNCHCTL).unwrap();
        std::fs::set_permissions(&launchctl, std::fs::Permissions::from_mode(0o755)).unwrap();
        let agents = UserAgents::fixture(home.path());
        Self {
            home,
            state,
            agents,
        }
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

/// A core a move placed here keeps the move's own item, paired or not.
#[cfg(unix)]
#[test]
fn a_moved_core_s_item_is_the_move_s() {
    use crate::core_move::handover::{Handover, HandoverState, hold};
    let machine = Machine::new();
    hold(&machine.state)
        .unwrap()
        .write(&Handover::new("i1", "m", "c", HandoverState::Active))
        .unwrap();
    let installs = std::cell::Cell::new(0);
    machine.pair();
    assert_eq!(machine.reconcile(false, &installs), Outcome::Moved);
    assert_eq!(installs.get(), 0);
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
