//! Which TCP ports this machine is listening on, and where each listener was
//! started from, as the node reads them for the core (`listening_ports`).
//! Unix uses `lsof`; Windows uses the platform's bounded native listener
//! observation. Deciding which pane a listener belongs to is the core's.

#[cfg(any(not(windows), test))]
use std::collections::BTreeMap;
#[cfg(any(not(windows), test))]
use std::process::Command;
#[cfg(any(not(windows), test))]
use std::time::Duration;

use hide_node_link::ports::{ListeningPort, ListeningPorts};

/// One address a process listens on.
#[cfg(any(not(windows), test))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
}

#[cfg(windows)]
pub fn read() -> ListeningPorts {
    let sockets = match hide_platform::listeners::read() {
        Ok(sockets) => sockets,
        Err(error) => {
            return ListeningPorts {
                entries: Vec::new(),
                unavailable_reason: Some(format!(
                    "native TCP listener observation failed: {error}"
                )),
            };
        }
    };
    let mut entries: Vec<_> = sockets
        .into_iter()
        .map(|socket| {
            let host = match socket.address {
                std::net::SocketAddr::V4(address) => {
                    if address.ip().is_unspecified() {
                        "127.0.0.1".to_owned()
                    } else {
                        address.ip().to_string()
                    }
                }
                std::net::SocketAddr::V6(address) => {
                    if address.ip().is_unspecified() {
                        "::1".to_owned()
                    } else if address.scope_id() == 0 {
                        address.ip().to_string()
                    } else {
                        format!("{}%{}", address.ip(), address.scope_id())
                    }
                }
            };
            ListeningPort {
                host,
                port: socket.address.port(),
                cwd: socket.cwd.as_str().to_owned(),
            }
        })
        .collect();
    entries.sort_by(|left, right| {
        left.port
            .cmp(&right.port)
            .then_with(|| left.cwd.cmp(&right.cwd))
            .then_with(|| left.host.cmp(&right.host))
    });
    entries.dedup();
    ListeningPorts {
        entries,
        unavailable_reason: None,
    }
}

#[cfg(not(windows))]
pub fn read() -> ListeningPorts {
    let listeners = match listening_sockets() {
        Ok(listeners) => listeners,
        Err(reason) => {
            return ListeningPorts {
                entries: Vec::new(),
                unavailable_reason: Some(reason),
            };
        }
    };
    if listeners.is_empty() {
        return ListeningPorts::default();
    }
    let directories = match working_directories(&listeners.keys().copied().collect::<Vec<_>>()) {
        Ok(directories) => directories,
        Err(reason) => {
            return ListeningPorts {
                entries: Vec::new(),
                unavailable_reason: Some(reason),
            };
        }
    };

    let mut entries = Vec::new();
    for (pid, ports) in listeners {
        // A listener whose working directory could not be read belongs to no
        // pane, because the only rule for attributing it is that directory.
        let Some(cwd) = directories.get(&pid) else {
            continue;
        };
        for endpoint in ports {
            entries.push(ListeningPort {
                host: endpoint.host,
                port: endpoint.port,
                cwd: cwd.clone(),
            });
        }
    }
    entries.sort_by(|left, right| {
        left.port
            .cmp(&right.port)
            .then_with(|| left.cwd.cmp(&right.cwd))
            .then_with(|| left.host.cmp(&right.host))
    });
    entries.dedup();
    ListeningPorts {
        entries,
        unavailable_reason: None,
    }
}

/// Listening TCP sockets, as a pid to ports map.
#[cfg(not(windows))]
fn listening_sockets() -> Result<BTreeMap<u32, Vec<Endpoint>>, String> {
    let output = run_lsof(&["-nP", "-iTCP", "-sTCP:LISTEN", "-F", "pnt"])?;
    Ok(parse_listening_sockets(&output))
}

/// The working directory of each named process.
#[cfg(not(windows))]
fn working_directories(pids: &[u32]) -> Result<BTreeMap<u32, String>, String> {
    if pids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let joined = pids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let output = run_lsof(&["-a", "-d", "cwd", "-p", &joined, "-F", "pn"])?;
    Ok(parse_working_directories(&output))
}

/// How long one `lsof` may run. A cleanup waits on this read while it holds
/// the daemon's only cleanup lane, so a hung `lsof` must end as an unavailable
/// read, which fails closed, and not as a lane that never frees.
#[cfg(not(windows))]
const LSOF_DEADLINE: Duration = Duration::from_secs(10);

#[cfg(not(windows))]
fn run_lsof(arguments: &[&str]) -> Result<String, String> {
    // `-w` keeps warnings (an unreachable FUSE mount, say) off stderr, so
    // what lsof writes there is an error and `run_within` can read it as one.
    run_within(
        Command::new("lsof").arg("-w").args(arguments),
        LSOF_DEADLINE,
    )
}

#[cfg(any(not(windows), test))]
fn run_within(command: &mut Command, deadline: Duration) -> Result<String, String> {
    let output = crate::worktrees::output_within(command, deadline)
        .map_err(|error| format!("lsof could not be run: {error}"))?
        .ok_or_else(|| {
            format!(
                "lsof did not finish within {} seconds and was stopped",
                deadline.as_secs().max(1)
            )
        })?;
    lsof_answer(&output)
}

/// What one finished `lsof` said, read by its exit status as its manual
/// defines it: 0 lists everything asked about, and 1 means something asked
/// about was not found, which is how lsof answers a machine with no listener
/// (exit 1, nothing printed) and a process that exited between the two calls.
/// Either way what it did print is true. An error lsof detected is written to
/// stderr, any other status is not lsof's answer, and a signal ended it before
/// it answered; those are failures.
#[cfg(any(not(windows), test))]
fn lsof_answer(output: &std::process::Output) -> Result<String, String> {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    match output.status.code() {
        Some(0 | 1) if stderr.is_empty() => {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        }
        Some(1) => Err(format!("lsof reported an error: {stderr}")),
        _ if stderr.is_empty() => Err(format!("lsof exited with {}", output.status)),
        _ => Err(format!("lsof exited with {}: {stderr}", output.status)),
    }
}

/// Parses `lsof -F pn` field output.
///
/// The format is one field per line, tagged by its first character, and a `p`
/// line applies to every following line until the next `p`. A socket's `n`
/// field is an address such as `127.0.0.1:5173` or `*:8080`, so the port is
/// what follows the last colon.
#[cfg(any(not(windows), test))]
pub fn parse_listening_sockets(output: &str) -> BTreeMap<u32, Vec<Endpoint>> {
    let mut sockets: BTreeMap<u32, Vec<Endpoint>> = BTreeMap::new();
    let mut pid = None;
    let mut family = None;
    for line in output.lines() {
        let Some((tag, value)) = line.split_at_checked(1) else {
            continue;
        };
        match tag {
            "p" => {
                pid = value.trim().parse::<u32>().ok();
                family = None;
            }
            "t" => family = Some(value.trim()),
            "n" => {
                let Some(pid) = pid else { continue };
                // An address with an arrow is a connection, not a listener.
                if value.contains("->") {
                    continue;
                }
                let Some((host, port)) = value.rsplit_once(':') else {
                    continue;
                };
                let Ok(port) = port.trim().parse::<u16>() else {
                    continue;
                };
                if port == 0 {
                    continue;
                }
                let host = host.trim().trim_start_matches('[').trim_end_matches(']');
                let host = match host {
                    "*" if family == Some("IPv6") => "::1".to_owned(),
                    "*" if family == Some("IPv4") => "127.0.0.1".to_owned(),
                    "0.0.0.0" => "127.0.0.1".to_owned(),
                    "::" => "::1".to_owned(),
                    host => match host.parse::<std::net::IpAddr>() {
                        Ok(address) => address.to_string(),
                        Err(_) => continue,
                    },
                };
                let endpoint = Endpoint { host, port };
                let ports = sockets.entry(pid).or_default();
                if !ports.contains(&endpoint) {
                    ports.push(endpoint);
                }
            }
            _ => {}
        }
    }
    sockets
}

#[cfg(any(not(windows), test))]
pub fn parse_working_directories(output: &str) -> BTreeMap<u32, String> {
    let mut directories = BTreeMap::new();
    let mut pid = None;
    for line in output.lines() {
        let Some((tag, value)) = line.split_at_checked(1) else {
            continue;
        };
        match tag {
            "p" => pid = value.trim().parse::<u32>().ok(),
            "n" => {
                let Some(pid) = pid else { continue };
                let value = value.trim();
                if !value.is_empty() {
                    directories.entry(pid).or_insert_with(|| value.to_owned());
                }
            }
            _ => {}
        }
    }
    directories
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn a_read_that_outlives_its_deadline_is_stopped_and_reported_unavailable() {
        let started = Instant::now();
        let error =
            run_within(Command::new("sleep").arg("30"), Duration::from_millis(200)).unwrap_err();
        assert!(error.contains("did not finish"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(10));
        let ok = run_within(Command::new("echo").arg("p1"), Duration::from_secs(10)).unwrap();
        assert_eq!(ok.trim(), "p1");
    }

    #[cfg(unix)]
    fn finished(shell: &str) -> Result<String, String> {
        run_within(
            Command::new("sh").args(["-c", shell]),
            Duration::from_secs(10),
        )
    }

    #[cfg(unix)]
    #[test]
    fn nothing_found_is_an_empty_answer_and_only_a_failed_run_is_an_error() {
        // lsof's answer on a machine with no listener: exit 1, nothing printed.
        assert_eq!(finished("exit 1"), Ok(String::new()));
        // A process gone between the calls: exit 1 with what was still found.
        assert_eq!(finished("echo p1; exit 1"), Ok("p1\n".to_owned()));
        assert_eq!(finished("echo p1"), Ok("p1\n".to_owned()));

        let detected =
            finished("echo p1; echo 'lsof: status error on /x' >&2; exit 1").unwrap_err();
        assert!(detected.contains("status error on /x"), "{detected}");
        let other = finished("exit 2").unwrap_err();
        assert!(other.contains("exited with"), "{other}");
        let signalled = finished("kill -9 $$").unwrap_err();
        assert!(signalled.contains("exited with"), "{signalled}");
        let missing = run_within(
            &mut Command::new("hide-no-such-lsof"),
            Duration::from_secs(10),
        )
        .unwrap_err();
        assert!(missing.contains("could not be run"), "{missing}");
    }

    #[test]
    fn field_output_groups_every_port_under_the_process_that_listens_on_it() {
        let output = "p501\ntIPv4\nn*:8080\nn127.0.0.1:5173\np777\nn[::1]:3000\n";
        let sockets = parse_listening_sockets(output);
        assert_eq!(
            sockets.get(&501),
            Some(&vec![
                Endpoint {
                    host: "127.0.0.1".into(),
                    port: 8080
                },
                Endpoint {
                    host: "127.0.0.1".into(),
                    port: 5173
                }
            ])
        );
        assert_eq!(
            sockets.get(&777),
            Some(&vec![Endpoint {
                host: "::1".into(),
                port: 3000
            }])
        );
    }

    #[test]
    fn an_established_connection_is_not_a_listener() {
        let output = "p501\nn127.0.0.1:5173->127.0.0.1:61234\n";
        assert!(parse_listening_sockets(output).is_empty());
    }

    #[test]
    fn a_process_keeps_the_first_working_directory_reported_for_it() {
        let output = "p501\nn/srv/app\np777\nn/srv/other\n";
        let directories = parse_working_directories(output);
        assert_eq!(directories.get(&501).map(String::as_str), Some("/srv/app"));
        assert_eq!(
            directories.get(&777).map(String::as_str),
            Some("/srv/other")
        );
    }
}
