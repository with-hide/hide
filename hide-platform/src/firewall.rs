//! The Windows Defender Firewall rules that let machines on the same
//! network reach this account's peer endpoint directly (PRD
//! device-connect-iroh D-38): one inbound UDP rule for the state folder's
//! port, on every network profile, and one for mDNS (UDP 5353) on private
//! and domain networks, all in the rule group `Hide`, so removing Hide
//! removes them together. Reading them needs no rights; adding or removing
//! them needs an administrator, which the desktop app asks Windows for.
//! Every other system answers `Unsupported`: macOS asks its own Local
//! Network permission (D-83), and Linux's firewalls are the operator's.

use std::io;
#[cfg(windows)]
use std::process::Command;
#[cfg(windows)]
use std::time::Duration;

/// The rule group every Hide rule is in.
pub const GROUP: &str = "Hide";
/// The mDNS rule's name.
pub const MDNS_RULE: &str = "Hide mDNS";

/// The name of the rule for `port`.
pub fn rule_name(port: u16) -> String {
    format!("Hide {port}")
}

/// How long one PowerShell run may take: its first start on a machine can
/// take seconds while it loads the firewall module.
#[cfg(windows)]
const COMMAND_WITHIN: Duration = Duration::from_secs(60);

/// How long Windows's permission prompt may stay open before the ask is
/// given up: long enough for an account that is not an administrator to
/// have one type a password. The desktop app waits a little past it.
pub const PROMPT_WITHIN: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// Whether both rules for `port` are in place.
pub fn allowed(port: u16) -> io::Result<bool> {
    #[cfg(windows)]
    {
        let script = format!(
            "if ((Get-NetFirewallRule -DisplayName '{}' -ErrorAction SilentlyContinue) -and (Get-NetFirewallRule -DisplayName '{MDNS_RULE}' -ErrorAction SilentlyContinue)) {{ exit 0 }} exit 3",
            rule_name(port)
        );
        let finished = powershell(&script)?;
        match finished.code {
            Some(0) => Ok(true),
            Some(3) => Ok(false),
            code => Err(io::Error::other(format!(
                "the firewall rules could not be read (exit {code:?}): {}",
                finished.last_error_line()
            ))),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = port;
        Err(unsupported())
    }
}

/// Puts the rules for `port` in place, replacing one of the same name, and
/// takes out the rule for `retired`, the port it replaced. Needs an
/// administrator.
pub fn allow(port: u16, retired: Option<u16>) -> io::Result<()> {
    #[cfg(windows)]
    {
        let name = rule_name(port);
        let retire = retired
            .filter(|retired| *retired != port)
            .map(|retired| {
                format!(
                    "Get-NetFirewallRule -DisplayName '{}' -ErrorAction SilentlyContinue | Remove-NetFirewallRule; ",
                    rule_name(retired)
                )
            })
            .unwrap_or_default();
        let script = format!(
            concat!(
                "$ErrorActionPreference = 'Stop'; ",
                "{retire}",
                "Get-NetFirewallRule -DisplayName '{name}' -ErrorAction SilentlyContinue | Remove-NetFirewallRule; ",
                "New-NetFirewallRule -Group '{GROUP}' -DisplayName '{name}' -Direction Inbound -Protocol UDP -LocalPort {port} -Action Allow -Profile Any | Out-Null; ",
                "if (-not (Get-NetFirewallRule -DisplayName '{mdns}' -ErrorAction SilentlyContinue)) {{ ",
                "New-NetFirewallRule -Group '{GROUP}' -DisplayName '{mdns}' -Direction Inbound -Protocol UDP -LocalPort 5353 -Action Allow -Profile Private,Domain | Out-Null }}"
            ),
            retire = retire,
            name = name,
            GROUP = GROUP,
            port = port,
            mdns = MDNS_RULE,
        );
        checked(&script, "added")
    }
    #[cfg(not(windows))]
    {
        let _ = (port, retired);
        Err(unsupported())
    }
}

/// Removes every rule of the group. Needs an administrator.
pub fn remove_all() -> io::Result<()> {
    #[cfg(windows)]
    {
        checked(
            &format!(
                "$ErrorActionPreference = 'Stop'; Get-NetFirewallRule -Group '{GROUP}' -ErrorAction SilentlyContinue | Remove-NetFirewallRule"
            ),
            "removed",
        )
    }
    #[cfg(not(windows))]
    Err(unsupported())
}

/// Whether any rule of the group is in place, for any port.
pub fn any() -> io::Result<bool> {
    #[cfg(windows)]
    {
        let finished = powershell(&format!(
            "if (Get-NetFirewallRule -Group '{GROUP}' -ErrorAction SilentlyContinue) {{ exit 0 }} exit 3"
        ))?;
        match finished.code {
            Some(0) => Ok(true),
            Some(3) => Ok(false),
            code => Err(io::Error::other(format!(
                "the firewall rules could not be read (exit {code:?}): {}",
                finished.last_error_line()
            ))),
        }
    }
    #[cfg(not(windows))]
    Err(unsupported())
}

/// What asking Windows to run a program as an administrator came to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Elevated {
    /// It ran and exited 0.
    Done,
    /// The operator declined Windows's permission prompt.
    Declined,
    /// It ran and failed, with this exit.
    Failed(Option<i32>),
}

/// Windows's exit for a prompt the operator declined (`ERROR_CANCELLED`).
pub const DECLINED_EXIT: i32 = 1223;

/// Runs `program` with `arguments` as an administrator, through Windows's
/// permission prompt, and waits for it: what an uninstaller does to remove
/// the rules, which a per-user install cannot.
pub fn run_elevated(program: &std::path::Path, arguments: &[&str]) -> io::Result<Elevated> {
    #[cfg(windows)]
    {
        let quoted = |value: &str| format!("'{}'", value.replace('\'', "''"));
        let arguments: Vec<String> = arguments.iter().map(|argument| quoted(argument)).collect();
        let script = format!(
            // Only a declined prompt is a decline: Windows says it with
            // ERROR_CANCELLED inside the exception Start-Process throws, and
            // any other failure to start is a failure.
            "try {{ $p = Start-Process -FilePath {} -ArgumentList {} -Verb RunAs -WindowStyle Hidden -Wait -PassThru; exit $p.ExitCode }} catch {{ $e = $_.Exception; while ($e) {{ if ($e -is [System.ComponentModel.Win32Exception] -and $e.NativeErrorCode -eq {DECLINED_EXIT}) {{ exit {DECLINED_EXIT} }}; $e = $e.InnerException }}; exit 1 }}",
            quoted(&program.to_string_lossy()),
            arguments.join(","),
        );
        let finished = powershell_within(&script, PROMPT_WITHIN)?;
        Ok(match finished.code {
            Some(0) => Elevated::Done,
            Some(DECLINED_EXIT) => Elevated::Declined,
            code => Elevated::Failed(code),
        })
    }
    #[cfg(not(windows))]
    {
        let _ = (program, arguments);
        Err(unsupported())
    }
}

#[cfg(not(windows))]
fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "this system's firewall is not Windows Defender Firewall",
    )
}

#[cfg(windows)]
fn checked(script: &str, done: &str) -> io::Result<()> {
    let finished = powershell(script)?;
    if finished.code == Some(0) {
        return Ok(());
    }
    Err(io::Error::other(format!(
        "the firewall rules could not be {done} (exit {:?}): {}",
        finished.code,
        finished.last_error_line()
    )))
}

/// Runs `script` in Windows PowerShell with module auto-loading off and the
/// firewall's module imported by name, which costs under a second where
/// auto-loading can first build a cache of every module (docs/TESTING.md).
#[cfg(windows)]
fn powershell(script: &str) -> io::Result<crate::process::Finished> {
    powershell_within(script, COMMAND_WITHIN)
}

/// [`powershell`] given up `within` from now.
#[cfg(windows)]
fn powershell_within(script: &str, within: Duration) -> io::Result<crate::process::Finished> {
    let system = crate::host::variable("SystemRoot")
        .map(std::path::PathBuf::from)
        .ok_or_else(|| io::Error::other("SystemRoot is not set"))?;
    let mut command = Command::new(
        system
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe"),
    );
    crate::process::restrict_to_login_environment(&mut command);
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        &format!("$PSModuleAutoLoadingPreference = 'None'; Import-Module NetSecurity; {script}"),
    ]);
    crate::process::run_to_end(&mut command, within, &Default::default())
        .map_err(|error| io::Error::other(format!("PowerShell did not answer: {error:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_port_s_rule_is_named_by_its_port() {
        assert_eq!(rule_name(51234), "Hide 51234");
    }

    #[cfg(not(windows))]
    #[test]
    fn other_systems_keep_no_rules() {
        assert_eq!(
            allowed(51234).unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
        assert_eq!(
            allow(51234, Some(50000)).unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
        assert_eq!(remove_all().unwrap_err().kind(), io::ErrorKind::Unsupported);
    }
}
