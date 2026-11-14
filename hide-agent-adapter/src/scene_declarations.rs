//! Each agent's fourteen scenes: the door the journey opens each through and
//! what the survey of 2026-11-12 found (`docs/VERIFICATION.md#agent-scenes`).
//! `docs/INSTALL.md` carries the table these declarations generate.
//!
//! A `Safe` finding is written only for the version `contracts/agent-clis.json`
//! pins, so a scene that was seen safe on the installed build alone is
//! `Unknown` here, with what was seen in its reason. The agents whose bell is
//! off (OpenCode, Pi, omp, Antigravity) were measured with a forced letter, which
//! skips the bell declaration, so their findings show whether the composer
//! read held the line and nothing about Hide's draft record or the 30 s wait.

use crate::{
    Capability, CapabilityAbsence,
    scene::{
        Door, EngineRead, Finding, Guard, Key, Looked, SceneCase, SceneDeclaration, Script, Seen,
        Stage, StartKind,
    },
    usage::Support,
};

/// The day the survey ran.
const ON: &str = "2026-11-12";
const PROCEDURE: &str = "docs/VERIFICATION.md#agent-scenes";

/// What a scene seen on a build other than the pin needs.
const AT_PIN: &str = "Measure it again on the pinned version; a safe finding is written only for the pin (docs/VERIFICATION.md#agent-scenes).";
/// A start the journey cannot make yet.
const NEEDS_EMPTY_ACCOUNT: &str = "A start with an empty account folder.";
const NEEDS_UPDATE_FEED: &str = "A fake release feed that makes the agent announce an update.";
const NEEDS_FREE_GROK: &str = "A Grok login with free usage left.";

/// The sentence every scene measured with a forced letter ends with.
macro_rules! forced {
    ($why:literal) => {
        concat!(
            $why,
            " Forced letter: check the draft record and the 30 s wait with the bell on."
        )
    };
}

const IDLE: EngineRead = EngineRead::Idle;
const BLOCKED: EngineRead = EngineRead::Blocked;
const WORKING: EngineRead = EngineRead::Working;
const NOT_READ: EngineRead = EngineRead::Unknown;

const fn looked(version: &'static str, engine: EngineRead) -> Looked {
    Looked {
        version,
        on: ON,
        engine,
    }
}

const fn safe(
    version: &'static str,
    engine: EngineRead,
    held_by: Guard,
    why: &'static str,
) -> Finding {
    Finding::Safe(Seen {
        looked: looked(version, engine),
        held_by: Some(held_by),
        why,
        procedure: PROCEDURE,
    })
}

/// A scene where the line was typed into the screen, or never arrived.
const fn hazard(
    version: &'static str,
    engine: EngineRead,
    held_by: Option<Guard>,
    why: &'static str,
    closes_with: &'static str,
) -> Finding {
    Finding::Unsafe {
        seen: Seen {
            looked: looked(version, engine),
            held_by,
            why,
            procedure: PROCEDURE,
        },
        closes_with,
    }
}

/// A scene the survey tried and could not decide.
const fn unknown(
    version: &'static str,
    engine: EngineRead,
    why: &'static str,
    closes_with: &'static str,
) -> Finding {
    Finding::Unknown {
        looked: Some(looked(version, engine)),
        why,
        closes_with,
    }
}

const fn has(door: Door, finding: Finding) -> Capability<SceneCase> {
    Support::Supported(SceneCase {
        door,
        finding,
        release_check: false,
    })
}

/// A scene the journey's ordinary run also checks the release of
/// (`SceneCase::release_check`).
const fn has_checked(door: Door, finding: Finding) -> Capability<SceneCase> {
    Support::Supported(SceneCase {
        door,
        finding,
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

const REST: Door = Door::cell("letter");
const WORKING_DOOR: Door = Door::cell("letter_busy");
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
/// A turn OpenCode stops with a second Esc.
const INTERRUPTED_BY_TWO_ESC: Door = Door::Keys {
    from: Stage::Working,
    open: &[Key::Esc, Key::Esc],
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
    rest: has(
        REST,
        safe(
            "2.1.302",
            IDLE,
            Guard::OnePrompt,
            "The letter cell: one typed prompt at rest, confirmed by the hook and answered once.",
        ),
    ),
    rest_draft: has(
        DRAFT,
        safe(
            "2.1.302",
            IDLE,
            Guard::DraftRecord,
            "3 of 3: held while the text stood, typed once after the box was cleared, the text never overwritten.",
        ),
    ),
    working: has(
        WORKING_DOOR,
        safe(
            "2.1.302",
            WORKING,
            Guard::OnePrompt,
            "The letter_busy cell: held through the turn, then answered once, in order.",
        ),
    ),
    interrupted: has(
        INTERRUPTED,
        safe(
            "2.1.302",
            IDLE,
            Guard::DraftRecord,
            "3 of 3: Esc puts the text back in the box; held by the draft record, delivered once.",
        ),
    ),
    session_picker: has(
        command!("/resume"),
        safe(
            "2.1.302",
            IDLE,
            Guard::ComposerNotClear,
            "3 of 3: held because the composer reads not clear while the picker is open, delivered once after it closed.",
        ),
    ),
    model_picker: has(
        command!("/model"),
        safe(
            "2.1.302",
            IDLE,
            Guard::DraftRecord,
            "3 of 3: held by the draft record, delivered once after the picker closed.",
        ),
    ),
    popup: has(
        popup!("@"),
        safe(
            "2.1.302",
            IDLE,
            Guard::DraftRecord,
            "3 of 3: the @ file list; held by the draft record, delivered once.",
        ),
    ),
    tool_approval: has(
        SHELL,
        unknown(
            "2.1.322",
            BLOCKED,
            "Seen on the installed build, not the pin: Enter on the dialog held the letter (engine blocked) and it arrived once; after Esc (refused) the engine stayed blocked and the letter waited over 60 s with nothing typed. Real login, the model chose the action, 2 requests, not CI-verified.",
            "Measure it on the pinned version, and decide whether a refused dialog that leaves the engine blocked needs a release rule.",
        ),
    ),
    question: has(
        ASK,
        unknown(
            "2.1.322",
            BLOCKED,
            "Seen safe on the installed build, not the pin: held by the engine reading blocked, delivered once after the answer. Real login, the model chose the action, 2 requests, not CI-verified.",
            AT_PIN,
        ),
    ),
    plan_approval: has(
        PLAN,
        unknown(
            "2.1.322",
            BLOCKED,
            "Seen on the installed build, not the pin: Enter (approve) held the letter and it arrived once; Esc (refuse) left the engine blocked and the letter waited over 60 s, as for the tool approval. Real login, 2 requests, not CI-verified.",
            "Measure it on the pinned version, and decide whether a refused dialog that leaves the engine blocked needs a release rule.",
        ),
    ),
    trust_dialog: has(
        UNTRUSTED,
        unknown(
            "2.1.322",
            IDLE,
            "Seen safe on the installed build, not the pin: a start in an untrusted folder is refused as having no native session and request send is refused, so no bell can ring.",
            AT_PIN,
        ),
    ),
    hooks_review: NO_HOOK_REVIEW,
    first_run: has(
        EMPTY_ACCOUNT,
        unknown(
            "2.1.302",
            NOT_READ,
            "The harness cannot start the agent in an empty account folder, so the screen was never reached.",
            NEEDS_EMPTY_ACCOUNT,
        ),
    ),
    update_notice: has(
        UPDATE_CHECK,
        unknown(
            "2.1.302",
            NOT_READ,
            "The journey turns the updater off and no offline way to show the notice is known, so the screen was never reached.",
            NEEDS_UPDATE_FEED,
        ),
    ),
};

pub(super) const CODEX: SceneDeclaration = SceneDeclaration {
    rest: has(
        REST,
        safe(
            "0.163.0",
            IDLE,
            Guard::OnePrompt,
            "The letter cell: one typed prompt at rest, answered once.",
        ),
    ),
    rest_draft: has(
        DRAFT,
        safe(
            "0.163.0",
            IDLE,
            Guard::DraftRecord,
            "3 of 3: held by the draft record, delivered once.",
        ),
    ),
    working: has(
        WORKING_DOOR,
        safe(
            "0.163.0",
            WORKING,
            Guard::OnePrompt,
            "The letter_busy cell: held through the turn, answered once.",
        ),
    ),
    interrupted: has(
        INTERRUPTED,
        safe(
            "0.163.0",
            IDLE,
            Guard::DraftRecord,
            "Esc leaves the box empty and the engine idle; the letter was held 30 s by the draft record, then typed once into the empty box and answered once. The survey's rule read it unsafe 3 of 3 because it counts any bell while the scene is open; judged safe by hand.",
        ),
    ),
    session_picker: has(
        command!("/resume"),
        safe(
            "0.163.0",
            IDLE,
            Guard::DraftRecord,
            "3 of 3: held by the draft record, delivered once.",
        ),
    ),
    // What `letter_menu` proved before the scenes: a letter held while
    // `/model` is open is delivered once after Esc.
    model_picker: has_checked(
        command!("/model"),
        safe(
            "0.163.0",
            IDLE,
            Guard::NotReady,
            "3 of 3: held while the picker is open (the engine reported not ready), delivered once after Esc.",
        ),
    ),
    popup: has(
        popup!("@"),
        safe(
            "0.163.0",
            IDLE,
            Guard::DraftRecord,
            "3 of 3: the @ list; held by the draft record, delivered once.",
        ),
    ),
    tool_approval: has(
        SHELL,
        unknown(
            "0.167.1",
            BLOCKED,
            "Seen safe on the installed build, not the pin: Enter and Esc both left the letter held by the engine reading blocked, and it arrived once. Real login, the model chose the action, 2 requests, not CI-verified.",
            AT_PIN,
        ),
    ),
    question: has(
        ASK,
        unknown(
            "0.167.1",
            WORKING,
            "Seen safe on the installed build, not the pin: in Plan mode the question held the letter through the session's native wait and it arrived once, but the engine reads working, not blocked; in the default mode the model answered with no dialog. Real login, 2 requests, not CI-verified.",
            "Measure it on the pinned version, and decide whether the engine should read Codex's question as blocked.",
        ),
    ),
    plan_approval: has(
        PLAN,
        unknown(
            "0.167.1",
            BLOCKED,
            "Seen on the installed build, not the pin: with the plan dialog open the letter was held by the session's native wait and nothing was typed; choosing to implement opens a second approval where it is held too. The survey's rule read it unsafe because the letter was not delivered inside its window; judged safe by hand. Real login, 2 requests, not CI-verified.",
            "Measure it on the pinned version, and make the survey wait longer for a dialog that stays open.",
        ),
    ),
    trust_dialog: has(
        UNTRUSTED,
        unknown(
            "0.167.1",
            IDLE,
            "Seen safe on the installed build, not the pin: a start in an untrusted folder is refused as having no native session and request send is refused, so no bell can ring.",
            AT_PIN,
        ),
    ),
    hooks_review: has(
        Door::start(StartKind::UnreviewedHooks),
        unknown(
            "0.163.0",
            NOT_READ,
            "The harness has no start that clears Codex's own trust of Hide's hooks, so the review screen was never reached.",
            "A start that leaves Codex's hook trust unrecorded.",
        ),
    ),
    first_run: has(
        EMPTY_ACCOUNT,
        unknown(
            "0.163.0",
            NOT_READ,
            "The harness cannot start the agent in an empty account folder, so the screen was never reached.",
            NEEDS_EMPTY_ACCOUNT,
        ),
    ),
    update_notice: has(
        UPDATE_CHECK,
        unknown(
            "0.163.0",
            NOT_READ,
            "The journey turns Codex's update check off and turning it on needs the network, so the screen was never reached.",
            NEEDS_UPDATE_FEED,
        ),
    ),
};

pub(super) const GROK: SceneDeclaration = SceneDeclaration {
    rest: has(
        REST,
        safe(
            "1.0.52",
            IDLE,
            Guard::OnePrompt,
            "The letter cell: one typed prompt at rest, confirmed through the turn end.",
        ),
    ),
    rest_draft: has(
        DRAFT,
        hazard(
            "1.0.52",
            IDLE,
            Some(Guard::DraftRecord),
            "3 of 3, and once over 300 s: nothing was overwritten, but after the person clears the box Grok shows a dim suggestion that Hide's composer read does not take for an empty box, so the letter stays pending and never arrives.",
            "Declare Grok's composer so that its dim suggestion reads as an empty box.",
        ),
    ),
    working: has(
        WORKING_DOOR,
        safe(
            "1.0.52",
            WORKING,
            Guard::OnePrompt,
            "The letter_busy cell: held through the turn, delivered after it, confirmed through the turn end.",
        ),
    ),
    interrupted: has(
        INTERRUPTED,
        unknown(
            "1.0.52",
            WORKING,
            "Esc does not stop a Grok turn and one Ctrl+C only blocks the clock, so no clean interrupted screen was reached.",
            "A key that interrupts Grok's turn and leaves its prompt restored.",
        ),
    ),
    session_picker: has(
        command!("/resume"),
        hazard(
            "1.0.52",
            IDLE,
            Some(Guard::DraftRecord),
            "3 of 3: nothing was overwritten, but the letter stays pending because the composer read does not read the box as empty after the picker closed.",
            "Declare Grok's composer so that its dim suggestion reads as an empty box.",
        ),
    ),
    model_picker: has(
        command!("/model"),
        hazard(
            "1.0.52",
            IDLE,
            Some(Guard::DraftRecord),
            "3 of 3: nothing was overwritten, but the letter stays pending because the composer read does not read the box as empty after the picker closed.",
            "Declare Grok's composer so that its dim suggestion reads as an empty box.",
        ),
    ),
    popup: has(
        popup!("/"),
        hazard(
            "1.0.52",
            IDLE,
            Some(Guard::DraftRecord),
            "3 of 3: nothing was overwritten, but the letter stays pending because the composer read does not read the box as empty after the list closed.",
            "Declare Grok's composer so that its dim suggestion reads as an empty box.",
        ),
    ),
    tool_approval: has(
        SHELL,
        unknown(
            "1.0.77",
            NOT_READ,
            "The real login answered 'You hit your free usage limit', so the model never called the tool and the screen was never reached. Real login, not CI-verified.",
            NEEDS_FREE_GROK,
        ),
    ),
    question: has(
        ASK,
        unknown(
            "1.0.77",
            NOT_READ,
            "The real login answered 'You hit your free usage limit', so the model never called the tool and the screen was never reached. Real login, not CI-verified.",
            NEEDS_FREE_GROK,
        ),
    ),
    plan_approval: has(
        PLAN,
        unknown(
            "1.0.77",
            NOT_READ,
            "The real login answered 'You hit your free usage limit', so the model never called the tool and the screen was never reached. Real login, not CI-verified.",
            NEEDS_FREE_GROK,
        ),
    ),
    trust_dialog: has(
        UNTRUSTED,
        unknown(
            "1.0.77",
            IDLE,
            "Seen safe on the installed build, not the pin: with 'Do you trust the contents of this directory?' open, a start is refused as having no native session and request send is refused, so no bell can ring.",
            AT_PIN,
        ),
    ),
    hooks_review: NO_HOOK_REVIEW,
    first_run: has(
        EMPTY_ACCOUNT,
        unknown(
            "1.0.52",
            NOT_READ,
            "The harness cannot start the agent in an empty account folder, so the screen was never reached.",
            NEEDS_EMPTY_ACCOUNT,
        ),
    ),
    update_notice: has(
        UPDATE_CHECK,
        unknown(
            "1.0.52",
            NOT_READ,
            "The journey turns the updater off and no offline way to show the notice is known, so the screen was never reached.",
            NEEDS_UPDATE_FEED,
        ),
    ),
};

pub(super) const OPENCODE: SceneDeclaration = SceneDeclaration {
    rest: has(
        REST,
        safe(
            "2.0.29",
            IDLE,
            Guard::OnePrompt,
            forced!(
                "A forced letter was typed once into the empty box, confirmed by the hook, delivered once and answered."
            ),
        ),
    ),
    rest_draft: has(
        DRAFT,
        safe(
            "2.0.29",
            IDLE,
            Guard::ComposerNotClear,
            forced!(
                "3 of 3: a letter forced over text was refused as a non-empty composer and nothing was typed."
            ),
        ),
    ),
    working: has(
        WORKING_DOOR,
        safe(
            "2.0.29",
            WORKING,
            Guard::OnePrompt,
            forced!(
                "One run: a forced letter was held while the turn ran with no bell typed, then handed over by the plugin after the turn and answered once; the survey read it unknown because it counts a clock hold."
            ),
        ),
    ),
    interrupted: has(
        INTERRUPTED_BY_TWO_ESC,
        safe(
            "2.0.29",
            IDLE,
            Guard::OnePrompt,
            forced!(
                "One run: two Esc presses stopped the turn, the box was empty and the engine idle, and a forced letter was typed once and answered once; the survey's rule read it unsafe because it counts any bell while the scene is open, judged safe by hand."
            ),
        ),
    ),
    session_picker: has(
        command!("/sessions"),
        hazard(
            "2.0.29",
            IDLE,
            None,
            forced!(
                "2 of 3 runs (the third did not decide): the composer read empty because the picker is a dialog above the box, so a forced letter was typed into the picker's search and left there ('No sessions found')."
            ),
            "OpenCode's composer declaration reads an open session picker as not found, or a bell-on build is measured and Hide's draft record and 30 s wait are seen to hold it.",
        ),
    ),
    model_picker: has(
        command!("/models"),
        safe(
            "2.0.29",
            IDLE,
            Guard::ComposerNotClear,
            forced!(
                "3 of 3: the picker overlaps the composer area, so the composer read not empty and refused the line; this rests on the picker's shape."
            ),
        ),
    ),
    popup: has(
        popup!("/"),
        safe(
            "2.0.29",
            IDLE,
            Guard::ComposerNotClear,
            forced!(
                "3 of 3: the / list leaves a / in the box, so the composer read not empty and refused the line."
            ),
        ),
    ),
    tool_approval: has(
        SHELL,
        safe(
            "2.0.29",
            WORKING,
            Guard::ComposerNotClear,
            forced!(
                "One run on a real login with permissions set to ask: the composer read not found under the dialog and refused the line, and the tool did not run. The engine read the dialog working, so no screen rule reads it blocked. The model chose the action, not CI-verified."
            ),
        ),
    ),
    question: has(
        ASK,
        safe(
            "2.0.29",
            WORKING,
            Guard::ComposerNotClear,
            forced!(
                "One run on a real login: the composer read not found under the question dialog and refused the line. The engine read the dialog working. The model chose the action, not CI-verified."
            ),
        ),
    ),
    plan_approval: has(
        PLAN,
        unknown(
            "2.0.29",
            WORKING,
            "Tab did not switch to the Plan agent, so no plan approval dialog was reached.",
            "The key or command that switches OpenCode to its Plan agent, then measure.",
        ),
    ),
    trust_dialog: NO_FOLDER_TRUST,
    hooks_review: NO_HOOK_REVIEW,
    first_run: has(
        EMPTY_ACCOUNT,
        unknown(
            "2.0.29",
            NOT_READ,
            "The harness cannot start the agent with no provider configured, so the screen was never reached.",
            NEEDS_EMPTY_ACCOUNT,
        ),
    ),
    update_notice: has(
        UPDATE_CHECK,
        unknown(
            "2.0.29",
            NOT_READ,
            "The harness cannot start the agent with its update check on, so the screen was never reached.",
            NEEDS_UPDATE_FEED,
        ),
    ),
};

pub(super) const PI: SceneDeclaration = SceneDeclaration {
    rest: has(
        REST,
        safe(
            "1.0.4",
            IDLE,
            Guard::OnePrompt,
            forced!(
                "A forced letter was typed once into the empty box, delivered once and answered."
            ),
        ),
    ),
    rest_draft: has(
        DRAFT,
        safe(
            "1.0.4",
            IDLE,
            Guard::ComposerNotClear,
            forced!(
                "3 of 3: a letter forced over text was refused as a non-empty composer and nothing was typed."
            ),
        ),
    ),
    working: has(
        WORKING_DOOR,
        safe(
            "1.0.4",
            WORKING,
            Guard::OnePrompt,
            forced!(
                "One run: a forced letter was held while the turn ran with no bell typed, then delivered once at a tool handover; the survey read it unknown because it does not know a hook delivery."
            ),
        ),
    ),
    interrupted: has(
        INTERRUPTED,
        safe(
            "1.0.4",
            IDLE,
            Guard::OnePrompt,
            forced!(
                "One run: Esc aborted the running command, the box was empty and the engine idle, and a forced letter was typed once and answered once; the survey's rule read it unsafe because it counts any bell while the scene is open, judged safe by hand."
            ),
        ),
    ),
    session_picker: has(
        command!("/resume"),
        safe(
            "1.0.4",
            IDLE,
            Guard::ComposerNotClear,
            forced!(
                "3 of 3: the composer read the picker's > search line as not empty and refused the line; this rests on the picker's shape."
            ),
        ),
    ),
    model_picker: has(
        command!("/model"),
        safe(
            "1.0.4",
            IDLE,
            Guard::ComposerNotClear,
            forced!(
                "3 of 3: the composer read not found over the /model list and refused the line."
            ),
        ),
    ),
    popup: has(
        popup!("/"),
        safe(
            "1.0.4",
            IDLE,
            Guard::ComposerNotClear,
            forced!("3 of 3: the / list; the composer read not empty and refused the line."),
        ),
    ),
    tool_approval: lacks(
        "Pi runs its tools without asking the operator.",
        "docs/INSTALL.md#what-each-system-supports",
        "A Pi version that asks before a tool call.",
    ),
    question: lacks(
        "Pi has no question tool of its own: `pi --help` lists four tools (read, bash, edit, write), and a question needs an extension.",
        "docs/INSTALL.md#what-a-letters-bell-does-on-each-screen",
        "A Pi version with a built-in question tool.",
    ),
    plan_approval: lacks(
        "Pi has no plan mode of its own: planning is an extension.",
        "docs/INSTALL.md#what-a-letters-bell-does-on-each-screen",
        "A Pi version with a built-in plan mode.",
    ),
    trust_dialog: has(
        UNTRUSTED,
        safe(
            "1.0.4",
            IDLE,
            Guard::NoSession,
            "Pi's folder-trust dialog (a repository with .pi/settings.json) opened and the engine read it idle; a start there is refused as having no native session and request send is refused, so no bell can ring. A dialog that opens after the session is registered was not measured.",
        ),
    ),
    hooks_review: NO_HOOK_REVIEW,
    first_run: has(
        EMPTY_ACCOUNT,
        unknown(
            "1.0.4",
            NOT_READ,
            "The harness cannot start the agent in an empty account folder, so the screen was never reached.",
            NEEDS_EMPTY_ACCOUNT,
        ),
    ),
    update_notice: has(
        UPDATE_CHECK,
        unknown(
            "1.0.4",
            NOT_READ,
            "The harness cannot start the agent with its update check on (the journey sets PI_OFFLINE), so the screen was never reached.",
            NEEDS_UPDATE_FEED,
        ),
    ),
};

pub(super) const OMP: SceneDeclaration = SceneDeclaration {
    rest: has(
        REST,
        safe(
            "18.7.0",
            IDLE,
            Guard::OnePrompt,
            forced!(
                "A forced letter was typed once into the empty box, confirmed by the hook, delivered once and answered."
            ),
        ),
    ),
    rest_draft: has(
        DRAFT,
        safe(
            "18.7.0",
            IDLE,
            Guard::ComposerNotClear,
            forced!(
                "3 of 3: a letter forced over text was refused as a non-empty composer and nothing was typed."
            ),
        ),
    ),
    working: has(
        WORKING_DOOR,
        safe(
            "18.7.0",
            WORKING,
            Guard::OnePrompt,
            forced!(
                "One run: a forced letter was held while the turn ran with no bell typed, then delivered once at a tool handover and confirmed by the hook; the survey read it unknown because it counts a clock hold."
            ),
        ),
    ),
    interrupted: has(
        INTERRUPTED,
        safe(
            "18.7.0",
            IDLE,
            Guard::OnePrompt,
            forced!(
                "One run: Esc stopped the turn, the box was empty and the engine idle, and a forced letter was typed once and answered once; the survey's rule read it unsafe because it counts any bell while the scene is open, judged safe by hand."
            ),
        ),
    ),
    session_picker: has(
        command!("/resume"),
        safe(
            "18.7.0",
            IDLE,
            Guard::ComposerNotClear,
            forced!(
                "3 of 3: the /resume table is a bordered box over the composer area, so the composer read not empty and refused the line; this rests on its shape."
            ),
        ),
    ),
    model_picker: has(
        command!("/model"),
        safe(
            "18.7.0",
            IDLE,
            Guard::ComposerNotClear,
            forced!(
                "3 of 3: the /model table covers the composer, which read not empty and refused the line; this rests on its shape."
            ),
        ),
    ),
    popup: has(
        popup!("/"),
        safe(
            "18.7.0",
            IDLE,
            Guard::ComposerNotClear,
            forced!("3 of 3: the / list; the composer read not empty and refused the line."),
        ),
    ),
    tool_approval: has(
        SHELL,
        safe(
            "18.7.0",
            WORKING,
            Guard::ComposerNotClear,
            forced!(
                "One run on a real login with --approval-mode always-ask and no bypass option: the 'Allow tool: bash' dialog stood, the composer read not empty and refused the line, and the tool did not run. omp has no screen rules, so the engine read it working. The model chose the action, not CI-verified."
            ),
        ),
    ),
    question: has(
        ASK,
        safe(
            "18.7.0",
            WORKING,
            Guard::ComposerNotClear,
            forced!(
                "One run on a real login: the composer read not empty under the ask dialog and refused the line. omp has no screen rules, so the engine read it working. The model chose the action, not CI-verified."
            ),
        ),
    ),
    plan_approval: has(
        PLAN,
        safe(
            "18.7.0",
            IDLE,
            Guard::ComposerNotClear,
            forced!(
                "One run on a real login: after /plan the 'Plan mode - next step' dialog stood, the composer read not empty and refused the line. The engine read idle and cannot see an approval waiting. The model chose the action, not CI-verified."
            ),
        ),
    ),
    trust_dialog: NO_FOLDER_TRUST,
    hooks_review: NO_HOOK_REVIEW,
    first_run: has(
        EMPTY_ACCOUNT,
        unknown(
            "18.7.0",
            NOT_READ,
            "The harness cannot start the agent in an empty account folder, where omp holds a start's prompt through its setup, so the screen was never reached.",
            NEEDS_EMPTY_ACCOUNT,
        ),
    ),
    update_notice: has(
        UPDATE_CHECK,
        unknown(
            "18.7.0",
            IDLE,
            "A non-modal 'Update Available' line in the chat was seen on the rest, working and interrupted runs, where a forced letter was typed normally; whether omp ever asks an interactive update question was not seen.",
            "A look at omp's change log or a run that shows whether its update notice ever asks a question.",
        ),
    ),
};

pub(super) const ANTIGRAVITY: SceneDeclaration = SceneDeclaration {
    rest: has(
        REST,
        unknown(
            "1.5.6",
            IDLE,
            forced!(
                "Seen safe on the installed build, not the pin: a letter rides the next prompt through the PreInvocation hook once with no bell, and a forced letter was typed once into the empty box, delivered once and answered, 3 of 3."
            ),
            AT_PIN,
        ),
    ),
    rest_draft: has(
        DRAFT,
        unknown(
            "1.5.6",
            IDLE,
            forced!(
                "Seen safe on the installed build, not the pin: 3 of 3, a letter forced over text was refused as a non-empty composer and nothing was typed."
            ),
            AT_PIN,
        ),
    ),
    working: has(
        WORKING_DOOR,
        unknown(
            "1.5.6",
            WORKING,
            forced!(
                "Seen safe on the installed build, not the pin: 3 of 3, a forced letter was held as working, then typed once right after the turn ended and delivered once; the survey counts working as a clock hold."
            ),
            AT_PIN,
        ),
    ),
    interrupted: has(
        INTERRUPTED,
        unknown(
            "1.5.6",
            WORKING,
            forced!(
                "Esc shows 'Interrupted' and an empty box, but the engine stays working for over 60 s, so a forced letter stayed held as working and was never delivered; the survey's later repetitions could not start."
            ),
            "Have the engine read an Esc interrupt (a screen rule or a hook), then measure it again on the pinned version.",
        ),
    ),
    session_picker: has(
        command!("/resume"),
        hazard(
            "1.5.6",
            IDLE,
            None,
            forced!(
                "3 of 3: the composer read empty because the input row is empty and the Conversations list draws below it, so a forced letter was typed into the list's search and left there, and was not delivered."
            ),
            "agy's composer declaration reads the open Conversations list as not found (or an engine rule reads it blocked), then measure it again on the pinned version.",
        ),
    ),
    model_picker: has(
        command!("/model"),
        unknown(
            "1.5.6",
            IDLE,
            forced!(
                "Seen safe on the installed build, not the pin: 3 of 3, the composer read not found over the Switch Model list and refused the line."
            ),
            AT_PIN,
        ),
    ),
    popup: has(
        popup!("/"),
        unknown(
            "1.5.6",
            IDLE,
            forced!(
                "Seen safe on the installed build, not the pin: 3 of 3, the / command list; the composer read not empty and refused the line."
            ),
            AT_PIN,
        ),
    ),
    tool_approval: has(
        SHELL,
        unknown(
            "1.5.6",
            WORKING,
            forced!(
                "Seen safe on the installed build, not the pin: 3 of 3, the composer read not empty under the permission window and refused the line, and the command did not run; the engine query read working while the session's watch reported blocked once. Whether the letter arrives after the window closes was not measured."
            ),
            "Measure it on the pinned version, and measure the release after the window closes on a build with the bell on.",
        ),
    ),
    question: has(
        ASK,
        unknown(
            "1.5.6",
            WORKING,
            forced!(
                "Seen safe on the installed build, not the pin: 3 of 3, the composer read not empty under the question window and refused the line; the same window read blocked once and idle twice, so the composer alone carries it."
            ),
            "Have the engine read the question window as blocked every time, then measure it again on the pinned version.",
        ),
    ),
    plan_approval: has(
        PLAN,
        unknown(
            "1.5.6",
            NOT_READ,
            "No tool of agy's lets a scripted model reach a plan approval (its tool list has no plan exit), so the screen was never reached.",
            "A scripted model or a real login that reaches agy's plan approval.",
        ),
    ),
    trust_dialog: has(
        UNTRUSTED,
        unknown(
            "1.5.6",
            BLOCKED,
            forced!(
                "Seen on the installed build, not the pin: 3 of 3, the 'Do you trust the contents of this project?' window stood, the screen rules read it blocked (the antigravity.toml trust_prompt rule is right), and a forced letter was refused as a composer it could not find."
            ),
            AT_PIN,
        ),
    ),
    hooks_review: NO_HOOK_REVIEW,
    first_run: has(
        EMPTY_ACCOUNT,
        unknown(
            "1.5.6",
            NOT_READ,
            forced!(
                "3 of 3: the first-run screen (theme, security notice) was reached with a fake key and an emptied onboarding cache, and a forced letter was refused; the real first run opens a sign-in in the browser and needs a throwaway account, so it was not seen."
            ),
            "A first run in a throwaway account, sign-in included.",
        ),
    ),
    update_notice: has(
        UPDATE_CHECK,
        unknown(
            "1.5.6",
            NOT_READ,
            "The harness cannot start the agent with its update check on (auto-update is off and the network is blocked), so the screen was never reached.",
            NEEDS_UPDATE_FEED,
        ),
    ),
};
