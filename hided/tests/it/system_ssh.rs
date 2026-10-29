//! A device reached with the system `ssh` against a real `sshd` (PRD
//! system-ssh-device-link B1-B3, B6, B9, B12, B14, B22): an alias that
//! connects in a terminal connects the same, what ssh asks reaches the
//! attempt's sign-in and nothing else, and an attempt nobody may be asked
//! for asks nothing. Each attempt stops once the device's start script
//! has answered, at the install, for want of a helper build: the sign-in
//! is what these tests are about, and the full connection is the node
//! contract's and the bootstrap lane's.
//!
//! The default-key and password journeys need a key in this account's
//! `~/.ssh` and a second account with a password, which only a CI runner
//! may have; the lane's workflow provides them and names them in
//! `HIDE_E2E_SSH_DEFAULT_KEY` and `HIDE_E2E_SSH_PASSWORD`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use hide_node_link::device::{
    Answer, Attempt, DeviceConnector, EstablishError, HOST_CONSENT_CONTRACT, HostConsent, Refusal,
    Secret, SignIn, SignInWait, SshProblem, Wake,
};

/// The row's side of an attempt: answers its questions in order and keeps
/// what it was asked and shown.
#[derive(Default)]
struct Row {
    answers: Mutex<Vec<Answer>>,
    asked: Mutex<Vec<SignInWait>>,
    shown: Mutex<Vec<SignInWait>>,
}

impl Row {
    fn answering(answers: Vec<Answer>) -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(answers.into_iter().rev().collect()),
            ..Self::default()
        })
    }

    fn asked(&self) -> Vec<SignInWait> {
        self.asked.lock().unwrap().clone()
    }
}

impl SignIn for Row {
    fn show(&self, wait: Option<SignInWait>) {
        if let Some(wait) = wait {
            self.shown.lock().unwrap().push(wait);
        }
    }

    fn ask(&self, wait: SignInWait) -> Result<Answer, Refusal> {
        self.asked.lock().unwrap().push(wait);
        self.answers.lock().unwrap().pop().ok_or(Refusal::Cancelled)
    }
}

fn text(answer: &str) -> Answer {
    Answer::Text(Secret::new(answer.to_owned()))
}

#[cfg(unix)]
mod unix {
    use super::*;
    use crate::support::sshd::{Sshd, account};

    struct Fixture {
        _root: tempfile::TempDir,
        base: PathBuf,
        sshd: Sshd,
        local: PathBuf,
        packages: PathBuf,
    }

    fn fixture() -> Fixture {
        // Short, so the askpass desk's socket under it fits an address.
        let root = tempfile::Builder::new()
            .prefix("ss")
            .tempdir_in("/tmp")
            .unwrap();
        let base = std::fs::canonicalize(root.path()).unwrap();
        let home = base.join("h");
        std::fs::create_dir_all(&home).unwrap();
        let sshd = Sshd::start(
            &base.join("k"),
            &[
                ("HOME".into(), home.clone().into()),
                ("PATH".into(), "/usr/bin:/bin:/usr/sbin:/sbin".into()),
                ("HIDE_STATE_DIR".into(), base.join("st").into()),
            ],
        )
        .unwrap();
        let local = base.join("l");
        std::fs::create_dir_all(local.join(".ssh")).unwrap();
        // A build folder without a helper: the attempt stops at the install.
        let packages = base.join("p");
        std::fs::create_dir(&packages).unwrap();
        Fixture {
            _root: root,
            base,
            sshd,
            local,
            packages,
        }
    }

    impl Fixture {
        /// One attempt on `alias` under `config`, with `row` as its sign-in.
        fn attempt(
            &self,
            config: &str,
            alias: &str,
            wake: Wake,
            row: Arc<Row>,
        ) -> Result<hide_node_link::device::Established, EstablishError> {
            std::fs::write(self.local.join(".ssh/config"), config).unwrap();
            let transport = hide_node::ssh::Connector::new(Some(self.packages.clone()))
                .with_askpass_program(PathBuf::from(env!("CARGO_BIN_EXE_hide")))
                .transport(&self.local, "system-ssh", alias, None)
                .unwrap();
            transport.establish(
                &HostConsent {
                    contract: HOST_CONSENT_CONTRACT,
                    helper_root: "~/helper".to_owned(),
                    cli_dir: None,
                    granted_at_unix_ms: 1,
                    identity: None,
                },
                &[],
                Attempt {
                    wake,
                    sign_in: row,
                    on_close: Box::new(|_| {}),
                },
            )
        }

        /// A key of its own, its passphrase `passphrase`, that the server
        /// takes.
        fn key(&self, name: &str, passphrase: &str) -> PathBuf {
            let key = self.base.join(name);
            let status = std::process::Command::new("/usr/bin/ssh-keygen")
                .args(["-q", "-t", "ed25519", "-C", name, "-N", passphrase, "-f"])
                .arg(&key)
                .status()
                .unwrap();
            assert!(status.success());
            self.sshd.authorize(&key.with_extension("pub")).unwrap();
            key
        }
    }

    /// The device's start script answered: ssh signed in and the shell ran
    /// it, and only the missing helper build stopped the attempt.
    fn reached_the_device(result: Result<hide_node_link::device::Established, EstablishError>) {
        match result {
            Err(EstablishError::Unsupported(reason)) => assert!(
                reason.contains("does not include the device helper"),
                "{reason}"
            ),
            Err(error) => panic!("the attempt did not reach the device: {error}"),
            Ok(_) => panic!("the fixture has no helper to start"),
        }
    }

    /// B2, B22: a ProxyJump alias connects through its jump host as the
    /// terminal would, with one ssh of Hide's and nothing asked.
    #[test]
    #[ignore = "the system's sshd; the remote mailbox lane runs it"]
    fn an_alias_behind_a_jump_host_connects_as_in_a_terminal() {
        let fixture = fixture();
        let sshd = &fixture.sshd;
        let target = sshd
            .client_config("target")
            .replace("Host target\n", "Host target\n  ProxyJump jump\n");
        let config = format!("{}{target}", sshd.client_config("jump"));
        let row = Arc::new(Row::default());
        reached_the_device(fixture.attempt(&config, "target", Wake::Allowed, row.clone()));
        assert!(row.asked().is_empty());
        assert_eq!(sshd.accepted(), 2, "the jump host and the device");
    }

    /// B12, B14, D-08: a server ssh has never seen is asked about on the
    /// row with its fingerprint, nothing chosen for the operator; trusted,
    /// OpenSSH records it and the next attempt asks nothing. An attempt
    /// nobody may be asked for stops as needing a sign-in instead.
    #[test]
    #[ignore = "the system's sshd; the remote mailbox lane runs it"]
    fn an_unknown_server_is_trusted_on_the_row_and_known_after() {
        let fixture = fixture();
        let sshd = &fixture.sshd;
        let known = fixture.base.join("fresh_known_hosts");
        let config = sshd.client_config("device").replace(
            &sshd.known_hosts.display().to_string(),
            &known.display().to_string(),
        );

        let unattended = Arc::new(Row::default());
        match fixture.attempt(&config, "device", Wake::NotAllowed, unattended.clone()) {
            Err(EstablishError::Ssh(failure)) => {
                assert_eq!(
                    failure.problem,
                    SshProblem::SignInNeeded,
                    "{}",
                    failure.detail
                )
            }
            other => panic!("an unattended attempt went on: {:?}", other.err()),
        }
        // The core's row refuses an unattended attempt's question at once and
        // shows nothing (herdr-core `ssh_sign_in`); ssh then gives up.
        assert_eq!(unattended.asked().len(), 1);
        assert!(!known.exists() || std::fs::read_to_string(&known).unwrap().is_empty());

        let row = Row::answering(vec![Answer::Accept]);
        reached_the_device(fixture.attempt(&config, "device", Wake::Allowed, row.clone()));
        let fingerprint = sshd.fingerprint().unwrap();
        assert!(
            matches!(
                row.asked().as_slice(),
                [SignInWait::HostKey { host, fingerprint: shown }]
                    if host.contains("127.0.0.1") && shown == &fingerprint
            ),
            "{:?}",
            row.asked()
        );
        assert!(
            std::fs::read_to_string(&known)
                .unwrap()
                .contains("ssh-ed25519")
        );

        let again = Arc::new(Row::default());
        reached_the_device(fixture.attempt(&config, "device", Wake::Allowed, again.clone()));
        assert!(again.asked().is_empty());
    }

    /// B6, B7, B9, B14, B22: a key's passphrase is asked on the row with the
    /// key's file, again after a wrong answer, and once for the whole
    /// connection; an attempt nobody may be asked for asks nothing.
    #[test]
    #[ignore = "the system's sshd; the remote mailbox lane runs it"]
    fn a_keys_passphrase_is_asked_on_the_row_once_per_connection() {
        let fixture = fixture();
        let key = fixture.key("locked", "fixture passphrase");
        let config = fixture.sshd.client_config("device").replace(
            &fixture.sshd.client_key.display().to_string(),
            &key.display().to_string(),
        );

        let unattended = Arc::new(Row::default());
        let refused = fixture.attempt(&config, "device", Wake::NotAllowed, unattended.clone());
        assert!(
            matches!(&refused, Err(EstablishError::Ssh(failure)) if failure.problem == SshProblem::SignInNeeded),
            "{:?}",
            refused.err()
        );
        // The core's row refuses an unattended attempt's question at once and
        // shows nothing (herdr-core `ssh_sign_in`); ssh then gives up.
        assert_eq!(unattended.asked().len(), 1);

        let row = Row::answering(vec![text("wrong"), text("fixture passphrase")]);
        reached_the_device(fixture.attempt(&config, "device", Wake::Allowed, row.clone()));
        let asked = row.asked();
        assert_eq!(asked.len(), 2, "{asked:?}");
        for question in &asked {
            assert!(
                matches!(question, SignInWait::Secret { key_file: Some(file), .. } if Path::new(file) == key),
                "{question:?}"
            );
        }
    }

    /// B1: a key ssh finds by itself in the account's own `~/.ssh`, named
    /// nowhere in the alias, signs in as it does in a terminal.
    #[test]
    #[ignore = "a key in the runner's own ~/.ssh; the remote mailbox lane provides it"]
    fn a_default_key_named_nowhere_signs_in() {
        let Some(public) = std::env::var_os("HIDE_E2E_SSH_DEFAULT_KEY") else {
            panic!("set HIDE_E2E_SSH_DEFAULT_KEY to the default key's public file on a CI runner");
        };
        let fixture = fixture();
        fixture.sshd.authorize(Path::new(&public)).unwrap();
        let config = format!(
            "Host device\n  HostName 127.0.0.1\n  Port {}\n  User {}\n  UserKnownHostsFile {}\n  GlobalKnownHostsFile /dev/null\n",
            fixture.sshd.port,
            account(),
            fixture.sshd.known_hosts.display()
        );
        let row = Arc::new(Row::default());
        reached_the_device(fixture.attempt(&config, "device", Wake::Allowed, row.clone()));
        assert!(row.asked().is_empty());
    }

    /// B6: a password is answered on the row. The account and its server
    /// are the runner's (`user:password:port`).
    #[test]
    #[ignore = "a password account on the runner; the remote mailbox lane provides it"]
    fn a_password_is_answered_on_the_row() {
        let Some(account) = std::env::var("HIDE_E2E_SSH_PASSWORD").ok() else {
            panic!("set HIDE_E2E_SSH_PASSWORD to user:password:port on a CI runner");
        };
        let mut parts = account.splitn(3, ':');
        let (user, password, port) = (
            parts.next().unwrap(),
            parts.next().unwrap(),
            parts.next().unwrap(),
        );
        let fixture = fixture();
        let config = format!(
            "Host device\n  HostName 127.0.0.1\n  Port {port}\n  User {user}\n  PubkeyAuthentication no\n  IdentityAgent none\n  UserKnownHostsFile {}\n  GlobalKnownHostsFile /dev/null\n  StrictHostKeyChecking accept-new\n",
            fixture.base.join("password_known_hosts").display()
        );
        let row = Row::answering(vec![text(password)]);
        reached_the_device(fixture.attempt(&config, "device", Wake::Allowed, row.clone()));
        assert!(
            matches!(row.asked().as_slice(), [SignInWait::Secret { prompt, key_file: None }] if prompt.contains("assword")),
            "{:?}",
            row.asked()
        );
    }
}

/// Windows' own OpenSSH hands its question to Hide's askpass and reads the
/// answer back (PRD system-ssh-device-link Risks, B34): `ssh-keygen` asks a
/// key's passphrase through the same `readpass` code `ssh.exe` does, and
/// needs no server, which a Windows runner has none of.
#[cfg(windows)]
#[test]
#[ignore = "the runner's inbox OpenSSH; the windows check lane runs it"]
fn windows_inbox_openssh_asks_through_hides_askpass() {
    let openssh = PathBuf::from(r"C:\Windows\System32\OpenSSH");
    let folder = tempfile::tempdir().unwrap();
    let key = folder.path().join("locked");
    let made = std::process::Command::new(openssh.join("ssh-keygen.exe"))
        .args(["-q", "-t", "ed25519", "-N", "fixture passphrase", "-f"])
        .arg(&key)
        .output()
        .unwrap();
    assert!(made.status.success(), "{made:?}");
    let row = Row::answering(vec![text("fixture passphrase")]);
    let desk = hide_node::ssh::askpass::Desk::open(&std::env::temp_dir(), row.clone()).unwrap();
    let read = std::process::Command::new(openssh.join("ssh-keygen.exe"))
        .args(["-y", "-f"])
        .arg(&key)
        .envs(desk.environment(Path::new(env!("CARGO_BIN_EXE_hide"))))
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(read.status.success(), "{read:?}");
    assert!(String::from_utf8_lossy(&read.stdout).starts_with("ssh-ed25519 "));
    assert!(
        matches!(row.asked().as_slice(), [SignInWait::Secret { .. }]),
        "{:?}",
        row.asked()
    );
}
