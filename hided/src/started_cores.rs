//! A Hide this machine switched on another machine keeps up with this
//! machine's build (PRD device-connect-iroh B23).
//!
//! A machine that was a helper device, or a WSL distribution, runs a Hide
//! of its own once it is switched, started from the programs a machine
//! that reaches it over a login had installed there. While no app there
//! updates it, a machine that reaches it does: it puts its own build beside
//! the one that runs, through the login its switch went through
//! (`hide_peer::store::Login`, kept with the host's record), and runs that
//! build's `hided core-update` there, which decides under the account's own
//! lock what the core is and whether it rises (`crate::core_update`). A
//! newer core is never lowered, two machines that raise one core do so one
//! after the other, and a new build that does not start gives way to the
//! one that ran. An account that has the desktop app answers that its app
//! updates it, and a core nothing would start again stays on the build it
//! runs: neither is a failure, and neither shows on the machine's row.
//!
//! [`raise_on`] is one such raise on a login. When it runs and what a row
//! shows of it are the pairing's (`crate::pairing`).

use hide_node::ssh::host::HelperPackages;
use serde_json::json;

use crate::core_update::{self, Outcome};
use crate::pairing_ssh::{Channel, Login, NotInstalled};

/// How a raise ended when nothing stood in its way.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Raised {
    /// The core runs this build now: it restarted, and the connection to
    /// it is made again.
    Updated,
    /// It ran this build already.
    Current,
    /// It runs a newer build, which another machine brought.
    Kept,
    /// Its account has the desktop app, which updates it from then on.
    LeftToApp,
    /// Nothing on its account starts its core again (a WSL distribution
    /// with no systemd), so it stays on the build it runs: a new build that
    /// did not start would leave the account with no core.
    NotKept,
}

/// Why a core was not raised, as a row names it; what was said is in the
/// diagnostic log.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Failure {
    /// The login did not reach the account.
    NotReached,
    /// This build did not start there, and the core runs the build it ran.
    RolledBack,
    /// Another machine's update, or a move, runs there now.
    Busy,
    /// Anything else: the build did not go in, or it answered nothing this
    /// build reads.
    Failed,
}

/// Raises the core `login` reaches to this build, from `packages`.
pub(crate) fn raise_on(login: &dyn Login, packages: &HelperPackages) -> Result<Raised, Failure> {
    let intent = core_update::new_intent();
    let log = |kind: &str, fields: serde_json::Value| {
        let mut record = json!({
            "component": "started_cores",
            "kind": kind,
            "intent": intent,
            "target": login.name(),
        });
        if let (Some(record), serde_json::Value::Object(fields)) = (record.as_object_mut(), fields)
        {
            record.extend(fields);
        }
        hide_core::diagnostic!(record);
    };
    log("raise.started", json!({}));
    let failed = |failure: Failure, reason: String| {
        log(
            "raise.failed",
            json!({"failure": format!("{failure:?}"), "reason": reason}),
        );
        failure
    };
    // A raise runs with nobody at the row, and may have waited its turn
    // behind another: a distribution turned off meanwhile is not started
    // for it, by this question or by anything below (B43).
    match login.on() {
        Ok(true) => {}
        Ok(false) => return Err(failed(Failure::NotReached, "the target is off".to_owned())),
        Err(reason) => return Err(failed(Failure::NotReached, reason)),
    }
    // Said before a build is copied there, which this one would refuse.
    if login.channel() == Channel::Wsl && !crate::pairing_ssh::runs_systemd(login) {
        log("raise.not_kept", json!({"read": "no_systemd"}));
        return Ok(Raised::NotKept);
    }
    let program = login
        .install(packages, hide_kit::layout::HELPER_ROOT)
        .map_err(|error| match error {
            NotInstalled::NotReached(reason) => failed(Failure::NotReached, reason),
            NotInstalled::Failed(reason) => failed(Failure::Failed, reason),
        })?;
    let argv = [
        program,
        "core-update".to_owned(),
        "--intent".to_owned(),
        intent.clone(),
    ];
    let ran = login
        .run(&argv, core_update::OUTPUT_CAP, core_update::WITHIN)
        .map_err(|reason| failed(Failure::NotReached, reason))?;
    let raised = match core_update::printed(&String::from_utf8_lossy(&ran.stdout)) {
        Some(Outcome::Updated { .. }) => Raised::Updated,
        Some(Outcome::Current) => Raised::Current,
        Some(Outcome::Kept) => Raised::Kept,
        Some(Outcome::LeftToApp) => Raised::LeftToApp,
        Some(Outcome::Refused { reason }) if reason == core_update::NOT_KEPT => {
            log("raise.not_kept", json!({"read": "core_update"}));
            return Ok(Raised::NotKept);
        }
        Some(Outcome::RolledBack { reason, .. }) => {
            return Err(failed(Failure::RolledBack, reason));
        }
        Some(Outcome::Refused { reason })
            if reason == core_update::BUSY || reason == core_update::MOVE_RUNNING =>
        {
            return Err(failed(Failure::Busy, reason));
        }
        Some(Outcome::Refused { reason }) => return Err(failed(Failure::Failed, reason)),
        None => {
            let said: String = String::from_utf8_lossy(&ran.stderr)
                .trim()
                .chars()
                .take(512)
                .collect();
            return Err(failed(
                Failure::Failed,
                format!("core-update exited {}: {said}", ran.status),
            ));
        }
    };
    log("raise.done", json!({"raised": format!("{raised:?}")}));
    Ok(raised)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::time::Duration;

    use hide_node::ssh::session::Ran;
    use hide_peer::protocol::Candidate;

    use super::*;
    use crate::pairing_ssh::Channel;

    /// A login that takes the build or says why not, and answers the
    /// command it is given with `said`; asked whether systemd started the
    /// target, it answers `systemd`, and counts that question, which
    /// would start a target that is not `on`.
    struct Account {
        channel: Channel,
        on: bool,
        systemd: bool,
        probed: std::cell::Cell<u32>,
        install: Result<String, fn(String) -> NotInstalled>,
        installs: std::cell::Cell<u32>,
        said: Result<&'static str, &'static str>,
        ran: RefCell<Vec<Vec<String>>>,
    }

    impl Login for Account {
        fn channel(&self) -> Channel {
            self.channel
        }

        fn name(&self) -> &str {
            "studio"
        }

        fn system(&self) -> Result<String, String> {
            Ok("linux".to_owned())
        }

        fn on(&self) -> Result<bool, String> {
            Ok(self.on)
        }

        fn run(&self, argv: &[String], _: usize, _: Duration) -> Result<Ran, String> {
            if argv[0] == "test" {
                self.probed.set(self.probed.get() + 1);
                return Ok(Ran {
                    status: i32::from(!self.systemd),
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                });
            }
            self.ran.borrow_mut().push(argv.to_vec());
            self.said
                .map(|line| Ran {
                    status: 0,
                    stdout: format!("{line}\n").into_bytes(),
                    stderr: Vec::new(),
                })
                .map_err(str::to_owned)
        }

        fn first(&self, _: &[Candidate]) -> Vec<Candidate> {
            Vec::new()
        }

        fn install(&self, _: &HelperPackages, root: &str) -> Result<String, NotInstalled> {
            assert_eq!(root, hide_kit::layout::HELPER_ROOT);
            self.installs.set(self.installs.get() + 1);
            self.install.clone().map_err(|kind| kind("no".to_owned()))
        }
    }

    fn account(said: Result<&'static str, &'static str>) -> Account {
        Account {
            channel: Channel::Ssh,
            on: true,
            systemd: true,
            probed: Default::default(),
            install: Ok("/home/me/.hide/host-helper/aaaaaaaaaaaaaaaa/hided".to_owned()),
            installs: Default::default(),
            said,
            ran: RefCell::default(),
        }
    }

    fn raise(account: &Account) -> Result<Raised, Failure> {
        raise_on(account, &HelperPackages::new(None, crate::cli::RELEASE))
    }

    // B23: the build this machine put there is the one that is run, as a
    // raise (no build is named for it), and each answer of it reads as what
    // a row shows.
    #[test]
    fn a_raise_runs_the_build_it_put_there_and_reads_its_answer() {
        let updated = account(Ok(
            r#"{"kind":"updated","pid":7,"program":"/home/me/.hide/core/a/hided"}"#,
        ));
        assert_eq!(raise(&updated), Ok(Raised::Updated));
        let ran = updated.ran.borrow();
        assert_eq!(
            ran[0][..3],
            [
                "/home/me/.hide/host-helper/aaaaaaaaaaaaaaaa/hided",
                "core-update",
                "--intent"
            ]
        );
        assert_eq!(ran[0].len(), 4);
        for (said, raised) in [
            (r#"{"kind":"current"}"#, Ok(Raised::Current)),
            (r#"{"kind":"kept"}"#, Ok(Raised::Kept)),
            (
                r#"{"kind":"rolled_back","pid":7,"reason":"no links"}"#,
                Err(Failure::RolledBack),
            ),
            (r#"{"kind":"refused","reason":"busy"}"#, Err(Failure::Busy)),
            (
                r#"{"kind":"refused","reason":"move_running"}"#,
                Err(Failure::Busy),
            ),
            // Neither is a failure: an app there updates its own core, and
            // a core nothing starts again stays on the build it runs.
            (r#"{"kind":"left_to_app"}"#, Ok(Raised::LeftToApp)),
            (
                r#"{"kind":"refused","reason":"core_not_kept"}"#,
                Ok(Raised::NotKept),
            ),
            (
                r#"{"kind":"refused","reason":"other_build"}"#,
                Err(Failure::Failed),
            ),
            ("hided: unknown word", Err(Failure::Failed)),
        ] {
            assert_eq!(raise(&account(Ok(said))), raised, "{said}");
        }
    }

    // B23: a distribution systemd did not start has nothing that would
    // start its core again, so its raise ends before a build is copied
    // there; one systemd started is raised like any other machine.
    #[test]
    fn a_distribution_with_no_systemd_is_not_raised_and_takes_no_build() {
        let distribution = |systemd| Account {
            channel: Channel::Wsl,
            systemd,
            ..account(Ok(r#"{"kind":"current"}"#))
        };
        let without = distribution(false);
        assert_eq!(raise(&without), Ok(Raised::NotKept));
        assert_eq!(without.installs.get(), 0);
        assert!(without.ran.borrow().is_empty());
        let with = distribution(true);
        assert_eq!(raise(&with), Ok(Raised::Current));
        assert_eq!(with.installs.get(), 1);
    }

    // B43: a raise that waited its turn finds its distribution turned off.
    // Nothing is run there, the question about systemd included, since any
    // command would start it; the row says it was not reached.
    #[test]
    fn a_distribution_that_is_off_is_not_started_for_a_raise() {
        let off = Account {
            channel: Channel::Wsl,
            on: false,
            ..account(Ok(r#"{"kind":"current"}"#))
        };
        assert_eq!(raise(&off), Err(Failure::NotReached));
        assert_eq!(off.probed.get(), 0);
        assert_eq!(off.installs.get(), 0);
        assert!(off.ran.borrow().is_empty());
    }

    // B23: a login that does not reach the account, at the copy or at the
    // run, is told apart from a build that did not go in; nothing is run
    // when nothing was put there.
    #[test]
    fn a_raise_that_does_not_reach_the_account_says_so_and_runs_nothing() {
        for (kind, failure) in [
            (
                NotInstalled::NotReached as fn(String) -> NotInstalled,
                Failure::NotReached,
            ),
            (NotInstalled::Failed, Failure::Failed),
        ] {
            let unreached = Account {
                install: Err(kind),
                ..account(Ok(r#"{"kind":"current"}"#))
            };
            assert_eq!(raise(&unreached), Err(failure));
            assert!(unreached.ran.borrow().is_empty());
        }
        assert_eq!(raise(&account(Err("ssh ended"))), Err(Failure::NotReached));
    }
}
