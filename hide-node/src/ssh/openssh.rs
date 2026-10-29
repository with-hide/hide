//! The system `ssh` program as a device's link (PRD system-ssh-device-link
//! D-01, D-02, D-12, D-13): what `ssh -G` says an alias resolves to, the one
//! command line that opens the link, and, in this one place, what a run that
//! did not connect said, classified for the device row. The raw words go to
//! the diagnostic log; the row gets the class and at most one line.
//!
//! Hide adds to the alias only what the account's configuration left at
//! OpenSSH's default (a connect timeout, keepalives), because a command-line
//! `-o` wins over the configuration (ssh_config(5), first value wins), and
//! what the connection itself needs to be one owned process with clean input
//! and output: no terminal, none of the alias's port forwards, no shared
//! master connection and no command of the configuration's own. `LogLevel` is
//! never lowered, so a sign-in banner still reaches the row.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;

use hide_node_link::device::{Refusal, SshFailure, SshProblem};
use hide_platform::process::{RunFailure, run_to_end};

/// How long `ssh -G` and `ssh -V` may take; `-G` runs the configuration's
/// `Match exec` commands, the operator's own.
const QUERY_DEADLINE: Duration = Duration::from_secs(10);
/// What Hide sets when the configuration leaves it at OpenSSH's default
/// (B28): a dial that gets no answer gives up, and a link whose network went
/// away ends within a minute instead of hanging until TCP gives up.
const CONNECT_TIMEOUT_SECONDS: u32 = 15;
const ALIVE_INTERVAL_SECONDS: u32 = 15;
const ALIVE_COUNT: u32 = 3;
/// The most of `ssh -G`'s answer read; a configuration's answer is a few
/// kilobytes.
const QUERY_OUTPUT_LIMIT: usize = 256 * 1024;

/// The `ssh` this computer has, and the configuration it reads.
#[derive(Clone, Debug)]
pub struct SshProgram {
    path: PathBuf,
    /// The account home Hide was given when it is not the one the user
    /// database names (a test's private HOME): `ssh` reads its files from
    /// the user database's home, so it is told this home's config and given
    /// it as `HOME`. `None` reads exactly what a terminal's `ssh` reads,
    /// `/etc/ssh/ssh_config` included.
    private_home: Option<PathBuf>,
}

/// The release of OpenSSH a program reported.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    /// Microsoft's port, whose `ssh.exe` reads askpass by its own rule.
    pub windows: bool,
}

impl Version {
    /// Reads `ssh -V`'s line: `OpenSSH_9.6p1 Ubuntu-3ubuntu13, OpenSSL ...`,
    /// `OpenSSH_for_Windows_8.1p1, LibreSSL 3.0.2`.
    pub fn read(line: &str) -> Option<Self> {
        let rest = line.trim().strip_prefix("OpenSSH_")?;
        let (windows, rest) = match rest.strip_prefix("for_Windows_") {
            Some(rest) => (true, rest),
            None => (false, rest),
        };
        let mut numbers = rest.split(|byte: char| !byte.is_ascii_digit());
        let major = numbers.next()?.parse().ok()?;
        let minor = numbers.next()?.parse().ok()?;
        Some(Self {
            major,
            minor,
            windows,
        })
    }

    /// Whether this `ssh` hands every question to the askpass program when
    /// told to (`SSH_ASKPASS_REQUIRE=force`, OpenSSH 8.4). Microsoft's port
    /// asks it whenever `SSH_ASKPASS` is set (its `readpass.c`); B21 holds
    /// an older OpenSSH elsewhere to keys and the agent.
    pub fn asks_through_askpass(self) -> bool {
        self.windows || (self.major, self.minor) >= (8, 4)
    }
}

/// What `ssh -G` resolved an alias to: where it signs in, and the options
/// the connection reads to decide what Hide may add.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Resolved {
    pub user: String,
    pub hostname: String,
    pub port: u16,
    options: BTreeMap<String, Vec<String>>,
}

impl Resolved {
    /// Reads `ssh -G`'s `keyword value` lines; keywords are lowercase there,
    /// and a keyword given more than once (`identityfile`) keeps each value.
    pub fn read(output: &str) -> Option<Self> {
        let mut options: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for line in output.lines() {
            let line = line.trim();
            let Some((key, value)) = line.split_once(char::is_whitespace) else {
                continue;
            };
            options
                .entry(key.to_ascii_lowercase())
                .or_default()
                .push(value.trim().to_owned());
        }
        let one = |key: &str| options.get(key).and_then(|values| values.first()).cloned();
        let user = one("user").filter(|user| !user.is_empty())?;
        let hostname = one("hostname").filter(|host| !host.is_empty())?;
        let port = one("port")?.parse().ok()?;
        Some(Self {
            user,
            hostname,
            port,
            options,
        })
    }

    fn value(&self, key: &str) -> Option<&str> {
        self.options
            .get(key)
            .and_then(|values| values.first())
            .map(String::as_str)
    }

    /// Whether the alias reaches its server through a proxy command or a
    /// jump host, whose failure the row names as the proxy's.
    pub fn proxied(&self) -> bool {
        self.value("proxycommand")
            .is_some_and(|value| value != "none")
            || self.value("proxyjump").is_some_and(|value| value != "none")
    }

    /// The `-o` options the link adds, in order (see the module comment).
    pub fn link_options(&self, version: Version) -> Vec<String> {
        let mut options = Vec::new();
        if self
            .value("connecttimeout")
            .is_none_or(|value| value == "none")
        {
            options.push(format!("ConnectTimeout={CONNECT_TIMEOUT_SECONDS}"));
        }
        if self
            .value("serveraliveinterval")
            .is_none_or(|value| value == "0")
        {
            options.push(format!("ServerAliveInterval={ALIVE_INTERVAL_SECONDS}"));
            if self
                .value("serveralivecountmax")
                .is_none_or(|value| value == "3")
            {
                options.push(format!("ServerAliveCountMax={ALIVE_COUNT}"));
            }
        }
        // The connection's own shape, not a default the operator could have
        // chosen otherwise: the alias's forwards are its terminal session's,
        // a master another ssh shares would outlive this device's link (B26),
        // and the link's input and output are the helper's.
        options.push("ClearAllForwardings=yes".to_owned());
        options.push("ControlMaster=no".to_owned());
        options.push("ControlPath=none".to_owned());
        if self
            .value("remotecommand")
            .is_some_and(|value| value != "none")
        {
            options.push("RemoteCommand=none".to_owned());
        }
        // Printed only by an ssh that knows the keyword, so only that ssh is
        // told it.
        if self.value("forkafterauthentication") == Some("yes") {
            options.push("ForkAfterAuthentication=no".to_owned());
        }
        if self.value("stdinnull") == Some("yes") {
            options.push("StdinNull=no".to_owned());
        }
        if self
            .value("sessiontype")
            .is_some_and(|value| value != "default")
        {
            options.push("SessionType=default".to_owned());
        }
        // B21: an ssh that cannot hand Hide a question never asks one on a
        // terminal it would not find; keys and the agent still sign in.
        if !version.asks_through_askpass() {
            options.push("BatchMode=yes".to_owned());
        }
        options
    }
}

/// Every program's version, read once per program path for the life of
/// this process; `ssh -V` is a process start per answer otherwise.
static VERSIONS: Mutex<Vec<(PathBuf, Version)>> = Mutex::new(Vec::new());

impl SshProgram {
    /// `path` as the account at `home` runs it.
    pub fn new(path: PathBuf, home: &Path) -> Self {
        let own = hide_platform::user_agents::account_home()
            .ok()
            .is_some_and(|account| account == home);
        Self {
            path,
            private_home: (!own).then(|| home.to_owned()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `ssh` with the account's configuration: `-F` and `HOME` for a private
    /// home, and an environment without the Herdr pane identity this
    /// process may carry, so the alias's proxy command sees the account's own.
    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.path);
        for (key, _) in std::env::vars_os() {
            let name = key.to_string_lossy();
            if name.starts_with("HERDR_") || name.starts_with("HIDE_") {
                command.env_remove(&key);
            }
        }
        if let Some(home) = &self.private_home {
            command
                .env("HOME", home)
                .arg("-F")
                .arg(home.join(".ssh").join("config"));
        }
        command
    }

    /// What `ssh -V` reports, read once per program.
    pub fn version(&self) -> Result<Version, SshFailure> {
        if let Some((_, version)) = lock(&VERSIONS).iter().find(|(path, _)| path == &self.path) {
            return Ok(*version);
        }
        let mut command = Command::new(&self.path);
        command.arg("-V");
        let finished = self.query(&mut command, "ssh -V")?;
        // `-V` prints on standard error.
        let said = format!("{}{}", finished.stderr, finished.stdout);
        let version = said
            .lines()
            .find_map(Version::read)
            .ok_or_else(|| SshFailure {
                problem: SshProblem::Unclassified,
                detail: format!(
                    "ssh -V did not name an OpenSSH release: {}",
                    last_line(said.lines())
                ),
            })?;
        let mut versions = lock(&VERSIONS);
        if versions.len() < 16 {
            versions.push((self.path.clone(), version));
        }
        Ok(version)
    }

    /// What `ssh -G` resolves `alias` to.
    pub fn resolve(&self, alias: &str) -> Result<Resolved, SshFailure> {
        let mut command = self.command();
        command.args(["-G", "--"]).arg(alias);
        let finished = self.query(&mut command, "ssh -G")?;
        if !finished.succeeded() {
            return Err(SshFailure {
                problem: SshProblem::Unclassified,
                detail: last_line(finished.stderr.lines()),
            });
        }
        Resolved::read(&finished.stdout).ok_or_else(|| SshFailure {
            problem: SshProblem::Unclassified,
            detail: format!("ssh -G did not say where {alias} signs in"),
        })
    }

    /// The arguments that open the link to `alias`, its remote command last.
    pub fn link_args(
        &self,
        alias: &str,
        resolved: &Resolved,
        version: Version,
        remote_command: &str,
    ) -> Vec<OsString> {
        let mut args: Vec<OsString> = vec!["-T".into()];
        for option in resolved.link_options(version) {
            args.push("-o".into());
            args.push(option.into());
        }
        args.push("--".into());
        args.push(alias.into());
        args.push(remote_command.into());
        args
    }

    fn query(
        &self,
        command: &mut Command,
        what: &str,
    ) -> Result<hide_platform::process::Finished, SshFailure> {
        let stop = std::sync::atomic::AtomicBool::new(false);
        match run_to_end(command, QUERY_DEADLINE, &stop) {
            Ok(finished) if finished.stdout.len() + finished.stderr.len() <= QUERY_OUTPUT_LIMIT => {
                Ok(finished)
            }
            Ok(_) => Err(SshFailure {
                problem: SshProblem::Unclassified,
                detail: format!("{what} answered more than Hide reads"),
            }),
            Err(RunFailure::Start(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                Err(missing(&self.path))
            }
            Err(RunFailure::TimedOut) => Err(SshFailure {
                problem: SshProblem::Unclassified,
                detail: format!(
                    "{what} did not answer within {} s",
                    QUERY_DEADLINE.as_secs()
                ),
            }),
            Err(error) => Err(SshFailure {
                problem: SshProblem::Unclassified,
                detail: format!("{what} could not run: {error:?}"),
            }),
        }
    }
}

/// B20: no `ssh` on this computer.
pub fn missing(path: &Path) -> SshFailure {
    SshFailure {
        problem: SshProblem::SshMissing,
        detail: format!("{} was not found on this computer", path.display()),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The last line that says something, trimmed; ssh ends its error output
/// with the line that names the failure.
pub fn last_line<'a>(lines: impl DoubleEndedIterator<Item = &'a str>) -> String {
    lines
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("ssh ended without saying why")
        .chars()
        .take(300)
        .collect()
}

/// What one line of ssh's error output says while it signs in.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SignInLine {
    /// A sign-in that goes on in a browser: a `tailscale ssh` check banner,
    /// a `cloudflared access` login, any https link a banner or a proxy
    /// prints (D-10). An ordinary banner's link ends its wait as soon as the
    /// connection goes on.
    Browser(String),
    /// Anything else, kept for the classification.
    Other,
}

/// Reads one line of ssh's error output during sign-in.
pub fn sign_in_line(line: &str) -> SignInLine {
    let Some(start) = line.find("https://") else {
        return SignInLine::Other;
    };
    let url: String = line[start..]
        .chars()
        .take_while(|character| !character.is_whitespace() && !character.is_control())
        .collect();
    let url = url.trim_end_matches(['.', ',', ';', ')', '\'', '"', '>']);
    if url.len() <= "https://".len() || url.len() > 2048 {
        return SignInLine::Other;
    }
    SignInLine::Browser(url.to_owned())
}

/// How an attempt that did not connect ended, as the classification reads
/// it.
pub struct Ending<'a> {
    /// ssh's error output, line by line, in order.
    pub stderr: &'a [String],
    /// The first question the attempt could not have answered, and why.
    pub refusal: Option<Refusal>,
    /// The alias goes through a proxy command or a jump host.
    pub proxied: bool,
    /// This ssh cannot hand Hide a question (B21).
    pub prompts_unavailable: bool,
    /// A browser check was shown and not finished.
    pub browser_waited: bool,
}

/// Classifies an attempt that ended before the device's start script
/// answered (B18, B19). Deliberately by what ssh printed: OpenSSH has no
/// machine-readable failure, and these sentences have held since 7.x; a
/// sentence this does not know is shown as it is (B19).
pub fn classify(ending: &Ending<'_>) -> SshFailure {
    let has = |needle: &str| ending.stderr.iter().any(|line| line.contains(needle));
    let failure = |problem, detail: &str| SshFailure {
        problem,
        detail: detail.to_owned(),
    };
    let last = || last_line(ending.stderr.iter().map(String::as_str));
    if let Some(refusal) = ending.refusal {
        return match refusal {
            Refusal::NotAllowed if ending.browser_waited => failure(
                SshProblem::SignInNeeded,
                "Signing in needs a check in the browser",
            ),
            Refusal::NotAllowed => failure(SshProblem::SignInNeeded, "Signing in needs an answer"),
            Refusal::Cancelled => failure(SshProblem::SignInNeeded, "The sign-in was cancelled"),
            Refusal::TimedOut => failure(
                SshProblem::SignInNeeded,
                "The sign-in question was not answered in time",
            ),
            Refusal::Ended => failure(SshProblem::SignInNeeded, "The sign-in ended"),
        };
    }
    if ending.browser_waited {
        return failure(
            SshProblem::BrowserTimeout,
            "The check in the browser was not finished in time",
        );
    }
    if let Some(line) = ending
        .stderr
        .iter()
        .find(|line| line.trim_start().starts_with("tailscale: "))
    {
        return SshFailure {
            problem: SshProblem::PolicyRefused,
            detail: line.trim().chars().take(300).collect(),
        };
    }
    if has("REMOTE HOST IDENTIFICATION HAS CHANGED") || has("Host key for ") && has("has changed") {
        return failure(
            SshProblem::HostKeyChanged,
            "The server's key differs from the one known_hosts holds",
        );
    }
    if has("Could not resolve hostname") {
        return failure(SshProblem::NameNotFound, &last());
    }
    // An unknown key under BatchMode, or a sign-in that needed a question,
    // on an ssh that cannot ask one.
    if ending.prompts_unavailable
        && (has("Permission denied") || has("Host key verification failed"))
    {
        return failure(
            SshProblem::OpenSshOld,
            "This OpenSSH cannot ask for a password or a new server key",
        );
    }
    if has("Permission denied") || has("Too many authentication failures") {
        return failure(SshProblem::SignInRefused, &last());
    }
    // The proxy (or the jump host behind it) ended before the server spoke:
    // ssh then names the far end `UNKNOWN port 65535`.
    if ending.proxied
        && (has("UNKNOWN port 65535")
            || has("stdio forwarding failed")
            || has("channel 0: open failed")
            || has("kex_exchange_identification"))
    {
        return failure(SshProblem::ProxyFailed, &last());
    }
    for unreachable in [
        "Connection refused",
        "Connection timed out",
        "Operation timed out",
        "No route to host",
        "Network is unreachable",
        "Host is down",
        "Connection reset",
        "Connection closed by remote host",
    ] {
        if has(unreachable) {
            return failure(SshProblem::Unreachable, &last());
        }
    }
    failure(SshProblem::Unclassified, &last())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolved(extra: &str) -> Resolved {
        Resolved::read(&format!(
            "user al\nhostname studio.example.test\nport 22\nconnecttimeout none\nserveraliveinterval 0\nserveralivecountmax 3\nloglevel INFO\nidentityfile ~/.ssh/id_ed25519\nidentityfile ~/.ssh/id_rsa\n{extra}"
        ))
        .unwrap()
    }

    const CURRENT: Version = Version {
        major: 9,
        minor: 6,
        windows: false,
    };

    #[test]
    fn versions_are_read_from_each_ports_line() {
        assert_eq!(
            Version::read("OpenSSH_9.6p1 Ubuntu-3ubuntu13.5, OpenSSL 3.0.13 30 Jan 2024"),
            Some(CURRENT)
        );
        let windows = Version::read("OpenSSH_for_Windows_8.1p1, LibreSSL 3.0.2").unwrap();
        assert_eq!(
            (windows.major, windows.minor, windows.windows),
            (8, 1, true)
        );
        assert!(windows.asks_through_askpass());
        assert!(
            !Version::read("OpenSSH_8.3p1")
                .unwrap()
                .asks_through_askpass()
        );
        assert!(
            Version::read("OpenSSH_8.4p1")
                .unwrap()
                .asks_through_askpass()
        );
        assert_eq!(Version::read("Sun_SSH_1.1"), None);
    }

    #[test]
    fn an_alias_left_at_defaults_gets_the_timeouts_and_the_connections_own_shape() {
        assert_eq!(
            resolved("").link_options(CURRENT),
            [
                "ConnectTimeout=15",
                "ServerAliveInterval=15",
                "ServerAliveCountMax=3",
                "ClearAllForwardings=yes",
                "ControlMaster=no",
                "ControlPath=none",
            ]
        );
    }

    #[test]
    fn what_the_configuration_chose_is_never_overridden() {
        let mut chosen = resolved("");
        chosen
            .options
            .insert("connecttimeout".into(), vec!["4".into()]);
        chosen
            .options
            .insert("serveraliveinterval".into(), vec!["30".into()]);
        let options = chosen.link_options(CURRENT);
        assert!(
            !options
                .iter()
                .any(|option| option.starts_with("ConnectTimeout"))
        );
        assert!(
            !options
                .iter()
                .any(|option| option.starts_with("ServerAlive"))
        );
        // A count the configuration chose stays when only the interval is
        // Hide's.
        let mut count = resolved("");
        count
            .options
            .insert("serveralivecountmax".into(), vec!["9".into()]);
        let options = count.link_options(CURRENT);
        assert!(options.contains(&"ServerAliveInterval=15".to_owned()));
        assert!(
            !options
                .iter()
                .any(|option| option.starts_with("ServerAliveCountMax"))
        );
    }

    #[test]
    fn a_configuration_that_would_take_the_links_input_is_undone() {
        let options = resolved(
            "remotecommand tmux attach\nforkafterauthentication yes\nstdinnull yes\nsessiontype none\n",
        )
        .link_options(CURRENT);
        for expected in [
            "RemoteCommand=none",
            "ForkAfterAuthentication=no",
            "StdinNull=no",
            "SessionType=default",
        ] {
            assert!(
                options.contains(&expected.to_owned()),
                "{expected}: {options:?}"
            );
        }
        let plain = resolved("forkafterauthentication no\nstdinnull no\nsessiontype default\n")
            .link_options(CURRENT);
        assert!(
            !plain
                .iter()
                .any(|option| option.starts_with("RemoteCommand")
                    || option.starts_with("ForkAfter")
                    || option.starts_with("StdinNull")
                    || option.starts_with("SessionType"))
        );
    }

    #[test]
    fn an_ssh_that_cannot_ask_signs_in_with_keys_alone() {
        let old = Version::read("OpenSSH_8.2p1").unwrap();
        assert!(
            resolved("")
                .link_options(old)
                .contains(&"BatchMode=yes".to_owned())
        );
        assert!(
            !resolved("")
                .link_options(CURRENT)
                .contains(&"BatchMode=yes".to_owned())
        );
    }

    #[test]
    fn a_resolution_without_its_address_is_not_one() {
        assert_eq!(Resolved::read("user al\nport 22\n"), None);
        assert_eq!(Resolved::read("user al\nhostname h\nport twenty\n"), None);
        let proxied = resolved("proxyjump bastion\n");
        assert!(proxied.proxied());
        assert!(!resolved("proxycommand none\n").proxied());
    }

    #[test]
    fn a_banner_link_is_a_browser_wait_and_its_punctuation_is_not_part_of_it() {
        assert_eq!(
            sign_in_line("# To authenticate, visit: https://login.tailscale.com/a/1a2b3c."),
            SignInLine::Browser("https://login.tailscale.com/a/1a2b3c".to_owned())
        );
        assert_eq!(
            sign_in_line(
                "Please open the following URL and log in with your Cloudflare account: https://team.cloudflareaccess.com/cdn-cgi/access/cli?token=x"
            ),
            SignInLine::Browser(
                "https://team.cloudflareaccess.com/cdn-cgi/access/cli?token=x".to_owned()
            )
        );
        assert_eq!(sign_in_line("Authorized use only."), SignInLine::Other);
        assert_eq!(sign_in_line("see https:// for more"), SignInLine::Other);
    }

    fn ending(stderr: &[&str]) -> SshFailure {
        let lines: Vec<String> = stderr.iter().map(|line| (*line).to_owned()).collect();
        classify(&Ending {
            stderr: &lines,
            refusal: None,
            proxied: false,
            prompts_unavailable: false,
            browser_waited: false,
        })
    }

    #[test]
    fn each_failure_ssh_prints_has_its_class() {
        let cases: &[(&[&str], SshProblem)] = &[
            (
                &[
                    "ssh: Could not resolve hostname studio.invalid: nodename nor servname provided, or not known",
                ],
                SshProblem::NameNotFound,
            ),
            (
                &["ssh: connect to host 127.0.0.1 port 1: Connection refused"],
                SshProblem::Unreachable,
            ),
            (
                &["ssh: connect to host 10.0.0.9 port 22: Operation timed out"],
                SshProblem::Unreachable,
            ),
            (
                &["al@studio: Permission denied (publickey,password)."],
                SshProblem::SignInRefused,
            ),
            (
                &[
                    "@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@",
                    "@    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @",
                    "Host key verification failed.",
                ],
                SshProblem::HostKeyChanged,
            ),
            (
                &[
                    "tailscale: tailnet policy does not permit you to SSH to this node",
                    "Connection closed by 100.64.0.2 port 22",
                ],
                SshProblem::PolicyRefused,
            ),
            (&["something new went wrong"], SshProblem::Unclassified),
        ];
        for (stderr, problem) in cases {
            assert_eq!(ending(stderr).problem, *problem, "{stderr:?}");
        }
        assert_eq!(
            ending(&["tailscale: tailnet policy does not permit you to SSH to this node"]).detail,
            "tailscale: tailnet policy does not permit you to SSH to this node"
        );
        assert_eq!(
            ending(&["first", "the last line", "  "]).detail,
            "the last line"
        );
    }

    #[test]
    fn a_proxy_that_ended_is_the_proxys_failure_only_for_a_proxied_alias() {
        let lines = vec![
            "kex_exchange_identification: Connection closed by remote host".to_owned(),
            "Connection closed by UNKNOWN port 65535".to_owned(),
        ];
        let proxied = classify(&Ending {
            stderr: &lines,
            refusal: None,
            proxied: true,
            prompts_unavailable: false,
            browser_waited: false,
        });
        assert_eq!(proxied.problem, SshProblem::ProxyFailed);
        let direct = classify(&Ending {
            stderr: &lines,
            refusal: None,
            proxied: false,
            prompts_unavailable: false,
            browser_waited: false,
        });
        assert_eq!(direct.problem, SshProblem::Unreachable);
    }

    #[test]
    fn a_question_nobody_could_answer_is_a_sign_in_the_operator_owes() {
        let lines = vec!["al@studio: Permission denied (publickey,password).".to_owned()];
        for refusal in [Refusal::NotAllowed, Refusal::Cancelled, Refusal::TimedOut] {
            let failure = classify(&Ending {
                stderr: &lines,
                refusal: Some(refusal),
                proxied: false,
                prompts_unavailable: false,
                browser_waited: false,
            });
            assert_eq!(failure.problem, SshProblem::SignInNeeded, "{refusal:?}");
        }
        let browser = classify(&Ending {
            stderr: &[],
            refusal: None,
            proxied: false,
            prompts_unavailable: false,
            browser_waited: true,
        });
        assert_eq!(browser.problem, SshProblem::BrowserTimeout);
        let old = classify(&Ending {
            stderr: &["Host key verification failed.".to_owned()],
            refusal: None,
            proxied: false,
            prompts_unavailable: true,
            browser_waited: false,
        });
        assert_eq!(old.problem, SshProblem::OpenSshOld);
    }
}
