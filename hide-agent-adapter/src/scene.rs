//! Every screen a letter's bell can meet, declared the same way for every
//! agent (`docs/VERIFICATION.md#agent-scenes`).
//!
//! A screen is a [`Scene`]; an agent declares each of the fourteen in one
//! named field of [`SceneDeclaration`], so a row that leaves one out does not
//! compile. A declared scene is a [`Door`] the journey opens it through and
//! the [`Finding`] a survey recorded for it; a scene the agent does not have
//! is an absence with its reason, evidence and what would change it, the
//! type `guidance` and `memory` already use. The journey's `scene_<name>`
//! cells and the contract tests are projections of this one declaration.

use crate::{Capability, usage::Support};

/// Every screen a letter's bell can meet. The same list for every agent.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Scene {
    /// The agent rests with an empty composer.
    Rest,
    /// The composer holds text the operator was typing.
    RestDraft,
    /// A turn runs.
    Working,
    /// The operator stopped a turn and the agent restored its prompt.
    Interrupted,
    /// The picker that opens another session.
    SessionPicker,
    /// The picker that names a model.
    ModelPicker,
    /// The list a `/` or `@` opens over the composer.
    Popup,
    /// A tool call waits for the operator's approval.
    ToolApproval,
    /// The agent's question tool waits for an answer.
    Question,
    /// A plan waits for the operator's approval.
    PlanApproval,
    /// The first-start question whether to trust the folder.
    TrustDialog,
    /// The first-start review of Hide's hooks.
    HooksReview,
    /// The first-start setup or sign-in.
    FirstRun,
    /// The notice that a newer version exists.
    UpdateNotice,
}

impl Scene {
    /// In the order the guides and the journey list them.
    pub const ALL: [Scene; 14] = [
        Scene::Rest,
        Scene::RestDraft,
        Scene::Working,
        Scene::Interrupted,
        Scene::SessionPicker,
        Scene::ModelPicker,
        Scene::Popup,
        Scene::ToolApproval,
        Scene::Question,
        Scene::PlanApproval,
        Scene::TrustDialog,
        Scene::HooksReview,
        Scene::FirstRun,
        Scene::UpdateNotice,
    ];

    /// The name the contract, the journey's `scene_<name>` cell and the
    /// guides use.
    pub const fn name(self) -> &'static str {
        match self {
            Scene::Rest => "rest",
            Scene::RestDraft => "rest_draft",
            Scene::Working => "working",
            Scene::Interrupted => "interrupted",
            Scene::SessionPicker => "session_picker",
            Scene::ModelPicker => "model_picker",
            Scene::Popup => "popup",
            Scene::ToolApproval => "tool_approval",
            Scene::Question => "question",
            Scene::PlanApproval => "plan_approval",
            Scene::TrustDialog => "trust_dialog",
            Scene::HooksReview => "hooks_review",
            Scene::FirstRun => "first_run",
            Scene::UpdateNotice => "update_notice",
        }
    }
}

/// How the journey puts one agent on one scene.
#[derive(Clone, Copy, Debug, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Door {
    /// Proven by a journey cell that already exists: `letter`, `letter_busy`.
    Cell { cell: &'static str },
    /// Keys typed into the pane, from rest or from a held turn; `close`
    /// leaves the scene without choosing anything.
    Keys {
        from: Stage,
        open: &'static [Key],
        close: &'static [Key],
    },
    /// A start that meets the screen on its own.
    Start { start: StartKind },
    /// A turn whose model call is scripted: a shell command, an edit, a
    /// question, a plan.
    Turn { script: Script },
}

impl Door {
    pub const fn cell(cell: &'static str) -> Self {
        Door::Cell { cell }
    }

    pub const fn start(start: StartKind) -> Self {
        Door::Start { start }
    }

    pub const fn turn(script: Script) -> Self {
        Door::Turn { script }
    }

    /// The word the contract carries for the door's kind.
    pub const fn kind(&self) -> &'static str {
        match self {
            Door::Cell { .. } => "cell",
            Door::Keys { .. } => "keys",
            Door::Start { .. } => "start",
            Door::Turn { .. } => "turn",
        }
    }
}

/// Where the pane is when the keys of a [`Door::Keys`] are typed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Settled at rest, its composer checked.
    Rest,
    /// A held turn is running.
    Working,
}

/// A key the journey sends, by the names `hide pane send --key` takes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "key", rename_all = "snake_case")]
pub enum Key {
    Text {
        text: &'static str,
    },
    Enter,
    Esc,
    Tab,
    ShiftTab,
    Up,
    Down,
    /// Erases the composer's text with Backspace until it reads empty, the
    /// way the `composer` cell does.
    ClearComposer,
}

impl Key {
    pub const fn text(text: &'static str) -> Self {
        Key::Text { text }
    }
}

/// A start of the agent that meets a screen by itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "kind", content = "option", rename_all = "snake_case")]
pub enum StartKind {
    /// A folder the account never trusted.
    UntrustedFolder,
    /// Hide's hooks installed and the agent's own review of them not given.
    UnreviewedHooks,
    /// An account folder that never ran the agent.
    EmptyAccount,
    /// The agent's check for a newer version left on.
    UpdateCheckOn,
    /// A start with an option of the agent's own.
    Option(&'static str),
}

/// The model call a [`Door::Turn`] scripts.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Script {
    Shell,
    Edit,
    Ask,
    Plan,
}

/// What the engine reads in a scene, as a survey recorded it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineRead {
    Idle,
    Done,
    Blocked,
    Working,
    Unknown,
    Starting,
}

/// What held a letter in a scene, as a survey recorded it; a clock is never
/// one, because a clock only delays the line.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Guard {
    /// The engine reads the screen `blocked`.
    EngineBlocked,
    /// The composer reads as not clear before the bell types.
    ComposerNotClear,
    /// The session reader says a question or plan approval waits.
    NativeWait,
    /// Hide routed a key after the last submission.
    DraftRecord,
    /// The engine reports the agent launching or not ready.
    NotReady,
    /// The scene has a prompt and the line was one prompt.
    OnePrompt,
}

/// Where and when a survey saw a scene, copied into the declaration.
#[derive(Clone, Copy, Debug)]
pub struct Seen {
    /// The CLI version that ran; equal to the pin for a safe scene.
    pub version: &'static str,
    /// `YYYY-MM-DD`.
    pub on: &'static str,
    pub engine: EngineRead,
    pub held_by: Guard,
    /// One line: what the line did, or why it was held.
    pub why: &'static str,
    /// A repository anchor that says how it was measured.
    pub procedure: &'static str,
}

/// What a survey found in one scene of one agent.
#[derive(Clone, Copy, Debug)]
pub enum Finding {
    /// The line was held or was one prompt, through a structural guard,
    /// three times.
    Safe(Seen),
    /// The line answered the screen or was left in it.
    Unsafe {
        seen: Seen,
        closes_with: &'static str,
    },
    /// Not surveyed, or the survey could not decide.
    Unknown {
        why: &'static str,
        closes_with: &'static str,
    },
}

impl Finding {
    /// The word the contract and the guides carry.
    pub const fn word(&self) -> &'static str {
        match self {
            Finding::Safe(_) => "safe",
            Finding::Unsafe { .. } => "unsafe",
            Finding::Unknown { .. } => "unknown",
        }
    }

    pub const fn is_safe(&self) -> bool {
        matches!(self, Finding::Safe(_))
    }
}

/// One scene an agent has: its door and what a survey found.
#[derive(Clone, Copy, Debug)]
pub struct SceneCase {
    pub door: Door,
    pub finding: Finding,
    /// The journey's ordinary run also checks the release of a letter held
    /// while this scene is open (what the `letter_menu` cell proved): held
    /// while the scene stands, delivered once after it is closed. Only a
    /// scene opened by keys has one. It is a check of Hide's draft record,
    /// not a finding, so an `Unknown` scene can carry it.
    pub release_check: bool,
}

/// One field per scene and no default: a new agent that leaves one out does
/// not compile, and a new scene is a compile error until every agent names it.
#[derive(Clone, Copy, Debug)]
pub struct SceneDeclaration {
    pub rest: Capability<SceneCase>,
    pub rest_draft: Capability<SceneCase>,
    pub working: Capability<SceneCase>,
    pub interrupted: Capability<SceneCase>,
    pub session_picker: Capability<SceneCase>,
    pub model_picker: Capability<SceneCase>,
    pub popup: Capability<SceneCase>,
    pub tool_approval: Capability<SceneCase>,
    pub question: Capability<SceneCase>,
    pub plan_approval: Capability<SceneCase>,
    pub trust_dialog: Capability<SceneCase>,
    pub hooks_review: Capability<SceneCase>,
    pub first_run: Capability<SceneCase>,
    pub update_notice: Capability<SceneCase>,
}

impl SceneDeclaration {
    /// Destructures every field, so a field added without a place here does
    /// not compile.
    pub const fn cells(&self) -> [(Scene, &Capability<SceneCase>); 14] {
        let Self {
            rest,
            rest_draft,
            working,
            interrupted,
            session_picker,
            model_picker,
            popup,
            tool_approval,
            question,
            plan_approval,
            trust_dialog,
            hooks_review,
            first_run,
            update_notice,
        } = self;
        [
            (Scene::Rest, rest),
            (Scene::RestDraft, rest_draft),
            (Scene::Working, working),
            (Scene::Interrupted, interrupted),
            (Scene::SessionPicker, session_picker),
            (Scene::ModelPicker, model_picker),
            (Scene::Popup, popup),
            (Scene::ToolApproval, tool_approval),
            (Scene::Question, question),
            (Scene::PlanApproval, plan_approval),
            (Scene::TrustDialog, trust_dialog),
            (Scene::HooksReview, hooks_review),
            (Scene::FirstRun, first_run),
            (Scene::UpdateNotice, update_notice),
        ]
    }

    pub const fn get(&self, scene: Scene) -> &Capability<SceneCase> {
        match scene {
            Scene::Rest => &self.rest,
            Scene::RestDraft => &self.rest_draft,
            Scene::Working => &self.working,
            Scene::Interrupted => &self.interrupted,
            Scene::SessionPicker => &self.session_picker,
            Scene::ModelPicker => &self.model_picker,
            Scene::Popup => &self.popup,
            Scene::ToolApproval => &self.tool_approval,
            Scene::Question => &self.question,
            Scene::PlanApproval => &self.plan_approval,
            Scene::TrustDialog => &self.trust_dialog,
            Scene::HooksReview => &self.hooks_review,
            Scene::FirstRun => &self.first_run,
            Scene::UpdateNotice => &self.update_notice,
        }
    }

    /// Whether a bell is safe on every screen the agent shows: `rest` is
    /// declared and safe, and every other scene is safe or one the agent does
    /// not have. An unknown or unsafe scene keeps it off, which is what
    /// `docs/delivery.md` says an unknown screen means.
    ///
    /// Nothing calls this yet: every finding is unknown until a survey closes
    /// it, so reading the bell from the findings now would switch off the
    /// bell of every agent that rings today. The table node switches the
    /// adapter's `bell` over to it once the findings are measured.
    pub const fn bell_from_findings(&self) -> bool {
        let mut at = 0;
        let cells = self.cells();
        let mut rest_safe = false;
        while at < cells.len() {
            let (scene, capability) = cells[at];
            let clear = match capability {
                Support::Supported(case) => case.finding.is_safe(),
                Support::Unsupported(_) => !matches!(scene, Scene::Rest),
            };
            if !clear {
                return false;
            }
            if matches!(scene, Scene::Rest) {
                rest_safe = true;
            }
            at += 1;
        }
        rest_safe
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CapabilityAbsence;

    const GONE: CapabilityAbsence = CapabilityAbsence {
        reason: "This host fixture has no such screen.",
        evidence: "docs/agent-hooks.md",
        would_change: "A version that adds it.",
    };

    const SEEN: Seen = Seen {
        version: "1.0.0",
        on: "2026-11-11",
        engine: EngineRead::Blocked,
        held_by: Guard::EngineBlocked,
        why: "held",
        procedure: "docs/VERIFICATION.md#agent-scenes",
    };

    const SAFE: Capability<SceneCase> = Support::Supported(SceneCase {
        door: Door::cell("letter"),
        finding: Finding::Safe(SEEN),
        release_check: false,
    });

    const UNKNOWN: Capability<SceneCase> = Support::Supported(SceneCase {
        door: Door::cell("letter"),
        finding: Finding::Unknown {
            why: "not surveyed",
            closes_with: "a survey",
        },
        release_check: false,
    });

    const ALL_SAFE: SceneDeclaration = SceneDeclaration {
        rest: SAFE,
        rest_draft: SAFE,
        working: SAFE,
        interrupted: SAFE,
        session_picker: SAFE,
        model_picker: SAFE,
        popup: SAFE,
        tool_approval: SAFE,
        question: SAFE,
        plan_approval: SAFE,
        trust_dialog: SAFE,
        hooks_review: SAFE,
        first_run: SAFE,
        update_notice: SAFE,
    };

    #[test]
    fn scene_names_are_unique_and_match_the_contract_spelling() {
        let mut seen = std::collections::BTreeSet::new();
        for scene in Scene::ALL {
            assert!(seen.insert(scene.name()), "duplicate {}", scene.name());
            assert_eq!(
                serde_json::to_value(scene).unwrap(),
                scene.name(),
                "the serialized name is the journey cell's"
            );
        }
        assert_eq!(seen.len(), 14);
    }

    #[test]
    fn cells_lists_every_scene_once_in_table_order_and_get_agrees() {
        let declared: Vec<Scene> = ALL_SAFE.cells().iter().map(|(scene, _)| *scene).collect();
        assert_eq!(declared, Scene::ALL);
        for (scene, field) in ALL_SAFE.cells() {
            assert!(
                std::ptr::eq(ALL_SAFE.get(scene), field),
                "{}: get and cells name different fields",
                scene.name()
            );
        }
    }

    #[test]
    fn the_bell_follows_the_findings_and_only_safe_or_absent_scenes_clear_it() {
        assert!(ALL_SAFE.bell_from_findings());
        // A screen the agent does not have does not count.
        let gone = Support::Unsupported(GONE);
        assert!(
            SceneDeclaration {
                hooks_review: gone,
                trust_dialog: gone,
                ..ALL_SAFE
            }
            .bell_from_findings()
        );
        // An unknown scene keeps the bell off, so does an unsafe one.
        assert!(
            !SceneDeclaration {
                popup: UNKNOWN,
                ..ALL_SAFE
            }
            .bell_from_findings()
        );
        let unsafe_case = Support::Supported(SceneCase {
            door: Door::cell("letter"),
            finding: Finding::Unsafe {
                seen: SEEN,
                closes_with: "a composer declaration",
            },
            release_check: false,
        });
        assert!(
            !SceneDeclaration {
                model_picker: unsafe_case,
                ..ALL_SAFE
            }
            .bell_from_findings()
        );
        // An agent with no `rest` has nothing a bell could type into.
        assert!(
            !SceneDeclaration {
                rest: Support::Unsupported(GONE),
                ..ALL_SAFE
            }
            .bell_from_findings()
        );
    }

    #[test]
    fn no_agent_rings_by_findings_before_a_survey_has_closed_them() {
        // Every finding is unknown today, which is why the adapter's `bell`
        // is still declared by hand: reading it from the findings now would
        // silence the agents that ring.
        for row in crate::ADAPTERS {
            assert!(
                !row.scenes.bell_from_findings(),
                "{}: a finding closed before the survey that decides it; switch `bell` to the findings in the same change",
                row.id
            );
        }
    }
}
