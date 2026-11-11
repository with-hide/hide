//! Each agent's fourteen scenes: the door the journey opens each through and
//! what a survey found. Every finding is `Unknown` here: no survey has run on
//! the pinned CLIs, so a finding is written only by the change that copies a
//! survey's record into it (`docs/VERIFICATION.md#agent-scenes`). A door
//! marked in its `why` as unconfirmed is the design's best reading of the
//! agent's own commands, which the survey checks first.

use crate::{
    Capability, CapabilityAbsence,
    scene::{Door, Finding, Key, SceneCase, SceneDeclaration, Script, Stage, StartKind},
    usage::Support,
};

const SURVEY: &str = "A survey on the pinned CLI that holds the line three times through a structural guard (docs/VERIFICATION.md#agent-scenes).";

const fn unknown(why: &'static str) -> Finding {
    Finding::Unknown {
        why,
        closes_with: SURVEY,
    }
}

/// A scene the agent has, not yet surveyed.
const fn has(door: Door, why: &'static str) -> Capability<SceneCase> {
    Support::Supported(SceneCase {
        door,
        finding: unknown(why),
        release_check: false,
    })
}

/// A scene the journey's ordinary run also checks the release of
/// (`SceneCase::release_check`).
const fn has_checked(door: Door, why: &'static str) -> Capability<SceneCase> {
    Support::Supported(SceneCase {
        door,
        finding: unknown(why),
        release_check: true,
    })
}

const fn lacks(
    reason: &'static str,
    evidence: &'static str,
    would_change: &'static str,
) -> Capability<SceneCase> {
    Support::Unsupported(CapabilityAbsence {
        reason,
        evidence,
        would_change,
    })
}

const NOT_SURVEYED: &str = "No survey has measured this screen on the agent's pinned CLI.";
const OLD_HERDR: &str = "Only an earlier measurement exists, made on Herdr with an older agent build before Hide's own engine typed letters; it is no evidence for the current build.";
const DOOR_UNCONFIRMED: &str = "The command that opens it is the design's reading of the agent, not yet confirmed in the agent's own command list.";
const NO_FAKE_TOOLS: &str = "A scripted tool call needs a real login: the journey's fake model calls no tools, so this screen is surveyed on a real login and is not checked in CI.";
const FAKE_NO_FIRST_RUN: &str = "A start with an empty account folder is not yet scripted; the journey's accounts are set up before the agent starts.";

const REST: Door = Door::cell("letter");
const WORKING: Door = Door::cell("letter_busy");
/// Text the operator was typing: five characters, erased afterwards.
const DRAFT: Door = Door::Keys {
    from: Stage::Rest,
    open: &[Key::Text { text: "hello" }],
    close: &[Key::ClearComposer],
};
/// A turn stopped with Esc, which restores its prompt to the composer.
const INTERRUPTED: Door = Door::Keys {
    from: Stage::Working,
    open: &[Key::Esc],
    close: &[Key::ClearComposer],
};

/// A built-in picker the operator opens with a slash command and closes with
/// Esc.
macro_rules! command {
    ($command:literal) => {
        Door::Keys {
            from: Stage::Rest,
            open: &[Key::Text { text: $command }, Key::Enter],
            close: &[Key::Esc],
        }
    };
}

/// A list the first character of a word opens over the composer, left open.
macro_rules! popup {
    ($character:literal) => {
        Door::Keys {
            from: Stage::Rest,
            open: &[Key::Text { text: $character }],
            close: &[Key::Esc, Key::ClearComposer],
        }
    };
}

const UNTRUSTED: Door = Door::start(StartKind::UntrustedFolder);
const EMPTY_ACCOUNT: Door = Door::start(StartKind::EmptyAccount);
const UPDATE_CHECK: Door = Door::start(StartKind::UpdateCheckOn);
const SHELL: Door = Door::turn(Script::Shell);
const ASK: Door = Door::turn(Script::Ask);
const PLAN: Door = Door::turn(Script::Plan);

const NO_HOOK_REVIEW: Capability<SceneCase> = lacks(
    "The agent runs a hook from the user's folder without asking the operator to approve it.",
    "docs/agent-hooks.md#which-agents-ask-for-a-review",
    "A CLI version that asks the operator to approve a hook before it runs.",
);

const NO_FOLDER_TRUST: Capability<SceneCase> = lacks(
    "The agent asks no question about trusting a folder.",
    "docs/agent-hooks.md#folder-trust-for-the-worktrees-hide-makes",
    "A CLI version that asks whether to trust a folder.",
);

pub(super) const CLAUDE_CODE: SceneDeclaration = SceneDeclaration {
    rest: has(REST, "Rings today: the journey's `letter` cell proves one typed prompt at rest."),
    rest_draft: has(DRAFT, NOT_SURVEYED),
    working: has(WORKING, "The `letter_busy` cell proves a letter after the held turn; the other effects while it works are not surveyed."),
    interrupted: has(INTERRUPTED, "Hide's delivery guide says an Esc restores the prompt and no prompt hook runs; not surveyed on the current build."),
    session_picker: has(command!("/resume"), NOT_SURVEYED),
    model_picker: has(command!("/model"), NOT_SURVEYED),
    popup: has(popup!("@"), NOT_SURVEYED),
    tool_approval: has(SHELL, NO_FAKE_TOOLS),
    question: has(ASK, NO_FAKE_TOOLS),
    plan_approval: has(PLAN, NO_FAKE_TOOLS),
    trust_dialog: has(UNTRUSTED, "The start the journey makes records trust first; this scene must start without it."),
    hooks_review: NO_HOOK_REVIEW,
    first_run: has(EMPTY_ACCOUNT, FAKE_NO_FIRST_RUN),
    update_notice: has(UPDATE_CHECK, "The journey turns the updater off and no offline way to show the notice is known."),
};

pub(super) const CODEX: SceneDeclaration = SceneDeclaration {
    rest: has(REST, "Rings today: the journey's `letter` cell proves one typed prompt at rest."),
    rest_draft: has(DRAFT, NOT_SURVEYED),
    working: has(WORKING, "The `letter_busy` cell proves a letter after the held turn; the other effects while it works are not surveyed."),
    interrupted: has(INTERRUPTED, NOT_SURVEYED),
    session_picker: has(command!("/resume"), NOT_SURVEYED),
    // What `letter_menu` proved: a letter held while `/model` is open is
    // delivered once after Esc. It closes the picker 10 s in, inside the
    // 30 s quiet period, so it shows the draft record and nothing after it.
    model_picker: has_checked(
        command!("/model"),
        "The journey's release check passes (held while the picker is open, delivered once after Esc), which closes the picker inside the 30 s quiet period; the effect past it is not surveyed.",
    ),
    popup: has(popup!("@"), NOT_SURVEYED),
    tool_approval: has(SHELL, NO_FAKE_TOOLS),
    question: has(ASK, NO_FAKE_TOOLS),
    plan_approval: has(PLAN, NO_FAKE_TOOLS),
    trust_dialog: has(UNTRUSTED, "The start the journey makes records trust first; this scene must start without it."),
    hooks_review: has(
        Door::start(StartKind::UnreviewedHooks),
        "The kit the journey writes records Codex's trust of Hide's hooks; this scene must start without it.",
    ),
    first_run: has(EMPTY_ACCOUNT, FAKE_NO_FIRST_RUN),
    update_notice: has(UPDATE_CHECK, "The journey turns Codex's update check off; turning it on needs the network."),
};

pub(super) const GROK: SceneDeclaration = SceneDeclaration {
    rest: has(REST, "Rings today: the journey's `letter` cell proves one typed prompt at rest."),
    rest_draft: has(DRAFT, NOT_SURVEYED),
    working: has(WORKING, "A kind with tool letters is not rung while it works; the line may only wait in the agent's own queue. Not surveyed."),
    interrupted: has(INTERRUPTED, DOOR_UNCONFIRMED),
    session_picker: has(command!("/resume"), OLD_HERDR),
    model_picker: has(command!("/model"), OLD_HERDR),
    popup: has(popup!("/"), DOOR_UNCONFIRMED),
    tool_approval: has(SHELL, NO_FAKE_TOOLS),
    question: has(ASK, NO_FAKE_TOOLS),
    plan_approval: has(PLAN, NO_FAKE_TOOLS),
    trust_dialog: has(UNTRUSTED, "Grok 1.0.70 holds a start's prompt through the dialog; the current build is not surveyed."),
    hooks_review: NO_HOOK_REVIEW,
    first_run: has(EMPTY_ACCOUNT, FAKE_NO_FIRST_RUN),
    update_notice: has(UPDATE_CHECK, "The journey turns the updater off; `grok update` exists and no offline way to show the notice is known."),
};

pub(super) const OPENCODE: SceneDeclaration = SceneDeclaration {
    rest: has(REST, "Its brief is typed; the `letter` cell proves a letter rides the next prompt, and the bell is off."),
    rest_draft: has(DRAFT, NOT_SURVEYED),
    working: has(WORKING, NOT_SURVEYED),
    interrupted: has(INTERRUPTED, DOOR_UNCONFIRMED),
    session_picker: has(
        command!("/sessions"),
        "A bell's Enter in the previous-session picker opened another session (2026-10-07, on Herdr with an earlier OpenCode); the own engine is not surveyed, and the command name is unconfirmed.",
    ),
    model_picker: has(command!("/models"), DOOR_UNCONFIRMED),
    popup: has(popup!("/"), DOOR_UNCONFIRMED),
    tool_approval: has(SHELL, NO_FAKE_TOOLS),
    question: has(ASK, NO_FAKE_TOOLS),
    plan_approval: has(PLAN, "OpenCode has a plan agent; whether it asks for an approval is not in the repository."),
    trust_dialog: NO_FOLDER_TRUST,
    hooks_review: NO_HOOK_REVIEW,
    first_run: has(EMPTY_ACCOUNT, "A start with no provider configured is not yet scripted."),
    update_notice: has(UPDATE_CHECK, "The journey turns OpenCode's update off; no offline way to show the notice is known."),
};

pub(super) const PI: SceneDeclaration = SceneDeclaration {
    rest: has(REST, "A prompt hook and no bell today: the `letter` cell proves a letter rides the next prompt."),
    rest_draft: has(DRAFT, NOT_SURVEYED),
    working: has(WORKING, NOT_SURVEYED),
    interrupted: has(INTERRUPTED, DOOR_UNCONFIRMED),
    session_picker: has(
        command!("/resume"),
        "The previous-session picker read `done` and a bell's Enter resumed another session (2026-10-07, on Herdr); the own engine is not surveyed.",
    ),
    model_picker: has(command!("/model"), DOOR_UNCONFIRMED),
    popup: has(popup!("/"), "Whether Pi has a list that opens over the composer is not documented here."),
    tool_approval: lacks(
        "Pi runs its tools without asking the operator.",
        "docs/INSTALL.md#what-each-system-supports",
        "A Pi version that asks before a tool call.",
    ),
    question: has(ASK, "No question tool of Pi's is documented in the repository."),
    plan_approval: has(PLAN, "Whether Pi asks for a plan's approval is not documented in the repository."),
    trust_dialog: has(
        UNTRUSTED,
        "Pi's engine reads the dialog `idle` and its Enter picked the highlighted item (2026-10-07, on Herdr); the own engine is not surveyed.",
    ),
    hooks_review: NO_HOOK_REVIEW,
    first_run: has(EMPTY_ACCOUNT, "Whether Pi has a first-run screen is not documented in the repository."),
    update_notice: has(UPDATE_CHECK, "The journey sets PI_OFFLINE; no offline way to show the notice is known."),
};

pub(super) const OMP: SceneDeclaration = SceneDeclaration {
    rest: has(REST, "A prompt hook and no bell today: the `letter` cell proves a letter rides the next prompt."),
    rest_draft: has(DRAFT, NOT_SURVEYED),
    working: has(WORKING, NOT_SURVEYED),
    interrupted: has(INTERRUPTED, DOOR_UNCONFIRMED),
    session_picker: has(
        command!("/resume"),
        "The previous-session picker read `done` and a bell's Enter resumed another session (2026-10-07, on Herdr); the own engine is not surveyed.",
    ),
    model_picker: has(command!("/model"), DOOR_UNCONFIRMED),
    popup: has(popup!("/"), "Whether omp has a list that opens over the composer is not documented here."),
    tool_approval: has(
        SHELL,
        "Needs a start with `--approval-mode always-ask`, and a real login for the scripted call.",
    ),
    question: has(ASK, "No question tool of omp's is documented in the repository."),
    plan_approval: has(PLAN, "Whether omp asks for a plan's approval is not documented in the repository."),
    trust_dialog: NO_FOLDER_TRUST,
    hooks_review: NO_HOOK_REVIEW,
    first_run: has(EMPTY_ACCOUNT, "omp holds a start's prompt through its first-run setup; the screen itself is not surveyed."),
    update_notice: has(UPDATE_CHECK, "omp shows an update notice; the journey turns its check off."),
};

pub(super) const ANTIGRAVITY: SceneDeclaration = SceneDeclaration {
    rest: has(REST, "A prompt hook and no bell today: the `letter` cell proves a letter rides the next prompt."),
    rest_draft: has(DRAFT, NOT_SURVEYED),
    working: has(WORKING, "Letters reach it at its `PreInvocation` hook while it works; not surveyed past that."),
    interrupted: has(INTERRUPTED, DOOR_UNCONFIRMED),
    session_picker: has(command!("/resume"), "Only `/fork` and `/rename` are documented in the repository; the command that opens a session picker is unconfirmed."),
    model_picker: has(command!("/model"), DOOR_UNCONFIRMED),
    popup: has(popup!("/"), DOOR_UNCONFIRMED),
    tool_approval: has(SHELL, NO_FAKE_TOOLS),
    question: has(ASK, NO_FAKE_TOOLS),
    plan_approval: has(PLAN, "Whether agy asks for a plan's approval is not documented in the repository."),
    trust_dialog: has(
        UNTRUSTED,
        "The screen rules read the dialog `blocked` while the guides call it unmeasured; one is stale and the survey settles it.",
    ),
    hooks_review: NO_HOOK_REVIEW,
    first_run: has(EMPTY_ACCOUNT, "A sign-in screen opens the operator's browser; surveyed only in a throwaway account."),
    update_notice: has(UPDATE_CHECK, "The journey turns agy's auto-update off; no offline way to show the notice is known."),
};
