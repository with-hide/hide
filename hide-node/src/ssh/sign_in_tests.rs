//! Sign-in against an in-process SSH server that accepts only the keys a test
//! names, from real key files in a temporary home and a real SSH agent (a
//! Unix socket, or a named pipe on Windows). The server records every key it
//! was asked to sign in with, so a test sees the order a client offered them.
//! It answers the `none` method as a test sets: refused, as OpenSSH's sshd
//! does, or accepted or held behind a banner, as Tailscale SSH does.

use futures_util::stream;
use russh::keys::ssh_key::LineEnding;
use russh::keys::ssh_key::private::Ed25519Keypair;
use russh::keys::ssh_key::public::KeyData;
use russh::keys::{PrivateKey, PublicKey};
use russh::server::{self, Auth};

use super::*;

/// Sign-in's bounds, scaled down so a held sign-in ends within a test.
const TEST_WAIT: SignInWait = SignInWait {
    grace: Duration::from_millis(200),
    held: Duration::from_secs(1),
    check: Duration::from_secs(2),
};

const CHECK_BANNER: &str = "# Tailscale SSH requires an additional check.\n# To authenticate, visit: https://login.tailscale.com/a/l1234abcd\n";

/// A key made from a fixed seed, so every run uses the same key.
fn key(seed: u8) -> PrivateKey {
    PrivateKey::from(Ed25519Keypair::from_seed(&[seed; 32]))
}

/// `ssh-keygen -t ed25519 -N 'fixture passphrase' -C hide-fixture`: a key
/// behind a passphrase, as a customer's default `~/.ssh/id_ed25519` was.
const LOCKED_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAACmFlczI1Ni1jdHIAAAAGYmNyeXB0AAAAGAAAABDPbIbU21
o34DJGei/iAV9WAAAAGAAAAAEAAAAzAAAAC3NzaC1lZDI1NTE5AAAAIAKDNjy5IfskWtry
dDarEzpsvx/meXl22mzXRTh77KvRAAAAkFoitCNr5jOf/UR8HlYa4VO/CXPnXTIMssAql6
V0Vz3VHjZLwW+4o8tKdWBmsMbWs6Ut1pFW8gLp6ViWsiKJiyjMOdP9BcmDMFzAdPR6q2bg
ib+7SesXM+WNwStsVJUyjXFJhy/nityltrzl3VS8UtcBrX1WjG1MKY1fZRPetfKQsp5iZf
2HznHRZXpVffAT8Q==
-----END OPENSSH PRIVATE KEY-----
";

fn locked_key_unlocked() -> PrivateKey {
    PrivateKey::from_openssh(LOCKED_KEY)
        .unwrap()
        .decrypt("fixture passphrase")
        .unwrap()
}

/// How the fixture server answers the `none` method.
#[derive(Clone)]
enum NoneAnswer {
    Refuse,
    /// Tailscale SSH: the tailnet already identified the client.
    Accept,
    /// Tailscale SSH's check mode: this banner, then no answer until the
    /// check is confirmed, if it ever is.
    Hold(&'static str, Option<Duration>),
}

#[derive(Clone)]
struct Server {
    none: NoneAnswer,
    /// The client's shown check, which a held sign-in reads when it confirms.
    check: Arc<Mutex<Option<String>>>,
    /// What the client showed at the moment the check was confirmed.
    shown_at_confirm: Arc<Mutex<Option<Option<String>>>>,
    accepts: Arc<Vec<PublicKey>>,
    tried: Arc<Mutex<Vec<PublicKey>>>,
}

impl server::Handler for Server {
    type Error = russh::Error;

    async fn authentication_banner(&mut self) -> Result<Option<String>, Self::Error> {
        Ok(match self.none {
            NoneAnswer::Hold(banner, _) => Some(banner.to_owned()),
            NoneAnswer::Refuse | NoneAnswer::Accept => None,
        })
    }

    async fn auth_none(&mut self, _user: &str) -> Result<Auth, Self::Error> {
        match self.none {
            NoneAnswer::Refuse => Ok(Auth::reject()),
            NoneAnswer::Accept => Ok(Auth::Accept),
            NoneAnswer::Hold(_, Some(confirmed)) => {
                tokio::time::sleep(confirmed).await;
                *lock_recover(&self.shown_at_confirm) = Some(lock_recover(&self.check).clone());
                Ok(Auth::Accept)
            }
            NoneAnswer::Hold(_, None) => {
                tokio::time::sleep(Duration::from_secs(60)).await;
                Ok(Auth::reject())
            }
        }
    }

    async fn auth_publickey(&mut self, user: &str, key: &PublicKey) -> Result<Auth, Self::Error> {
        self.tried.lock().unwrap().push(key.clone());
        let accepted = user == "example"
            && self
                .accepts
                .iter()
                .any(|accepted| accepted.key_data() == key.key_data());
        Ok(if accepted {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }
}

/// A home with `~/.ssh`, a server on a loopback port that home's
/// `known_hosts` trusts, and the keys the server accepts.
struct Fixture {
    home: tempfile::TempDir,
    runtime: Runtime,
    port: u16,
    tried: Arc<Mutex<Vec<PublicKey>>>,
    /// Where a held sign-in shows the address it waits on.
    check: Arc<Mutex<Option<String>>>,
    shown_at_confirm: Arc<Mutex<Option<Option<String>>>>,
}

impl Fixture {
    fn new(accepts: &[&PublicKey]) -> Self {
        Self::answering(NoneAnswer::Refuse, accepts)
    }

    fn answering(none: NoneAnswer, accepts: &[&PublicKey]) -> Self {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir(home.path().join(".ssh")).unwrap();
        let runtime = Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let host_key = key(200);
        let listener = runtime.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::fs::write(
            home.path().join(".ssh/known_hosts"),
            format!(
                "[127.0.0.1]:{port} {}\n",
                host_key.public_key().to_openssh().unwrap()
            ),
        )
        .unwrap();
        let config = Arc::new(server::Config {
            keys: vec![host_key],
            auth_rejection_time: Duration::ZERO,
            auth_rejection_time_initial: Some(Duration::ZERO),
            ..server::Config::default()
        });
        let tried = Arc::new(Mutex::new(Vec::new()));
        let check = Arc::new(Mutex::new(None));
        let shown_at_confirm = Arc::new(Mutex::new(None));
        let handler = Server {
            none,
            check: Arc::clone(&check),
            shown_at_confirm: Arc::clone(&shown_at_confirm),
            accepts: Arc::new(accepts.iter().map(|key| (*key).clone()).collect()),
            tried: Arc::clone(&tried),
        };
        runtime.spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let config = Arc::clone(&config);
                let handler = handler.clone();
                tokio::spawn(async move {
                    if let Ok(session) = server::run_stream(config, socket, handler).await {
                        let _ = session.await;
                    }
                });
            }
        });
        Self {
            home,
            runtime,
            port,
            tried,
            check,
            shown_at_confirm,
        }
    }

    fn ssh_dir(&self) -> PathBuf {
        self.home.path().join(".ssh")
    }

    fn write_key(&self, name: &str, key: &PrivateKey) {
        std::fs::write(
            self.ssh_dir().join(name),
            key.to_openssh(LineEnding::LF).unwrap().as_bytes(),
        )
        .unwrap();
    }

    /// Signs in as `box`, whose config block carries `lines`.
    fn sign_in(&self, lines: &str) -> RemoteResult<()> {
        let alias = SshAlias::from_config_contents(
            "box",
            &format!(
                "Host box\n  HostName 127.0.0.1\n  Port {}\n  User example\n{lines}",
                self.port
            ),
            &self.ssh_dir(),
        )?;
        let handler = KnownHostHandler::new(&alias);
        let banner = Arc::clone(&handler.banner);
        self.runtime.block_on(async {
            let mut session = client::connect(
                Arc::new(client::Config::default()),
                ("127.0.0.1", self.port),
                handler,
            )
            .await
            .expect("the fixture server's host key is trusted");
            authenticate(&mut session, &alias, &banner, &self.check, TEST_WAIT).await
        })
    }

    /// The keys the server was asked to sign in with since the last call;
    /// a key's comment never travels, so only its key data is compared.
    fn tried(&self) -> Vec<KeyData> {
        std::mem::take(&mut *self.tried.lock().unwrap())
            .iter()
            .map(|key| key.key_data().clone())
            .collect()
    }
}

/// A real SSH agent on a socket of the test's own, and the line that points
/// an alias at it.
struct Agent {
    path: PathBuf,
    _dir: tempfile::TempDir,
}

impl Agent {
    fn start(fixture: &Fixture) -> Self {
        let dir = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        let path = {
            let path = dir.path().join("agent.sock");
            let listener = fixture
                .runtime
                .block_on(async { tokio::net::UnixListener::bind(&path) })
                .unwrap();
            let connections = stream::unfold(listener, |listener| async move {
                let accepted = listener.accept().await.map(|(socket, _)| socket);
                Some((accepted, listener))
            });
            fixture
                .runtime
                .spawn(russh::keys::agent::server::serve(Box::pin(connections), ()));
            path
        };
        #[cfg(windows)]
        let path = {
            use tokio::net::windows::named_pipe::ServerOptions;
            let name = format!(
                r"\\.\pipe\hide-sign-in-test-{}-{}",
                std::process::id(),
                fixture.port
            );
            let first = fixture
                .runtime
                .block_on(async { ServerOptions::new().first_pipe_instance(true).create(&name) })
                .unwrap();
            let pipe = name.clone();
            let connections = stream::unfold(first, move |server| {
                let pipe = pipe.clone();
                async move {
                    let connected = server.connect().await;
                    let next = ServerOptions::new().create(&pipe).ok()?;
                    Some((connected.map(|()| server), next))
                }
            });
            fixture
                .runtime
                .spawn(russh::keys::agent::server::serve(Box::pin(connections), ()));
            PathBuf::from(name)
        };
        Self { path, _dir: dir }
    }

    fn line(&self) -> String {
        format!("  IdentityAgent \"{}\"\n", self.path.display())
    }

    fn add(&self, fixture: &Fixture, key: &PrivateKey) {
        fixture.runtime.block_on(async {
            #[cfg(unix)]
            let mut client = AgentClient::connect_uds(&self.path).await.unwrap();
            #[cfg(windows)]
            let mut client = AgentClient::connect_named_pipe(&self.path).await.unwrap();
            client.add_identity(key, &[]).await.unwrap();
        });
    }
}

/// The customer's case: `ssh box` works because OpenSSH reads the default
/// `~/.ssh/id_ed25519` the config never names, while the agent is empty.
#[test]
fn a_default_key_signs_in_when_the_config_names_none_and_the_agent_is_empty() {
    let default = key(1);
    let fixture = Fixture::new(&[default.public_key()]);
    fixture.write_key("id_ed25519", &default);
    let agent = Agent::start(&fixture);

    fixture.sign_in(&agent.line()).unwrap();
    assert_eq!(fixture.tried(), [default.public_key().key_data().clone()]);
}

/// Every `IdentityFile` line is tried, in order, until the server accepts
/// one, and a named file keeps the defaults out.
#[test]
fn every_identity_file_is_tried_in_order() {
    let (first, second, default) = (key(1), key(2), key(3));
    let fixture = Fixture::new(&[second.public_key(), default.public_key()]);
    fixture.write_key("first", &first);
    fixture.write_key("second", &second);
    fixture.write_key("id_ed25519", &default);

    fixture
        .sign_in(
            "  IdentityAgent none\n  IdentityFile ~/.ssh/first\n  IdentityFile ~/.ssh/second\n",
        )
        .unwrap();
    assert_eq!(
        fixture.tried(),
        [
            first.public_key().key_data().clone(),
            second.public_key().key_data().clone()
        ]
    );
}

/// A key behind a passphrase is never offered from its file; the refusal
/// names it with the command that hands it to the agent, and once the agent
/// holds it, sign-in goes through the agent.
#[test]
fn a_passphrase_key_is_named_with_the_command_that_loads_it_into_the_agent() {
    let unlocked = locked_key_unlocked();
    let fixture = Fixture::new(&[unlocked.public_key()]);
    std::fs::write(fixture.ssh_dir().join("id_ed25519"), LOCKED_KEY).unwrap();
    let agent = Agent::start(&fixture);

    let refused = fixture.sign_in(&agent.line()).unwrap_err();
    assert_eq!(refused.stage(), RemoteStage::Auth);
    let reason = &refused.diagnostic().reason;
    assert!(reason.contains("holds no identities"), "{reason}");
    let load = if cfg!(target_os = "macos") {
        "`ssh-add --apple-use-keychain ~/.ssh/id_ed25519`"
    } else {
        "`ssh-add ~/.ssh/id_ed25519`"
    };
    assert!(
        reason.contains(&format!(
            "~/.ssh/id_ed25519 is protected by a passphrase Hide cannot enter: load it into the agent with {load}"
        )),
        "{reason}"
    );
    assert!(fixture.tried().is_empty());

    agent.add(&fixture, &unlocked);
    fixture.sign_in(&agent.line()).unwrap();
    assert_eq!(fixture.tried(), [unlocked.public_key().key_data().clone()]);
}

/// OpenSSH's order: the agent's key for a configured file first, then the
/// agent's other keys, then files the agent does not hold. `IdentitiesOnly`
/// keeps the agent's other keys back. A server that refuses all of them is
/// reported apart from an agent that held nothing.
#[test]
fn agent_keys_for_the_files_go_first_and_identities_only_keeps_the_rest_back() {
    let (other, named, file_only) = (key(1), key(2), key(3));
    let fixture = Fixture::new(&[]);
    fixture.write_key("named", &named);
    fixture.write_key("file_only", &file_only);
    let agent = Agent::start(&fixture);
    agent.add(&fixture, &other);
    agent.add(&fixture, &named);
    let files = "  IdentityFile ~/.ssh/file_only\n  IdentityFile ~/.ssh/named\n";

    let refused = fixture
        .sign_in(&format!("{}{files}", agent.line()))
        .unwrap_err();
    assert_eq!(
        fixture.tried(),
        [
            named.public_key().key_data().clone(),
            other.public_key().key_data().clone(),
            file_only.public_key().key_data().clone()
        ]
    );
    let reason = &refused.diagnostic().reason;
    assert!(
        reason.contains(
            "the server rejected every key offered (2 from the SSH agent reached through the ssh config IdentityAgent, 1 from key files)"
        ),
        "{reason}"
    );

    fixture
        .sign_in(&format!("{}{files}  IdentitiesOnly yes\n", agent.line()))
        .unwrap_err();
    assert_eq!(
        fixture.tried(),
        [
            named.public_key().key_data().clone(),
            file_only.public_key().key_data().clone()
        ]
    );
}

/// With no agent and no key file at all, the refusal says both, so the
/// operator knows there was nothing to offer rather than a refused key.
#[test]
fn nothing_to_offer_says_why() {
    let fixture = Fixture::new(&[]);
    let refused = fixture.sign_in("  IdentityAgent none\n").unwrap_err();
    let reason = &refused.diagnostic().reason;
    assert!(
        reason.contains(
            "no key was offered: the ssh config sets IdentityAgent none; none of OpenSSH's default key files exists in ~/.ssh"
        ),
        "{reason}"
    );
    assert!(!refused.diagnostic().retryable, "{reason}");

    let refused = fixture
        .sign_in("  IdentityAgent none\n  IdentityFile ~/.ssh/missing\n")
        .unwrap_err();
    let reason = &refused.diagnostic().reason;
    assert!(reason.contains("~/.ssh/missing does not exist"), "{reason}");
}

/// Tailscale SSH: the tailnet identified the client, so the `none` method
/// signs in with no key and no agent, as `ssh` does.
#[test]
fn a_server_that_accepts_none_signs_in_without_a_key() {
    let fixture = Fixture::answering(NoneAnswer::Accept, &[]);
    fixture.sign_in("  IdentityAgent none\n").unwrap();
    assert!(fixture.tried().is_empty());
}

/// Tailscale SSH's check mode holds the `none` method until the operator
/// confirms in a browser: while it waits, the address from its banner (and
/// nothing else from the banner) is shown, and a confirmed check signs in
/// and takes the address away.
#[test]
fn a_held_sign_in_shows_its_address_until_the_check_is_confirmed() {
    let fixture = Fixture::answering(
        NoneAnswer::Hold(CHECK_BANNER, Some(Duration::from_millis(1200))),
        &[],
    );
    fixture.sign_in("  IdentityAgent none\n").unwrap();
    assert_eq!(
        *lock_recover(&fixture.shown_at_confirm),
        Some(Some("https://login.tailscale.com/a/l1234abcd".to_owned()))
    );
    assert_eq!(*lock_recover(&fixture.check), None);
}

/// A check that is never confirmed ends at its bound with a reason the row
/// reads as its own class, and the address is no longer shown.
#[test]
fn an_unconfirmed_check_ends_at_its_bound_as_its_own_problem() {
    let fixture = Fixture::answering(NoneAnswer::Hold(CHECK_BANNER, None), &[]);
    let started = Instant::now();
    let held = fixture.sign_in("  IdentityAgent none\n").unwrap_err();
    assert!(
        started.elapsed() >= TEST_WAIT.check,
        "{:?}",
        started.elapsed()
    );
    assert_eq!(held.stage(), RemoteStage::Auth);
    assert_eq!(
        hide_node_link::device::connection_problem(&held.to_string()),
        Some("sign_in_check")
    );
    assert!(!held.diagnostic().reason.contains("login.tailscale.com"));
    assert_eq!(*lock_recover(&fixture.check), None);
}

/// A hold whose banner names no address is not a check: it ends at the
/// shorter bound, as a refused sign-in.
#[test]
fn a_hold_without_an_address_ends_as_a_refused_sign_in() {
    let fixture = Fixture::answering(NoneAnswer::Hold("Authorized use only.\n", None), &[]);
    let started = Instant::now();
    let held = fixture.sign_in("  IdentityAgent none\n").unwrap_err();
    assert!(
        started.elapsed() < TEST_WAIT.check,
        "{:?}",
        started.elapsed()
    );
    assert_eq!(
        held.diagnostic().reason,
        "the server held sign-in without answering"
    );
    assert_eq!(
        hide_node_link::device::connection_problem(&held.to_string()),
        Some("authentication")
    );
}
