//! The one Tailscale-aware file (PRD D-01). Mobile reaches the phone through
//! the operator's own tailnet: `tailscale serve` publishes this daemon's
//! loopback port at `https://<mac>.<tailnet>.ts.net` with Tailscale's
//! certificate. Everything that knows the word Tailscale lives here: finding
//! the CLI, reading `status --json` into the checklist, reading
//! `serve status --json` for who owns port 443, and adding or removing the
//! one entry hide recorded. Removing this file and adding a relay module is
//! the whole transport swap.
//!
//! Every command runs as a child process with a deadline, never under a lock
//! the snapshot path takes. A failure keeps the command line and stderr for
//! the `mobile_transport` record.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;

use super::store::ServeRecord;

/// How long one `tailscale` command may take before it counts as failed.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(15);

/// Where to download Tailscale, shown on the first checklist step.
pub const DOWNLOAD_URL: &str = "https://tailscale.com/download";
/// Where MagicDNS and HTTPS certificates are turned on.
pub const ADMIN_DNS_URL: &str = "https://login.tailscale.com/admin/dns";

/// Which tailscale CLI to run. A pinned path is the only one tried (and a
/// missing pinned path reads as not installed); otherwise the CLI where the
/// system's Tailscale app installs it (`hide_platform::host::tailscale_cli`),
/// then `tailscale` on PATH.
#[derive(Clone, Debug)]
pub struct CliSource {
    pub pinned: Option<PathBuf>,
    pub search_path: Option<String>,
}

impl CliSource {
    pub fn resolve(&self) -> Option<PathBuf> {
        if let Some(pinned) = &self.pinned {
            return pinned.is_file().then(|| pinned.clone());
        }
        if let Ok(installed) = hide_platform::host::tailscale_cli()
            && installed.is_file()
        {
            return Some(installed);
        }
        self.search_path
            .as_deref()
            .and_then(|path| hide_platform::host::find_program(path.as_ref(), "tailscale"))
    }
}

/// A command that did not succeed: what ran, and what it said.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandFailure {
    pub command: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
}

impl CommandFailure {
    pub fn message(&self) -> String {
        let stderr = self.stderr.trim();
        if stderr.is_empty() {
            match self.exit_code {
                Some(code) => format!("`{}` exited with {code}", self.command),
                None => format!("`{}` did not finish", self.command),
            }
        } else {
            stderr.lines().next().unwrap_or(stderr).to_owned()
        }
    }
}

async fn run(program: &Path, args: &[&str]) -> Result<Vec<u8>, CommandFailure> {
    let command = format!("tailscale {}", args.join(" "));
    let child = tokio::process::Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| CommandFailure {
            command: command.clone(),
            stderr: error.to_string(),
            exit_code: None,
        })?;
    match tokio::time::timeout(COMMAND_TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(output)) if output.status.success() => Ok(output.stdout),
        Ok(Ok(output)) => Err(CommandFailure {
            command,
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            exit_code: output.status.code(),
        }),
        Ok(Err(error)) => Err(CommandFailure {
            command,
            stderr: error.to_string(),
            exit_code: None,
        }),
        Err(_) => Err(CommandFailure {
            command,
            stderr: format!("timed out after {}s", COMMAND_TIMEOUT.as_secs()),
            exit_code: None,
        }),
    }
}

/// What `tailscale status --json` says about this Mac.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Status {
    pub logged_in: bool,
    /// The machine's name in the tailnet, shown on the login step.
    pub host_name: Option<String>,
    /// `mac.tailnet.ts.net`, without the trailing dot.
    pub dns_name: Option<String>,
    pub magic_dns: bool,
    /// HTTPS certificates are available for this Mac's name.
    pub https: bool,
}

pub fn parse_status(bytes: &[u8]) -> Result<Status, String> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    let logged_in = value.get("BackendState").and_then(Value::as_str) == Some("Running");
    let own = value.get("Self");
    let dns_name = own
        .and_then(|own| own.get("DNSName"))
        .and_then(Value::as_str)
        .map(|name| name.trim_end_matches('.').to_owned())
        .filter(|name| !name.is_empty());
    let host_name = own
        .and_then(|own| own.get("HostName"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .filter(|name| !name.is_empty());
    let magic_dns = value
        .pointer("/CurrentTailnet/MagicDNSEnabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let https = match (
        &dns_name,
        value.get("CertDomains").and_then(Value::as_array),
    ) {
        (Some(name), Some(domains)) => domains
            .iter()
            .filter_map(Value::as_str)
            .any(|domain| domain.trim_end_matches('.') == name),
        _ => false,
    };
    Ok(Status {
        logged_in,
        host_name,
        dns_name,
        magic_dns,
        https,
    })
}

pub async fn status(program: &Path) -> Result<Status, CommandFailure> {
    let bytes = run(program, &["status", "--json"]).await?;
    parse_status(&bytes).map_err(|message| CommandFailure {
        command: "tailscale status --json".to_owned(),
        stderr: format!("unreadable answer: {message}"),
        exit_code: Some(0),
    })
}

/// The state of one checklist step on this Mac.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StepState {
    Ok,
    Failed,
    /// An earlier step failed, so this one was not checked.
    Waiting,
}

/// The three steps hide can check on this Mac, in order; the phone step is
/// guidance only and is not here (PRD D-05).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Checklist {
    pub installed: StepState,
    pub logged_in: StepState,
    pub https: StepState,
    pub host_name: Option<String>,
    pub dns_name: Option<String>,
}

impl Checklist {
    pub fn not_installed() -> Self {
        Self {
            installed: StepState::Failed,
            logged_in: StepState::Waiting,
            https: StepState::Waiting,
            host_name: None,
            dns_name: None,
        }
    }

    pub fn from_status(status: &Status) -> Self {
        let logged_in = status.logged_in && status.dns_name.is_some();
        let https = logged_in && status.magic_dns && status.https;
        Self {
            installed: StepState::Ok,
            logged_in: if logged_in {
                StepState::Ok
            } else {
                StepState::Failed
            },
            https: match (logged_in, https) {
                (false, _) => StepState::Waiting,
                (true, true) => StepState::Ok,
                (true, false) => StepState::Failed,
            },
            host_name: status.host_name.clone(),
            dns_name: status.dns_name.clone().filter(|_| logged_in),
        }
    }

    pub fn passed(&self) -> bool {
        self.installed == StepState::Ok
            && self.logged_in == StepState::Ok
            && self.https == StepState::Ok
    }
}

/// Who holds HTTPS port 443 of this Mac's tailnet name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Ownership {
    /// Nothing is served there.
    Free,
    /// Only the entry hide recorded, proxying to `port`.
    Ours { port: u16 },
    /// Something hide did not add; `target` names it for the Settings line.
    /// `ours_too` is set when hide's recorded entry sits beside it.
    Foreign { target: String, ours_too: bool },
}

/// The proxy target hide gives `tailscale serve` for a daemon port.
pub fn proxy_target(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

/// Whether Tailscale Funnel publishes `dns_name`'s port 443 to the whole
/// internet. hide's entry is for the tailnet only (PRD D-01), so a Funnel
/// there keeps Mobile unexposed. A foreground `tailscale funnel` session
/// keeps its own config under `Foreground.<session>`, so those count too.
pub fn funnel_on(serve: &Value, dns_name: &str) -> bool {
    let key = format!("{dns_name}:443");
    let allows = |config: &Value| {
        config
            .get("AllowFunnel")
            .and_then(|funnel| funnel.get(&key))
            .and_then(Value::as_bool)
            == Some(true)
    };
    allows(serve)
        || serve
            .get("Foreground")
            .and_then(Value::as_object)
            .is_some_and(|sessions| sessions.values().any(allows))
}

/// The line Settings > Mobile shows while Funnel is on for this address.
pub const FUNNEL_MESSAGE: &str = "Tailscale Funnel publishes this address to the internet; Hide opens only inside the tailnet, so turning Funnel off lets it continue";

/// Reads `serve status --json` for port 443 of `dns_name`: hide's entry is
/// the `/` handler proxying to the recorded port; anything else there, and a
/// raw TCP forward on 443, belongs to someone else.
pub fn ownership(
    serve: &Value,
    dns_name: &str,
    record: Option<&ServeRecord>,
) -> Result<Ownership, String> {
    if !serve.is_object() {
        return Err("serve status is not a JSON object".to_owned());
    }
    let recorded_port = record
        .filter(|record| record.dns_name == dns_name)
        .map(|record| record.port);
    let mut ours = None;
    let mut foreign = Vec::new();
    let host = format!("{dns_name}:443");
    if let Some(web) = serve.get("Web").and_then(Value::as_object) {
        for (key, entry) in web {
            let on_443 = key == &host || key.ends_with(":443");
            if !on_443 {
                continue;
            }
            let handlers = entry.get("Handlers").and_then(Value::as_object);
            for (path, handler) in handlers.into_iter().flatten() {
                let proxy = handler.get("Proxy").and_then(Value::as_str);
                let is_ours = key == &host
                    && path == "/"
                    && recorded_port.is_some_and(|port| proxy == Some(proxy_target(port).as_str()));
                if is_ours {
                    ours = recorded_port;
                } else {
                    let target = proxy
                        .map(str::to_owned)
                        .or_else(|| {
                            handler
                                .get("Path")
                                .and_then(Value::as_str)
                                .map(|path| format!("files at {path}"))
                        })
                        .or_else(|| handler.get("Text").map(|_| "a text response".to_owned()))
                        .unwrap_or_else(|| "another handler".to_owned());
                    foreign.push(format!("https://{key}{path} → {target}"));
                }
            }
        }
    }
    if let Some(tcp) = serve.pointer("/TCP/443")
        && tcp.get("HTTPS").and_then(Value::as_bool) != Some(true)
    {
        let target = tcp
            .get("TCPForward")
            .and_then(Value::as_str)
            .unwrap_or("a TCP forward");
        foreign.push(format!("tcp :443 → {target}"));
    }
    Ok(match (foreign.first(), ours) {
        (Some(first), ours) => Ownership::Foreign {
            target: first.clone(),
            ours_too: ours.is_some(),
        },
        (None, Some(port)) => Ownership::Ours { port },
        (None, None) => Ownership::Free,
    })
}

pub async fn serve_status(program: &Path) -> Result<Value, CommandFailure> {
    let bytes = run(program, &["serve", "status", "--json"]).await?;
    // An empty configuration may print nothing at all.
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(Value::Object(Default::default()));
    }
    serde_json::from_slice(&bytes).map_err(|error| CommandFailure {
        command: "tailscale serve status --json".to_owned(),
        stderr: format!("unreadable answer: {error}"),
        exit_code: Some(0),
    })
}

/// Serves the daemon's loopback port at `https://<name>/` in the background.
pub async fn serve_add(program: &Path, port: u16) -> Result<(), CommandFailure> {
    let target = proxy_target(port);
    run(
        program,
        &["serve", "--bg", "--yes", "--https=443", target.as_str()],
    )
    .await
    .map(|_| ())
}

/// Removes the `/` handler on HTTPS port 443, which is hide's entry: this is
/// called only after `ownership` found hide's recorded entry there.
pub async fn serve_remove(program: &Path) -> Result<(), CommandFailure> {
    run(
        program,
        &["serve", "--yes", "--https=443", "--set-path=/", "off"],
    )
    .await
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn record(port: u16) -> ServeRecord {
        ServeRecord {
            dns_name: "mac.tailnet.ts.net".into(),
            port,
            added_at: 1,
        }
    }

    #[test]
    fn status_reads_login_name_and_https() {
        let ready = json!({
            "BackendState": "Running",
            "Self": {"DNSName": "mac.tailnet.ts.net.", "HostName": "mac"},
            "CurrentTailnet": {"MagicDNSEnabled": true},
            "CertDomains": ["mac.tailnet.ts.net"],
        });
        let status = parse_status(ready.to_string().as_bytes()).unwrap();
        assert!(status.logged_in && status.https && status.magic_dns);
        assert_eq!(status.dns_name.as_deref(), Some("mac.tailnet.ts.net"));
        let checklist = Checklist::from_status(&status);
        assert!(checklist.passed());

        let no_https = json!({
            "BackendState": "Running",
            "Self": {"DNSName": "mac.tailnet.ts.net.", "HostName": "mac"},
            "CurrentTailnet": {"MagicDNSEnabled": true},
        });
        let checklist =
            Checklist::from_status(&parse_status(no_https.to_string().as_bytes()).unwrap());
        assert_eq!(checklist.logged_in, StepState::Ok);
        assert_eq!(checklist.https, StepState::Failed);

        let logged_out = json!({"BackendState": "NeedsLogin", "Self": {"DNSName": ""}});
        let checklist =
            Checklist::from_status(&parse_status(logged_out.to_string().as_bytes()).unwrap());
        assert_eq!(checklist.logged_in, StepState::Failed);
        assert_eq!(checklist.https, StepState::Waiting);
        assert!(!checklist.passed());
    }

    #[test]
    fn port_443_belongs_to_hide_only_when_it_matches_the_record() {
        let ours = json!({"TCP": {"443": {"HTTPS": true}}, "Web": {"mac.tailnet.ts.net:443": {"Handlers": {"/": {"Proxy": "http://127.0.0.1:4000"}}}}});
        assert_eq!(
            ownership(&ours, "mac.tailnet.ts.net", Some(&record(4000))).unwrap(),
            Ownership::Ours { port: 4000 }
        );
        // The same entry without a record is someone else's.
        assert!(matches!(
            ownership(&ours, "mac.tailnet.ts.net", None).unwrap(),
            Ownership::Foreign {
                ours_too: false,
                ..
            }
        ));
        // A record for another port does not claim it either.
        assert!(matches!(
            ownership(&ours, "mac.tailnet.ts.net", Some(&record(5000))).unwrap(),
            Ownership::Foreign { .. }
        ));
        assert_eq!(
            ownership(&json!({}), "mac.tailnet.ts.net", Some(&record(4000))).unwrap(),
            Ownership::Free
        );
        let beside = json!({"Web": {"mac.tailnet.ts.net:443": {"Handlers": {
            "/": {"Proxy": "http://127.0.0.1:4000"},
            "/grafana": {"Proxy": "http://127.0.0.1:3000"},
        }}}});
        match ownership(&beside, "mac.tailnet.ts.net", Some(&record(4000))).unwrap() {
            Ownership::Foreign { target, ours_too } => {
                assert!(ours_too);
                assert!(target.contains("/grafana"), "{target}");
            }
            other => panic!("{other:?}"),
        }
        let tcp = json!({"TCP": {"443": {"TCPForward": "127.0.0.1:22"}}});
        assert!(matches!(
            ownership(&tcp, "mac.tailnet.ts.net", None).unwrap(),
            Ownership::Foreign { .. }
        ));
    }

    #[test]
    fn a_pinned_cli_is_the_only_one_tried() {
        let dir = tempfile::tempdir().unwrap();
        let missing = CliSource {
            pinned: Some(dir.path().join("tailscale")),
            search_path: Some("/usr/bin:/bin".into()),
        };
        assert_eq!(missing.resolve(), None);
        let script = dir.path().join("tailscale");
        std::fs::write(&script, "#!/bin/sh\n").unwrap();
        assert_eq!(
            CliSource {
                pinned: Some(script.clone()),
                search_path: None
            }
            .resolve(),
            Some(script)
        );
    }

    #[test]
    fn a_funnel_on_the_address_is_seen_and_another_address_is_not() {
        let serve = serde_json::json!({"AllowFunnel": {"mac.tailnet-name.ts.net:443": true, "other.ts.net:443": false}});
        assert!(funnel_on(&serve, "mac.tailnet-name.ts.net"));
        assert!(!funnel_on(&serve, "other.ts.net"));
        assert!(!funnel_on(
            &serde_json::json!({}),
            "mac.tailnet-name.ts.net"
        ));
        let foreground = serde_json::json!({"Foreground": {"a1b2": {"AllowFunnel": {"mac.tailnet-name.ts.net:443": true}}}});
        assert!(funnel_on(&foreground, "mac.tailnet-name.ts.net"));
    }
}
