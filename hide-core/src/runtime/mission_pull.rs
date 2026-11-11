//! The operator's checkout brought up to a closed mission's destination
//! (PRD mission-destination D-26 to D-31, D-43, D-47): when a mission closes
//! as done or `merge_wait`, or a waiting one turns done, Hide fetches the
//! project's `origin` once, whatever its `auto_fetch` says, and moves the
//! checkout the mission was made in forward to its destination with the
//! checkout's own `받기` (`Call::BranchCatchUp`, fast-forward only). It is
//! the one automatic write to the operator's checkout, so it happens only
//! when the checkout has the destination out, tracks `origin/<destination>`,
//! is behind it and has no commit of its own; anything else leaves it as it
//! is. A cancelled mission, and one where no change reached the destination,
//! fetch and pull nothing.
//!
//! What to pull is chosen under the lock from memory (the book and the
//! worktree catalog); the fetch, the read and the fast-forward run on one
//! issue worker at a time, off the lock, so a close never waits for them.
//! Which branch the checkout has out is the read's, made after the fetch,
//! never the catalog's, and the fast-forward is refused when `HEAD` has
//! another branch out by then.
//! What came of it is one diagnostic (`mission.folder_pull`) and, when the
//! operator has something to read, one line of the mission's report, which a
//! later close replaces rather than adds to. Nothing else is shown: no
//! alert, banner or sheet, and no setting turns it off (B40).

use std::collections::VecDeque;
use std::time::Duration;

use hide_node_link::branch::{CATCH_UP_WAIT, CatchUpOutcome};
use hide_node_link::protocol::Call;
use hide_node_link::worktrees::{CheckoutGit, FETCH_TIMEOUT, FetchOutcome, FetchSkip};
use serde_json::json;

use super::Runtime;
use super::mission_landing::{NodeLanding, node_landing};
use crate::missions::{MissionStatus, NodeKind};
use crate::node_access::NodeLink;

/// How long the checkout's read may take: the node bounds `CheckoutGit` at
/// 3 s, as the header's read does (`checkout_git.rs`).
const CHECKOUT_READ: Duration = Duration::from_secs(4);

/// The pulls owed and the one out. Memory only: a restart forgets a pull
/// that was owed, as it forgets a fetch; the next close asks again.
#[derive(Default)]
pub(crate) struct MissionPulls {
    /// Missions whose close owes a pull, oldest first, each once.
    owed: VecDeque<String>,
    /// The mission whose pull is out on the worker.
    in_flight: Option<String>,
}

#[cfg(test)]
impl MissionPulls {
    /// No pull is owed or out.
    pub(crate) fn idle(&self) -> bool {
        self.owed.is_empty() && self.in_flight.is_none()
    }
}

/// What a pull reads and writes, copied under the lock.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Job {
    /// The repository's main worktree, which the fetch is asked of.
    pub(super) project: String,
    /// The checkout the mission was made in, the one that moves.
    pub(super) checkout: String,
    pub(super) destination: String,
}

/// What came of a pull.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Pulled {
    /// The checkout moved forward by this many commits.
    Updated { commits: u32 },
    /// The checkout already holds everything its destination has.
    Current,
    /// The checkout stayed where it is, this many commits behind.
    Behind { commits: u32, why: Held },
    /// The checkout has another branch out, or none; the results are in the
    /// destination.
    Elsewhere { branch: Option<String> },
    /// Nothing was done, for a reason only the log keeps.
    Skipped {
        reason: Skip,
        detail: Option<String>,
    },
}

/// Why a checkout behind its destination was left as it is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Held {
    /// It has commits of its own: only a merge or a rebase takes the rest.
    Diverged,
    /// The fast-forward would overwrite a change the operator has not
    /// committed, or a file they keep ignored.
    Changes,
    /// It is partway through a merge, rebase, cherry-pick or revert.
    InProgress,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Skip {
    /// No change of the mission reached its destination (D-47).
    NothingLanded,
    /// The mission's project is on a connected device (D-28).
    Device,
    /// The worktree catalog does not list the checkout, or the project's
    /// default branch for a mission with no destination.
    Unlisted,
    /// The project's node could not be reached.
    Unreachable,
    /// The fetch failed, or another Git held the repository's refs.
    FetchFailed,
    /// The repository has no `origin` to fetch from.
    NoOrigin,
    /// The checkout's branch does not track `origin/<destination>`.
    NoUpstream,
    /// The checkout could not be read.
    Unread,
    /// `HEAD` moved between the read and the fast-forward.
    HeadMoved,
    /// The fast-forward itself answered an error.
    Failed,
    /// The mission opened again before its pull started.
    Reopened,
}

impl Skip {
    fn as_str(self) -> &'static str {
        match self {
            Self::NothingLanded => "nothing_landed",
            Self::Device => "device",
            Self::Unlisted => "unlisted",
            Self::Unreachable => "unreachable",
            Self::FetchFailed => "fetch_failed",
            Self::NoOrigin => "no_origin",
            Self::NoUpstream => "no_upstream",
            Self::Unread => "unread",
            Self::HeadMoved => "head_moved",
            Self::Failed => "failed",
            Self::Reopened => "reopened",
        }
    }
}

impl Held {
    fn as_str(self) -> &'static str {
        match self {
            Self::Diverged => "diverged",
            Self::Changes => "changes",
            Self::InProgress => "in_progress",
        }
    }
}

impl Pulled {
    fn skipped(reason: Skip) -> Self {
        Self::Skipped {
            reason,
            detail: None,
        }
    }

    fn skipped_with(reason: Skip, detail: impl Into<String>) -> Self {
        Self::Skipped {
            reason,
            detail: Some(detail.into()),
        }
    }

    /// The log's outcome, reason and commit count.
    fn logged(&self) -> (&'static str, Option<&'static str>, Option<u32>) {
        match self {
            Self::Updated { commits } => ("updated", None, Some(*commits)),
            Self::Current => ("current", None, None),
            Self::Behind { commits, why } => ("behind", Some(why.as_str()), Some(*commits)),
            Self::Elsewhere { branch: Some(_) } => ("elsewhere", Some("other_branch"), None),
            Self::Elsewhere { branch: None } => ("elsewhere", Some("detached"), None),
            Self::Skipped { reason, .. } => ("skipped", Some(reason.as_str()), None),
        }
    }

    /// The report's line for the operator, in the words the PRD fixes
    /// (D-26, D-31); none when nothing was done for a reason only the log
    /// keeps (D-28).
    pub(crate) fn line(&self, destination: &str) -> Option<String> {
        match self {
            Self::Updated { commits } => {
                Some(format!("내 폴더도 최신으로 당겨왔어요 ({commits}커밋)"))
            }
            Self::Current => Some("내 폴더도 이미 최신이에요".to_owned()),
            Self::Behind { commits, .. } => Some(format!("내 폴더 {commits}커밋 뒤 · git pull")),
            Self::Elsewhere { branch } => Some(format!(
                "결과는 {destination}에 들어갔어요 · 내 폴더는 {}",
                branch.as_deref().unwrap_or("detached HEAD")
            )),
            Self::Skipped { .. } => None,
        }
    }
}

/// Fetches the project's `origin`, reads the checkout and moves it forward
/// to its destination when that is a fast-forward of the branch it has out;
/// every other case leaves it as it is (D-26 to D-31). Runs off the lock,
/// on the node that holds the project.
pub(super) fn pull(node: &dyn NodeLink, job: &Job) -> Pulled {
    let fetched = crate::node_access::call_as::<FetchOutcome>(
        node,
        Call::FetchRemote {
            path: job.project.clone(),
        },
        FETCH_TIMEOUT,
    );
    match fetched {
        Ok(FetchOutcome::Fetched) => {}
        Ok(FetchOutcome::Skipped {
            reason: FetchSkip::NoOrigin | FetchSkip::NoTrackingRefs,
        }) => return Pulled::skipped(Skip::NoOrigin),
        // Another Git holds the refs: what this pull would read is not the
        // remote as it is now.
        Ok(FetchOutcome::Skipped {
            reason: FetchSkip::Busy,
        }) => return Pulled::skipped_with(Skip::FetchFailed, "a ref lock was held"),
        Ok(FetchOutcome::Failed { reason, .. }) => {
            return Pulled::skipped_with(Skip::FetchFailed, reason);
        }
        Err(error) => return Pulled::skipped_with(Skip::FetchFailed, error.to_string()),
    }
    let facts = match crate::node_access::call_as::<CheckoutGit>(
        node,
        Call::CheckoutGit {
            path: job.checkout.clone(),
        },
        CHECKOUT_READ,
    ) {
        Ok(facts) => facts,
        Err(error) => return Pulled::skipped_with(Skip::Unread, error.to_string()),
    };
    // The branch read now, not the catalog's, which may be older than a
    // switch: a branch of another name is left, whatever it tracks.
    if facts.branch.as_deref() != Some(job.destination.as_str()) {
        return Pulled::Elsewhere {
            branch: facts.branch,
        };
    }
    // The destination tracking another branch would be moved to that one.
    let tracked = format!("origin/{}", job.destination);
    if facts.upstream_state != "tracked" || facts.upstream.as_deref() != Some(tracked.as_str()) {
        return Pulled::skipped(Skip::NoUpstream);
    }
    let (Some(head), Some(ahead), Some(behind)) = (facts.head_sha, facts.ahead, facts.behind)
    else {
        return Pulled::skipped_with(Skip::Unread, "the checkout's HEAD or counts were not read");
    };
    if behind == 0 {
        return Pulled::Current;
    }
    if ahead > 0 {
        return Pulled::Behind {
            commits: behind,
            why: Held::Diverged,
        };
    }
    let caught = crate::node_access::call_as::<CatchUpOutcome>(
        node,
        Call::BranchCatchUp {
            root: job.checkout.clone(),
            head,
            branch: Some(job.destination.clone()),
        },
        CATCH_UP_WAIT,
    );
    let held = |why| Pulled::Behind {
        commits: behind,
        why,
    };
    match caught {
        Ok(CatchUpOutcome::Updated { .. }) => Pulled::Updated { commits: behind },
        Ok(CatchUpOutcome::AlreadyCurrent { .. }) => Pulled::Current,
        Ok(CatchUpOutcome::Diverged) => held(Held::Diverged),
        Ok(CatchUpOutcome::Dirty { .. }) => held(Held::Changes),
        Ok(CatchUpOutcome::InProgress) => held(Held::InProgress),
        Ok(CatchUpOutcome::HeadMoved) => Pulled::skipped(Skip::HeadMoved),
        // Switched to another branch at the same commit since the read.
        Ok(CatchUpOutcome::OtherBranch { branch }) => Pulled::Elsewhere { branch },
        Ok(CatchUpOutcome::NoUpstream) => Pulled::skipped(Skip::NoUpstream),
        Err(error) => Pulled::skipped_with(Skip::Failed, error.to_string()),
    }
}

/// `report` with `line` as its last paragraph in place of `before`, the
/// line an earlier close added.
fn with_line(report: Option<&str>, before: Option<&str>, line: &str) -> String {
    let body = report.unwrap_or_default();
    let body = before
        .and_then(|before| body.trim_end().strip_suffix(before))
        .unwrap_or(body)
        .trim_end();
    if body.is_empty() {
        line.to_owned()
    } else {
        format!("{body}\n\n{line}")
    }
}

impl Runtime {
    /// A close of mission `id` as done or `merge_wait`, or a waiting one
    /// turning done, owes its operator's checkout a pull (D-43); a cancel
    /// never calls this. The pull starts once the one before it ended.
    pub(super) fn owe_folder_pull(&mut self, id: &str) {
        let pulls = &mut self.missions.pulls;
        if !pulls.owed.iter().any(|owed| owed == id) {
            pulls.owed.push_back(id.to_owned());
        }
        self.drive_folder_pulls();
    }

    /// Starts the next pull owed while none is out.
    fn drive_folder_pulls(&mut self) {
        while self.missions.pulls.in_flight.is_none() {
            let Some(id) = self.missions.pulls.owed.pop_front() else {
                return;
            };
            let (job, node) = match self.folder_pull_job(&id) {
                Ok(found) => found,
                Err(reason) => {
                    self.note_folder_pull(&id, None, &Pulled::skipped(reason));
                    continue;
                }
            };
            self.missions.pulls.in_flight = Some(id.clone());
            let (asked, mission) = (job.clone(), id.clone());
            let spawned = self.spawn_issue_worker(
                "mission-pull",
                move || pull(node.as_ref(), &asked),
                move |runtime, pulled| {
                    runtime.missions.pulls.in_flight = None;
                    let changed = runtime.note_folder_pull(&mission, Some(&job), &pulled);
                    runtime.drive_folder_pulls();
                    changed
                },
            );
            if let Err(error) = spawned {
                self.missions.pulls.in_flight = None;
                self.note_folder_pull(&id, None, &Pulled::skipped_with(Skip::Failed, error));
            }
        }
    }

    /// What mission `id`'s pull reads and the node it asks, from memory
    /// only; or why there is no pull.
    fn folder_pull_job(&mut self, id: &str) -> Result<(Job, std::sync::Arc<dyn NodeLink>), Skip> {
        let mission = self.missions.book.get(id).ok_or(Skip::Reopened)?;
        if !matches!(
            mission.status,
            MissionStatus::Done | MissionStatus::MergeWait
        ) {
            return Err(Skip::Reopened);
        }
        let landed = mission.nodes.iter().any(|node| {
            node.kind == NodeKind::Change
                && matches!(
                    node_landing(mission, node, false),
                    Some(NodeLanding::Landed { .. })
                )
        });
        if !landed {
            return Err(Skip::NothingLanded);
        }
        if mission.device_id != self.node.as_str() {
            return Err(Skip::Device);
        }
        let checkout = if mission.checkout_path.is_empty() {
            mission.project.clone()
        } else {
            mission.checkout_path.clone()
        };
        let listed = self
            .project_worktrees_on(&mission.device_id, &mission.project)
            .ok_or(Skip::Unlisted)?;
        let destination = mission
            .destination
            .clone()
            .or_else(|| listed.base_branch.clone())
            .ok_or(Skip::Unlisted)?;
        if !listed
            .worktrees
            .iter()
            .any(|worktree| worktree.path == checkout)
        {
            return Err(Skip::Unlisted);
        }
        let job = Job {
            project: mission.project.clone(),
            checkout,
            destination,
        };
        let device = mission.device_id.clone();
        let node = self.node_link(&device).map_err(|_| Skip::Unreachable)?;
        Ok((job, node))
    }

    /// Logs what came of mission `id`'s pull and puts its line in the
    /// report of a mission still closed; whether the book changed.
    fn note_folder_pull(&mut self, id: &str, job: Option<&Job>, pulled: &Pulled) -> bool {
        let (outcome, reason, commits) = pulled.logged();
        let detail = match pulled {
            Pulled::Skipped { detail, .. } => detail.as_deref(),
            _ => None,
        };
        crate::diagnostic!(json!({
            "component": "missions", "kind": "mission.folder_pull",
            "mission": id, "outcome": outcome, "reason": reason, "commits": commits,
            "checkout": job.map(|job| job.checkout.as_str()),
            "destination": job.map(|job| job.destination.as_str()),
            "message": detail,
        }));
        let Some(line) = job.and_then(|job| pulled.line(&job.destination)) else {
            return false;
        };
        let Some(mission) = self.missions.book.get_mut(id).filter(|mission| {
            matches!(
                mission.status,
                MissionStatus::Done | MissionStatus::MergeWait
            )
        }) else {
            return false;
        };
        let report = with_line(
            mission.report.as_deref(),
            mission.folder_line.as_deref(),
            &line,
        );
        if mission.report.as_deref() == Some(report.as_str())
            && mission.folder_line.as_deref() == Some(line.as_str())
        {
            return false;
        }
        mission.report = Some(report);
        mission.folder_line = Some(line);
        self.save_missions();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::process::Command;

    /// The host's own git answering for a real repository on this machine.
    struct OwnHost(hide_host::serve::Env);

    impl NodeLink for OwnHost {
        fn call(
            &self,
            call: Call,
            _timeout: Duration,
        ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
            hide_host::serve::handle_in(call, &self.0)
                .map(crate::node_access::LinkAnswer::Parsed)
                .map_err(crate::node_access::LinkError::Refused)
        }
    }

    fn own_host() -> OwnHost {
        OwnHost(hide_host::serve::Env::standalone(None))
    }

    /// The operator's hand between the read and the fast-forward: runs
    /// `before` once, right before the node is asked to catch up.
    struct Between<F: Fn()>(OwnHost, F);

    impl<F: Fn() + Send + Sync> NodeLink for Between<F> {
        fn call(
            &self,
            call: Call,
            timeout: Duration,
        ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
            if matches!(call, Call::BranchCatchUp { .. }) {
                (self.1)();
            }
            self.0.call(call, timeout)
        }
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=Hide",
                "-c",
                "user.email=hide@example.invalid",
            ])
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "init.defaultBranch=main",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?}: {output:?}");
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    fn commit(dir: &Path, name: &str, text: &str) {
        std::fs::write(dir.join(name), text).unwrap();
        git(dir, &["add", name]);
        git(dir, &["commit", "-q", "-m", name]);
    }

    /// A repository with a local bare `origin`, the operator's checkout of it
    /// on `feat/login` tracking `origin/feat/login`, and a second clone that
    /// stands for the merges the mission made on the remote. All inside the
    /// test's own folder.
    struct Repos {
        _root: tempfile::TempDir,
        checkout: std::path::PathBuf,
        merges: std::path::PathBuf,
    }

    impl Repos {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let origin = root.path().join("origin.git");
            git(
                root.path(),
                &["init", "-q", "--bare", origin.to_str().unwrap()],
            );
            let checkout = root.path().join("project");
            git(
                root.path(),
                &[
                    "clone",
                    "-q",
                    origin.to_str().unwrap(),
                    checkout.to_str().unwrap(),
                ],
            );
            commit(&checkout, "README", "one\n");
            git(&checkout, &["push", "-q", "origin", "HEAD:main"]);
            git(&checkout, &["switch", "-q", "-c", "feat/login"]);
            git(&checkout, &["push", "-q", "-u", "origin", "feat/login"]);
            let merges = root.path().join("merges");
            git(
                root.path(),
                &[
                    "clone",
                    "-q",
                    "-b",
                    "feat/login",
                    origin.to_str().unwrap(),
                    merges.to_str().unwrap(),
                ],
            );
            Self {
                _root: root,
                checkout,
                merges,
            }
        }

        /// The mission's results reach `origin/feat/login`: `count` commits.
        fn land(&self, count: usize) {
            for at in 0..count {
                commit(&self.merges, &format!("landed-{at}"), "landed\n");
            }
            git(&self.merges, &["push", "-q", "origin", "feat/login"]);
        }

        fn job(&self) -> Job {
            Job {
                project: self.checkout.to_string_lossy().into_owned(),
                checkout: self.checkout.to_string_lossy().into_owned(),
                destination: "feat/login".into(),
            }
        }

        fn branch(&self) -> String {
            git(&self.checkout, &["branch", "--show-current"])
        }

        fn head(&self) -> String {
            git(&self.checkout, &["rev-parse", "HEAD"])
        }

        fn origin_head(&self) -> String {
            git(&self.checkout, &["rev-parse", "origin/feat/login"])
        }
    }

    /// B34: a checkout on the destination that is only behind moves forward
    /// to it, with the commits it took; the fetch is the pull's own, so the
    /// project's `auto_fetch` plays no part, and a second close finds it
    /// current and changes nothing (B38).
    #[test]
    fn a_checkout_on_the_destination_only_behind_is_fast_forwarded_by_its_own_fetch() {
        let repos = Repos::new();
        repos.land(3);
        let job = repos.job();
        assert_eq!(pull(&own_host(), &job), Pulled::Updated { commits: 3 });
        assert_eq!(repos.head(), repos.origin_head());
        assert_eq!(
            Pulled::Updated { commits: 3 }.line("feat/login").as_deref(),
            Some("내 폴더도 최신으로 당겨왔어요 (3커밋)")
        );
        let before = repos.head();
        assert_eq!(pull(&own_host(), &job), Pulled::Current);
        assert_eq!(repos.head(), before);
    }

    /// A mission with direct changes merged into the destination in this
    /// checkout itself leaves it at or ahead of `origin/<destination>`:
    /// that is current, never behind or diverged, and nothing moves.
    #[test]
    fn a_checkout_at_or_ahead_of_its_destination_on_origin_reads_current() {
        let repos = Repos::new();
        let job = repos.job();
        assert_eq!(pull(&own_host(), &job), Pulled::Current);
        git(&repos.checkout, &["switch", "-q", "-c", "fix"]);
        commit(&repos.checkout, "fix", "fixed\n");
        git(&repos.checkout, &["switch", "-q", "feat/login"]);
        git(
            &repos.checkout,
            &["merge", "-q", "--no-ff", "-m", "join fix", "fix"],
        );
        let head = repos.head();
        let pulled = pull(&own_host(), &job);
        assert_eq!(pulled, Pulled::Current);
        assert_eq!(repos.head(), head);
        assert_eq!(
            pulled.line("feat/login").as_deref(),
            Some("내 폴더도 이미 최신이에요")
        );
    }

    /// A project with no remote at all fetches nothing and pulls nothing,
    /// and the report says nothing of the folder.
    #[test]
    fn a_project_with_no_remote_is_skipped_with_no_line() {
        let repos = Repos::new();
        git(&repos.checkout, &["remote", "remove", "origin"]);
        let head = repos.head();
        let pulled = pull(&own_host(), &repos.job());
        assert_eq!(pulled, Pulled::skipped(Skip::NoOrigin));
        assert_eq!(pulled.line("feat/login"), None);
        assert_eq!(repos.head(), head);
    }

    /// B35: a checkout with a commit of its own, or with a change the
    /// fast-forward would overwrite, stays exactly as it is and the report
    /// says how far behind it is.
    #[test]
    fn a_diverged_checkout_or_one_whose_changes_the_fast_forward_would_overwrite_stays() {
        let repos = Repos::new();
        repos.land(2);
        commit(&repos.checkout, "mine", "mine\n");
        let head = repos.head();
        let pulled = pull(&own_host(), &repos.job());
        assert_eq!(
            pulled,
            Pulled::Behind {
                commits: 2,
                why: Held::Diverged
            }
        );
        assert_eq!(repos.head(), head);
        assert_eq!(
            pulled.line("feat/login").as_deref(),
            Some("내 폴더 2커밋 뒤 · git pull")
        );

        let repos = Repos::new();
        repos.land(1);
        std::fs::write(repos.checkout.join("landed-0"), "the operator's own\n").unwrap();
        let head = repos.head();
        assert_eq!(
            pull(&own_host(), &repos.job()),
            Pulled::Behind {
                commits: 1,
                why: Held::Changes
            }
        );
        assert_eq!(repos.head(), head);
        assert_eq!(
            std::fs::read_to_string(repos.checkout.join("landed-0")).unwrap(),
            "the operator's own\n"
        );
    }

    /// B36: a checkout on another branch, or detached, is never switched or
    /// moved; the report says the results are in the destination.
    #[test]
    fn a_checkout_on_another_branch_or_detached_is_left_and_named() {
        let repos = Repos::new();
        repos.land(1);
        git(&repos.checkout, &["switch", "-q", "main"]);
        let head = repos.head();
        let pulled = pull(&own_host(), &repos.job());
        assert_eq!(
            pulled,
            Pulled::Elsewhere {
                branch: Some("main".into())
            }
        );
        assert_eq!(git(&repos.checkout, &["branch", "--show-current"]), "main");
        assert_eq!(repos.head(), head);
        assert_eq!(
            pulled.line("feat/login").as_deref(),
            Some("결과는 feat/login에 들어갔어요 · 내 폴더는 main")
        );

        git(&repos.checkout, &["switch", "-q", "--detach", "HEAD"]);
        let pulled = pull(&own_host(), &repos.job());
        assert_eq!(pulled, Pulled::Elsewhere { branch: None });
        assert_eq!(repos.head(), head);
        assert_eq!(
            pulled.line("feat/login").as_deref(),
            Some("결과는 feat/login에 들어갔어요 · 내 폴더는 detached HEAD")
        );
    }

    /// B36 with a catalog older than a switch: a branch cut from the
    /// destination's own upstream (`git switch -c topic origin/main`) tracks
    /// it and is behind it, yet is another branch, so nothing moves.
    #[test]
    fn a_branch_cut_from_the_destinations_upstream_is_left_and_named() {
        let repos = Repos::new();
        repos.land(1);
        git(&repos.merges, &["push", "-q", "origin", "feat/login:main"]);
        git(
            &repos.checkout,
            &["switch", "-q", "-c", "topic", "origin/main"],
        );
        let head = repos.head();
        let job = Job {
            destination: "main".into(),
            ..repos.job()
        };
        let pulled = pull(&own_host(), &job);
        assert_eq!(
            pulled,
            Pulled::Elsewhere {
                branch: Some("topic".into())
            }
        );
        assert_eq!(repos.head(), head);
        assert_eq!(repos.branch(), "topic");
        assert_eq!(
            pulled.line("main").as_deref(),
            Some("결과는 main에 들어갔어요 · 내 폴더는 topic")
        );
    }

    /// B36 between the read and the fast-forward: the operator switched to
    /// another branch at the same commit that tracks the same upstream, so
    /// only the branch the node is asked to keep tells them apart.
    #[test]
    fn a_switch_to_another_branch_after_the_read_is_not_fast_forwarded() {
        let repos = Repos::new();
        repos.land(1);
        let head = repos.head();
        let checkout = repos.checkout.clone();
        let link = Between(own_host(), move || {
            git(&checkout, &["switch", "-q", "-c", "other"]);
            git(&checkout, &["branch", "-q", "-u", "origin/feat/login"]);
        });
        let pulled = pull(&link, &repos.job());
        assert_eq!(
            pulled,
            Pulled::Elsewhere {
                branch: Some("other".into())
            }
        );
        assert_eq!(repos.head(), head);
        assert_eq!(repos.branch(), "other");
        assert_eq!(git(&repos.checkout, &["rev-parse", "feat/login"]), head);
        assert_eq!(
            pulled.line("feat/login").as_deref(),
            Some("결과는 feat/login에 들어갔어요 · 내 폴더는 other")
        );
    }

    /// B37: a fetch that fails leaves the checkout as it is and says nothing
    /// in the report, only in the log.
    #[test]
    fn a_failed_fetch_skips_the_pull_and_leaves_the_checkout() {
        let repos = Repos::new();
        repos.land(1);
        let gone = repos.checkout.with_file_name("gone.git");
        git(
            &repos.checkout,
            &["remote", "set-url", "origin", gone.to_str().unwrap()],
        );
        let head = repos.head();
        let pulled = pull(&own_host(), &repos.job());
        assert!(
            matches!(
                pulled,
                Pulled::Skipped {
                    reason: Skip::FetchFailed,
                    ..
                }
            ),
            "{pulled:?}"
        );
        assert_eq!(pulled.line("feat/login"), None);
        assert_eq!(repos.head(), head);
    }

    /// B37: `HEAD` moved between the read and the fast-forward (the operator
    /// committed in that moment): the fast-forward is not made.
    #[test]
    fn a_head_that_moves_between_the_read_and_the_fast_forward_skips_the_pull() {
        let repos = Repos::new();
        repos.land(1);
        let checkout = repos.checkout.clone();
        let link = Between(own_host(), move || commit(&checkout, "meanwhile", "x\n"));
        let pulled = pull(&link, &repos.job());
        assert_eq!(pulled, Pulled::skipped(Skip::HeadMoved));
        assert_eq!(pulled.line("feat/login"), None);
        assert_eq!(
            git(&repos.checkout, &["log", "-1", "--format=%s"]),
            "meanwhile"
        );
    }

    /// A destination that exists only here, which the checkout does not
    /// track on the remote, is not a pull: the log says so.
    #[test]
    fn a_checkout_that_does_not_track_its_destination_on_origin_is_not_pulled() {
        let repos = Repos::new();
        git(&repos.checkout, &["branch", "-q", "--unset-upstream"]);
        assert_eq!(
            pull(&own_host(), &repos.job()),
            Pulled::skipped(Skip::NoUpstream)
        );
    }

    #[test]
    fn a_later_close_replaces_the_line_an_earlier_one_added() {
        let first = with_line(Some("Done.\n"), None, "내 폴더 2커밋 뒤 · git pull");
        assert_eq!(first, "Done.\n\n내 폴더 2커밋 뒤 · git pull");
        let again = with_line(
            Some(&first),
            Some("내 폴더 2커밋 뒤 · git pull"),
            "내 폴더도 최신으로 당겨왔어요 (3커밋)",
        );
        assert_eq!(again, "Done.\n\n내 폴더도 최신으로 당겨왔어요 (3커밋)");
        // A report the Conductor wrote anew keeps all of its own words.
        assert_eq!(
            with_line(Some("Merged."), Some("내 폴더 2커밋 뒤 · git pull"), "x"),
            "Merged.\n\nx"
        );
        assert_eq!(with_line(None, None, "x"), "x");
    }
}
