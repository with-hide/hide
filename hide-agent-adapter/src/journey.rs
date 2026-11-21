//! The agent journey check's contract (`scripts/agent_journey`): which
//! journey cells each adapter declares and the launch facts a spawn of it
//! takes, projected from the declarations so the check never reads Rust
//! source and never keeps a second support table.

use crate::scene::{Door, Scene, SceneCase};
use crate::usage::Support;

#[derive(serde::Serialize)]
pub struct JourneyAdapter {
    pub id: &'static str,
    /// The agent kind `hide agent spawn --kind` takes and the engine reports.
    #[serde(rename = "agent_cli")]
    pub engine_kind: &'static str,
    /// The agent's own folder in its HOME, whose presence tells the kit the
    /// agent is set up there, as a CLI that has run once leaves it.
    pub home_folder: Vec<String>,
    /// The words that give a start its first prompt, `{}` replaced by the
    /// prompt (`LaunchDialect::prompt_form`); absent for an agent Hide cannot
    /// start.
    pub prompt_form: Option<&'static [&'static str]>,
    /// How the start command takes a model, `{}` replaced by the value;
    /// empty when the agent takes none.
    pub model_flag: &'static [&'static str],
    /// The name of each option a full-access start adds, which the `agent
    /// CLIs` lane finds in each pinned CLI's `--help`; empty when the agent
    /// asks no permission question or Hide starts none.
    pub full_access_flag: Vec<&'static str>,
    /// The name a row shows for the agent while it has no task title, which
    /// the `title` cell must see replaced (`sidebar::provider_name`).
    pub untitled: &'static str,
    /// Every journey cell, `true` when the adapter declares what it measures;
    /// an undeclared cell reads N/A, never a failure.
    pub cells: JourneyCells,
    /// How the core hands a letter to the agent at rest; absent when the
    /// letter cell is not declared.
    pub letter: Option<JourneyLetter>,
    /// Paths this no-tool busy probe can observe: a bell or a root Stop
    /// continuation. Tool-only paths need a tool-using probe of their own.
    pub busy_letter: Vec<&'static str>,
    /// All fourteen screens a letter's bell can meet, each with the door the
    /// journey opens it through and what a survey found
    /// (`AgentAdapter::scenes`); the `scene_<name>` cells are these, so the
    /// runner reads the keys, the start and the script from here and never
    /// from Rust.
    pub scenes: Vec<JourneyScene>,
    /// Private fixture Stop HTTP-hook file, with sessionId,
    /// lastAssistantMessage, stopHookActive and end_turn input semantics.
    pub stop_observer: Option<&'static [&'static str]>,
    /// The variables a start with a profile sets to name the account it runs
    /// in (`[account.vars]`), which a wrapper the journey puts around the
    /// agent keeps as the start set them; empty for an agent a profile does
    /// not move.
    pub account_vars: Vec<&'static str>,
    /// The agent's input ladder (`AgentAdapter::input`), each rung by its
    /// name, its own way in first and `typed` last: the cells check that a
    /// brief and a letter took the rung the agent declares.
    pub input: Vec<&'static str>,
    /// How a sleeping agent is woken; absent when it cannot sleep.
    pub wake: Option<JourneyWake>,
    /// Where the native title the `title` cell reads comes from, measured the
    /// way the operator makes it; absent when the cell is not declared.
    pub title: Option<JourneyTitle>,
    /// The most model requests a signed-in CI run of the journey may make
    /// for this agent, which its lane fails above: each turn the declared
    /// cells start, at most `requests_per_turn` each; absent for an agent
    /// Hide cannot start.
    pub request_budget: Option<u32>,
    pub fake_home: FakeHome,
    /// The oldest CLI version Hide puts its integration in place for
    /// (`AgentAdapter::minimum`), which the daily agent canary's record shows
    /// beside the latest release it measured.
    pub minimum_version: Option<&'static str>,
    /// The update command of each documented way of installing the CLI
    /// that Hide can tell apart (`AgentAdapter::updates`), which a screen
    /// shows an operator whose copy is older than `minimum_version`.
    pub update_commands: Vec<&'static str>,
}

/// A private HOME is admitted only with a declared key route and update policy.
/// An unavailable policy remains an explicit exception, never an inferred default.
#[derive(Clone, Copy, Debug, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum FakeHome {
    Proven {
        key_env: &'static str,
        auto_update: AutoUpdate,
        evidence: &'static str,
    },
    Exception {
        auto_update: Option<AutoUpdate>,
        reason: &'static str,
    },
}

#[derive(Clone, Copy, Debug, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AutoUpdate {
    Environment {
        name: &'static str,
        value: &'static str,
    },
    Config {
        path: &'static str,
        key: &'static str,
        value: &'static str,
    },
}

/// A token-only turn takes one model response; the rest is room for an
/// agent that answers in more than one (a tool call it chose, a retry it
/// makes itself), and a run past it means the journey's turns are wrong.
pub const REQUESTS_PER_TURN: u32 = 3;

/// The most model requests one token-only turn may take, declared for each
/// agent's CLI: `REQUESTS_PER_TURN`, but Grok 1.0.52 makes four for every
/// turn, three chat requests and one more (its title request to `grok-4.6`
/// in a new session), measured with the journey's fake model on 2026-10-18.
const fn requests_per_turn(start: super::LaunchDialect) -> u32 {
    use super::LaunchDialect::*;
    match start {
        Grok => 4,
        Claude | Codex | OpenCode | Pi | Omp | Cursor | Antigravity => REQUESTS_PER_TURN,
    }
}

/// The path a letter takes to an agent at rest, as the core chooses it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JourneyLetter {
    /// The doorbell types its line into the composer once the pane rests
    /// (`bell`).
    Bell,
    /// The agent's prompt hook hands it over with the next prompt, whoever
    /// types it: a prompt hook and no bell (the core's
    /// `mailbox::next_prompt_letters`).
    NextPrompt,
    /// Only at a tool call the model makes while it works, or through a
    /// `hide inbox` it runs: `tool_letters` and no prompt hook.
    Tool,
}

/// How the operator's Wake reaches a sleeping agent.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JourneyWake {
    /// The pane stays and `agent_wake` restarts the agent in it.
    Pane,
    /// Sleep closes the pane and leaves a sleeping session, which
    /// `wake_sleeping_session` opens in a new pane
    /// (`LaunchDialect::closes_pane_when_sleeping`).
    SleepingSession,
}

/// One scene of the contract: what the journey does to open it and what a
/// survey found there.
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct JourneyScene {
    pub scene: Scene,
    /// Absent when the agent has no such screen.
    pub door: Option<Door>,
    /// `safe`, `unsafe`, `unknown`, or `none` for a screen the agent does not
    /// have.
    pub finding: &'static str,
    /// The journey's ordinary run checks the release of a letter held in this
    /// scene (`SceneCase::release_check`).
    pub release_check: bool,
}

fn journey_scenes(row: &super::AgentAdapter) -> Vec<JourneyScene> {
    row.scenes
        .cells()
        .into_iter()
        .map(|(scene, capability)| match capability {
            Support::Supported(SceneCase {
                door,
                finding,
                release_check,
            }) => JourneyScene {
                scene,
                door: Some(*door),
                finding: finding.word(),
                release_check: *release_check,
            },
            Support::Unsupported(_) => JourneyScene {
                scene,
                door: None,
                finding: "none",
                release_check: false,
            },
        })
        .collect()
}

/// Whether the journey has a `scene_<name>` cell for this scene: the agent
/// starts, has the scene, and the journey must open it itself (a door that
/// another cell proves needs no cell of its own).
fn scene_cell(row: &super::AgentAdapter, scene: Scene) -> bool {
    row.start.is_some()
        && matches!(
            row.scenes.get(scene),
            Support::Supported(SceneCase { door, .. }) if !matches!(door, Door::Cell { .. })
        )
}

/// The native title source a `title` cell measures (with-hide/hide-private
/// docs/task-identity.md: the automatic or source-less slot of each format).
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JourneyTitle {
    /// The agent titles its session itself, with a model request of its own
    /// that the journey's model answers.
    Automatic,
    /// Only a name the operator gives the session with this command: Pi's
    /// `session_info.name`, which its `/name` writes.
    Named(&'static str),
}

#[derive(serde::Serialize)]
pub struct JourneyCells {
    /// Started through `hide agent spawn`, in a folder the account trusts.
    pub start: bool,
    /// Started in a folder the account never trusted.
    pub start_untrusted: bool,
    /// A brief that the start carries as its prompt argument
    /// (`AgentApi::LaunchPrompt`), of the forms a command line and an option
    /// parser can take apart: many lines, a tab, a first character `-` or
    /// `@`, up to the brief's cap, recorded by the agent once and exactly,
    /// and never typed.
    pub launch_brief: bool,
    /// The first turn's reply, read by Hide's conversation reader.
    pub reply: bool,
    /// Hide's own hook integration reports the session.
    pub integration: bool,
    /// The start guidance and Memory reach the first turn.
    pub guidance_memory: bool,
    /// A letter reaches the agent and its reply.
    pub letter: bool,
    /// A letter sent while the agent works on a turn reaches it once, after
    /// that turn, which ends with its own answer: through a bell or a
    /// declared turn-end hook, without asking the model to use a tool.
    pub letter_busy: bool,
    /// The scenes the journey opens itself, one cell each, in the order of
    /// `Scene::ALL`: `rest` and `working` are the `letter` and `letter_busy`
    /// cells, so they have none. A cell is declared when the agent has the
    /// scene through a door that is not one of those cells; what it does
    /// depends on the door and the run's `--scenes` mode
    /// (`docs/VERIFICATION.md#agent-scenes`).
    pub scene_rest_draft: bool,
    pub scene_interrupted: bool,
    pub scene_session_picker: bool,
    pub scene_model_picker: bool,
    pub scene_popup: bool,
    pub scene_tool_approval: bool,
    pub scene_question: bool,
    pub scene_plan_approval: bool,
    pub scene_trust_dialog: bool,
    pub scene_hooks_review: bool,
    pub scene_first_run: bool,
    pub scene_update_notice: bool,
    /// Typed, erased, pasted and cleared text reads as the composer's own
    /// verdict on its screen. It starts no model turn.
    pub composer: bool,
    pub sleep_wake: bool,
    /// Wake keeps the native session.
    pub resume: bool,
    pub fork: bool,
    pub title: bool,
    pub usage: bool,
    /// A start with a profile whose account is not the agent's default (PRD
    /// agent-profiles B19): its first turn, a letter and a Wake, each read
    /// from that account. Declared for an agent a profile moves to an
    /// account of its own (`[account] by` a folder or a name).
    pub profile: bool,
}

/// The current probe proves context through a private Memory-card token.
/// A guidance-only agent needs its own probe before its declaration can ship;
/// never turn that supported context path into an undeclared N/A cell.
fn context_cell(row: &super::AgentAdapter) -> bool {
    let start = row.start.is_some();
    let guidance = row.guidance.is_some();
    let memory = row.memory.is_some();
    assert!(
        !start || !guidance || memory,
        "{}: guidance-only delivery needs a journey probe; the Memory-card probe cannot measure it",
        row.id
    );
    start && (guidance || memory)
}

fn busy_letter_paths(row: &super::AgentAdapter) -> Vec<&'static str> {
    let mut paths = Vec::new();
    if row.bell {
        paths.push(if row.tool_letters.is_some() {
            "bell_hook"
        } else {
            "bell"
        });
    }
    if row.turn_end_letters.is_some() {
        paths.push("turn_end");
    }
    paths
}

/// How many scenes the journey's ordinary run checks the release of: each
/// answers one letter, so each is one turn of the request budget.
fn release_checked_scenes(row: &super::AgentAdapter) -> u32 {
    Scene::ALL
        .into_iter()
        .filter(|&scene| {
            scene_cell(row, scene)
                && matches!(
                    row.scenes.get(scene),
                    Support::Supported(SceneCase {
                        release_check: true,
                        ..
                    })
                )
        })
        .count() as u32
}

/// Whether a profile can run the agent in an account of its own: its
/// definition moves the account by a folder or a name.
fn moves_account(row: &super::AgentAdapter) -> bool {
    use crate::definition::By;
    row.account_rule()
        .is_some_and(|rule| matches!(rule.by, By::Folder | By::Name))
}

pub fn journey_contract() -> impl Iterator<Item = JourneyAdapter> {
    super::ADAPTERS.iter().map(|row| {
        let start = row.start.is_some();
        let cells = JourneyCells {
            start,
            start_untrusted: start,
            launch_brief: start && row.launch_prompt(),
            reply: start && row.conversation.is_some(),
            integration: start,
            guidance_memory: context_cell(row),
            letter: start && (row.bell || row.tool_letters.is_some()),
            letter_busy: start && !busy_letter_paths(row).is_empty(),
            scene_rest_draft: scene_cell(row, Scene::RestDraft),
            scene_interrupted: scene_cell(row, Scene::Interrupted),
            scene_session_picker: scene_cell(row, Scene::SessionPicker),
            scene_model_picker: scene_cell(row, Scene::ModelPicker),
            scene_popup: scene_cell(row, Scene::Popup),
            scene_tool_approval: scene_cell(row, Scene::ToolApproval),
            scene_question: scene_cell(row, Scene::Question),
            scene_plan_approval: scene_cell(row, Scene::PlanApproval),
            scene_trust_dialog: scene_cell(row, Scene::TrustDialog),
            scene_hooks_review: scene_cell(row, Scene::HooksReview),
            scene_first_run: scene_cell(row, Scene::FirstRun),
            scene_update_notice: scene_cell(row, Scene::UpdateNotice),
            composer: start && row.composer.is_some(),
            sleep_wake: start && row.sleep.is_some(),
            resume: start && row.sleep.is_some() && row.resume.is_some(),
            fork: start && row.fork.is_some(),
            title: start && row.titles.is_some(),
            usage: start && matches!(row.usage.tokens, super::usage::Support::Supported(_)),
            profile: start && moves_account(row),
        };
        // The turns a run starts: the first prompt in the trusted and the
        // never-trusted start, the launch brief's start, the question about the Memory card the start
        // gave, the letter, the prompt typed into the fork (an agent that
        // tells its session only at its first turn, as Codex does, is
        // registered as a fork only then, and the operator types into a fork
        // to use it) and the prompt typed after Wake. The other cells judge
        // what Hide shows of those turns and start none; the composer cell
        // types text it never submits.
        // The busy letter starts two: the turn it waits behind and its own.
        // Each scene whose release the ordinary run checks starts one: the
        // letter held while it was open, answered once it is closed.
        // The launch brief starts one, in a start of its own. The profile's
        // start, letter and Wake start one each, in an account of their own.
        let turns = [
            start,
            cells.start_untrusted,
            cells.launch_brief,
            cells.guidance_memory,
            cells.letter,
            cells.letter_busy,
            cells.letter_busy,
            cells.fork,
            cells.resume,
            cells.profile,
            cells.profile && cells.letter,
            cells.profile && cells.resume,
        ]
        .into_iter()
        .filter(|&turn| turn)
        .count() as u32
            + release_checked_scenes(row);
        let letter = cells.letter.then(|| {
            if row.bell {
                JourneyLetter::Bell
            } else if row.prompt_hook.is_some() {
                JourneyLetter::NextPrompt
            } else {
                JourneyLetter::Tool
            }
        });
        let wake = cells.sleep_wake.then(|| match row.sleep {
            Some(dialect) if dialect.closes_pane_when_sleeping() => JourneyWake::SleepingSession,
            _ => JourneyWake::Pane,
        });
        let title = row
            .titles
            .supported()
            .filter(|_| cells.title)
            .map(|format| match format {
                super::SessionFormat::Pi => JourneyTitle::Named("/name"),
                super::SessionFormat::Claude
                | super::SessionFormat::Codex
                | super::SessionFormat::Grok
                | super::SessionFormat::Omp
                | super::SessionFormat::Cursor
                | super::SessionFormat::OpenCode
                | super::SessionFormat::Antigravity => JourneyTitle::Automatic,
            });
        JourneyAdapter {
            scenes: journey_scenes(row),
            id: row.id,
            fake_home: row.fake_home,
            engine_kind: row.engine_kind,
            home_folder: row
                .account_rule()
                .and_then(|rule| crate::default_folder(rule, crate::AccountPath::State))
                .expect("a built-in agent with Hide's features names its own folder"),
            prompt_form: row.start.map(|start| start.prompt_form()),
            model_flag: row.start.map_or(&[], |start| start.options().model),
            full_access_flag: row
                .start
                .map_or_else(Vec::new, |start| start.options().full_access.names()),
            untitled: row.sidebar_label.unwrap_or(row.engine_kind),
            cells,
            request_budget: row.start.map(|start| turns * requests_per_turn(start)),
            minimum_version: row.minimum.map(|minimum| minimum.version),
            update_commands: row.updates.iter().map(|route| route.command).collect(),
            letter,
            busy_letter: busy_letter_paths(row),
            stop_observer: row.turn_end_letters.map(|dialect| match dialect {
                super::HookDialect::Grok => &[".grok", "hooks", "journey-stop-observer.json"][..],
                _ => panic!("turn-end journey needs a declared Stop observer dialect"),
            }),
            input: row.input.iter().map(|rung| rung.name()).collect(),
            account_vars: row
                .account_rule()
                .filter(|_| moves_account(row))
                .map_or_else(Vec::new, |rule| {
                    rule.vars.iter().map(|var| var.name.as_str()).collect()
                }),
            wake,
            title,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentId, CapabilityAbsence, usage::Support};

    const ABSENT: CapabilityAbsence = CapabilityAbsence {
        reason: "This host fixture does not deliver that context.",
        evidence: "docs/agent-hooks.md",
        would_change: "Implement and measure the host's delivery path.",
    };

    #[test]
    fn busy_probe_paths_follow_capabilities_not_agent_names() {
        let mut row = *AgentId::Codex.adapter();
        assert_eq!(busy_letter_paths(&row), ["bell"]);
        row.turn_end_letters = AgentId::Grok.adapter().turn_end_letters;
        assert_eq!(busy_letter_paths(&row), ["bell", "turn_end"]);
        row.tool_letters = AgentId::Grok.adapter().tool_letters;
        assert_eq!(busy_letter_paths(&row), ["bell_hook", "turn_end"]);
        row.bell = false;
        assert_eq!(busy_letter_paths(&row), ["turn_end"]);
        row.turn_end_letters = None;
        assert!(busy_letter_paths(&row).is_empty());
    }

    #[test]
    fn supported_context_never_silently_becomes_an_undeclared_cell() {
        let mut row = *AgentId::Codex.adapter();
        assert!(context_cell(&row));
        row.guidance = Support::Unsupported(ABSENT);
        assert!(context_cell(&row), "Memory-only delivery is still measured");
        row.memory = Support::Unsupported(ABSENT);
        assert!(!context_cell(&row), "neither path is supported");
    }

    #[test]
    #[should_panic(expected = "guidance-only delivery needs a journey probe")]
    fn guidance_only_support_requires_a_probe_instead_of_a_silent_na() {
        let mut row = *AgentId::Codex.adapter();
        row.memory = Support::Unsupported(ABSENT);
        context_cell(&row);
    }

    #[test]
    fn a_scene_cell_follows_the_declaration_never_an_agent_name() {
        // The cell the Codex-only `letter_menu` was is now every agent's
        // `scene_model_picker`: it exists wherever the agent declares the
        // picker through keys, and goes when the declaration does.
        for row in crate::ADAPTERS {
            let cells = journey_contract()
                .find(|adapter| adapter.id == row.id)
                .unwrap()
                .cells;
            assert!(
                cells.scene_model_picker,
                "{}: declares the model picker through keys",
                row.id
            );
        }
        let mut row = *AgentId::Codex.adapter();
        assert!(scene_cell(&row, Scene::ModelPicker));
        row.scenes.model_picker = Support::Unsupported(ABSENT);
        assert!(!scene_cell(&row, Scene::ModelPicker));
        assert!(
            scene_cell(&row, Scene::Popup),
            "a scene the declaration keeps keeps its cell"
        );
        row.start = None;
        assert!(
            !scene_cell(&row, Scene::Popup),
            "an agent Hide cannot start has no scene cell"
        );
    }

    #[test]
    fn a_door_another_cell_proves_has_no_cell_of_its_own() {
        let mut row = *AgentId::Codex.adapter();
        for scene in [Scene::Rest, Scene::Working] {
            assert!(
                !scene_cell(&row, scene),
                "{}: proven by `letter` or `letter_busy`",
                scene.name()
            );
        }
        row.scenes.popup = row.scenes.rest;
        assert!(
            !scene_cell(&row, Scene::Popup),
            "a door that is another cell is that cell's"
        );
    }

    #[test]
    fn the_contract_lists_all_fourteen_scenes_with_a_door_or_an_absence() {
        for adapter in journey_contract() {
            let names: Vec<_> = adapter.scenes.iter().map(|case| case.scene).collect();
            assert_eq!(names, Scene::ALL, "{}", adapter.id);
            for case in &adapter.scenes {
                assert_eq!(
                    case.door.is_none(),
                    case.finding == "none",
                    "{} {}: a door and an absence are exclusive",
                    adapter.id,
                    case.scene.name()
                );
            }
        }
    }

    #[test]
    fn only_a_scene_with_a_release_check_adds_a_turn_to_the_request_budget() {
        let mut row = *AgentId::Codex.adapter();
        assert_eq!(release_checked_scenes(&row), 1);
        row.scenes.model_picker = row.scenes.popup;
        assert_eq!(
            release_checked_scenes(&row),
            0,
            "the same door without the check adds no turn"
        );
        assert_eq!(release_checked_scenes(AgentId::ClaudeCode.adapter()), 0);
    }
}
