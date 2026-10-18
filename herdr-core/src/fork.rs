//! Forking an agent pane into a new execution of the parent conversation.
//!
//! A fork asks Herdr to create a pane and start the new agent, then
//! registers both exact executions in Hide and publishes lineage. This
//! module decides which panes can be forked and spells each agent's resume
//! arguments; running the calls is [`crate::live`]'s job.

/// The agents whose own fork command this shell knows how to spell.
///
/// Each takes the confirmed native session id. A reported path must first be
/// converted by the shared native reader proof before a control is offered.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForkableAgent {
    Claude,
    Codex,
    Grok,
    Pi,
    Omp,
    OpenCode,
    Antigravity,
}

impl ForkableAgent {
    pub fn parse(agent_kind: &str) -> Option<Self> {
        match hide_agent_adapter::adapter(agent_kind)?.fork? {
            hide_agent_adapter::LaunchDialect::Claude => Some(Self::Claude),
            hide_agent_adapter::LaunchDialect::Codex => Some(Self::Codex),
            hide_agent_adapter::LaunchDialect::Grok => Some(Self::Grok),
            hide_agent_adapter::LaunchDialect::Pi => Some(Self::Pi),
            hide_agent_adapter::LaunchDialect::Omp => Some(Self::Omp),
            hide_agent_adapter::LaunchDialect::OpenCode => Some(Self::OpenCode),
            hide_agent_adapter::LaunchDialect::Antigravity => Some(Self::Antigravity),
            hide_agent_adapter::LaunchDialect::Cursor => None,
        }
    }

    pub fn kind(self) -> &'static str {
        match self {
            Self::Claude => {
                hide_agent_adapter::LaunchDialect::Claude
                    .adapter()
                    .herdr
                    .name
            }
            Self::Codex => {
                hide_agent_adapter::LaunchDialect::Codex
                    .adapter()
                    .herdr
                    .name
            }
            Self::Grok => hide_agent_adapter::LaunchDialect::Grok.adapter().herdr.name,
            Self::Pi => hide_agent_adapter::LaunchDialect::Pi.adapter().herdr.name,
            Self::Omp => hide_agent_adapter::LaunchDialect::Omp.adapter().herdr.name,
            Self::OpenCode => {
                hide_agent_adapter::LaunchDialect::OpenCode
                    .adapter()
                    .herdr
                    .name
            }
            Self::Antigravity => {
                hide_agent_adapter::LaunchDialect::Antigravity
                    .adapter()
                    .herdr
                    .name
            }
        }
    }

    /// The agent's own arguments for resuming a session as a new one. They
    /// are handed to `agent.start` as the agent's arguments, so they are the
    /// agent's vocabulary rather than Herdr's.
    pub fn resume_arguments(self, session_id: &str) -> Vec<String> {
        match self {
            // Grok 1.0.46 `--fork-session`: a new id from the resumed history.
            Self::Claude | Self::Grok => vec![
                "--resume".to_owned(),
                session_id.to_owned(),
                "--fork-session".to_owned(),
            ],
            Self::Codex => vec!["fork".to_owned(), session_id.to_owned()],
            Self::Pi | Self::Omp => vec!["--fork".to_owned(), session_id.to_owned()],
            // OpenCode 1.x (1.18.30 `cli/cmd/tui.ts`): `--fork` forks the
            // session `-s` names. 2.x's interactive command has no `--fork`
            // (2.0.29 `packages/cli/src/commands/commands.ts`, the root
            // command's options) and, given one, prints its help and exits;
            // its server forks the session instead and the pane opens the new
            // one ([`opencode_forked_arguments`]).
            // Which one a session takes is its node's to read
            // (`live::fork_arguments`).
            Self::OpenCode => vec!["-s".to_owned(), session_id.to_owned(), "--fork".to_owned()],
            // agy 1.4.0 forks only through its `/fork` command, typed once
            // the resumed conversation is up (`live::fork_antigravity`).
            Self::Antigravity => vec!["--conversation".to_owned(), session_id.to_owned()],
        }
    }
}

/// The arguments that open the session OpenCode 2.x's server forked
/// (`hide_node_link::protocol::OpenCodeFork::Forked`): the new session is
/// opened as a resume opens one, and an id that would reach the argument list
/// as anything but an id is refused.
pub(crate) fn opencode_forked_arguments(id: &str) -> Result<Vec<String>, String> {
    if !hide_session::valid_native_id(id) {
        return Err("OpenCode answered the fork with an invalid session id".into());
    }
    Ok(vec!["-s".to_owned(), id.to_owned()])
}

/// Where the operator put the fork when they chose it (PRD
/// fork-operator-root D-02): `Fork agent` opens it in a new tab right of the
/// source's tab, `Fork agent to the side` splits right inside the source's
/// tab. Every forkable agent takes both.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForkPlacement {
    Tab,
    Side,
}

/// Everything the fork needs, gathered before the runtime mutex is released
/// so the worker thread carries no reference back into runtime state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForkRequest {
    pub(crate) connection_generation: u64,
    pub parent_pane_id: String,
    pub agent: ForkableAgent,
    pub session_id: String,
    pub(crate) source_reference: Option<crate::sidebar::SessionAgentSessionPayload>,
    pub cwd: Option<String>,
    /// The agent name Herdr will list the fork under, unique per fork and
    /// already inside Herdr's name rule (`fork_name`).
    pub name: String,
    /// The label `Fork agent`'s tab is made with: its checkout's next
    /// automatic tab label, which the strip replaces with the agent's title.
    pub tab_label: Option<String>,
    pub placement: ForkPlacement,
    pub(crate) codex_daemon: crate::codex_launch::CodexDaemon,
    /// The model and effort the source was started with, which the fork
    /// starts with after its resume arguments.
    pub launch: hide_agent_adapter::LaunchChoice,
}

/// Herdr's own rule for an agent name, quoted from the error it answers with:
/// it must start with a lowercase letter and carry only lowercase letters,
/// digits, `-` or `_`, in 1 to 32 characters.
const MAX_NAME_CHARACTERS: usize = 32;

pub(crate) fn valid_agent_name(name: &str) -> bool {
    (1..=MAX_NAME_CHARACTERS).contains(&name.len())
        && name.starts_with(|character: char| character.is_ascii_lowercase())
        && name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
}

/// A name no other pane's fork can collide with, that Herdr will accept.
///
/// `agent.start` takes the name as required, and two forks of the same parent
/// are the expected case, so the parent's id alone is not enough to keep them
/// apart.
///
/// Herdr rejects a name that carries a capital or runs past 32 characters, and
/// a pane id is not required to be lowercase or short: `w2X:p2F` is an
/// ordinary id, and it produced `fork-w2X-p2F-2-1788624371518`, which Herdr
/// refused with `invalid_agent_name`. The case is folded rather than replaced
/// so the id stays readable in Herdr's agent list, and a name that would run
/// long keeps a readable head and a digest of the whole name, which is what
/// keeps two forks of different panes apart after the truncation.
pub fn fork_name(parent_pane_id: &str, nonce: &str) -> String {
    bounded_name(format!(
        "{FORK_PREFIX}{}-{}",
        sanitize(parent_pane_id),
        sanitize(nonce)
    ))
}

/// The name a woken agent gets when it had none of its own: one per pane,
/// inside Herdr's rule by the same folding and truncation a fork name uses.
pub fn wake_name(pane_id: &str) -> String {
    bounded_name(format!("wake-{}", sanitize(pane_id)))
}

/// The name of an agent a task starts in the pane it made (an issue's Start,
/// a pull request's 맡기기, New agent): Herdr refuses a name another agent
/// already holds, so it is the pane's, like a wake's, and two tasks running
/// the same provider never collide.
pub fn task_agent_name(kind: &str, pane_id: &str) -> String {
    bounded_name(format!("hide-{}-{}", sanitize(kind), sanitize(pane_id)))
}

/// Every fork name's head, which `bounded_name` keeps.
const FORK_PREFIX: &str = "fork-";

/// Whether Hide made `name` up for the `kind` agent in `pane_id` rather than
/// someone giving it: that pane's task or wake name, or a fork's, which
/// spells its parent's pane and a sequence. Such a name only spells a pane
/// id, so ⌘K neither draws nor finds an agent by it.
pub(crate) fn hide_made_name(name: &str, kind: &str, pane_id: &str) -> bool {
    name == task_agent_name(kind, pane_id) || name == wake_name(pane_id) || is_fork_name(name)
}

/// Whether `name` has the shape `fork_name` gives it: the parent's pane, the
/// fork's sequence and its time in milliseconds, or that cut by
/// `bounded_name` to the limit with a digest at its end. The fork's parent is
/// not known from the child's row, so the shape is all there is to compare;
/// a name someone gave that merely starts with `fork-` keeps its name.
fn is_fork_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix(FORK_PREFIX) else {
        return false;
    };
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    let mut parts = rest.rsplitn(3, '-');
    let whole = matches!(
        (parts.next(), parts.next(), parts.next()),
        (Some(millis), Some(sequence), Some(parent))
            if digits(millis) && digits(sequence) && !parent.is_empty()
    );
    let cut = name.len() == MAX_NAME_CHARACTERS
        && name.rsplit_once('-').is_some_and(|(_, digest)| {
            digest.len() == DIGEST_CHARACTERS
                && digest
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        });
    whole || cut
}

fn bounded_name(full: String) -> String {
    if full.len() <= MAX_NAME_CHARACTERS {
        return full;
    }
    let digest = digest_of(&full);
    let head = MAX_NAME_CHARACTERS - digest.len() - 1;
    format!("{}-{}", &full[..head], digest)
}

/// How many hex characters `digest_of` writes.
const DIGEST_CHARACTERS: usize = 12;

/// Twelve hex characters of a hash of the whole name, so what truncation drops
/// still tells two names apart. `DefaultHasher::new` is seeded with fixed
/// keys, so the same name gives the same digest in every process.
fn digest_of(value: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    format!(
        "{:0width$x}",
        hasher.finish() & 0xffff_ffff_ffff,
        width = DIGEST_CHARACTERS
    )
}

fn sanitize(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = sanitized.trim_matches('-').to_owned();
    if trimmed.is_empty() {
        "pane".to_owned()
    } else {
        trimmed
    }
}

/// Whether the provider and confirmed native identity permit a fork.
/// A native path must pass the shared reader's proof before becoming this id.
pub fn is_forkable(agent_kind: Option<&str>, session_id: Option<&str>) -> bool {
    let Some(agent_kind) = agent_kind else {
        return false;
    };
    ForkableAgent::parse(agent_kind).is_some()
        && session_id.is_some_and(|session_id| !session_id.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A row Herdr lists running `kind` with `session`.
    fn listed(kind: &str, session: Option<&str>) -> crate::model::SidebarAgentSnapshot {
        let agent = serde_json::json!({
            "id": "w1:p2", "pane_id": "w1:p2", "agent": kind, "agent_status": "idle", "state_change_seq": 1,
        });
        let mut row = crate::sidebar::project_agents(
            crate::sidebar::owned_label_fixture(serde_json::json!({"agents": [agent]})).unwrap(),
        )
        .agents
        .remove(0);
        row.session_id = session.map(str::to_owned);
        row
    }

    #[test]
    fn a_forkable_agent_with_a_session_offers_fork_and_one_without_says_why() {
        let fork = |kind, session| crate::runtime::pane_fork_snapshot(Some(&listed(kind, session)));
        let ready = fork("claude", Some("s-1"));
        assert!(ready.available && ready.reason.is_none());
        let waiting = fork("claude", None);
        assert!(!waiting.available);
        assert!(waiting.reason.is_some());
        // An agent that has no fork command, and a pane with no agent, offer
        // nothing and owe no reason.
        let never = fork("cursor", Some("s-1"));
        assert!(!never.available && never.reason.is_none());
        assert_eq!(
            crate::runtime::pane_fork_snapshot(None),
            crate::model::PaneForkSnapshot::default()
        );
    }

    fn request(agent: ForkableAgent) -> ForkRequest {
        ForkRequest {
            source_reference: None,
            connection_generation: 0,
            codex_daemon: Default::default(),
            parent_pane_id: "w1:p2".to_owned(),
            agent,
            session_id: "3f2b1c00-0000-4000-8000-000000000001".to_owned(),
            cwd: Some("/checkout".to_owned()),
            name: "fork-w1-p2-abc".to_owned(),
            tab_label: Some("Tab 2".to_owned()),
            placement: ForkPlacement::Tab,
            launch: Default::default(),
        }
    }

    #[test]
    fn a_claude_fork_resumes_the_parent_session_as_a_new_one() {
        let request = request(ForkableAgent::Claude);
        assert_eq!(
            request.agent.resume_arguments(&request.session_id),
            [
                "--resume",
                "3f2b1c00-0000-4000-8000-000000000001",
                "--fork-session"
            ]
        );
    }

    #[test]
    fn a_codex_fork_uses_that_agents_own_fork_subcommand() {
        let request = request(ForkableAgent::Codex);
        assert_eq!(
            request.agent.resume_arguments(&request.session_id),
            ["fork", "3f2b1c00-0000-4000-8000-000000000001"]
        );
    }

    #[test]
    fn a_pi_fork_uses_the_confirmed_native_id_and_the_installed_cli_dialect() {
        let request = request(ForkableAgent::Pi);
        assert_eq!(request.agent.kind(), "pi");
        assert_eq!(
            request.agent.resume_arguments(&request.session_id),
            ["--fork", "3f2b1c00-0000-4000-8000-000000000001"]
        );
        assert!(is_forkable(Some("pi"), Some("native-session")));
    }

    #[test]
    fn a_grok_fork_resumes_the_confirmed_native_id_as_a_new_session() {
        let request = request(ForkableAgent::Grok);
        assert_eq!(request.agent.kind(), "grok");
        assert_eq!(
            request.agent.resume_arguments(&request.session_id),
            [
                "--resume",
                "3f2b1c00-0000-4000-8000-000000000001",
                "--fork-session"
            ]
        );
        assert!(is_forkable(Some("grok"), Some("native-session")));
    }

    #[test]
    fn an_omp_fork_uses_the_confirmed_native_id_and_the_installed_cli_dialect() {
        let request = request(ForkableAgent::Omp);
        assert_eq!(request.agent.kind(), "omp");
        assert_eq!(
            request.agent.resume_arguments(&request.session_id),
            ["--fork", "3f2b1c00-0000-4000-8000-000000000001"]
        );
        assert!(is_forkable(Some("omp"), Some("native-session")));
    }

    #[test]
    fn an_opencode_fork_continues_the_confirmed_session_as_a_new_one() {
        let request = request(ForkableAgent::OpenCode);
        assert_eq!(request.agent.kind(), "opencode");
        assert_eq!(
            request.agent.resume_arguments(&request.session_id),
            ["-s", "3f2b1c00-0000-4000-8000-000000000001", "--fork"]
        );
        assert!(is_forkable(Some("OpenCode"), Some("ses_native")));
    }

    #[test]
    fn an_opencode_2_fork_opens_the_session_its_server_made_and_never_a_flag() {
        assert_eq!(
            opencode_forked_arguments("ses_fork").unwrap(),
            ["-s", "ses_fork"]
        );
        assert!(opencode_forked_arguments("--dangerously-skip-permissions").is_err());
    }

    #[test]
    fn only_agents_with_a_known_fork_command_are_forkable() {
        assert!(is_forkable(Some("claude"), Some("session")));
        assert!(is_forkable(Some("Codex"), Some("session")));
        assert!(!is_forkable(Some("gemini"), Some("session")));
        assert!(!is_forkable(None, Some("session")));
    }

    #[test]
    fn an_agent_without_a_recorded_session_is_not_forkable() {
        assert!(!is_forkable(Some("claude"), None));
        assert!(!is_forkable(Some("claude"), Some("  ")));
    }

    #[test]
    fn two_forks_of_one_parent_do_not_share_a_name() {
        assert_ne!(fork_name("w1:p2", "abc"), fork_name("w1:p2", "def"));
        assert_eq!(fork_name("w1:p2", "abc"), "fork-w1-p2-abc");
    }

    /// ⌘K draws and finds an agent by a name someone gave it, never by one
    /// Hide made up from a pane id.
    #[test]
    fn only_the_names_hide_made_from_a_pane_read_as_hide_made() {
        let long_parent = "workspace-with-a-very-long-name:pane-42";
        for made in [
            task_agent_name("claude", "w4:p1"),
            wake_name("w4:p1"),
            fork_name("w2X:p2F", "2-1788624371518"),
            fork_name(long_parent, "17-1788624371518"),
        ] {
            assert!(hide_made_name(&made, "claude", "w4:p1"), "{made:?}");
        }
        for given in [
            "observer-instant-pane-topology",
            "fork-reviewer",
            "fork-w1-p2-review",
            "hide-claude-w8-p1",
            "hide-codex-w4-p1",
        ] {
            assert!(!hide_made_name(given, "claude", "w4:p1"), "{given:?}");
        }
    }

    #[test]
    fn two_tasks_running_one_provider_get_names_herdr_accepts_and_keeps_apart() {
        let first = task_agent_name("claude", "w4:p1");
        let second = task_agent_name("claude", "w8P:p1");
        assert_eq!(first, "hide-claude-w4-p1");
        assert_ne!(first, second);
        assert!(herdr_accepts(&first) && herdr_accepts(&second));
    }

    /// Herdr's own rule, from the error it answers an unacceptable name with:
    /// `agent name must start with a lowercase letter and contain only
    /// lowercase letters, digits, '-' or '_' (1-32 characters)`.
    fn herdr_accepts(name: &str) -> bool {
        let length = name.chars().count();
        (1..=32).contains(&length)
            && name.starts_with(|character: char| character.is_ascii_lowercase())
            && name.chars().all(|character| {
                character.is_ascii_lowercase()
                    || character.is_ascii_digit()
                    || character == '-'
                    || character == '_'
            })
    }

    /// R12. A pane id is not required to be lowercase. `w2X:p2F` is an
    /// ordinary one, and the name it used to produce was refused by Herdr with
    /// `invalid_agent_name`, so the fork made no pane and the operator saw
    /// only the failure. Every fork in the run that shipped this happened to
    /// be from a lowercase pane id, which is why none of them hit it.
    #[test]
    fn a_pane_id_with_capitals_still_makes_a_name_herdr_accepts() {
        // Both were refused in the field: a capital in the pane part, and a
        // capital in the workspace part.
        for (pane_id, nonce, expected) in [
            ("w2X:p2F", "2-1788624371518", "fork-w2x-p2f-2-1788624371518"),
            ("w4G:pQ", "1-1788624890761", "fork-w4g-pq-1-1788624890761"),
        ] {
            let name = fork_name(pane_id, nonce);
            assert_eq!(name, expected);
            assert!(herdr_accepts(&name), "Herdr would refuse {name:?}");
        }
    }

    /// Nothing may be added to the name afterwards: a long pane id and a long
    /// nonce have to come back inside the rule on their own, and still tell
    /// two forks apart.
    #[test]
    fn a_long_name_is_cut_to_herdrs_limit_and_still_separates_two_forks() {
        let long_pane = "workspace-with-a-very-long-name:pane-42";
        let first = fork_name(long_pane, "17-1788624371518");
        let second = fork_name(long_pane, "18-1788624371518");
        let other_pane = fork_name(
            "workspace-with-a-very-long-name:pane-43",
            "17-1788624371518",
        );

        for name in [&first, &second, &other_pane] {
            assert!(herdr_accepts(name), "Herdr would refuse {name:?}");
        }
        assert_ne!(first, second, "two forks of one parent share a name");
        assert_ne!(first, other_pane, "forks of two parents share a name");
        assert_eq!(
            fork_name(long_pane, "17-1788624371518"),
            first,
            "the name is not stable"
        );
    }
}
