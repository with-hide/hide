//! The read tools on the node that holds the Project (PRD hide-ai-suggestions
//! B7, B21): a file's lines through `Call::Bytes`, a session's events through
//! the session readers, a search and a history through the fixed
//! `GitCommand::Grep` and `GitCommand::Log`, and a failed CI job's log
//! through the node's `gh`. Each is a call the investigation's worker makes,
//! never under the runtime lock and never on the session-sync thread, and
//! each failure is a code, never a path or a message.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use hide_node_link::git::GitCommand;
use hide_node_link::protocol::Call;
use hide_node_link::{ErrorCode, LinkError, NodeLink};
use hide_session::{Agent, ConversationEvent, ProjectSession};

use super::super::ledger::SessionRef;
use super::super::policy;
use super::reads::{Excerpt, ReadRefused, ReadTools, SearchHit, SessionWindow};
use crate::github::Where;
use crate::node::NodeId;

const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a candidate's node is given to name the Project that holds it.
const IDENTITY_TIMEOUT: Duration = Duration::from_secs(5);
const SESSION_TIMEOUT: Duration = Duration::from_secs(60);
/// The most characters one event of a session shows.
const EVENT_CHARS: usize = 2_000;

/// The read tools of one Project's checkout.
pub struct NodeReads {
    node: Arc<dyn NodeLink>,
    /// The checkout the reads run in, as the node spells it.
    root: String,
    project: hide_project::ProjectIdentity,
    /// Where `gh` reads the Project's repository, when it is known.
    github: Option<Where>,
    /// The Project's sessions, read once a run.
    sessions: Option<Vec<ProjectSession>>,
}

impl NodeReads {
    pub(crate) fn new(
        node: Arc<dyn NodeLink>,
        root: String,
        project: hide_project::ProjectIdentity,
        github: Option<Where>,
    ) -> Self {
        Self {
            node,
            root,
            project,
            github,
            sessions: None,
        }
    }

    fn sessions(&mut self) -> Result<&[ProjectSession], ReadRefused> {
        if self.sessions.is_none() {
            let page = hide_node_link::readers::project_sessions(
                &*self.node,
                self.project.clone(),
                SESSION_TIMEOUT,
            )
            .map_err(refused)?;
            self.sessions = Some(page.rows);
        }
        Ok(self.sessions.as_deref().unwrap_or_default())
    }

    fn git(&self, command: GitCommand) -> Result<String, ReadRefused> {
        hide_node_link::call_as::<String>(
            &*self.node,
            Call::Git {
                root: self.root.clone(),
                command,
            },
            READ_TIMEOUT,
        )
        .map_err(refused)
    }
}

/// A folder the Project may be checked out in, with the node that holds it
/// and where `gh` reads its repository.
pub(crate) struct Checkout {
    pub node: NodeId,
    pub link: Arc<dyn NodeLink>,
    pub root: String,
    pub github: Option<Where>,
}

/// The read tools of one Project, whose checkout is found the first time a
/// read needs it: each candidate's node names the Project that holds it,
/// starting with the folder that is the Project's own root, and the first
/// that names this Project is read. A Project no candidate names refuses
/// every read as `project_unreachable`.
pub(crate) struct ProjectReads {
    project_id: String,
    candidates: Vec<Checkout>,
    found: Option<Result<NodeReads, ReadRefused>>,
}

impl ProjectReads {
    pub(crate) fn new(project_id: String, candidates: Vec<Checkout>) -> Self {
        Self {
            project_id,
            candidates,
            found: None,
        }
    }

    fn reads(&mut self) -> Result<&mut NodeReads, ReadRefused> {
        if self.found.is_none() {
            self.found = Some(self.locate());
        }
        match self.found.as_mut() {
            Some(Ok(reads)) => Ok(reads),
            Some(Err(refused)) => Err(refused.clone()),
            None => Err(ReadRefused::Unavailable("project_unreachable".into())),
        }
    }

    fn locate(&mut self) -> Result<NodeReads, ReadRefused> {
        let mut candidates = std::mem::take(&mut self.candidates);
        // A Project's id is its node and root, so the folder that is its
        // root is asked first and usually alone.
        candidates.sort_by_key(|checkout| {
            hide_project::project_id(checkout.node.as_str(), std::path::Path::new(&checkout.root))
                != self.project_id
        });
        for checkout in candidates {
            let Ok(identity) = crate::memory_hook::project_identity(
                checkout.link.as_ref(),
                &checkout.node,
                &checkout.root,
                IDENTITY_TIMEOUT,
            ) else {
                continue;
            };
            if identity.id == self.project_id {
                return Ok(NodeReads::new(
                    checkout.link,
                    checkout.root,
                    identity,
                    checkout.github,
                ));
            }
        }
        Err(ReadRefused::Unavailable("project_unreachable".into()))
    }
}

impl ReadTools for ProjectReads {
    fn session(
        &mut self,
        session: &SessionRef,
        window: SessionWindow,
    ) -> Result<Excerpt, ReadRefused> {
        self.reads()?.session(session, window)
    }

    fn ci_log(&mut self, pr: u32, check: &str) -> Result<Excerpt, ReadRefused> {
        self.reads()?.ci_log(pr, check)
    }

    fn file(&mut self, path: &str, from_line: u32, lines: u32) -> Result<Excerpt, ReadRefused> {
        self.reads()?.file(path, from_line, lines)
    }

    fn search(&mut self, text: &str, path: Option<&str>) -> Result<Vec<SearchHit>, ReadRefused> {
        self.reads()?.search(text, path)
    }

    fn git_log(&mut self, path: Option<&str>, count: u32) -> Result<Excerpt, ReadRefused> {
        self.reads()?.git_log(path, count)
    }
}

fn refused(error: LinkError) -> ReadRefused {
    match &error {
        LinkError::Refused(host) if host.code == ErrorCode::NotFound => ReadRefused::NotFound,
        _ => ReadRefused::Unavailable(crate::node_access::link_failure_code(&error)),
    }
}

fn cut(text: &str, chars: usize) -> String {
    let mut kept: String = text.chars().take(chars).collect();
    if kept.len() < text.len() {
        kept.push('…');
    }
    kept
}

fn shown(index: usize, event: &ConversationEvent) -> String {
    format!(
        "#{index} {} ({}): {}",
        event.role,
        event.kind.as_str(),
        cut(&event.text, EVENT_CHARS)
    )
}

/// The job a GitHub Actions check's page names
/// (`…/actions/runs/<run>/job/<job>`).
fn actions_job(url: &str) -> Option<u64> {
    let (_, rest) = url.split_once("/actions/runs/")?;
    let (_, job) = rest.split_once("/job/")?;
    job.split(['/', '?', '#']).next()?.parse().ok()
}

/// The events of one session a read shows, chosen as they stream past
/// page by page: `count` events from the window's pivot, with up to half of
/// them before it when the window is around a time.
struct EventWindow {
    window: SessionWindow,
    before: usize,
    count: usize,
    ring: VecDeque<(usize, String)>,
    kept: Vec<(usize, String)>,
    pivot: bool,
    seen: usize,
}

impl EventWindow {
    fn new(window: SessionWindow) -> Self {
        let (before, count) = match window {
            SessionWindow::Around { count, .. } => (count as usize / 2, count as usize),
            SessionWindow::From { count, .. } => (0, count as usize),
        };
        Self {
            window,
            before,
            count,
            ring: VecDeque::new(),
            kept: Vec::new(),
            pivot: false,
            seen: 0,
        }
    }

    /// One more event; `line` words it by its number, and only for an event
    /// the window may show.
    fn push(&mut self, at_unix_ms: u64, line: impl FnOnce(usize) -> String) {
        let index = self.seen;
        self.seen += 1;
        if !self.pivot {
            self.pivot = match self.window {
                SessionWindow::Around { at_unix_ms: at, .. } => at_unix_ms >= at,
                SessionWindow::From { from, .. } => index >= from as usize,
            };
            if self.pivot {
                self.kept.extend(self.ring.drain(..));
            }
        }
        if self.pivot {
            if !self.full() {
                self.kept.push((index, line(index)));
            }
        } else if self.before > 0 {
            self.ring.push_back((index, line(index)));
            if self.ring.len() > self.before {
                self.ring.pop_front();
            }
        }
    }

    fn full(&self) -> bool {
        self.kept.len() >= self.count
    }

    /// The excerpt, or `None` when the session has no event to show. A
    /// window around a time after the session's last event shows its end.
    fn finish(mut self, complete: bool) -> Option<Excerpt> {
        if !self.pivot && matches!(self.window, SessionWindow::Around { .. }) {
            self.kept.extend(self.ring.drain(..));
        }
        let first = self.kept.first()?.0;
        let last = self.kept.last()?.0;
        let of = if complete {
            format!("of {}", self.seen)
        } else {
            "of more".to_owned()
        };
        Some(Excerpt {
            text: self
                .kept
                .into_iter()
                .map(|(_, line)| line)
                .collect::<Vec<_>>()
                .join("\n"),
            truncated: false,
            span: Some(format!("events {first}-{last} {of}")),
        })
    }
}

impl ReadTools for NodeReads {
    fn session(
        &mut self,
        session: &SessionRef,
        window: SessionWindow,
    ) -> Result<Excerpt, ReadRefused> {
        let agent = Agent::from_kind(&session.agent)
            .ok_or_else(|| ReadRefused::Unavailable("session_agent_unknown".into()))?;
        let found = self
            .sessions()?
            .iter()
            .find(|known| known.agent == agent && known.id == session.id)
            .cloned()
            .ok_or(ReadRefused::NotFound)?;
        let scope = hide_session::SessionReadScope {
            id: found.id.clone(),
            cwd: found.checkout_path.to_string_lossy().into_owned(),
        };
        let mut shown_events = EventWindow::new(window);
        let mut consumed = 0u64;
        let mut checkpoint = None;
        let mut complete = false;
        for _ in 0..hide_session::cursor::ARCHIVE_PAGE_LIMIT {
            let read: hide_session::ConversationRead = hide_node_link::link::call_as_reader(
                &*self.node,
                agent,
                hide_node_link::sessions::ReaderFeature::Conversation,
                Call::SessionConversation {
                    agent,
                    path: found.locator.to_string_lossy().into_owned(),
                    scope: scope.clone(),
                    checkpoint: checkpoint.clone(),
                },
                SESSION_TIMEOUT,
            )
            .map_err(refused)?;
            consumed = consumed.saturating_add(read.read_bytes);
            for event in &read.events {
                shown_events.push(event.at_unix_ms, |index| shown(index, event));
            }
            if !read.has_more || read.read_bytes == 0 {
                complete = true;
                break;
            }
            if shown_events.full() || consumed > hide_session::SESSION_READ_LIMIT_BYTES {
                break;
            }
            checkpoint = Some(read.checkpoint);
        }
        shown_events.finish(complete).ok_or(ReadRefused::NotFound)
    }

    fn ci_log(&mut self, pr: u32, check: &str) -> Result<Excerpt, ReadRefused> {
        use hide_node_link::gh::GithubFailureCategory as Category;
        let unavailable = |error: crate::github::GhFailure| {
            ReadRefused::Unavailable(
                match error.category {
                    Category::NotInstalled => "github_not_installed",
                    Category::NotLoggedIn => "github_not_logged_in",
                    Category::NoGithubRemote => "github_no_remote",
                    Category::NetworkOrRateLimit => "github_unreachable",
                }
                .into(),
            )
        };
        let github = self
            .github
            .as_ref()
            .ok_or_else(|| ReadRefused::Unavailable("github_repository_unknown".into()))?;
        let feedback = crate::github::pr_feedback(&*self.node, github, pr)
            .map_err(|_| ReadRefused::Unavailable("github_unreadable".into()))?;
        let url = feedback
            .failed_checks
            .into_iter()
            .find(|failed| {
                failed
                    .name
                    .as_deref()
                    .and_then(super::super::ledger::normalize_tag)
                    == super::super::ledger::normalize_tag(check)
            })
            .ok_or(ReadRefused::NotFound)?
            .url
            .ok_or_else(|| ReadRefused::Unavailable("check_has_no_page".into()))?;
        let job = actions_job(&url)
            .ok_or_else(|| ReadRefused::Unavailable("check_not_on_github_actions".into()))?;
        let log = crate::github::failed_job_log(&*self.node, github, job).map_err(unavailable)?;
        let lines: Vec<&str> = log.lines().collect();
        let start = lines.len().saturating_sub(policy::CI_LOG_LINES);
        Ok(Excerpt {
            text: lines[start..]
                .iter()
                .map(|line| cut(line, policy::READ_LINE_CHARS))
                .collect::<Vec<_>>()
                .join("\n"),
            truncated: start > 0 || log.len() >= hide_node_link::gh::JOB_LOG_TAIL_BYTES,
            span: Some(format!(
                "last {} lines of the failed steps",
                lines.len() - start
            )),
        })
    }

    fn file(&mut self, path: &str, from_line: u32, lines: u32) -> Result<Excerpt, ReadRefused> {
        let range = crate::node_access::read_bytes(
            &*self.node,
            &self.root,
            path,
            0,
            policy::FILE_SOURCE_BYTES,
        )
        .map_err(refused)?;
        let bytes = range
            .bytes()
            .map_err(|_| ReadRefused::Unavailable("file_unreadable".into()))?;
        if bytes.contains(&0) {
            return Err(ReadRefused::Unavailable("binary_file".into()));
        }
        let whole = range.total <= bytes.len() as u64;
        let text = String::from_utf8_lossy(&bytes);
        let all: Vec<&str> = text.lines().collect();
        let from = (from_line as usize).saturating_sub(1);
        let to = (from + lines as usize).min(all.len());
        if from >= all.len() {
            return Err(ReadRefused::Unavailable("past_the_end".into()));
        }
        let of = if whole {
            format!("of {}", all.len())
        } else {
            "of more".to_owned()
        };
        Ok(Excerpt {
            text: all[from..to]
                .iter()
                .enumerate()
                .map(|(at, line)| {
                    format!("{}| {}", from + at + 1, cut(line, policy::READ_LINE_CHARS))
                })
                .collect::<Vec<_>>()
                .join("\n"),
            truncated: to < all.len() || !whole,
            span: Some(format!("lines {}-{} {of}", from + 1, to)),
        })
    }

    fn search(&mut self, text: &str, path: Option<&str>) -> Result<Vec<SearchHit>, ReadRefused> {
        let found = self.git(GitCommand::Grep {
            text: text.to_owned(),
            path: path.map(str::to_owned),
        })?;
        Ok(found
            .lines()
            .filter_map(|line| {
                let mut parts = line.splitn(3, '\0');
                Some(SearchHit {
                    path: parts.next()?.to_owned(),
                    line: parts.next()?.parse().ok()?,
                    text: parts.next()?.to_owned(),
                })
            })
            .take(policy::SEARCH_HITS)
            .collect())
    }

    fn git_log(&mut self, path: Option<&str>, count: u32) -> Result<Excerpt, ReadRefused> {
        let history = self.git(GitCommand::Log {
            path: path.map(str::to_owned),
            count,
        })?;
        let commits = history.lines().count();
        Ok(Excerpt {
            text: history,
            truncated: false,
            span: Some(format!("newest {commits}")),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_check_page_names_its_actions_job_or_none() {
        assert_eq!(
            actions_job("https://github.com/o/r/actions/runs/123/job/456"),
            Some(456)
        );
        assert_eq!(
            actions_job("https://github.com/o/r/actions/runs/123/job/456?pr=7"),
            Some(456)
        );
        assert_eq!(
            actions_job("https://app.circleci.com/pipelines/gh/o/r/1"),
            None
        );
        assert_eq!(actions_job("https://github.com/o/r/actions/runs/123"), None);
    }

    fn read(window: SessionWindow, times: &[u64]) -> Option<Excerpt> {
        let mut events = EventWindow::new(window);
        for at in times {
            events.push(*at, |index| format!("e{index}"));
        }
        events.finish(true)
    }

    #[test]
    fn a_session_read_shows_the_events_around_a_record_or_from_a_number() {
        let times: Vec<u64> = (0..20).map(|at| at * 10).collect();
        let around = read(
            SessionWindow::Around {
                at_unix_ms: 95,
                count: 4,
            },
            &times,
        )
        .unwrap();
        assert_eq!(around.text, "e8\ne9\ne10\ne11");
        assert_eq!(around.span.as_deref(), Some("events 8-11 of 20"));
        let from = read(SessionWindow::From { from: 18, count: 5 }, &times).unwrap();
        assert_eq!(from.text, "e18\ne19");
        let late = read(
            SessionWindow::Around {
                at_unix_ms: 1_000,
                count: 4,
            },
            &times,
        )
        .unwrap();
        assert_eq!(late.text, "e18\ne19", "a time past the end shows the end");
        assert!(read(SessionWindow::From { from: 30, count: 5 }, &times).is_none());
    }

    fn git(root: &std::path::Path, arguments: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success(), "git {arguments:?}");
    }

    /// The reads against the core's own node and a real repository: a
    /// file's numbered lines, a search's hits, and the history of a path.
    #[test]
    fn the_reads_answer_from_the_projects_own_node() {
        let home = tempfile::tempdir().unwrap();
        let checkout = tempfile::tempdir().unwrap();
        let root = checkout.path();
        git(root, &["init", "-q", "-b", "main"]);
        git(root, &["config", "user.name", "Fixture"]);
        git(root, &["config", "user.email", "fixture@example.invalid"]);
        git(root, &["config", "commit.gpgsign", "false"]);
        std::fs::create_dir_all(root.join("scripts")).unwrap();
        std::fs::write(
            root.join("scripts/test.sh"),
            "#!/bin/sh\nset -e\ncargo test --workspace\n",
        )
        .unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "-qm", "Add the test script"]);
        let wire = hide_platform::path::to_wire(root).unwrap();
        let mut reads = NodeReads::new(
            Arc::new(hide_node::Local::new(Some(home.path().to_path_buf()))),
            wire.clone(),
            hide_project::ProjectIdentity {
                id: "project".into(),
                root: root.to_path_buf(),
                checkout_root: root.to_path_buf(),
                device_id: "local".into(),
                kind: hide_project::ProjectKind::Git,
            },
            Some(Where::Checkout(root.to_path_buf())),
        );

        let file = reads.file("scripts/test.sh", 2, 5).unwrap();
        assert_eq!(file.text, "2| set -e\n3| cargo test --workspace");
        assert_eq!(file.span.as_deref(), Some("lines 2-3 of 3"));
        assert_eq!(
            reads.file("scripts/missing.sh", 1, 5),
            Err(ReadRefused::NotFound)
        );

        let hits = reads.search("CARGO TEST", None).unwrap();
        assert_eq!(
            hits,
            [SearchHit {
                path: "scripts/test.sh".into(),
                line: 3,
                text: "cargo test --workspace".into(),
            }]
        );
        let history = reads.git_log(Some("scripts/test.sh"), 5).unwrap();
        assert!(
            history.text.ends_with("Add the test script"),
            "{}",
            history.text
        );
        assert_eq!(history.span.as_deref(), Some("newest 1"));
    }
}
