//! Project guardrails: the four protections an operator turns on per project
//! in Project Settings > Working style, refused at an agent's pre-tool hook
//! (PRD agent-guardrails).
//!
//! The core publishes one policy file per machine, `~/.hide/agent-hooks/
//! guardrails.json` ([`policy_path`]), naming each project of that
//! machine with a protection on: its main worktree root, the base branch the
//! operator chose (none for automatic, which is `origin/HEAD`, or with no
//! remote the main worktree's branch) and its
//! [`Rules`]. The hook reads that file and the repository's own Git files and
//! decides here, on the machine the agent runs on; no tool call asks the
//! daemon (D-11).
//!
//! A protection is a guardrail, not a security boundary (D-18): a write a
//! shell makes through `sed -i` or a script is not seen, and only the shell
//! words this module recognises are refused.
//!
//! What fails, and how (D-27):
//!
//! - the policy file is missing, unreadable or not this version: the call
//!   runs and [`record_unreadable`] writes why, one line per ten minutes; the core writes the file, empty when no protection is on, at
//!   every start and device connection (D-28), so a missing one is a fault;
//! - a fact a protection that is on needs cannot be read (the checkout's
//!   repository, its branch, the base branch, whether it has uncommitted
//!   changes): the call is refused and the reason says so;
//! - a panic, or a payload that does not arrive in time, lets the call run
//!   and records a throttled `guard.failed` line ([`record_failure`]); a
//!   hook the runtime kills at its own timeout is that runtime's default and
//!   leaves no line, since the process is gone.

use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

mod facts;
mod git;
pub mod payload;
#[cfg(test)]
mod tests;

use facts::{Facts, Located};

/// The file's version. A file of another version is read as unreadable, so
/// an older helper never applies a newer core's rules by a guess.
pub const POLICY_VERSION: u32 = 1;

/// The most paths one project protects, and the longest one in characters
/// (D-17).
pub const PATH_LIMIT: usize = 32;
pub const PATH_LENGTH_LIMIT: usize = 200;

/// The largest policy file read: 256 projects of 32 paths of 200 four-byte
/// characters fit.
const POLICY_SIZE_LIMIT: u64 = 8 * 1024 * 1024;

/// The paths a project protects until the operator changes the list (B15).
pub const DEFAULT_PATHS: [&str; 5] = [".env", ".env.*", "*.pem", "*.key", "~/.ssh/"];

/// What `.env.*` leaves out: the example files a repository commits (B15).
const ENV_EXAMPLES: [&str; 2] = [".env.example", ".env.sample"];

/// The policy file's place under `home`, the machine's account home.
pub fn policy_path(home: &Path) -> PathBuf {
    home.join(".hide")
        .join("agent-hooks")
        .join("guardrails.json")
}

fn default_paths() -> Vec<String> {
    DEFAULT_PATHS
        .iter()
        .map(|path| (*path).to_owned())
        .collect()
}

/// One project's protections, as Project Settings stores them and the policy
/// file carries them. Every switch starts off (D-13). A field a newer build
/// added is kept in [`Rules::unknown`], so a record that build wrote loses
/// nothing when this one saves it again; the policy file's own version keeps
/// such a field from ever being judged by a guess.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Rules {
    /// 작업 폴더 지키기: the original checkout keeps its branch, and an agent
    /// writes only in its own checkout of the repository (D-14).
    #[serde(default)]
    pub worktree_folder: bool,
    /// 기준 브랜치 지키기: no writes or commits where the base branch is checked
    /// out, and no push to it (D-15).
    #[serde(default)]
    pub base_branch: bool,
    /// Under `base_branch`: no `gh pr merge` either (D-15).
    #[serde(default)]
    pub agent_merge: bool,
    /// 위험한 Git 명령 막기 (D-16).
    #[serde(default)]
    pub git_commands: bool,
    /// 파일 지키기 over [`Rules::paths`] (D-17).
    #[serde(default)]
    pub files: bool,
    #[serde(default = "default_paths")]
    pub paths: Vec<String>,
    #[serde(flatten, default)]
    pub unknown: serde_json::Map<String, serde_json::Value>,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            worktree_folder: false,
            base_branch: false,
            agent_merge: false,
            git_commands: false,
            files: false,
            paths: default_paths(),
            unknown: serde_json::Map::new(),
        }
    }
}

/// Why a path list cannot be saved; the code the settings write answers with.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathRefusal {
    TooMany,
    TooLong,
    Duplicate,
    Invalid,
}

impl PathRefusal {
    pub fn code(self) -> &'static str {
        match self {
            Self::TooMany => "project_settings_paths_limit",
            Self::TooLong => "project_settings_path_too_long",
            Self::Duplicate => "project_settings_path_duplicate",
            Self::Invalid => "project_settings_path_invalid",
        }
    }
}

impl Rules {
    /// Whether any protection is on. The merge option counts only under the
    /// base branch protection, where the screen shows it.
    pub fn any(&self) -> bool {
        self.worktree_folder || self.base_branch || self.git_commands || self.files
    }

    /// Whether the list can be saved: at most [`PATH_LIMIT`] entries, each a
    /// gitignore pattern relative to the project or a `~/` path of at most
    /// [`PATH_LENGTH_LIMIT`] characters, none twice (B16).
    pub fn validate(&self) -> Result<(), PathRefusal> {
        if self.paths.len() > PATH_LIMIT {
            return Err(PathRefusal::TooMany);
        }
        for (index, path) in self.paths.iter().enumerate() {
            if path.chars().count() > PATH_LENGTH_LIMIT {
                return Err(PathRefusal::TooLong);
            }
            validate_path(path)?;
            if self.paths[..index].contains(path) {
                return Err(PathRefusal::Duplicate);
            }
        }
        Ok(())
    }
}

fn validate_path(path: &str) -> Result<(), PathRefusal> {
    if path.trim().is_empty() || path != path.trim() || path.chars().any(char::is_control) {
        return Err(PathRefusal::Invalid);
    }
    let pattern = match path.strip_prefix('~') {
        // `~/x` is a path under the home folder; `~user` and a bare `~` are not.
        Some(rest) => {
            let rest = rest.strip_prefix('/').ok_or(PathRefusal::Invalid)?;
            if rest.is_empty() {
                return Err(PathRefusal::Invalid);
            }
            format!("/{rest}")
        }
        None => path.to_owned(),
    };
    // A comment or a negation alone protects nothing, and a pattern the
    // gitignore reader refuses (a backwards range, an unclosed `{`) matches
    // nothing.
    if pattern.starts_with('#') || pattern.starts_with('!') {
        return Err(PathRefusal::Invalid);
    }
    let mut builder = ignore::gitignore::GitignoreBuilder::new("/");
    builder
        .add_line(None, &pattern)
        .map_err(|_| PathRefusal::Invalid)?;
    builder.build().map_err(|_| PathRefusal::Invalid)?;
    Ok(())
}

/// The policy file: every project of this machine with a protection on.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub version: u32,
    pub projects: Vec<ProjectPolicy>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectPolicy {
    /// The project's main worktree root, as this machine's catalog spells it.
    pub root: String,
    /// The base branch the operator chose; `None` reads `origin/HEAD`, or in
    /// a repository with no remote its main worktree's branch.
    #[serde(default)]
    pub base: Option<String>,
    pub rules: Rules,
}

impl Policy {
    pub fn new(projects: Vec<ProjectPolicy>) -> Self {
        Self {
            version: POLICY_VERSION,
            projects,
        }
    }
}

/// Why the policy file was not read: the call runs, and this is recorded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyFailure {
    Missing,
    Unreadable,
    Corrupt,
    Version,
}

impl PolicyFailure {
    pub fn code(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Unreadable => "unreadable",
            Self::Corrupt => "corrupt",
            Self::Version => "version",
        }
    }
}

/// Reads the policy file under `home`.
pub fn read_policy(home: &Path) -> Result<Policy, PolicyFailure> {
    use std::io::Read;
    let file = match hide_platform::fs::open_regular(&policy_path(home)) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(PolicyFailure::Missing);
        }
        Err(_) => return Err(PolicyFailure::Unreadable),
    };
    let mut bytes = Vec::new();
    file.take(POLICY_SIZE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| PolicyFailure::Unreadable)?;
    if bytes.len() as u64 > POLICY_SIZE_LIMIT {
        return Err(PolicyFailure::Corrupt);
    }
    #[derive(Deserialize)]
    struct Versioned {
        version: u32,
    }
    let versioned: Versioned =
        serde_json::from_slice(&bytes).map_err(|_| PolicyFailure::Corrupt)?;
    if versioned.version != POLICY_VERSION {
        return Err(PolicyFailure::Version);
    }
    serde_json::from_slice(&bytes).map_err(|_| PolicyFailure::Corrupt)
}

/// Replaces the policy file under `home` in one step, private to the account.
/// Only the node does this, for the core (`Call::GuardrailPolicy`).
pub fn write_policy(home: &Path, policy: &Policy) -> std::io::Result<()> {
    let path = policy_path(home);
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("policy parent unavailable"))?;
    hide_platform::fs::private::create_dir_all(parent)?;
    let bytes = serde_json::to_vec(policy).map_err(std::io::Error::other)?;
    hide_platform::fs::atomic::write_file(&path, &bytes, hide_platform::fs::Access::Private)?;
    Ok(())
}

/// What a tool call does, as its runtime's payload says.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    Shell(String),
    Write(Vec<String>),
    Read(Vec<String>),
    /// A file write whose target was cut off with the rest of a payload too
    /// large to read whole (agy names the call after its content).
    UnreadWrite,
}

/// The protection a refusal names, and the code its log line carries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Rule {
    WorktreeFolder,
    BaseBranch,
    AgentMerge,
    GitCommands,
    Files,
}

impl Rule {
    pub fn code(self) -> &'static str {
        match self {
            Self::WorktreeFolder => "worktree_folder",
            Self::BaseBranch => "base_branch",
            Self::AgentMerge => "agent_merge",
            Self::GitCommands => "git_commands",
            Self::Files => "files",
        }
    }

    /// The protection's name as the English screen shows it.
    fn title(self) -> &'static str {
        match self {
            Self::WorktreeFolder => "Guard working folders",
            Self::BaseBranch => "Guard the base branch",
            Self::AgentMerge => "Block agents from merging pull requests",
            Self::GitCommands => "Block risky Git commands",
            Self::Files => "Guard files",
        }
    }

    fn next_step(self) -> &'static str {
        match self {
            Self::WorktreeFolder => {
                "Do this work in your own worktree (hide agent spawn, or git worktree add), or ask the operator to turn this protection off in Project Settings > Working style."
            }
            Self::BaseBranch => {
                "Make the change on another branch in a worktree and open a pull request, or ask the operator to turn this protection off in Project Settings > Working style."
            }
            Self::AgentMerge => {
                "Leave the merge to a reviewer, or ask the operator to turn this option off in Project Settings > Working style."
            }
            Self::GitCommands => {
                "Use the safe form named above, or ask the operator to run it or to turn this protection off in Project Settings > Working style."
            }
            Self::Files => {
                "Ask the operator for what you need from it, or ask them to change the protected list in Project Settings > Working style."
            }
        }
    }
}

/// Why one call is refused: the rule, and the sentence that says what the
/// call did. A fact that could not be read names that instead.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Refusal {
    pub rule: Rule,
    pub why: String,
    pub unread: bool,
}

impl Refusal {
    fn new(rule: Rule, why: impl Into<String>) -> Self {
        Self {
            rule,
            why: why.into(),
            unread: false,
        }
    }

    fn unread(rule: Rule, fact: &str) -> Self {
        Self {
            rule,
            why: format!("Hide could not read {fact}, which this protection needs"),
            unread: true,
        }
    }

    /// The base branch is the operator's to name when the repository names
    /// none (for Auto, no `origin/HEAD`, and a remote that keeps the main
    /// worktree's branch from counting), so the reason says where.
    fn base_unknown() -> Self {
        Self::unread(Rule::BaseBranch, "the base branch")
    }

    /// The reason the agent reads: English, the protection's name, what the
    /// call did and what to do instead (D-19).
    pub fn reason(&self) -> String {
        let next = match (self.unread, self.rule) {
            (true, Rule::BaseBranch) => {
                "Ask the operator to choose the base branch in Project Settings > Working style."
            }
            (true, _) => "Try again in a moment, or ask the operator.",
            (false, rule) => rule.next_step(),
        };
        format!(
            "Not run: the \"{}\" protection is on for this project in Hide. {}. {}",
            self.rule.title(),
            self.why,
            next
        )
    }
}

/// The decision for one call.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Outcome {
    Allow,
    Refuse(Refusal),
    PolicyUnreadable(PolicyFailure),
}

/// Whether the checkout at a path has uncommitted changes, untracked files
/// counted when the flag says so; `None` when Git could not say. Asked only
/// for a command that would discard them.
pub type Dirty<'a> = &'a dyn Fn(&Path, bool) -> Option<bool>;

/// The decision for `action`, run in `cwd`, under the policy file in `home`.
#[cfg(test)]
pub fn decide(home: &Path, cwd: &Path, action: &Action, dirty: Dirty<'_>) -> Outcome {
    let policy = match read_policy(home) {
        Ok(policy) => policy,
        Err(failure) => return Outcome::PolicyUnreadable(failure),
    };
    decide_with(&policy, home, cwd, action, dirty)
}

/// [`decide`] with the policy already read.
pub fn decide_with(
    policy: &Policy,
    home: &Path,
    cwd: &Path,
    action: &Action,
    dirty: Dirty<'_>,
) -> Outcome {
    if !policy.projects.iter().any(|project| project.rules.any()) {
        return Outcome::Allow;
    }
    let facts = Facts::new(policy, home);
    let cwd = absolute(home, cwd, cwd);
    let refusal = match action {
        Action::Shell(command) => git::shell(&facts, &cwd, command, dirty),
        Action::Write(paths) => paths
            .iter()
            .find_map(|path| write(&facts, &cwd, &absolute(home, &cwd, Path::new(path)))),
        Action::Read(paths) => paths
            .iter()
            .find_map(|path| protected(&facts, &cwd, &absolute(home, &cwd, Path::new(path)))),
        Action::UnreadWrite => unread_write(&facts, &cwd),
    };
    if facts.word_limit.get() {
        record_word_limit(home);
    }
    refusal.map_or(Outcome::Allow, Outcome::Refuse)
}

/// A write in `cwd` whose target Hide could not read: refused in a project
/// where a protection judges writes, since any target may be one it guards.
fn unread_write(facts: &Facts<'_>, cwd: &Path) -> Option<Refusal> {
    let located = facts.locate(cwd);
    let rules = &located.project(facts)?.policy.rules;
    let rule = if rules.files {
        Rule::Files
    } else if rules.worktree_folder {
        Rule::WorktreeFolder
    } else if rules.base_branch {
        Rule::BaseBranch
    } else {
        return None;
    };
    Some(Refusal::unread(
        rule,
        "the file this call writes, cut off with the rest of a call too large to read",
    ))
}

/// A file tool's write of `target` (B7, B8, B15).
fn write(facts: &Facts<'_>, cwd: &Path, target: &Path) -> Option<Refusal> {
    let target = &facts::canonical_prefix(target);
    let located = facts.locate(target);
    if let Some(project) = located.project(facts) {
        let rules = &project.policy.rules;
        if let Located::Unreadable = located.place {
            let rule = if rules.worktree_folder {
                Rule::WorktreeFolder
            } else if rules.base_branch {
                Rule::BaseBranch
            } else {
                Rule::Files
            };
            if rule != Rule::Files || rules.files {
                return Some(Refusal::unread(
                    rule,
                    "the repository of the file it writes",
                ));
            }
        }
        if let Located::In(checkout) = &located.place {
            if rules.worktree_folder {
                match facts.locate(cwd).place {
                    Located::In(own)
                        if own.main_root == checkout.main_root && own.root != checkout.root =>
                    {
                        return Some(Refusal::new(
                            Rule::WorktreeFolder,
                            "This call writes into another checkout of the same repository",
                        ));
                    }
                    Located::Unreadable => {
                        return Some(Refusal::unread(
                            Rule::WorktreeFolder,
                            "the checkout this agent works in",
                        ));
                    }
                    _ => {}
                }
            }
            if rules.base_branch {
                match facts.base(project.policy, checkout) {
                    Some(base) if checkout.branch.as_deref() == Some(base.as_str()) => {
                        return Some(Refusal::new(
                            Rule::BaseBranch,
                            format!(
                                "This call writes in a checkout that has the base branch {base} checked out"
                            ),
                        ));
                    }
                    Some(_) => {}
                    None => return Some(Refusal::base_unknown()),
                }
            }
        }
    }
    protected(facts, cwd, target)
}

/// The files protection for one path an agent names (B15).
fn protected(facts: &Facts<'_>, cwd: &Path, target: &Path) -> Option<Refusal> {
    let refusal = || {
        Refusal::new(
            Rule::Files,
            "This call reads or changes a path on the project's protected list",
        )
    };
    let target = &facts::canonical_prefix(target);
    let located = facts.locate(target);
    let own = facts.locate(cwd);
    let mut home_rules: Vec<&ProjectPolicy> = Vec::new();
    for place in [&located, &own] {
        if let Some(project) = place.project(facts)
            && project.policy.rules.files
            && !home_rules.contains(&project.policy)
        {
            home_rules.push(project.policy);
        }
    }
    // A path is matched against its checkout's root; one in a repository
    // that cannot be read, against its project's root, which is known.
    let checkout_root = match &located.place {
        Located::In(checkout) => Some(checkout.root.as_path()),
        Located::Unreadable => facts.project_root(&located),
        Located::Outside => None,
    };
    if let (Some(project), Some(root)) = (located.project(facts), checkout_root)
        && project.policy.rules.files
        && let Ok(relative) = target.strip_prefix(root)
        && matches(
            facts,
            root,
            &project.policy.rules.paths,
            relative,
            target,
            false,
        )
    {
        return Some(refusal());
    }
    if let Ok(relative) = target.strip_prefix(&facts.home) {
        for project in home_rules {
            if matches(
                facts,
                &facts.home,
                &project.rules.paths,
                relative,
                target,
                true,
            ) {
                return Some(refusal());
            }
        }
    }
    None
}

/// Whether `relative`, below `root`, is matched by the project's relative
/// patterns (`home` false) or its `~/` patterns (`home` true), with case or
/// without as the folder compares names (B24).
fn matches(
    facts: &Facts<'_>,
    root: &Path,
    paths: &[String],
    relative: &Path,
    target: &Path,
    home: bool,
) -> bool {
    let mut builder = ignore::gitignore::GitignoreBuilder::new(root);
    let _ = builder.case_insensitive(!facts.case_sensitive(root));
    let mut any = false;
    for path in paths {
        let line = match (path.strip_prefix("~/"), home) {
            (Some(rest), true) => format!("/{rest}"),
            (None, false) => path.clone(),
            _ => continue,
        };
        if builder.add_line(None, &line).is_err() {
            continue;
        }
        any = true;
        // An example the operator listed stays protected.
        if !home && path == ".env.*" {
            for example in ENV_EXAMPLES {
                if !paths.iter().any(|listed| listed == example) {
                    let _ = builder.add_line(None, &format!("!{example}"));
                }
            }
        }
    }
    if !any || relative.as_os_str().is_empty() {
        return false;
    }
    let Ok(matcher) = builder.build() else {
        return false;
    };
    matcher
        .matched_path_or_any_parents(relative, target.is_dir())
        .is_ignore()
}

/// `path` made absolute against `cwd` (`~` and `~/x` against `home`) and
/// with `.` and `..` taken out by name.
pub(crate) fn absolute(home: &Path, cwd: &Path, path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    let joined = if text == "~" {
        home.to_path_buf()
    } else if let Some(rest) = text.strip_prefix("~/") {
        home.join(rest)
    } else if let Some(native) = cfg!(windows)
        .then(|| hide_platform::path::shell_drive_path(&text))
        .flatten()
    {
        // Git Bash, the shell Claude Code runs on Windows, names a drive
        // `/c/...`.
        PathBuf::from(native)
    } else if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    let mut normal = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normal.pop();
            }
            other => normal.push(other.as_os_str()),
        }
    }
    normal
}

/// How long `git status` may take to say whether a checkout has changes,
/// inside the hook entry's eight seconds.
const STATUS_BUDGET: Duration = Duration::from_secs(2);

/// Whether the checkout at `root` has uncommitted changes, from `git
/// status`: the one Git process the guardrails start, and only for a command
/// that would discard changes (D-16). Untracked files count only for `git
/// clean`, the one such command that removes them. It takes no lock a
/// concurrent `git add` would meet. `None` when Git did not answer in time
/// or at all.
///
/// The repository is the agent's to configure and the hook runs with the
/// operator's rights, so Git runs no fsmonitor command, and no `GIT_*`
/// variable the hook inherited points it at another repository or config.
pub fn git_dirty(root: &Path, untracked: bool) -> Option<bool> {
    let mut command = Command::new("git");
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
    command
        .args(["-c", "core.fsmonitor=false", "--no-optional-locks", "-C"])
        .arg(root)
        .args([
            "status",
            "--porcelain",
            if untracked {
                "--untracked-files=normal"
            } else {
                "--untracked-files=no"
            },
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = hide_platform::process::OwnedChild::spawn(&mut command).ok()?;
    match child.capture_until(Instant::now() + STATUS_BUDGET, 4096) {
        Ok(output) if output.status.success() => Some(!output.stdout.is_empty()),
        // More than the bound is certainly something.
        Err(failure)
            if matches!(
                failure.kind,
                hide_platform::process::CaptureFailureKind::OutputLimit { .. }
            ) =>
        {
            Some(true)
        }
        _ => None,
    }
}

/// One refusal, as one structured line in the guard's log: the protection,
/// the runtime and the pane. The command, its paths and file contents are
/// never written (B19). The refusal stands when the line cannot be written.
pub fn record_refusal(home: &Path, runtime: &str, pane: Option<&str>, refusal: &Refusal) {
    let line = serde_json::json!({
        "component": "guardrails",
        "kind": "guard.refused",
        "rule": refusal.rule.code(),
        "unread": refusal.unread,
        "runtime": runtime,
        "pane": pane,
        "at_unix_ms": crate::spawn_guard::now_ms(),
    })
    .to_string();
    let _ = crate::spawn_guard::append_line(&crate::spawn_guard::log_path(home), &line);
}

/// A shell call named more words than the files protection checks, so the
/// rest went unchecked: one line per ten minutes, to the log only.
fn record_word_limit(home: &Path) {
    if crate::delivery::claim(home, "guardrails_words") {
        let line = serde_json::json!({
            "component": "guardrails",
            "kind": "guard.word_limit",
            "limit": git::WORD_LIMIT,
            "at_unix_ms": crate::spawn_guard::now_ms(),
        })
        .to_string();
        let _ = crate::spawn_guard::append_line(&crate::spawn_guard::log_path(home), &line);
    }
}

/// The policy file could not be read, so the call ran: one line per ten
/// minutes, to the log only (B21; the core writes the file again at its next
/// start or device connection).
pub fn record_unreadable(home: &Path, runtime: &str, failure: PolicyFailure) {
    if crate::delivery::claim(home, "guardrails") {
        let line = serde_json::json!({
            "component": "guardrails",
            "kind": "policy.unreadable",
            "cause": failure.code(),
            "runtime": runtime,
            "at_unix_ms": crate::spawn_guard::now_ms(),
        })
        .to_string();
        let _ = crate::spawn_guard::append_line(&crate::spawn_guard::log_path(home), &line);
    }
}

/// The hook failed before it judged a call (`cause`: `panic`, or
/// `payload_deadline` when the payload did not arrive in time), so the call
/// ran: one line per ten minutes, only while some project has a protection
/// on, since nothing was there to judge otherwise (B21).
pub fn record_failure(home: &Path, runtime: &str, cause: &str) {
    let on = read_policy(home)
        .is_ok_and(|policy| policy.projects.iter().any(|project| project.rules.any()));
    if on && crate::delivery::claim(home, "guardrails_failed") {
        let line = serde_json::json!({
            "component": "guardrails",
            "kind": "guard.failed",
            "cause": cause,
            "runtime": runtime,
            "at_unix_ms": crate::spawn_guard::now_ms(),
        })
        .to_string();
        let _ = crate::spawn_guard::append_line(&crate::spawn_guard::log_path(home), &line);
    }
}

/// The hook's whole guardrail step for one tool call of `runtime`: the
/// reason to refuse it with, or `None` to let it run. The policy is read
/// first, so a machine with no protection on parses nothing more (B27).
pub fn refusal_for(
    home: &Path,
    runtime: &str,
    pane: Option<&str>,
    call: impl FnOnce() -> Option<payload::ToolCall>,
) -> Option<String> {
    let policy = match read_policy(home) {
        Ok(policy) => policy,
        Err(failure) => {
            record_unreadable(home, runtime, failure);
            return None;
        }
    };
    if !policy.projects.iter().any(|project| project.rules.any()) {
        return None;
    }
    let call = call()?;
    // A relative working folder is the hook's own, which the runtime started
    // it in; read against itself it would name no project.
    let here = std::env::current_dir().ok();
    let cwd = match call.cwd {
        Some(cwd) if cwd.is_relative() => here?.join(cwd),
        Some(cwd) => cwd,
        None => here?,
    };
    // One `git status` per checkout and question, however many commands of
    // the call would discard changes there.
    let asked = std::cell::RefCell::new(std::collections::HashMap::new());
    let dirty = |root: &Path, untracked: bool| {
        *asked
            .borrow_mut()
            .entry((root.to_path_buf(), untracked))
            .or_insert_with(|| git_dirty(root, untracked))
    };
    match decide_with(&policy, home, &cwd, &call.action, &dirty) {
        Outcome::Refuse(refusal) => {
            record_refusal(home, runtime, pane, &refusal);
            Some(refusal.reason())
        }
        Outcome::Allow | Outcome::PolicyUnreadable(_) => None,
    }
}
