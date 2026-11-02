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
  status) cat "$S/status.json" ;;
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
