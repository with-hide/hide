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

    /// The heading of the scene's column in the support table.
    pub const fn column(self) -> &'static str {
        match self {
            Scene::Rest => "rest",
            Scene::RestDraft => "draft",
            Scene::Working => "working",
            Scene::Interrupted => "interrupted",
            Scene::SessionPicker => "session",
            Scene::ModelPicker => "model",
            Scene::Popup => "popup",
            Scene::ToolApproval => "approval",
            Scene::Question => "question",
            Scene::PlanApproval => "plan",
            Scene::TrustDialog => "trust",
            Scene::HooksReview => "hooks",
            Scene::FirstRun => "first run",
            Scene::UpdateNotice => "update",
        }
    }

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

impl EngineRead {
    /// The letter the support table draws.
    pub const fn letter(self) -> char {
        match self {
            EngineRead::Idle => 'I',
            EngineRead::Done => 'D',
            EngineRead::Blocked => 'B',
            EngineRead::Working => 'W',
            EngineRead::Unknown => '?',
            EngineRead::Starting => 'S',
        }
    }
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
    /// No native session is registered, so a letter is refused before any
    /// bell.
    NoSession,
    /// The scene has a prompt and the line was one prompt.
    OnePrompt,
}

/// Where and when a survey looked at a scene, whatever it found.
#[derive(Clone, Copy, Debug)]
pub struct Looked {
    /// The CLI version that ran; equal to the pin for a safe scene.
    pub version: &'static str,
    /// `YYYY-MM-DD`.
    pub on: &'static str,
    pub engine: EngineRead,
}

/// What a survey saw in a scene it could judge.
#[derive(Clone, Copy, Debug)]
pub struct Seen {
    pub looked: Looked,
    /// What held the line while the scene was open; `None` when nothing did
    /// and the line was typed into it.
    pub held_by: Option<Guard>,
    /// One line: what the line did, or why it was held.
    pub why: &'static str,
    /// A repository anchor that says how it was measured.
    pub procedure: &'static str,
}

/// What a survey found in one scene of one agent.
#[derive(Clone, Copy, Debug)]
pub enum Finding {
    /// The line was held or was one prompt, through a structural guard,
    /// and nothing was left behind.
    Safe(Seen),
    /// The line answered the screen, was left in it, or never arrived.
    Unsafe {
        seen: Seen,
        closes_with: &'static str,
    },
    /// Not surveyed, or the survey could not decide. `looked` is where a
    /// survey tried, and `None` for a scene no survey has tried.
    Unknown {
        looked: Option<Looked>,
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

    /// Where and when a survey looked, when one did.
    pub const fn looked(&self) -> Option<&Looked> {
        match self {
            Finding::Safe(seen) | Finding::Unsafe { seen, .. } => Some(&seen.looked),
            Finding::Unknown { looked, .. } => looked.as_ref(),
        }
    }

    /// What happened, or why nothing is known.
    pub const fn why(&self) -> &'static str {
        match self {
            Finding::Safe(seen) | Finding::Unsafe { seen, .. } => seen.why,
            Finding::Unknown { why, .. } => why,
        }
    }

    /// What would close the finding, for one that is not safe.
    pub const fn closes_with(&self) -> Option<&'static str> {
        match self {
            Finding::Safe(_) => None,
            Finding::Unsafe { closes_with, .. } | Finding::Unknown { closes_with, .. } => {
                Some(closes_with)
            }
        }
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
}

/// What `docs/INSTALL.md` carries between its `bell-scenes` markers: every
/// agent against every scene, the findings that are not safe, where and when
/// each agent was looked at, what the engine read, and the screens an agent
/// does not have. It is read from the declarations, so nobody edits it by
/// hand and the adapter contract compares it byte for byte
/// (`export_scene_table --markdown` writes it).
pub fn table_markdown() -> String {
    let mut out = String::new();
    let columns: Vec<&str> = Scene::ALL.iter().map(|scene| scene.column()).collect();
    let divider = |count: usize| format!("|{}\n", " --- |".repeat(count));

    out.push_str(&format!("| Agent | {} | Bell |\n", columns.join(" | ")));
    out.push_str(&divider(columns.len() + 2));
    for row in crate::ADAPTERS {
        let values: Vec<&str> = row
            .scenes
            .cells()
            .iter()
            .map(|(_, capability)| match capability {
                Support::Supported(case) => case.finding.word(),
                Support::Unsupported(_) => "none",
            })
            .collect();
        out.push_str(&format!(
            "| {} | {} | {} |\n",
            row.label,
            values.join(" | "),
            if row.bell { "on" } else { "off" }
        ));
    }

    out.push_str("\n#### Hazards and gaps\n\n");
    out.push_str("| Agent | Screen | Value | What happened, or why it is not known | Version | Date | Closes with |\n");
    out.push_str(&divider(7));
    for row in crate::ADAPTERS {
        for (scene, capability) in row.scenes.cells() {
            let Support::Supported(case) = capability else {
                continue;
            };
            let Some(closes_with) = case.finding.closes_with() else {
                continue;
            };
            let (version, on) = case
                .finding
                .looked()
                .map_or(("-", "-"), |looked| (looked.version, looked.on));
            out.push_str(&format!(
                "| {} | `{}` | {} | {} | {} | {} | {} |\n",
                row.label,
                scene.name(),
                case.finding.word(),
                cell_text(case.finding.why()),
                version,
                on,
                cell_text(closes_with),
            ));
        }
    }

    out.push_str("\n#### Seen on\n\n");
    out.push_str("| Agent | Version and date of each finding |\n");
    out.push_str(&divider(2));
    for row in crate::ADAPTERS {
        let mut seen: Vec<(&str, &str)> = Vec::new();
        for (_, capability) in row.scenes.cells() {
            if let Support::Supported(case) = capability
                && let Some(looked) = case.finding.looked()
                && !seen.contains(&(looked.version, looked.on))
            {
                seen.push((looked.version, looked.on));
            }
        }
        seen.sort_unstable();
        let text: Vec<String> = seen
            .iter()
            .map(|(version, on)| format!("{version} ({on})"))
            .collect();
        out.push_str(&format!("| {} | {} |\n", row.label, text.join(", ")));
    }

    out.push_str("\n#### What the engine read\n\n");
    out.push_str("`I` idle, `D` done, `B` blocked, `W` working, `S` starting, `?` not read, `-` no such screen.\n\n");
    out.push_str(&format!("| Agent | {} |\n", columns.join(" | ")));
    out.push_str(&divider(columns.len() + 1));
    for row in crate::ADAPTERS {
        let letters: Vec<String> = row
            .scenes
            .cells()
            .iter()
            .map(|(_, capability)| match capability {
                Support::Supported(case) => case
                    .finding
                    .looked()
                    .map_or('?', |looked| looked.engine.letter())
                    .to_string(),
                Support::Unsupported(_) => "-".to_owned(),
            })
            .collect();
        out.push_str(&format!("| {} | {} |\n", row.label, letters.join(" | ")));
    }

    out.push_str("\n#### Screens an agent does not have\n\n");
    out.push_str("| Agent | Screen | Why | What would change it |\n");
    out.push_str(&divider(4));
    for row in crate::ADAPTERS {
        for (scene, capability) in row.scenes.cells() {
            if let Support::Unsupported(absence) = capability {
                out.push_str(&format!(
                    "| {} | `{}` | {} | {} |\n",
                    row.label,
                    scene.name(),
                    cell_text(absence.reason),
                    cell_text(absence.would_change),
                ));
            }
        }
    }
    out
}

/// A sentence in a table cell: one line, and no pipe to end the cell early.
fn cell_text(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
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

    const LOOKED: Looked = Looked {
        version: "1.0.0",
        on: "2026-11-11",
        engine: EngineRead::Blocked,
    };

    const SEEN: Seen = Seen {
        looked: LOOKED,
        held_by: Some(Guard::EngineBlocked),
        why: "held",
        procedure: "docs/VERIFICATION.md#agent-scenes",
    };

    const SAFE: Finding = Finding::Safe(SEEN);

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
    fn a_finding_reads_the_same_whichever_kind_it_is() {
        let unsafe_finding = Finding::Unsafe {
            seen: SEEN,
            closes_with: "a composer declaration",
        };
        let unknown = Finding::Unknown {
            looked: Some(LOOKED),
            why: "not reached",
            closes_with: "a start door",
        };
        let untried = Finding::Unknown {
            looked: None,
            why: "not surveyed",
            closes_with: "a survey",
        };
        assert_eq!(
            [SAFE, unsafe_finding, unknown, untried].map(|finding| finding.word()),
            ["safe", "unsafe", "unknown", "unknown"]
        );
        assert!(SAFE.is_safe() && !unsafe_finding.is_safe());
        assert!(
            SAFE.closes_with().is_none(),
            "a safe finding closes nothing"
        );
        assert_eq!(unsafe_finding.closes_with(), Some("a composer declaration"));
        assert_eq!(unknown.why(), "not reached");
        assert_eq!(SAFE.looked().unwrap().on, "2026-11-11");
        assert!(untried.looked().is_none());
    }

    #[test]
    fn cells_lists_every_scene_once_in_table_order_and_get_agrees() {
        let case = Support::Supported(SceneCase {
            door: Door::cell("letter"),
            finding: SAFE,
            release_check: false,
        });
        let all = SceneDeclaration {
            rest: case,
            rest_draft: case,
            working: case,
            interrupted: case,
            session_picker: case,
            model_picker: case,
            popup: case,
            tool_approval: case,
            question: case,
            plan_approval: case,
            trust_dialog: case,
            hooks_review: case,
            first_run: case,
            update_notice: Support::Unsupported(GONE),
        };
        let declared: Vec<Scene> = all.cells().iter().map(|(scene, _)| *scene).collect();
        assert_eq!(declared, Scene::ALL);
        for (scene, field) in all.cells() {
            assert!(
                std::ptr::eq(all.get(scene), field),
                "{}: get and cells name different fields",
                scene.name()
            );
        }
    }

    #[test]
    fn the_table_has_a_row_and_a_column_for_everything_declared() {
        let table = table_markdown();
        let first = table.lines().next().unwrap();
        for scene in Scene::ALL {
            assert!(first.contains(scene.column()), "{}", scene.name());
        }
        for row in crate::ADAPTERS {
            assert!(
                table.contains(&format!("| {} | ", row.label)),
                "{}: no row",
                row.id
            );
        }
        assert!(
            !table.contains("\n\n\n"),
            "one blank line between the tables"
        );
    }

    #[test]
    fn a_sentence_in_a_cell_cannot_end_the_cell_or_the_row() {
        assert_eq!(cell_text("a | b\nc"), "a \\| b c");
    }
}
