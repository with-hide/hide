//! A `tailscale` CLI that keeps its whole state in files in the test's own
//! folder, so nothing reaches the machine's Tailscale; the Mobile and move
//! tests share it.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

pub const DNS: &str = "mac.tailnet-name.ts.net";

/// A `tailscale` that answers from files under `state`: `status.json`,
/// `serve.json`; `fail-serve` makes every serve change fail, `fail-remove`
/// only removals, and `apply-then-fail` applies an add and then fails it
/// (a `serve --bg` that timed out after it took); with `funnel` a removal
/// leaves the Funnel flag in place; every call is appended to `calls.log`.
/// `hold-status` keeps `status` from answering while the file exists (at most
/// 20 s, so a test that panics leaves no script waiting), and `hold-pipes`
/// makes `status` leave a process behind that keeps its output open, as a
/// child another thread started in the middle of the command's start does on
/// macOS. Like the real CLI, `status` leaves the peers out only when asked
/// with `--peers=false`; `crowded_tailnet` gives the answer with them.
/// Like the macOS app's executable, it acts as the CLI only when told to with
/// `TAILSCALE_BE_CLI=1`; otherwise it refuses, where the real one starts the
/// app and never answers. It looks at that variable alone, so a terminal the
/// test runs from (`TERM`, `SHLVL`) cannot stand in for it.
pub struct FakeTailscale {
    pub state: PathBuf,
    pub bin: PathBuf,
    /// The tailnet name this machine answers with.
    pub dns: String,
}

impl FakeTailscale {
    pub fn new(root: &Path) -> Self {
        Self::named(root, DNS)
    }

    /// One whose machine is `dns` on the tailnet, as another machine is.
    pub fn named(root: &Path, dns: &str) -> Self {
        let state = root.join("tailscale-state");
        std::fs::create_dir_all(&state).unwrap();
        let bin = root.join("tailscale");
        let script = format!(
            r#"#!/bin/sh
S='{state}'
if [ "$TAILSCALE_BE_CLI" != 1 ]; then echo "started as the Tailscale app, not the CLI" >&2; exit 3; fi
echo "$*" >> "$S/calls.log"
case "$1" in
  status)
    n=0; while [ -e "$S/hold-status" ] && [ "$n" -lt 400 ]; do sleep 0.05; n=$((n+1)); done
    if [ -e "$S/hold-pipes" ]; then sleep 20 & fi
    case "$*" in *--peers=false*) cat "$S/status.json" ;; *) cat "$S/status-with-peers.json" 2>/dev/null || cat "$S/status.json" ;; esac ;;
  serve)
    shift
    if [ "$1" = status ]; then cat "$S/serve.json" 2>/dev/null || echo '{{}}'; exit 0; fi
    if [ -e "$S/fail-serve" ]; then echo "serve config denied: access denied" >&2; exit 1; fi
    last=""; for a in "$@"; do last="$a"; done
    if [ "$last" = off ] && [ -e "$S/fail-remove" ]; then echo "remove denied" >&2; exit 1; fi
    if [ "$last" = off ] && [ -e "$S/funnel" ]; then echo '{{"AllowFunnel":{{"{dns}:443":true}}}}' > "$S/serve.json"; exit 0; fi
    if [ "$last" = off ]; then echo '{{}}' > "$S/serve.json"; exit 0; fi
    printf '{{"TCP":{{"443":{{"HTTPS":true}}}},"Web":{{"{dns}:443":{{"Handlers":{{"/":{{"Proxy":"%s"}}}}}}}}}}' "$last" > "$S/serve.json"
    if [ -e "$S/apply-then-fail" ]; then echo "timed out" >&2; exit 1; fi
    ;;
  *) exit 2 ;;
esac
"#,
            state = state.display(),
        );
        std::fs::write(&bin, script).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        Self {
            state,
            bin,
            dns: dns.to_owned(),
        }
    }

    pub fn status(&self, value: Value) {
        std::fs::write(self.state.join("status.json"), value.to_string()).unwrap();
    }

    pub fn logged_out(&self) {
        self.status(
            json!({"BackendState": "NeedsLogin", "Self": {"DNSName": "", "HostName": "mac"}}),
        );
    }

    pub fn https_off(&self) {
        self.status(json!({"BackendState": "Running", "Self": {"DNSName": format!("{}.", self.dns), "HostName": "mac"},
            "CurrentTailnet": {"MagicDNSEnabled": true}}));
    }

    pub fn ready(&self) {
        self.status(json!({"BackendState": "Running", "Self": {"DNSName": format!("{}.", self.dns), "HostName": "mac"},
            "CurrentTailnet": {"MagicDNSEnabled": true}, "CertDomains": [self.dns]}));
    }

    pub fn serve(&self) -> Value {
        std::fs::read_to_string(self.state.join("serve.json"))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or(json!({}))
    }

    pub fn set_serve(&self, value: Value) {
        std::fs::write(self.state.join("serve.json"), value.to_string()).unwrap();
    }

    pub fn calls(&self) -> String {
        std::fs::read_to_string(self.state.join("calls.log")).unwrap_or_default()
    }

    pub fn proxy(&self) -> Option<String> {
        self.serve()
            .pointer(&format!("/Web/{}:443/Handlers/~1/Proxy", self.dns))
            .and_then(Value::as_str)
            .map(str::to_owned)
    }

    /// From now on `status` waits until `release_status`.
    pub fn hold_status(&self) {
        std::fs::write(self.state.join("hold-status"), "").unwrap();
    }

    pub fn release_status(&self) {
        std::fs::remove_file(self.state.join("hold-status")).unwrap();
    }

    /// From now on `status` leaves a process holding the command's output
    /// open for 20 s after it has answered and exited.
    pub fn hold_pipes(&self) {
        std::fs::write(self.state.join("hold-pipes"), "").unwrap();
    }

    /// How many times `tailscale status` has been run.
    pub fn status_runs(&self) -> usize {
        self.calls()
            .lines()
            .filter(|line| line.starts_with("status "))
            .count()
    }

    /// A tailnet of `peers` machines: the answer that lists them is what
    /// `status` prints unless it is asked to leave them out. Each entry is
    /// smaller than a real one (about 1.2 KB on a tailnet of 27), so a test
    /// that needs the answer past a bound asks for more peers than the real
    /// tailnet would take.
    pub fn crowded_tailnet(&self, peers: usize) {
        let mut answer: Value =
            serde_json::from_str(&std::fs::read_to_string(self.state.join("status.json")).unwrap())
                .unwrap();
        let peer = |number: usize| {
            json!({
                "ID": format!("n{number}"), "HostName": format!("machine-{number}"),
                "DNSName": format!("machine-{number}.tailnet-name.ts.net."), "OS": "linux",
                "TailscaleIPs": [format!("100.64.{}.{}", number / 250, number % 250)],
                "Tags": ["tag:server", "tag:ci"], "Online": true,
                "KeyExpiry": "2027-01-01T00:00:00Z", "Relay": "fra", "RxBytes": 0, "TxBytes": 0,
                "Addrs": null, "AllowedIPs": [format!("100.64.{}.{}/32", number / 250, number % 250)],
                "PublicKey": format!("nodekey:{}", "0".repeat(64)),
                "Capabilities": ["https://tailscale.com/cap/file-sharing", "funnel", "ssh"],
                "CapMap": {"https://tailscale.com/cap/file-sharing": null, "funnel": null},
            })
        };
        answer["Peer"] = Value::Object(
            (0..peers)
                .map(|number| (format!("nodekey:{number:064}"), peer(number)))
                .collect(),
        );
        std::fs::write(
            self.state.join("status-with-peers.json"),
            serde_json::to_string_pretty(&answer).unwrap(),
        )
        .unwrap();
    }

    /// Returns once `status` has been run `count` times.
    pub async fn wait_for_status_runs(&self, count: usize) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while self.status_runs() < count {
            assert!(
                tokio::time::Instant::now() < deadline,
                "tailscale status was run {} times, not {count}",
                self.status_runs()
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// Switch-off shows `off` at once and removes the entry right after it.
    pub async fn wait_until_removed(&self) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while self.proxy().is_some() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "hide's serve entry was never removed"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
}
