//! A WSL distribution of this PC as a Hide of its own (PRD
//! device-connect-iroh D-27, B7, B36, B43; security spec 17): the commands
//! [`crate::pairing_ssh`] runs on a target over SSH, run in the
//! distribution through `wsl.exe`, and the carrier the connection between
//! the two cores runs on.
//!
//! `wsl.exe` signs nobody in: a distribution's default user is the PC
//! account's own, so the look, the pairing and a helper device's switch ask
//! nothing and compare no words, as over an SSH login that already signed
//! in.
//!
//! The connection itself goes the same way. `hide device carry` in the
//! distribution joins its input and output to the distribution's own core,
//! and the pipe `wsl.exe` gives it is the carrier (`hide_peer::pipe`):
//! only this PC makes one (D-62), each end proves its machine key on it
//! (D-63), and it is this PC's child, ended with the connection.
//!
//! A distribution that is off is started only by the operator: a dial made
//! in the background finds it off and stops there, and one made in the
//! minute after the operator opened it, or by a pairing they pressed, may
//! start it (B43, [`Carriers::may_wake`]); a switch starts none, whoever
//! asked for it ([`move_device`]).

use std::collections::HashMap;
use std::io::{self, Read};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use hide_node::ssh::host::wsl::WslAccess;
use hide_node::ssh::session::Ran;
use hide_peer::mux::Carrier;
use hide_peer::pairing::Me;
use hide_peer::protocol::{Candidate, PeerKey};
use hide_platform::process::OwnedChild;
use serde_json::json;

use crate::pairing_ssh::{self, Channel, Login, Looked, Paired, Reach, Switch, Switched};

/// How long after the operator opened a distribution a dial may start it.
const WAKE_FOR: Duration = Duration::from_secs(60);

/// The most of what a carrier's command said that is kept for the log.
const SAID_CAP: usize = 4096;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A map of instants, or a child to end: whole whatever panicked beside it.
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The default user's account in `distro`, reached through `wsl.exe`.
struct Wsl<'a> {
    access: &'a WslAccess,
    distro: &'a str,
}

impl Login for Wsl<'_> {
    fn channel(&self) -> Channel {
        Channel::Wsl
    }

    fn name(&self) -> &str {
        self.distro
    }

    fn system(&self) -> Result<String, String> {
        Ok("linux".to_owned())
    }

    fn on(&self) -> Result<bool, String> {
        self.access.runs(self.distro)
    }

    fn run(&self, argv: &[String], cap: usize, within: Duration) -> Result<Ran, String> {
        self.access.run(self.distro, argv, cap, within)
    }

    /// The distribution's name is how this PC reaches it, ahead of every
    /// address the distribution names for itself.
    fn first(&self, _own: &[Candidate]) -> Vec<Candidate> {
        vec![Candidate::Wsl(self.distro.to_owned())]
    }

    fn install(
        &self,
        packages: &hide_node::ssh::host::HelperPackages,
        root: &str,
    ) -> Result<String, pairing_ssh::NotInstalled> {
        self.access
            .install_build(self.distro, packages, root)
            .map_err(pairing_ssh::NotInstalled::Failed)
    }
}

/// Brings the Hide of `distro`, which this PC switched, to this build
/// (`crate::started_cores`), through `wsl.exe`.
pub fn raise(
    access: &WslAccess,
    distro: &str,
    packages: &hide_node::ssh::host::HelperPackages,
) -> Result<crate::started_cores::Raised, crate::started_cores::Failure> {
    crate::started_cores::raise_on(&Wsl { access, distro }, packages)
}

/// Asks `distro` whether it can pair through `wsl.exe`. Blocks for as long
/// as the distribution takes to start and answer.
///
/// A distribution's own Hide that is not running is started first: with
/// no systemd nothing there starts it, so it stopped with the distribution,
/// and the operator who opened its tile is who may wake it (B43). An
/// account that never ran a Hide of its own is left as it is.
pub fn look(access: &WslAccess, reach: &Reach, distro: &str) -> Looked {
    woken(&Wsl { access, distro }, reach)
}

fn woken(login: &Wsl<'_>, reach: &Reach) -> Looked {
    match pairing_ssh::look_on(login, reach) {
        Looked::NotRunning { system } => match pairing_ssh::start_core(login, reach) {
            Ok(()) => pairing_ssh::look_on(login, reach),
            Err(_) => Looked::NotRunning { system },
        },
        looked => looked,
    }
}

/// Has the Hide `distro` runs save a pending grant for this machine under
/// `request`, and answers what it said (`pairing_ssh::accept`).
pub fn accept(
    access: &WslAccess,
    reach: &Reach,
    distro: &str,
    request: &str,
    me: &Me,
    key: &PeerKey,
) -> Result<Paired, &'static str> {
    pairing_ssh::pair_on(&Wsl { access, distro }, reach, request, me, key)
}

/// One switch of the helper device `distro` is (`pairing_ssh::switch`); a
/// Hide of the distribution's own that stopped with it is started first,
/// as [`look`] starts it.
fn switch(access: &WslAccess, distro: &str, switch: &Switch<'_>) -> Result<Switched, &'static str> {
    let login = Wsl { access, distro };
    woken(&login, switch.reach);
    pairing_ssh::switch_on(&login, switch)
}

/// A switch found its distribution off, and asked it nothing (B43).
pub const OFF: &str = "wsl_off";

/// Whether the helper device `distro` is switched
/// (`pairing_ssh::may_move`). A distribution that is off is asked nothing,
/// since any command run in it would start it (B43), and is not moved.
pub fn may_move(
    access: &WslAccess,
    reach: &Reach,
    distro: &str,
) -> Result<pairing_ssh::Move, &'static str> {
    if !matches!(access.runs(distro), Ok(true)) {
        return Err(OFF);
    }
    pairing_ssh::may_move(&Wsl { access, distro }, reach)
}

/// One switch of the helper device `distro` is: a distribution that went
/// off since it was looked at ([`may_move`]) is asked nothing (B43).
pub fn move_device(
    access: &WslAccess,
    distro: &str,
    switch: &Switch<'_>,
) -> Result<Switched, &'static str> {
    if !matches!(access.runs(distro), Ok(true)) {
        return Err(OFF);
    }
    self::switch(access, distro, switch)
}

/// What makes the carriers of this PC's connections to its distributions.
pub struct Carriers {
    access: WslAccess,
    reach: Reach,
    /// The distributions the operator opened, each until an instant.
    waking: Mutex<HashMap<String, Instant>>,
}

impl Carriers {
    pub fn new(access: WslAccess, reach: Reach) -> Arc<Self> {
        Arc::new(Self {
            access,
            reach,
            waking: Mutex::default(),
        })
    }

    /// How this PC reaches WSL.
    pub fn access(&self) -> &WslAccess {
        &self.access
    }

    /// The operator opened `distro`, or pressed what pairs or switches it:
    /// a dial in the next [`WAKE_FOR`] may start it when it is off (B43).
    pub fn may_wake(&self, distro: &str) {
        let now = Instant::now();
        let mut waking = lock(&self.waking);
        waking.retain(|_, until| *until > now);
        waking.insert(distro.to_owned(), now + WAKE_FOR);
    }

    fn wakes(&self, distro: &str) -> bool {
        lock(&self.waking)
            .get(distro)
            .is_some_and(|until| *until > Instant::now())
    }

    /// Starts the carrier to `distro`'s core. Blocking: it asks `wsl.exe`
    /// which distributions run, unless the operator just opened this one.
    pub fn carrier(&self, distro: &str) -> io::Result<Carrier> {
        if !self.wakes(distro) && !self.access.runs(distro).map_err(io::Error::other)? {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                format!("{distro} is off; it starts when the operator opens it"),
            ));
        }
        let argv = pairing_ssh::hide_argv(&self.reach, "linux", &["device", "carry"]);
        let child = self.access.start(distro, &argv).map_err(io::Error::other)?;
        carrier_of(child, distro)
    }
}

/// The carrier a child's input and output are. The child is the carrier's:
/// its end ends the child and everything it started, which closes both
/// pipes under whichever thread waits on one.
fn carrier_of(mut child: OwnedChild, distro: &str) -> io::Result<Carrier> {
    let missing = |what: &str| io::Error::other(format!("the carrier's {what} is not piped"));
    let input = child.take_stdin().ok_or_else(|| missing("input"))?;
    let output = child.take_stdout().ok_or_else(|| missing("output"))?;
    let said = child.take_stderr().ok_or_else(|| missing("error output"))?;
    let name = distro.to_owned();
    std::thread::Builder::new()
        .name("peer-wsl-said".to_owned())
        .spawn(move || record_said(said, &name))?;
    let child = Mutex::new(Some(child));
    Ok(Carrier::new(output, input, move || {
        let Some(child) = lock(&child).take() else {
            return;
        };
        // Ended and waited for off the thread that asked: that thread is
        // the connection's, and a wait for `wsl.exe` to go is not its work.
        // With no thread to be had, the child is dropped, and so ended,
        // with the closure that holds it.
        let _ = std::thread::Builder::new()
            .name("peer-wsl-end".to_owned())
            .spawn(move || drop(child));
    }))
}

/// Reads what a carrier's command wrote to its error output until it ends,
/// so the command never waits on a full pipe, and records the first
/// [`SAID_CAP`] bytes: why a carrier that never connected did not.
fn record_said(mut said: impl Read, distro: &str) {
    let mut kept = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        match said.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                let room = SAID_CAP.saturating_sub(kept.len());
                kept.extend_from_slice(&buffer[..read.min(room)]);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    let text = hide_platform::wsl::decode(&kept);
    if let Some(line) = text.lines().rev().find(|line| !line.trim().is_empty()) {
        hide_core::diagnostic!(json!({
            "component": "pairing",
            "kind": "wsl.carrier_said",
            "distro": distro,
            "message": line.trim(),
        }));
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use hide_platform::wsl::Distribution;
    use std::io::Write;
    use std::path::{Path, PathBuf};

    /// A stand-in `wsl.exe`: it runs the command it is given on this
    /// machine, in a home of the test's, as `hide_node`'s own WSL tests do,
    /// and writes each call to a log. Whether its distribution runs
    /// systemd is the test's to say (a `systemd` file beside it), never
    /// the machine's the test runs on.
    const FAKE_WSL: &str = r#"#!/bin/sh
[ "$1" = "--help" ] && exit 0
here=$(dirname "$0")
printf '%s\n' "$*" >> "$here/calls.log"
if [ "$1" = "--list" ]; then
    cat "$here/running" 2>/dev/null
    exit 0
fi
while [ $# -gt 0 ]; do
    case $1 in
        -d|-u|--cd) shift 2 ;;
        -e) shift; break ;;
        *) echo "unexpected $1" >&2; exit 64 ;;
    esac
done
if [ "$*" = "test -d /run/systemd/system" ]; then
    [ -e "$here/systemd" ]
    exit $?
fi
HOME=$here/home
export HOME
cd "$HOME" || exit 1
exec "$@"
"#;

    struct Pc {
        dir: tempfile::TempDir,
        access: WslAccess,
        reach: Reach,
    }

    /// A stand-in program at `path`, started once before the test's
    /// deadlines run it (`stand_ins`): its text is fixed, and it finds the
    /// test's files beside itself or under the `HOME` it is run with.
    fn script(path: &Path, text: &str) {
        crate::stand_ins::program(path, text);
    }

    /// A PC whose one distribution, Ubuntu, has `hide` as its `hide`.
    fn pc(hide: Option<&str>) -> Pc {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("wsl.exe");
        script(&program, FAKE_WSL);
        std::fs::create_dir_all(dir.path().join("home")).unwrap();
        if let Some(hide) = hide {
            script(&dir.path().join("home/.local/bin/hide"), hide);
        }
        let found = Distribution {
            name: "Ubuntu".to_owned(),
            id: "25622596-db24-4e3a-9a3e-6b3f0d2e1c11".to_owned(),
            wsl_version: 2,
            installed: true,
            first_run_pending: false,
            default: true,
        };
        let access = WslAccess::with(program, Arc::new(move || Ok(vec![found.clone()])));
        let reach = Reach {
            home: PathBuf::from("/unused"),
            command_folder: None,
            hide: None,
        };
        Pc { dir, access, reach }
    }

    /// [`pc`], with an empty file `mark` in the distribution's home.
    fn pc_with(hide: &str, mark: &str) -> Pc {
        let pc = pc(Some(hide));
        std::fs::write(pc.dir.path().join("home").join(mark), "").unwrap();
        pc
    }

    impl Pc {
        fn calls(&self) -> Vec<String> {
            std::fs::read_to_string(self.dir.path().join("calls.log"))
                .unwrap_or_default()
                .lines()
                .map(str::to_owned)
                .collect()
        }

        fn running(&self, names: &str) {
            std::fs::write(self.dir.path().join("running"), names).unwrap();
        }
    }

    /// Waits for `ready` to answer; the bound is a hang guard.
    #[allow(clippy::disallowed_methods)] // a bounded polling helper
    fn until<T>(what: &str, mut ready: impl FnMut() -> Option<T>) -> T {
        let over = Instant::now() + Duration::from_secs(60);
        loop {
            if let Some(value) = ready() {
                return value;
            }
            assert!(Instant::now() < over, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(10)); // wait-ok: the next poll of a file the stand-in writes once it runs
        }
    }

    fn me() -> Me {
        Me {
            name: "Windows PC".to_owned(),
            device: "pc-device".to_owned(),
        }
    }

    /// A `hide` that answers `look-ssh` as a running Hide and `accept-ssh`
    /// with a pairing, and records the words it was given.
    const HIDE: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$HOME/hide.log"
case "$2" in
    look-ssh) echo '{"ok":true,"value":{"look":{"system":"linux","core":true}}}' ;;
    accept-ssh) printf '{"ok":true,"value":{"accepted":{"key":"%s","device":"distro-device","name":"Ubuntu","grant":"g1","candidates":[{"ip":"172.20.0.2:7842"}]}}}\n' "$(cat "$HOME/key")" ;;
    *) exit 2 ;;
esac
"#;

    /// A distribution that runs a Hide is looked at and paired through
    /// `wsl.exe` alone: the target is told the grant is asked over WSL, and
    /// the first address this PC saves for it is the distribution's name.
    #[test]
    fn a_distribution_is_looked_at_and_paired_through_wsl() {
        let pc = pc(Some(HIDE));
        let theirs = hide_peer::MachineKey::generate().unwrap().public();
        std::fs::write(pc.dir.path().join("home/key"), theirs.to_hex()).unwrap();
        assert_eq!(look(&pc.access, &pc.reach, "Ubuntu"), Looked::Ready);
        let mine = hide_peer::MachineKey::generate().unwrap().public();
        let paired = accept(&pc.access, &pc.reach, "Ubuntu", "req-1", &me(), &mine)
            .unwrap_or_else(|code| panic!("{code}"));
        assert_eq!(paired.accepted.key, theirs);
        assert_eq!(
            paired.first,
            vec![Candidate::Wsl("Ubuntu".to_owned())],
            "the distribution's name comes before its own addresses"
        );
        let asked = std::fs::read_to_string(pc.dir.path().join("home/hide.log")).unwrap();
        let accept = asked.lines().last().unwrap();
        assert_eq!(
            accept,
            format!(
                "device accept-ssh --request req-1 --key {} --device pc-device --name Windows PC --wsl",
                mine.to_hex()
            )
        );
        assert!(
            pc.calls()
                .iter()
                .all(|call| call.starts_with("-d Ubuntu --cd ~ -e ")),
            "{:?}",
            pc.calls()
        );
    }

    /// A helper distribution is switched through `wsl.exe` as an SSH device
    /// is over its login (B36): its Hide is asked for the grant with the
    /// helper's folder, and handed the device's projects once the pairing
    /// is admitted (D-48); one with no systemd says the operator opens it
    /// again after Windows restarts (B43), whoever started its Hide.
    #[test]
    fn a_distribution_is_switched_through_wsl_and_says_what_no_systemd_means() {
        const SWITCHED: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$HOME/hide.log"
case "$2" in
    look-ssh) echo '{"ok":true,"value":{"look":{"system":"linux","core":true}}}' ;;
    accept-ssh) printf '{"ok":true,"value":{"helper":"taken","accepted":{"key":"%s","device":"distro-device","name":"Ubuntu","grant":"g1","candidates":[]}}}\n' "$(cat "$HOME/key")" ;;
    projects-ssh) echo '{"ok":true,"value":{"projects":1}}' ;;
    *) exit 2 ;;
esac
"#;
        let pc = pc(Some(SWITCHED));
        let theirs = hide_peer::MachineKey::generate().unwrap().public();
        std::fs::write(pc.dir.path().join("home/key"), theirs.to_hex()).unwrap();
        let mine = hide_peer::MachineKey::generate().unwrap().public();
        let projects = ["/home/me/app".to_owned()];
        let asked = || {
            switch(
                &pc.access,
                "Ubuntu",
                &Switch {
                    reach: &pc.reach,
                    request: "req-1",
                    me: &me(),
                    key: &mine,
                    helper_root: Some("~/.hide/device"),
                    projects: &projects,
                    admitted: &|_| true,
                    install: false,
                    installing: &|| panic!("a Hide that runs is not started"),
                },
            )
            .unwrap_or_else(|code| panic!("{code}"))
        };
        let Switched::Paired {
            paired,
            folder,
            needs,
        } = asked()
        else {
            panic!("the distribution's Hide did not pair");
        };
        let paired = paired.expect("a new pairing");
        assert_eq!(paired.accepted.key, theirs);
        assert_eq!(paired.first, vec![Candidate::Wsl("Ubuntu".to_owned())]);
        assert!(matches!(folder, pairing_ssh::Folder::Taken));
        assert_eq!(needs, Some(pairing_ssh::Needs::OpenDistribution));
        let log = std::fs::read_to_string(pc.dir.path().join("home/hide.log")).unwrap();
        let said: Vec<&str> = log.lines().collect();
        assert_eq!(
            said[said.len() - 2],
            format!(
                "device accept-ssh --request req-1 --key {} --device pc-device --name Windows PC --helper ~/.hide/device --wsl",
                mine.to_hex()
            )
        );
        // The projects go on the same login, after the PC admitted the
        // pairing.
        assert_eq!(
            said[said.len() - 1],
            "device projects-ssh --project /home/me/app"
        );

        // A distribution systemd started has a Hide that ran before this
        // switch: it needs nothing said.
        std::fs::write(pc.dir.path().join("systemd"), "").unwrap();
        let Switched::Paired { needs, .. } = asked() else {
            panic!("the distribution's Hide did not pair");
        };
        assert_eq!(needs, None);
    }

    /// A switch's whole way on a machine that runs no Hide of its own,
    /// against a stand-in `hide` (the journeys' fixture account is one no
    /// switch moves, so this is where the way is held): nothing is started
    /// until the install is allowed; then a Hide is started over the same
    /// login and paired with, and the projects are handed over only once
    /// this PC admitted the pairing. A switch that stopped before that
    /// handed nothing, and the next one ends it: it starts no second Hide,
    /// pairs again as the same request and hands the projects (principle
    /// 11).
    #[test]
    fn a_machine_with_no_hide_is_started_paired_and_handed_its_projects_once_admitted() {
        const ABSENT: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$HOME/hide.log"
case "$1 $2" in
    "connect ") touch "$HOME/runs"; echo '{"ok":true}' ;;
    "device look-ssh")
        if [ -e "$HOME/runs" ]; then echo '{"ok":true,"value":{"look":{"system":"linux","core":true}}}'
        else echo '{"ok":false,"error":{"code":"hide_unavailable"},"look":{"system":"linux","core":false}}'; exit 1
        fi ;;
    "device accept-ssh") printf '{"ok":true,"value":{"helper":"taken","accepted":{"key":"%s","device":"distro-device","name":"Ubuntu","grant":"g1","candidates":[]}}}\n' "$(cat "$HOME/key")" ;;
    "device projects-ssh") echo '{"ok":true,"value":{"projects":1}}' ;;
    "always-on linger") echo '{"ok":true,"value":{"linger":"on","unattended":true}}' ;;
    *) exit 2 ;;
esac
"#;
        let pc = pc(Some(ABSENT));
        pc.running("Ubuntu\n");
        std::fs::write(pc.dir.path().join("systemd"), "").unwrap();
        let home = pc.dir.path().join("home");
        let theirs = hide_peer::MachineKey::generate().unwrap().public();
        std::fs::write(home.join("key"), theirs.to_hex()).unwrap();
        let mine = hide_peer::MachineKey::generate().unwrap().public();
        let projects = ["/home/me/app".to_owned()];
        let installs = std::sync::atomic::AtomicUsize::new(0);
        let asked = |install: bool, admitted: bool| {
            move_device(
                &pc.access,
                "Ubuntu",
                &Switch {
                    reach: &pc.reach,
                    request: "req-1",
                    me: &me(),
                    key: &mine,
                    helper_root: Some("~/.hide/device"),
                    projects: &projects,
                    admitted: &|_| admitted,
                    install,
                    installing: &|| {
                        installs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    },
                },
            )
        };
        let said = || {
            std::fs::read_to_string(home.join("hide.log"))
                .unwrap_or_default()
                .lines()
                .map(|line| line.split(' ').take(2).collect::<Vec<_>>().join(" "))
                .collect::<Vec<_>>()
        };
        let started = || installs.load(std::sync::atomic::Ordering::SeqCst);

        // Not allowed: the machine is only named, and nothing is started
        // (the first look is the one that would wake a Hide of its own).
        assert!(matches!(
            asked(false, true),
            Ok(Switched::InstallNeeded { system }) if system == "linux"
        ));
        assert_eq!(
            (said(), started()),
            (["device look-ssh"; 2].map(str::to_owned).to_vec(), 0)
        );

        // Allowed, and this PC did not admit the pairing (the connection
        // never confirmed it): a Hide was started and paired with, what it
        // still needs was read, and no project was handed over.
        std::fs::remove_file(home.join("hide.log")).unwrap();
        let Ok(Switched::Paired { paired, needs, .. }) = asked(true, false) else {
            panic!("the started Hide did not pair");
        };
        assert_eq!(paired.expect("a new pairing").accepted.key, theirs);
        assert_eq!(needs, None);
        assert_eq!(
            (said(), started()),
            (
                [
                    "device look-ssh",
                    "device look-ssh",
                    "connect",
                    "device look-ssh",
                    "device accept-ssh",
                    "always-on linger",
                ]
                .map(str::to_owned)
                .to_vec(),
                1
            )
        );

        // The next switch ends it: the Hide runs, so none is started, the
        // same request pairs, and the projects follow.
        std::fs::remove_file(home.join("hide.log")).unwrap();
        assert!(matches!(asked(true, true), Ok(Switched::Paired { .. })));
        assert_eq!(
            (said(), started()),
            (
                [
                    "device look-ssh",
                    "device look-ssh",
                    "device accept-ssh",
                    "device projects-ssh",
                ]
                .map(str::to_owned)
                .to_vec(),
                1
            )
        );
    }

    /// The operator's decision of 2026-11-24, and the rule of what moves
    /// with nobody asked: a distribution that is off is asked nothing, one
    /// whose Hide is not back after a restart with nobody there is asked
    /// that one question and left as it is, and one whose Hide is back is
    /// switched. What its `hide always-on linger` says is the answer; an
    /// older `hide`, or one that says nothing, is not known to be back.
    #[test]
    fn a_distribution_moves_by_itself_only_when_it_runs_and_its_hide_is_back_unattended() {
        const MOVED: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$HOME/hide.log"
[ "$1" = "always-on" ] && { cat "$HOME/linger"; exit 0; }
case "$2" in
    look-ssh) echo '{"ok":true,"value":{"look":{"system":"linux","core":true}}}' ;;
    accept-ssh) printf '{"ok":true,"value":{"helper":"taken","accepted":{"key":"%s","device":"distro-device","name":"Ubuntu","grant":"g1","candidates":[]}}}\n' "$(cat "$HOME/key")" ;;
    projects-ssh) echo '{"ok":true,"value":{"projects":0}}' ;;
    *) exit 2 ;;
esac
"#;
        let pc = pc_with(MOVED, "linger");
        std::fs::write(pc.dir.path().join("systemd"), "").unwrap();
        let home = pc.dir.path().join("home");
        let theirs = hide_peer::MachineKey::generate().unwrap().public();
        std::fs::write(home.join("key"), theirs.to_hex()).unwrap();
        let mine = hide_peer::MachineKey::generate().unwrap().public();
        let moved = || {
            move_device(
                &pc.access,
                "Ubuntu",
                &Switch {
                    reach: &pc.reach,
                    request: "req-1",
                    me: &me(),
                    key: &mine,
                    helper_root: Some("~/.hide/device"),
                    projects: &[],
                    admitted: &|_| true,
                    install: true,
                    installing: &|| {},
                },
            )
        };
        let may = || may_move(&pc.access, &pc.reach, "Ubuntu");
        let said = || {
            std::fs::read_to_string(home.join("hide.log"))
                .unwrap_or_default()
                .lines()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };

        // Off: nothing is run in it, by the look or by the switch.
        pc.running("");
        assert_eq!(may(), Err(OFF));
        assert_eq!(moved().err(), Some(OFF));
        assert_eq!(pc.calls(), ["--list --running --quiet"; 2]);
        assert!(said().is_empty());

        // On, and its Hide is not back with nobody there, or does not say.
        pc.running("Ubuntu");
        let left = [
            r#"{"ok":true,"value":{"linger":"needs_administrator","command":"sudo loginctl enable-linger me","unattended":false}}"#,
            r#"{"ok":true,"value":{"linger":"not_used","unattended":false}}"#,
            r#"{"ok":true,"value":{"linger":"on"}}"#,
            // Its `loginctl` could not be run: the command failed there.
            r#"{"ok":false,"reason":"linger: No such file or directory","unattended":true}"#,
            "",
        ];
        for answer in left {
            std::fs::write(home.join("linger"), answer).unwrap();
            assert!(!may().unwrap().back, "{answer}");
        }
        assert_eq!(
            said(),
            vec!["always-on linger --json"; left.len()],
            "a distribution that is left was asked for more than the one answer"
        );

        // Back with nobody there: it may move, and is switched as a press
        // switches it.
        std::fs::write(
            home.join("linger"),
            r#"{"ok":true,"value":{"linger":"on","unattended":true}}"#,
        )
        .unwrap();
        assert!(may().unwrap().back);
        let Ok(Switched::Paired { paired, .. }) = moved() else {
            panic!("the distribution was not switched");
        };
        assert_eq!(paired.expect("a new pairing").accepted.key, theirs);
    }

    /// A distribution's own Hide that stopped with it is started by the
    /// look the operator's choice of its tile asked for, and then answers
    /// (B43); one that cannot be started reads as not running.
    #[test]
    fn a_hide_that_stopped_with_its_distribution_is_started_by_the_look() {
        const STOPPED: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$HOME/hide.log"
case "$1 $2" in
    "connect ") [ -e "$HOME/broken" ] && exit 1; touch "$HOME/runs"; echo '{"ok":true}' ;;
    "device look-ssh")
        if [ -e "$HOME/runs" ]; then echo '{"ok":true,"value":{"look":{"system":"linux","core":true}}}'
        else echo '{"ok":false,"error":{"code":"hide_unavailable"},"look":{"system":"linux","core":true}}'; exit 1
        fi ;;
    *) exit 2 ;;
esac
"#;
        let pc = pc(Some(STOPPED));
        assert_eq!(look(&pc.access, &pc.reach, "Ubuntu"), Looked::Ready);
        let log = std::fs::read_to_string(pc.dir.path().join("home/hide.log")).unwrap();
        assert_eq!(
            log.lines().collect::<Vec<_>>(),
            ["device look-ssh", "connect", "device look-ssh"]
        );

        let broken = pc_with(STOPPED, "broken");
        assert_eq!(
            look(&broken.access, &broken.reach, "Ubuntu"),
            Looked::NotRunning {
                system: "linux".to_owned()
            }
        );
    }

    /// A distribution with no `hide`, or one too old to answer, reads as an
    /// SSH target's does, and one `wsl.exe` cannot run in is not reached
    /// under the channel's own code.
    #[test]
    fn a_distribution_with_no_hide_or_none_reached_reads_as_such() {
        let absent = pc(None);
        assert_eq!(
            look(&absent.access, &absent.reach, "Ubuntu"),
            Looked::Absent
        );
        let old = pc(Some("#!/bin/sh\necho 'usage: hide device list'\nexit 2\n"));
        assert_eq!(look(&old.access, &old.reach, "Ubuntu"), Looked::UpdateThere);

        let gone = pc(Some(HIDE));
        std::fs::remove_file(gone.dir.path().join("wsl.exe")).unwrap();
        assert_eq!(
            look(&gone.access, &gone.reach, "Ubuntu"),
            Looked::NotReached
        );
        let mine = hide_peer::MachineKey::generate().unwrap().public();
        let refused = accept(&gone.access, &gone.reach, "Ubuntu", "r", &me(), &mine);
        assert_eq!(refused.err(), Some("wsl_not_reached"));
        // A name `wsl.exe` would read as something else never reaches it.
        assert_eq!(
            look(&gone.access, &gone.reach, "-d evil"),
            Looked::NotReached
        );
    }

    /// A `hide device carry` that echoes: what the carrier's writer sends
    /// comes back to its reader.
    const ECHO: &str =
        "#!/bin/sh\n[ \"$1 $2\" = 'device carry' ] || exit 2\necho $$ > \"$HOME/pid\"\nexec cat\n";

    /// The carrier is the command's own input and output, and its end ends
    /// the command.
    #[test]
    fn a_carrier_is_the_commands_pipes_and_its_end_ends_the_command() {
        let pc = pc(Some(ECHO));
        pc.running("Ubuntu\n");
        let carriers = Carriers::new(pc.access.clone(), pc.reach.clone());
        let carrier = carriers.carrier("Ubuntu").unwrap();
        // The command runs before anything is asked of it: a carrier ended
        // sooner ends a `wsl.exe` that started nothing.
        let pid: u32 = until("the carrier's command to start", || {
            std::fs::read_to_string(pc.dir.path().join("home/pid"))
                .ok()
                .and_then(|pid| pid.trim().parse().ok())
        });
        // Named while it runs, so what is asked later is whether this
        // process ended, never whether its pid names something.
        let started = hide_platform::process::start_time(pid).unwrap();
        let (pipe, _accepted) = hide_peer::mux::Pipe::start(carrier, true).unwrap();
        // The echo sends this end's own frames back: its greeting is read
        // as the other end's, and its close as the other end's close.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let shutdown = hide_peer::protocol::CloseCode::Shutdown.number();
        let code = runtime.block_on(async {
            let mut closed = pipe.closed();
            pipe.close(shutdown);
            tokio::time::timeout(Duration::from_secs(20), closed.wait_for(Option::is_some))
                .await
                .expect("the carrier ended")
                .map(|code| *code)
                .unwrap()
        });
        assert_eq!(code, Some(shutdown));
        until("the carrier's command to end", || {
            let runs = hide_platform::process::is_alive(pid)
                && hide_platform::process::start_time(pid).ok() == Some(started);
            (!runs).then_some(())
        });
    }

    /// A background dial finds a distribution that is off and starts
    /// nothing in it; one the operator opened may be started.
    #[test]
    fn a_distribution_that_is_off_is_started_only_for_the_operator() {
        let pc = pc(Some(ECHO));
        pc.running("");
        let carriers = Carriers::new(pc.access.clone(), pc.reach.clone());
        let refused = carriers.carrier("Ubuntu").err().expect("off");
        assert_eq!(refused.kind(), io::ErrorKind::NotConnected);
        assert_eq!(pc.calls(), ["--list --running --quiet"]);

        carriers.may_wake("Ubuntu");
        let carrier = carriers.carrier("Ubuntu").unwrap();
        let calls = until("the carrier's command to be run", || {
            Some(pc.calls()).filter(|calls| calls.len() > 1)
        });
        carrier.end();
        assert_eq!(calls.len(), 2, "{calls:?}");
        assert!(
            calls[1].ends_with(".local/bin/hide device carry"),
            "{calls:?}"
        );
    }

    /// What a carrier's command says before it gives up is read to its end
    /// and kept to a bound.
    #[test]
    fn what_a_carrier_said_is_read_to_its_end() {
        let mut said = Vec::new();
        for _ in 0..4 * SAID_CAP {
            said.write_all(b"x").unwrap();
        }
        said.write_all(b"\nhide_unavailable\n").unwrap();
        let mut reader = io::Cursor::new(said);
        record_said(&mut reader, "Ubuntu");
        assert_eq!(reader.position() as usize, reader.get_ref().len());
    }
}
