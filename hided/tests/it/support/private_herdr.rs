//! A Herdr server of the test's own: the pinned binary on its own socket,
//! session, config and home, none of the operator's. Shared by the tests that
//! run hided against the pinned Herdr.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use hide_platform::process::OwnedChild;
use serde_json::{Value, json};

pub fn pinned_version() -> String {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../contracts/herdr-bundle.json");
    let manifest: Value = serde_json::from_slice(&std::fs::read(manifest).unwrap()).unwrap();
    manifest["version"].as_str().unwrap().to_owned()
}

pub fn herdr_command(bin: &Path, home: &Path, socket: &Path, config: &Path) -> Command {
    let mut command = Command::new(bin);
    for (key, _) in std::env::vars_os() {
        let key = key.to_string_lossy().to_uppercase();
        if key.starts_with("HERDR_") || key.starts_with("HIDE_") {
            command.env_remove(key);
        }
    }
    command
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("XDG_CONFIG_HOME", home.join("xdg-config"))
        .env("XDG_STATE_HOME", home.join("xdg-state"))
        .env("HERDR_SESSION", "hide-hided-contract")
        .env("HERDR_SOCKET_PATH", socket)
        .env("HERDR_CONFIG_PATH", config)
        .env("HERDR_DISABLE_SOUND", "1");
    command
}

/// How long a stopped server and its panes may take to end.
const SERVER_STOP: Duration = Duration::from_secs(30);

/// A Herdr server of the test's own, stopped on drop.
pub struct PrivateHerdr {
    pub bin: PathBuf,
    pub home: PathBuf,
    pub socket: PathBuf,
    pub config: PathBuf,
    pub log: PathBuf,
    server: Option<OwnedChild>,
}

impl PrivateHerdr {
    pub fn start(bin: PathBuf, root: &Path) -> Self {
        let version = String::from_utf8(
            Command::new(&bin)
                .arg("--version")
                .output()
                .expect("herdr --version")
                .stdout,
        )
        .unwrap();
        assert_eq!(
            version.split_whitespace().last().unwrap(),
            pinned_version(),
            "{} is not the pinned Herdr",
            bin.display()
        );
        Self::start_any(bin, root)
    }

    /// A server of any Herdr release, such as an older one a device's
    /// operator installed.
    pub fn start_any(bin: PathBuf, root: &Path) -> Self {
        let home = root.join("home");
        std::fs::create_dir_all(&home).unwrap();
        let config = root.join("herdr-config.toml");
        std::fs::write(
            &config,
            "[update]\nversion_check = false\nmanifest_check = false\n",
        )
        .unwrap();
        let socket = root.join("herdr.sock");
        let log = root.join("server.log");
        let output = std::fs::File::create(&log).unwrap();
        // Owned with what it starts: on Windows the pane shells outlive the
        // server and keep their folder, inside `root`, locked.
        let server = OwnedChild::spawn(
            herdr_command(&bin, &home, &socket, &config)
                .arg("server")
                .stdin(Stdio::null())
                .stdout(output.try_clone().unwrap())
                .stderr(output),
        )
        .expect("herdr server starts");
        let herdr = Self {
            bin,
            home,
            socket,
            config,
            log,
            server: Some(server),
        };
        herdr.wait_for_ping();
        herdr
    }

    /// Asks the starting server `ping` until it answers; the minute only ends
    /// a server that never comes up, and then its log is the report.
    #[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
    fn wait_for_ping(&self) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while hide_herdr_client::request_with_timeout(
            &self.socket,
            "ping",
            json!({}),
            Duration::from_secs(2),
        )
        .is_err()
        {
            assert!(
                Instant::now() < deadline,
                "the server never answered ping: {}",
                std::fs::read_to_string(&self.log).unwrap_or_default()
            );
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    /// Stops the server and returns once it and everything it started have
    /// ended: on Windows a pane's shell outlived the server it belonged to,
    /// and removing the fixture's folder then failed with a sharing violation
    /// (with-hide/hide-private#282).
    pub fn stop(&mut self) -> std::io::Result<()> {
        let Some(mut server) = self.server.take() else {
            return Ok(());
        };
        let _ = herdr_command(&self.bin, &self.home, &self.socket, &self.config)
            .args(["server", "stop"])
            .output();
        // A server that did not stop by then is ended with its tree.
        server
            .capture_until(Instant::now() + SERVER_STOP, 1)
            .map(|_| ())
            .map_err(std::io::Error::other)
    }
}

impl Drop for PrivateHerdr {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
