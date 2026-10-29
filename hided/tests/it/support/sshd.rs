//! The system's own OpenSSH server on loopback, for the device link's tests
//! (PRD system-ssh-device-link, test boundary): the device side of a
//! connection is a real `sshd`, so what the system `ssh` and its sign-in do
//! against it is what they do against a device.
//!
//! It runs as this account, which is the only account a server started
//! without root can sign in, with the fixture's host key and authorized key,
//! and every session it starts gets the fixture account's environment
//! (`SetEnv`), so a session's home is the fixture's private one. The client
//! side's config names its own known_hosts and key ([`Sshd::client_config`]),
//! so neither side reads or writes this account's `~/.ssh`.
#![allow(dead_code)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use hide_platform::process::OwnedChild;

const READY_BOUND: Duration = Duration::from_secs(10);

pub struct Sshd {
    pub port: u16,
    dir: PathBuf,
    child: std::sync::Mutex<Option<OwnedChild>>,
    /// The client key a connection signs in with.
    pub client_key: PathBuf,
    pub known_hosts: PathBuf,
}

/// This account's name, the one the server signs in.
pub fn account() -> String {
    let output = Command::new("id").arg("-un").output().expect("id -un");
    String::from_utf8(output.stdout)
        .expect("account name")
        .trim()
        .to_owned()
}

/// The server program: the system's own, which a CI lane installs where the
/// image has none.
fn program() -> Result<PathBuf> {
    for candidate in ["/usr/sbin/sshd", "/usr/local/sbin/sshd"] {
        if Path::new(candidate).is_file() {
            return Ok(PathBuf::from(candidate));
        }
    }
    bail!("these tests need the system's OpenSSH server (sshd); install openssh-server")
}

impl Sshd {
    /// Starts a server in `dir` whose sessions get `environment`.
    pub fn start(dir: &Path, environment: &[(OsString, OsString)]) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        let dir = std::fs::canonicalize(dir)?;
        for name in ["host", "client"] {
            let key = dir.join(name);
            if !key.exists() {
                let status = Command::new("/usr/bin/ssh-keygen")
                    .args(["-q", "-t", "ed25519", "-N", "", "-C", "fixture", "-f"])
                    .arg(&key)
                    .stdin(Stdio::null())
                    .status()?;
                ensure!(status.success(), "ssh-keygen failed for {}", key.display());
            }
        }
        std::fs::copy(dir.join("client.pub"), dir.join("authorized_keys"))?;
        let port = free_port()?;
        let mut set_env = String::from("SetEnv");
        for (name, value) in environment {
            let value = value.to_string_lossy();
            ensure!(
                !value.contains('"') && !value.contains('\n'),
                "a session variable the server cannot carry: {}",
                name.to_string_lossy()
            );
            set_env.push_str(&format!(" \"{}={value}\"", name.to_string_lossy()));
        }
        let config = format!(
            "ListenAddress 127.0.0.1:{port}\n\
             HostKey {host}\n\
             PidFile none\n\
             AuthorizedKeysFile {keys}\n\
             StrictModes no\n\
             UsePAM no\n\
             PasswordAuthentication no\n\
             KbdInteractiveAuthentication no\n\
             PubkeyAuthentication yes\n\
             PermitUserEnvironment no\n\
             LogLevel VERBOSE\n\
             {set_env}\n",
            host = dir.join("host").display(),
            keys = dir.join("authorized_keys").display(),
        );
        std::fs::write(dir.join("sshd_config"), config)?;
        let known_hosts = dir.join("known_hosts");
        let public = std::fs::read_to_string(dir.join("host.pub"))?;
        std::fs::write(
            &known_hosts,
            format!("[127.0.0.1]:{port} {}\n", public.trim()),
        )?;
        let sshd = Self {
            port,
            client_key: dir.join("client"),
            known_hosts,
            dir,
            child: std::sync::Mutex::new(None),
        };
        sshd.online(true)?;
        Ok(sshd)
    }

    /// A client config block for `alias` reaching this server: its key, its
    /// known_hosts, and no agent or other file of this account's.
    pub fn client_config(&self, alias: &str) -> String {
        format!(
            "Host {alias}\n  HostName 127.0.0.1\n  Port {}\n  User {}\n  IdentityFile {}\n  IdentitiesOnly yes\n  IdentityAgent none\n  UserKnownHostsFile {}\n  GlobalKnownHostsFile /dev/null\n",
            self.port,
            account(),
            self.client_key.display(),
            self.known_hosts.display()
        )
    }

    /// `false` ends the server and every session it holds, as a device that
    /// went away; `true` starts it again on the same port.
    pub fn online(&self, value: bool) -> Result<()> {
        let mut child = self.child.lock().expect("sshd child");
        if !value {
            if let Some(mut ended) = child.take() {
                ended.kill_tree().context("ending the SSH server")?;
                let _ = ended.wait();
            }
            return Ok(());
        }
        if child.is_some() {
            return Ok(());
        }
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join("sshd.log"))?;
        let mut command = Command::new(program()?);
        command
            .args(["-D", "-e", "-f"])
            .arg(self.dir.join("sshd_config"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log);
        let started = child.insert(OwnedChild::spawn(&mut command)?);
        let deadline = Instant::now() + READY_BOUND;
        while std::net::TcpStream::connect(("127.0.0.1", self.port)).is_err() {
            if let Some(status) = started.try_wait()? {
                bail!("sshd exited {status}: {}", self.log());
            }
            ensure!(
                Instant::now() < deadline,
                "sshd did not listen: {}",
                self.log()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        Ok(())
    }

    /// How many connections signed in since the server first started.
    pub fn accepted(&self) -> usize {
        self.log().matches("Accepted publickey for").count()
    }

    /// How many connections the server holds now: each is a process of its
    /// own under the server.
    pub fn open(&self) -> usize {
        let Some(pid) = self
            .child
            .lock()
            .expect("sshd child")
            .as_ref()
            .map(OwnedChild::id)
        else {
            return 0;
        };
        let output = Command::new("pgrep")
            .args(["-P", &pid.to_string()])
            .output();
        output.map_or(0, |output| {
            String::from_utf8_lossy(&output.stdout).lines().count()
        })
    }

    pub fn log(&self) -> String {
        std::fs::read_to_string(self.dir.join("sshd.log")).unwrap_or_default()
    }

    pub fn stop(&self) -> Result<()> {
        self.online(false)
    }
}

impl Drop for Sshd {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn free_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    Ok(listener.local_addr()?.port())
}
