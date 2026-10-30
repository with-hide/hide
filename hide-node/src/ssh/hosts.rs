//! The Host entries Add device offers: the concrete aliases of the account's
//! `~/.ssh/config` (and the files it includes) and where each one leads.
//!
//! Hide stores a device as an alias and a name and resolves everything else
//! through the SSH config at connection time (`runtime/devices.rs`), so this
//! module only reads: it never writes a config file, and it never opens a
//! connection. `ssh -G` prints a host's resolved configuration without
//! connecting, which is how the dialog can say `user@host:port` for an alias
//! and recognise two aliases that reach one machine (B50, B51).
//!
//! Every resource is capped (engineering rule 15): configuration files by
//! count, size and include depth, aliases by [`MAX_HOSTS`], each `ssh -G` by
//! [`RESOLVE_DEADLINE`], and the children by [`CONCURRENCY`] at a time, each an
//! owned child that ends with its tree (`hide_platform::process::run_to_end`).
//!
//! Asking where an alias leads is a [`Resolve`], so the listing's own rules
//! (the bound, each alias once, the order) are tested with answers the test
//! decides, and no test races a child against the deadline (issue 728).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use hide_node_link::device::{
    SshAddress as Address, SshHostEntry as Entry, SshHostListing as Listing,
    SshHostProblem as Problem,
};
use hide_platform::process::{Finished, RunFailure, restrict_to_login_environment, run_to_end};

/// Concrete aliases listed; more are reported as truncated.
pub(crate) const MAX_HOSTS: usize = 32;
/// Registered aliases the config does not list that are still resolved, so a
/// device added under a name that is gone from the config can be recognised.
pub(crate) const MAX_EXTRA: usize = 16;
/// How long one `ssh -G` may take.
pub(crate) const RESOLVE_DEADLINE: Duration = Duration::from_secs(3);
/// `ssh -G` children running at once.
const CONCURRENCY: usize = 4;
/// How deep `Include` may nest.
const MAX_INCLUDE_DEPTH: usize = 4;
/// Configuration files read in one listing.
const MAX_CONFIG_FILES: usize = 16;
/// The largest configuration file read; a bigger one is skipped.
const MAX_CONFIG_BYTES: u64 = 256 * 1024;
const MAX_INCLUDE_ENTRIES: usize = 1024;
const MAX_INCLUDE_WORK: usize = 1024 * 1024;
const MAX_INCLUDE_NAME_BYTES: usize = 4096;

/// The concrete aliases a config names, in file order.
#[derive(Debug, Default, Eq, PartialEq)]
pub(crate) struct ConfigAliases {
    pub(crate) aliases: Vec<String>,
    /// The read is incomplete: more than [`MAX_HOSTS`] concrete aliases, a
    /// file past the size, count or Include depth bounds, or an Include that
    /// could not be expanded; the rest are not listed.
    pub(crate) truncated: bool,
    /// A configuration file that exists could not be read (not a file, an
    /// I/O error, not UTF-8): what it names is unknown, which is not "no
    /// Host". A file that does not exist is simply none.
    pub(crate) unreadable: bool,
    /// Include refusal invalidates the entire listing, including extras.
    include_failed: bool,
}

/// Reads `<home>/.ssh/config` and what it includes. A missing config names
/// no host (the dialog then says where to add one, B52); one that could not
/// be read says so, and a read cut at a bound says it is incomplete.
pub(crate) fn read_aliases(home: &Path) -> ConfigAliases {
    let mut reader = Reader {
        home,
        files: 0,
        found: ConfigAliases::default(),
        includes: IncludeBudget::new(Instant::now() + RESOLVE_DEADLINE),
    };
    reader.file(&home.join(".ssh").join("config"), 0);
    reader.found
}

pub(super) struct Reader<'a> {
    pub(super) home: &'a Path,
    pub(super) files: usize,
    pub(super) found: ConfigAliases,
    includes: IncludeBudget,
}

impl Reader<'_> {
    fn file(&mut self, path: &Path, depth: usize) {
        if self.files >= MAX_CONFIG_FILES {
            self.found.truncated = true;
            return;
        }
        let metadata = match std::fs::metadata(path) {
            Ok(metadata) => metadata,
            // ssh skips an Include that names nothing, and no config is no Host.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(_) => {
                self.found.unreadable = true;
                return;
            }
        };
        if !metadata.is_file() {
            self.found.unreadable = true;
            return;
        }
        if metadata.len() > MAX_CONFIG_BYTES {
            self.found.truncated = true;
            return;
        }
        let Ok(contents) = std::fs::read_to_string(path) else {
            self.found.unreadable = true;
            return;
        };
        self.files += 1;
        for line in contents.lines() {
            let Some((keyword, arguments)) = split_line(line) else {
                continue;
            };
            if keyword.eq_ignore_ascii_case("host") {
                for argument in arguments {
                    self.alias(argument);
                }
            } else if keyword.eq_ignore_ascii_case("include") {
                // What an Include past the depth bound names is not read.
                if depth >= MAX_INCLUDE_DEPTH {
                    self.found.truncated = true;
                    continue;
                }
                for argument in arguments {
                    let included = match self.includes.expand(self.home, &argument) {
                        Ok(included) => included,
                        Err(_) => {
                            self.found.aliases.clear();
                            self.found.truncated = true;
                            self.found.include_failed = true;
                            return;
                        }
                    };
                    for included in included {
                        self.file(&included, depth + 1);
                        if self.found.include_failed {
                            return;
                        }
                    }
                }
            }
        }
    }

    /// Adds `pattern` when it names exactly one host: a pattern with a
    /// wildcard or a negation names none, and a leading `-` could be read as
    /// an option by `ssh`.
    fn alias(&mut self, pattern: String) {
        let concrete = !pattern.is_empty()
            && !pattern.starts_with(['!', '-'])
            && !pattern.contains(['*', '?'])
            && !pattern.chars().any(char::is_control);
        if !concrete || self.found.aliases.contains(&pattern) {
            return;
        }
        if self.found.aliases.len() >= MAX_HOSTS {
            self.found.truncated = true;
            return;
        }
        self.found.aliases.push(pattern);
    }
}

/// One shared allowance for every Include in a config read, including
/// directory enumeration and wildcard comparisons before any file is read.
pub(super) struct IncludeBudget {
    deadline: Instant,
    entries: usize,
    work: usize,
}

impl IncludeBudget {
    pub(super) fn new(deadline: Instant) -> Self {
        Self {
            deadline,
            entries: 0,
            work: MAX_INCLUDE_WORK,
        }
    }

    fn consume(&mut self, work: usize) -> Result<(), String> {
        if Instant::now() >= self.deadline {
            return Err("SSH configuration Include deadline expired".to_owned());
        }
        self.work = self
            .work
            .checked_sub(work)
            .ok_or_else(|| "SSH configuration Include work limit exceeded".to_owned())?;
        Ok(())
    }

    /// The files an `Include` argument names: `~` is the account's home, a
    /// relative path is under `~/.ssh`, and `*` or `?` in the last component
    /// match the folder's files, sorted by name as `ssh` reads them.
    pub(super) fn expand(&mut self, home: &Path, argument: &str) -> Result<Vec<PathBuf>, String> {
        self.consume(argument.len())?;
        if argument.len() > MAX_INCLUDE_NAME_BYTES {
            return Err("SSH configuration Include pattern is too long".to_owned());
        }
        let path = if let Some(rest) = argument.strip_prefix("~/") {
            home.join(rest)
        } else if Path::new(argument).is_absolute() {
            PathBuf::from(argument)
        } else {
            home.join(".ssh").join(argument)
        };
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            return Ok(Vec::new());
        };
        if !name.contains(['*', '?']) {
            return Ok(vec![path]);
        }
        let Some(folder) = path.parent() else {
            return Ok(Vec::new());
        };
        let entries = match std::fs::read_dir(folder) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(format!(
                    "SSH configuration Include could not be read: {error}"
                ));
            }
        };
        let mut matched = Vec::new();
        for entry in entries {
            self.consume(1)?;
            self.entries += 1;
            if self.entries > MAX_INCLUDE_ENTRIES {
                return Err("SSH configuration Include directory entry limit exceeded".to_owned());
            }
            let entry = entry
                .map_err(|error| format!("SSH configuration Include could not be read: {error}"))?;
            let filename = entry.file_name();
            if let Some(candidate) = filename.to_str()
                && wildcard_match(name, candidate, self)?
            {
                if matched.len() >= MAX_CONFIG_FILES {
                    return Err("SSH configuration exceeds the Include file limit".to_owned());
                }
                matched.push(entry.path());
            }
        }
        matched.sort();
        Ok(matched)
    }
}

/// `*` (any run) and `?` (any one character) against a file name.
fn wildcard_match(
    pattern: &str,
    candidate: &str,
    budget: &mut IncludeBudget,
) -> Result<bool, String> {
    budget.consume(pattern.len() + candidate.len())?;
    if pattern.len() > MAX_INCLUDE_NAME_BYTES || candidate.len() > MAX_INCLUDE_NAME_BYTES {
        return Err("SSH configuration Include pattern is too long".to_owned());
    }
    let pattern: Vec<char> = pattern.chars().collect();
    let candidate: Vec<char> = candidate.chars().collect();
    let (mut p, mut c, mut star, mut retry) = (0, 0, None, 0);
    while c < candidate.len() {
        budget.consume(1)?;
        if p < pattern.len()
            && (pattern[p] == '?' || (pattern[p] != '*' && pattern[p] == candidate[c]))
        {
            p += 1;
            c += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            p += 1;
            retry = c;
        } else if let Some(last_star) = star {
            retry += 1;
            c = retry;
            p = last_star + 1;
        } else {
            return Ok(false);
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        budget.consume(1)?;
        p += 1;
    }
    Ok(p == pattern.len())
}

/// One config line as `ssh` reads it: a keyword, then its arguments, which
/// are separated by blanks or a single `=`, may be double-quoted, and end at
/// an unquoted `#`. A blank or comment line has none.
pub(super) fn split_line(line: &str) -> Option<(String, Vec<String>)> {
    let line = line.trim_start();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let keyword_end = line
        .find(|c: char| c.is_whitespace() || c == '=')
        .unwrap_or(line.len());
    let keyword = line[..keyword_end].to_owned();
    let mut rest = line[keyword_end..].trim_start();
    rest = rest.strip_prefix('=').unwrap_or(rest).trim_start();
    let mut arguments = Vec::new();
    let mut chars = rest.chars().peekable();
    while let Some(&next) = chars.peek() {
        if next.is_whitespace() {
            chars.next();
            continue;
        }
        if next == '#' {
            break;
        }
        let mut argument = String::new();
        let mut quoted = false;
        while let Some(&c) = chars.peek() {
            if c == '"' {
                quoted = !quoted;
                chars.next();
            } else if c.is_whitespace() && !quoted {
                break;
            } else {
                argument.push(c);
                chars.next();
            }
        }
        arguments.push(argument);
    }
    Some((keyword, arguments))
}

/// The `user`, `hostname` and `port` lines of `ssh -G`'s output.
pub(crate) fn parse_resolved(output: &str) -> Option<Address> {
    let (mut user, mut host, mut port) = (None, None, None);
    for line in output.lines() {
        let Some((key, value)) = line.split_once(' ') else {
            continue;
        };
        let value = value.trim();
        match key {
            "user" if user.is_none() => user = Some(value.to_owned()),
            "hostname" if host.is_none() => host = Some(value.to_owned()),
            "port" if port.is_none() => port = value.parse::<u16>().ok(),
            _ => {}
        }
    }
    let (user, host, port) = (user?, host?, port?);
    let clean = |value: &str| !value.is_empty() && !value.chars().any(char::is_control);
    (clean(&user) && clean(&host)).then_some(Address { user, host, port })
}

/// Asks where one alias leads, given the account's home and the listing's
/// stop flag. The listing calls it from [`CONCURRENCY`] threads at once.
pub type Resolve = Arc<dyn Fn(&Path, &str, &AtomicBool) -> Result<Address, Problem> + Send + Sync>;

/// Asks the `ssh` program at `ssh`, one owned child per alias under
/// [`RESOLVE_DEADLINE`].
pub fn ssh_g(ssh: PathBuf) -> Resolve {
    Arc::new(move |home, alias, stop| {
        answer(run_to_end(
            &mut command(&ssh, home, alias),
            RESOLVE_DEADLINE,
            stop,
        ))
    })
}

/// `ssh -F <config> -G -- <alias>` in a login environment plus `HOME`. `-F`
/// names the account's own config, because `ssh` would otherwise read the
/// passwd entry's home and not the one hided was given.
fn command(ssh: &Path, home: &Path, alias: &str) -> Command {
    let mut command = Command::new(ssh);
    restrict_to_login_environment(&mut command);
    command
        .env("HOME", home)
        .arg("-F")
        .arg(home.join(".ssh").join("config"))
        .args(["-G", "--"])
        .arg(alias);
    command
}

/// What one run of `ssh -G` says about its alias.
fn answer(run: Result<Finished, RunFailure>) -> Result<Address, Problem> {
    match run {
        Ok(finished) if finished.succeeded() => {
            parse_resolved(&finished.stdout).ok_or(Problem::SshFailed)
        }
        Ok(_) => Err(Problem::SshFailed),
        Err(RunFailure::Start(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            Err(Problem::SshMissing)
        }
        Err(RunFailure::TimedOut) => Err(Problem::TimedOut),
        Err(_) => Err(Problem::SshFailed),
    }
}

/// Resolves every alias, [`CONCURRENCY`] at a time, in the order given. The
/// threads end with the call, and once `stop` is raised no further alias is
/// asked.
pub(crate) fn resolve_all(
    aliases: &[String],
    stop: &AtomicBool,
    resolve: impl Fn(&str) -> Result<Address, Problem> + Sync,
) -> Vec<Result<Address, Problem>> {
    let next = AtomicUsize::new(0);
    let results: Vec<std::sync::Mutex<Option<Result<Address, Problem>>>> = aliases
        .iter()
        .map(|_| std::sync::Mutex::new(None))
        .collect();
    std::thread::scope(|scope| {
        for _ in 0..CONCURRENCY.min(aliases.len()) {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::SeqCst);
                    let Some(alias) = aliases.get(index) else {
                        return;
                    };
                    let outcome = if stop.load(Ordering::SeqCst) {
                        Err(Problem::SshFailed)
                    } else {
                        resolve(alias)
                    };
                    if let Ok(mut slot) = results[index].lock() {
                        *slot = Some(outcome);
                    }
                }
            });
        }
    });
    results
        .into_iter()
        .map(|slot| {
            slot.into_inner()
                .ok()
                .flatten()
                .unwrap_or(Err(Problem::SshFailed))
        })
        .collect()
}

/// Reads the config and resolves its aliases and `registered` (the aliases of
/// the devices already added), as one bounded run.
pub(crate) fn list(
    resolve: &Resolve,
    home: &Path,
    registered: &[String],
    stop: &AtomicBool,
) -> Listing {
    let config = read_aliases(home);
    if config.include_failed {
        return Listing {
            entries: Vec::new(),
            registered: Default::default(),
            truncated: true,
            unreadable: config.unreadable,
        };
    }
    let mut wanted = config.aliases.clone();
    let extra: Vec<String> = registered
        .iter()
        .filter(|alias| !wanted.contains(alias) && !alias.starts_with('-'))
        .take(MAX_EXTRA)
        .cloned()
        .collect();
    wanted.extend(extra);
    let mut outcomes = resolve_all(&wanted, stop, |alias| resolve(home, alias, stop)).into_iter();
    let entries = config
        .aliases
        .iter()
        .map(|alias| Entry {
            alias: alias.clone(),
            address: outcomes.next().unwrap_or(Err(Problem::SshFailed)),
        })
        .collect::<Vec<_>>();
    let extra_resolved = wanted[config.aliases.len()..]
        .iter()
        .zip(outcomes)
        .filter_map(|(alias, outcome)| Some((alias.clone(), outcome.ok()?)));
    let registered = entries
        .iter()
        .filter(|entry| registered.contains(&entry.alias))
        .filter_map(|entry| Some((entry.alias.clone(), entry.address.clone().ok()?)))
        .chain(extra_resolved)
        .collect();
    Listing {
        entries,
        registered,
        truncated: config.truncated,
        unreadable: config.unreadable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, contents: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    fn aliases(config: &str) -> ConfigAliases {
        let home = tempfile::tempdir().unwrap();
        write(&home.path().join(".ssh/config"), config);
        read_aliases(home.path())
    }

    #[test]
    fn a_host_line_yields_each_concrete_alias_and_no_pattern() {
        let found = aliases(
            "Host studio mini\n  HostName 10.0.0.2\n\
             Host *\n  ServerAliveInterval 30\n\
             Host *.corp !bad build-?\n\
             host=quoted \"two words\"\n\
             Host trailing # a comment\n\
             Host -oProxyCommand=x\n\
             Host studio\n",
        );
        assert_eq!(
            found.aliases,
            ["studio", "mini", "quoted", "two words", "trailing"]
        );
        assert!(!found.truncated);
    }

    #[test]
    fn include_reads_relative_tilde_and_wildcard_files_in_order_within_bounds() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path();
        write(
            &root.join(".ssh/config"),
            "Include conf.d/*.conf\nHost first\nInclude ~/extra\nInclude missing\n",
        );
        write(&root.join(".ssh/conf.d/b.conf"), "Host from-b\n");
        write(&root.join(".ssh/conf.d/a.conf"), "Host from-a\n");
        write(&root.join(".ssh/conf.d/skip.txt"), "Host skipped\n");
        write(&root.join("extra"), "Host from-extra\nInclude extra\n");
        let found = read_aliases(root);
        assert_eq!(found.aliases, ["from-a", "from-b", "first", "from-extra"]);
        // `Include extra` from ~/extra names ~/.ssh/extra, which is not
        // there: ssh skips it, so the read is complete.
        assert!(!found.truncated);
        assert!(!found.unreadable);
    }

    #[test]
    fn a_missing_config_is_none_and_a_complete_one_is_complete() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(read_aliases(home.path()), ConfigAliases::default());
        let found = aliases("Host one\nInclude missing\n");
        assert_eq!(found.aliases, ["one"]);
        assert!(!found.truncated && !found.unreadable);
    }

    #[test]
    fn a_config_that_cannot_be_read_is_unreadable_not_empty() {
        let home = tempfile::tempdir().unwrap();
        let config = home.path().join(".ssh/config");
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        std::fs::write(&config, b"Host one\n\xff\xfe not text\n").unwrap();
        let found = read_aliases(home.path());
        assert!(found.unreadable);
        assert!(found.aliases.is_empty());
        // A folder where the config should be cannot be read either.
        std::fs::remove_file(&config).unwrap();
        std::fs::create_dir(&config).unwrap();
        assert!(read_aliases(home.path()).unreadable);
    }

    #[test]
    fn a_read_cut_at_a_bound_is_incomplete() {
        // Larger than a config file may be.
        let big = format!("Host big\n{}", "# padding\n".repeat(40_000));
        let oversize = aliases(&big);
        assert!(oversize.truncated && !oversize.unreadable);
        assert!(oversize.aliases.is_empty());
        // Includes nested past the depth bound.
        let home = tempfile::tempdir().unwrap();
        let root = home.path();
        write(&root.join(".ssh/config"), "Host top\nInclude d1\n");
        for level in 1..=MAX_INCLUDE_DEPTH + 1 {
            write(
                &root.join(format!(".ssh/d{level}")),
                &format!("Host d{level}\nInclude d{}\n", level + 1),
            );
        }
        let deep = read_aliases(root);
        assert!(deep.truncated);
        assert_eq!(deep.aliases.len(), MAX_INCLUDE_DEPTH + 1);
        // More files than one read takes.
        let home = tempfile::tempdir().unwrap();
        let root = home.path();
        let includes: String = (0..MAX_CONFIG_FILES)
            .map(|index| format!("Include f{index}\n"))
            .collect();
        write(&root.join(".ssh/config"), &includes);
        for index in 0..MAX_CONFIG_FILES {
            write(
                &root.join(format!(".ssh/f{index}")),
                &format!("Host f{index}\n"),
            );
        }
        let many = read_aliases(root);
        assert!(many.truncated);
        assert_eq!(many.aliases.len(), MAX_CONFIG_FILES - 1);
    }

    #[test]
    fn more_than_the_cap_is_truncated_not_listed() {
        let config: String = (0..MAX_HOSTS + 5)
            .map(|index| format!("Host h{index}\n"))
            .collect();
        let found = aliases(&config);
        assert_eq!(found.aliases.len(), MAX_HOSTS);
        assert!(found.truncated);
        assert_eq!(found.aliases[MAX_HOSTS - 1], format!("h{}", MAX_HOSTS - 1));
    }

    #[test]
    fn ssh_g_output_gives_the_address_and_nothing_less_does() {
        let resolved = parse_resolved(
            "host studio\nuser grab\nhostname 10.0.0.2\nport 2222\nidentityfile ~/.ssh/id\n",
        )
        .unwrap();
        assert_eq!(resolved.display(), "grab@10.0.0.2:2222");
        let v6 = parse_resolved("user grab\nhostname ::1\nport 22\n").unwrap();
        assert_eq!(v6.display(), "grab@[::1]:22");
        assert_eq!(parse_resolved("user grab\nhostname h\n"), None);
        assert_eq!(parse_resolved("user grab\nhostname h\nport nope\n"), None);
        assert_eq!(parse_resolved(""), None);
    }

    #[test]
    fn a_wildcard_matches_names_the_way_ssh_reads_include() {
        let mut budget = IncludeBudget::new(Instant::now() + RESOLVE_DEADLINE);
        assert!(wildcard_match("*.conf", "a.conf", &mut budget).unwrap());
        assert!(wildcard_match("a?.conf", "ab.conf", &mut budget).unwrap());
        assert!(!wildcard_match("*.conf", "a.txt", &mut budget).unwrap());
        assert!(!wildcard_match("a?.conf", "a.conf", &mut budget).unwrap());
        assert!(wildcard_match("*a?*b", "a한b", &mut budget).unwrap());
        assert!(wildcard_match("*a", "*ba", &mut budget).unwrap());
        assert!(wildcard_match("*한?", "*가한글", &mut budget).unwrap());
    }

    #[test]
    fn refused_include_never_resolves_registered_routes_from_a_partial_config() {
        let home = tempfile::tempdir().unwrap();
        write(
            &home.path().join(".ssh/config"),
            "Host first\nInclude fragments/*\n",
        );
        for index in 0..=MAX_CONFIG_FILES {
            write(
                &home.path().join(format!(".ssh/fragments/{index}")),
                "Host extra\n",
            );
        }
        let resolve: Resolve = Arc::new(|_, _, _| panic!("refused Include must not resolve SSH"));
        let listing = list(
            &resolve,
            home.path(),
            &["registered".into()],
            &AtomicBool::new(false),
        );
        assert!(listing.entries.is_empty());
        assert!(listing.registered.is_empty());
        assert!(listing.truncated);
    }

    #[test]
    fn include_deadline_and_match_work_are_finite_before_enumeration() {
        let home = tempfile::tempdir().unwrap();
        let mut expired = IncludeBudget::new(Instant::now());
        assert!(
            expired
                .expand(home.path(), "absent/*")
                .unwrap_err()
                .contains("deadline")
        );
        let mut budget = IncludeBudget::new(Instant::now() + RESOLVE_DEADLINE);
        budget.work = 2;
        assert!(
            wildcard_match("*z", "aaaa", &mut budget)
                .unwrap_err()
                .contains("work limit")
        );
    }

    /// Every alias is asked once and answered in the order given, and at most
    /// [`CONCURRENCY`] are asked at once: a call runs on the worker that made
    /// it, so the workers that called bound how many run together.
    #[test]
    fn resolving_asks_each_alias_once_from_a_bounded_number_of_workers_in_order() {
        let asked = std::sync::Mutex::new(Vec::new());
        let aliases: Vec<String> = (0..12).map(|index| format!("h{index}")).collect();
        let stop = AtomicBool::new(false);
        let resolved = resolve_all(&aliases, &stop, |alias| {
            asked
                .lock()
                .unwrap()
                .push((std::thread::current().id(), alias.to_owned()));
            Ok(Address {
                user: "u".to_owned(),
                host: alias.to_owned(),
                port: 22,
            })
        });
        assert_eq!(resolved.len(), aliases.len());
        for (alias, outcome) in aliases.iter().zip(&resolved) {
            assert_eq!(outcome.as_ref().unwrap().host, *alias, "in the order asked");
        }
        let asked = asked.into_inner().unwrap();
        let mut names: Vec<&str> = asked.iter().map(|(_, alias)| alias.as_str()).collect();
        names.sort_unstable();
        let mut want: Vec<&str> = aliases.iter().map(String::as_str).collect();
        want.sort_unstable();
        assert_eq!(names, want, "each alias was asked once");
        let workers: std::collections::HashSet<_> =
            asked.iter().map(|(worker, _)| *worker).collect();
        assert!(workers.len() <= CONCURRENCY, "{} workers", workers.len());
        assert!(!workers.contains(&std::thread::current().id()));
    }

    #[test]
    fn a_raised_stop_asks_no_alias() {
        let aliases: Vec<String> = (0..6).map(|index| format!("h{index}")).collect();
        let stop = AtomicBool::new(true);
        let resolved = resolve_all(&aliases, &stop, |alias| panic!("{alias} was asked"));
        assert_eq!(resolved, vec![Err(Problem::SshFailed); aliases.len()]);
    }

    #[test]
    fn ssh_g_names_the_accounts_own_config_and_home() {
        let home = Path::new("/account");
        let command = command(Path::new("/usr/bin/ssh"), home, "studio");
        let config = home.join(".ssh").join("config");
        assert_eq!(command.get_program(), "/usr/bin/ssh");
        assert_eq!(
            command
                .get_args()
                .map(|arg| arg.to_str().unwrap())
                .collect::<Vec<_>>(),
            ["-F", config.to_str().unwrap(), "-G", "--", "studio"]
        );
        assert!(
            command
                .get_envs()
                .any(|(key, value)| key == "HOME" && value == Some(home.as_os_str())),
            "HOME is the account's"
        );
    }

    #[test]
    fn each_way_ssh_g_can_end_has_one_answer() {
        let finished = |code, stdout: &str| {
            Ok(Finished {
                code,
                stdout: stdout.to_owned(),
                stderr: String::new(),
            })
        };
        let start = |kind| Err(RunFailure::Start(std::io::Error::from(kind)));
        assert_eq!(
            answer(finished(Some(0), "user u\nhostname h\nport 22\n")),
            Ok(Address {
                user: "u".to_owned(),
                host: "h".to_owned(),
                port: 22,
            })
        );
        assert_eq!(
            answer(finished(Some(0), "user u\n")),
            Err(Problem::SshFailed)
        );
        assert_eq!(
            answer(finished(Some(255), "user u\nhostname h\nport 22\n")),
            Err(Problem::SshFailed)
        );
        assert_eq!(answer(finished(None, "")), Err(Problem::SshFailed));
        assert_eq!(
            answer(start(std::io::ErrorKind::NotFound)),
            Err(Problem::SshMissing)
        );
        assert_eq!(
            answer(start(std::io::ErrorKind::PermissionDenied)),
            Err(Problem::SshFailed)
        );
        assert_eq!(answer(Err(RunFailure::TimedOut)), Err(Problem::TimedOut));
        assert_eq!(answer(Err(RunFailure::Stopped)), Err(Problem::SshFailed));
    }
}
