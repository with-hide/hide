//! The contract of the commands other tools call (`hide agent`, `ps`,
//! `request`, `inbox`, `watch`, `mission`, `task`, `device`, `pane`, `access`, `workspace
//! purpose`, `workspace remove`): each command's words, arguments and options, where its
//! answer sits in the line it prints, and the JSON Schema of that answer.
//!
//! The table below is what the CLI admits: a verb or a flag of these topics
//! that the table does not name is refused before any parser reads it
//! (`admit`), so a command or option cannot reach a caller without reaching
//! the contract, and the tests below hold each parser to what the table says
//! it takes. The answer schemas come from the core's answer types
//! (`hide_core::delivery::answer`). `hide contract --json` prints the
//! document and `hide version --json` its digest, and `contracts/hide-cli.json`
//! is the committed copy a review sees change.

use hide_core::delivery::{BODY_LIMIT, BUNDLE_LETTERS};
use serde_json::{Map, Value, json};

/// The document's own format; a caller that reads another refuses it.
pub const FORMAT: u32 = 1;

/// The topics whose every command is in the table.
const TOPICS: &[&str] = &[
    "agent", "ps", "request", "inbox", "watch", "mission", "task", "device", "pane", "access",
];

/// What a value must be for the command's parser to take it.
#[derive(Clone, Copy)]
pub enum Input {
    /// Non-empty, without control characters, not starting with `--`.
    Text,
    /// An id, name or intent: non-empty, at most 256 bytes, without control
    /// characters, not starting with `-`.
    Key,
    /// A native session id or session file path: non-empty, at most
    /// `hide_core::delivery::SESSION_LIMIT` bytes, without control
    /// characters, not starting with `-`.
    Session,
    /// A letter body: not blank, at most `BODY_LIMIT` bytes.
    Body,
    /// A SHA-256 as 64 lowercase hexadecimal digits.
    Sha256,
    /// A git commit: 7 to 64 hexadecimal digits.
    Commit,
    /// Two decimal integers as `<passed>/<total>`.
    Ratio,
    /// A span of whole minutes or hours, `30m` or `2h`.
    Span,
    /// A watch approval: not blank, at most 256 bytes, without control
    /// characters.
    Approval,
    /// A decimal integer from 0 to 2^64 - 1.
    Unsigned,
    /// A JSON document of the shape the command names.
    Json,
    /// Text that may also carry line breaks (LF, CR) and tabs, and no other
    /// control character: what `hide pane send --paste` admits.
    PasteText,
    OneOf(&'static [&'static str]),
}

impl Input {
    fn name(self) -> &'static str {
        match self {
            Text => "text",
            Key => "key",
            Session => "session",
            Body => "body",
            Sha256 => "sha256",
            Commit => "commit",
            Ratio => "ratio",
            Span => "span",
            Approval => "approval",
            Unsigned => "unsigned",
            Json => "json",
            PasteText => "paste_text",
            OneOf(_) => "one_of",
        }
    }
}

pub struct Arg {
    pub name: &'static str,
    pub value: Input,
    /// False for an argument the command takes without; one left out
    /// leaves out every argument after it.
    pub required: bool,
    /// The option that widens the value it admits, and to what.
    pub value_with: Option<(&'static str, Input)>,
}

pub struct Opt {
    pub name: &'static str,
    /// None for a switch.
    pub value: Option<Input>,
    pub required: bool,
    /// Options this one is refused without.
    pub requires: &'static [&'static str],
    /// Options this one cannot be combined with.
    pub conflicts: &'static [&'static str],
    /// For an option given more than once, at most how many times.
    pub repeats: Option<usize>,
}

pub struct Spec {
    /// The words after `hide`, mode flags such as `inbox --hook` included.
    pub words: &'static [&'static str],
    /// Positional arguments, in order.
    pub arguments: &'static [Arg],
    /// When the last argument repeats, at most how many times.
    pub repeats: Option<usize>,
    /// Arguments and options of which the command needs at least one, its
    /// forms; empty when it needs none of them in particular.
    pub needs_one_of: &'static [&'static str],
    pub options: &'static [Opt],
    /// What `--` passes through unread, if the command takes it.
    pub rest: Option<&'static str>,
    /// The answer schemas (`answers`) the command can return.
    pub answers: &'static [&'static str],
    /// The failure codes the contract declares for the command's own rule,
    /// beside those every command of its topic can answer (the daemon, the
    /// pane, the ledger). Not exhaustive; written only where declared.
    pub refusals: &'static [&'static str],
}

const fn required(name: &'static str, value: Input) -> Opt {
    Opt {
        name,
        value: Some(value),
        required: true,
        requires: &[],
        conflicts: &[],
        repeats: None,
    }
}

const fn optional(name: &'static str, value: Input) -> Opt {
    Opt {
        name,
        value: Some(value),
        required: false,
        requires: &[],
        conflicts: &[],
        repeats: None,
    }
}

const fn switch(name: &'static str) -> Opt {
    Opt {
        name,
        value: None,
        required: false,
        requires: &[],
        conflicts: &[],
        repeats: None,
    }
}

/// An option given up to `most` times.
const fn repeated(name: &'static str, value: Input, most: usize) -> Opt {
    Opt {
        name,
        value: Some(value),
        required: false,
        requires: &[],
        conflicts: &[],
        repeats: Some(most),
    }
}

/// An option whose value can instead be cleared by another switch.
const fn cleared(name: &'static str, value: Input, conflicts: &'static [&'static str]) -> Opt {
    Opt {
        name,
        value: Some(value),
        required: false,
        requires: &[],
        conflicts,
        repeats: None,
    }
}

/// The switch that clears another option's value.
const fn clearing(name: &'static str, conflicts: &'static [&'static str]) -> Opt {
    Opt {
        name,
        value: None,
        required: false,
        requires: &[],
        conflicts,
        repeats: None,
    }
}

const fn spec(
    words: &'static [&'static str],
    arguments: &'static [Arg],
    options: &'static [Opt],
    answers: &'static [&'static str],
) -> Spec {
    Spec {
        words,
        arguments,
        repeats: None,
        needs_one_of: &[],
        options,
        rest: None,
        answers,
        refusals: &[],
    }
}

use Input::{
    Approval, Body, Commit, Json, Key, OneOf, PasteText, Ratio, Session, Sha256, Span, Text,
    Unsigned,
};

const ID: &[Arg] = &[Arg {
    name: "id",
    value: Key,
    required: true,
    value_with: None,
}];
const TARGET: &[Arg] = &[Arg {
    name: "target",
    value: Key,
    required: true,
    value_with: None,
}];
const AGENT_ID: &[Arg] = &[Arg {
    name: "id",
    value: Text,
    required: true,
    value_with: None,
}];

/// An agent profile's agent and name (PRD agent-profiles D-33).
const PROFILE: &[Arg] = &[
    Arg {
        name: "kind",
        value: Text,
        required: true,
        value_with: None,
    },
    Arg {
        name: "name",
        value: Text,
        required: true,
        value_with: None,
    },
];

/// The refusals every `hide agent profile` change can answer.
const PROFILE_REFUSALS: &[&str] = &[
    hide_core::agent_profiles::REFUSED,
    hide_core::agent_profiles::IN_USE,
    hide_core::agent_profiles::GONE,
    hide_core::agent_profiles::BUILT_IN,
    hide_core::agent_profiles::UNSUPPORTED,
    hide_core::agent_profiles::UNREAD,
    hide_core::agent_profiles::UNANSWERED,
    hide_core::agent_profiles::INVALID,
    hide_core::agent_profiles::BUSY,
    "machine_unknown",
    "machine_unavailable",
];

const fn profile_spec(
    words: &'static [&'static str],
    arguments: &'static [Arg],
    options: &'static [Opt],
    answers: &'static [&'static str],
) -> Spec {
    Spec {
        words,
        arguments,
        repeats: None,
        needs_one_of: &[],
        options,
        rest: None,
        answers,
        refusals: PROFILE_REFUSALS,
    }
}

pub const COMMANDS: &[Spec] = &[
    profile_spec(
        &["agent", "profile", "list"],
        &[],
        &[
            optional("--kind", Text),
            optional("--machine", Text),
            switch("--json"),
        ],
        &["agent_profiles"],
    ),
    profile_spec(
        &["agent", "profile", "add"],
        PROFILE,
        &[
            optional("--account", Text),
            optional("--program", Text),
            repeated("--env", Text, 64),
            switch("--default"),
            optional("--machine", Text),
            switch("--json"),
        ],
        &["agent_profile"],
    ),
    profile_spec(
        &["agent", "profile", "edit"],
        PROFILE,
        &[
            optional("--rename", Text),
            cleared("--program", Text, &["--no-program"]),
            clearing("--no-program", &["--program"]),
            repeated("--env", Text, 64),
            repeated("--unset-env", Text, 64),
            optional("--machine", Text),
            switch("--json"),
        ],
        &["agent_profile"],
    ),
    profile_spec(
        &["agent", "profile", "default"],
        PROFILE,
        &[optional("--machine", Text), switch("--json")],
        &["agent_profile"],
    ),
    profile_spec(
        &["agent", "profile", "remove"],
        PROFILE,
        &[optional("--machine", Text), switch("--json")],
        &["agent_profile_removed"],
    ),
    profile_spec(
        &["agent", "profile", "reinstall"],
        PROFILE,
        &[optional("--machine", Text), switch("--json")],
        &["agent_profile"],
    ),
    spec(
        &["agent", "register"],
        &[],
        &[
            switch("--check"),
            optional("--machine", Text),
            required("--host-scope", Text),
            required("--session", Session),
            required("--instance", Text),
            required("--name", Text),
            required("--pane", Text),
            optional("--parent", Text),
            optional("--project", Text),
            switch("--json"),
        ],
        &["agent", "agent_register_check"],
    ),
    // Every registration, or with `--live` only those not ended (running
    // or asleep).
    spec(
        &["agent", "list"],
        &[],
        &[switch("--live"), switch("--json")],
        &["agent_list"],
    ),
    // A registration by its id with its agent's state, or a spawn by its id
    // or the caller's intent: its child's registration once it has one, the
    // spawn and its pane's state until then; else a registration by its name
    // or pane, the live one, else the last that ended there. It reads only.
    Spec {
        words: &["agent", "show"],
        arguments: AGENT_ID,
        repeats: None,
        needs_one_of: &[],
        options: &[switch("--json")],
        rest: None,
        answers: &["agent_show", "spawn_show"],
        refusals: &["agent_unavailable", "target_ambiguous"],
    },
    // The caller's own registration, by its attested pane, device and
    // session; only a pane-bound credential can ask.
    Spec {
        words: &["agent", "show", "here"],
        arguments: &[],
        repeats: None,
        needs_one_of: &[],
        options: &[switch("--json")],
        rest: None,
        answers: &["agent_show"],
        refusals: &[
            "agent_pane_required",
            "caller_identity_conflict",
            "participant_unavailable",
            "participant_registering",
            "participant_ended",
            "participant_session_changed",
            "ambiguous_participant",
        ],
    },
    // The caller declares that its next step is one only a person can take
    // (PRD ask-human): its lineage root stands in the operator's Needs You
    // until the operator presses Done, which sends the caller a letter. The
    // kind is `auth` or `permission`; the core, not the parser, refuses any
    // other with `human_kind_unknown` and the next action (a letter to the
    // parent), so the parser takes it as text.
    Spec {
        words: &["agent", "ask-human"],
        arguments: &[
            Arg {
                name: "kind",
                value: Text,
                required: true,
                value_with: None,
            },
            Arg {
                name: "where",
                value: Text,
                required: true,
                value_with: None,
            },
        ],
        repeats: None,
        needs_one_of: &[],
        options: &[switch("--json")],
        rest: None,
        answers: &["ask_human"],
        refusals: &[
            "human_kind_unknown",
            "human_place_required",
            "human_place_too_long",
            "agent_pane_required",
            "participant_unavailable",
            "participant_registering",
            "participant_ended",
            "participant_session_changed",
            "ambiguous_participant",
        ],
    },
    // Takes the caller's declaration down; `nothing_to_clear` when there
    // is none, which is no failure.
    Spec {
        words: &["agent", "ask-human", "--clear"],
        arguments: &[],
        repeats: None,
        needs_one_of: &[],
        options: &[switch("--json")],
        rest: None,
        answers: &["ask_human"],
        refusals: &[
            "agent_pane_required",
            "participant_unavailable",
            "participant_registering",
            "participant_ended",
            "participant_session_changed",
            "ambiguous_participant",
        ],
    },
    // The answer is the registration as it ended with what became of its
    // pane (`closed`, `kept`, `gone`, `other_agent`, `left`) and of its
    // conversation (`archived`, `archive_full`, `none`); `--keep-pane` ends
    // only the registration, as every end did before.
    Spec {
        words: &["agent", "end"],
        arguments: AGENT_ID,
        repeats: None,
        needs_one_of: &[],
        options: &[
            optional("--actor", Text),
            switch("--keep-pane"),
            switch("--json"),
        ],
        rest: None,
        answers: &["agent_end"],
        refusals: &[
            "agent_unavailable",
            "parent_authority_required",
            "actor_identity_conflict",
            "machine_unavailable",
            "pane_close_failed",
        ],
    },
    // The answer is `{id, state}`, state `ending` or `asleep`; asking again
    // answers the same (PRD mission-worker-sleep D-20).
    Spec {
        words: &["agent", "sleep"],
        arguments: AGENT_ID,
        repeats: None,
        needs_one_of: &[],
        options: &[optional("--actor", Text), switch("--json")],
        rest: None,
        answers: &["agent"],
        refusals: &[
            "agent_pane_required",
            "agent_unavailable",
            "agent_ended",
            "parent_authority_required",
            "actor_identity_conflict",
            "agent_sleep_unsupported",
            "agent_sleep_core_machine_only",
            "agent_sleep_working",
            "agent_sleep_waiting_for_you",
            "agent_sleep_activity_unknown",
            "agent_sleep_no_conversation",
            "agent_sleep_on_screen",
            "agent_sleep_waking",
            "agent_sleep_unreachable",
            "agent_sleep_busy",
            "agent_sleep_not_started",
        ],
    },
    // The answer is `{archived, pane_id}`: the archive has begun, its save
    // first and its closes after. A working session is refused
    // `archive_confirmation_required`, for the operator to confirm from
    // Hide's screen.
    Spec {
        words: &["agent", "archive"],
        arguments: AGENT_ID,
        repeats: None,
        needs_one_of: &[],
        options: &[optional("--actor", Text), switch("--json")],
        rest: None,
        answers: &["agent_archived"],
        refusals: &[
            "agent_pane_required",
            "agent_unavailable",
            "agent_ended",
            "parent_authority_required",
            "actor_identity_conflict",
            "archive_confirmation_required",
            "archive_not_offered",
            "archive_delegated",
            "archive_device_disconnected",
            "archive_session_unknown",
            "archive_status_unknown",
            "archive_target_unknown",
            "archive_too_many",
            "archive_busy",
            "archive_context_unavailable",
            "archive_full",
        ],
    },
    Spec {
        words: &["agent", "spawn"],
        arguments: &[],
        repeats: None,
        needs_one_of: &[],
        options: &[
            optional("--parent", Text),
            optional("--machine", Text),
            switch("--beside"),
            required("--name", Text),
            // One of the two: a plan step's intent is the core's to make.
            cleared("--intent", Text, &["--plan-node"]),
            cleared("--plan-node", Key, &["--intent"]),
            required("--kind", Text),
            // A plain folder's agent works in the folder itself, so both may
            // be left out; a Git project refuses that (`spawn_repo_required`).
            optional("--repo", Text),
            Opt {
                requires: &["--repo"],
                ..optional("--branch", Text)
            },
            Opt {
                requires: &["--branch"],
                ..optional("--path", Text)
            },
            optional("--profile", Text),
            optional("--model", Text),
            optional("--effort", Text),
            cleared("--brief", Text, &["--brief-file"]),
            cleared("--brief-file", Text, &["--brief"]),
            switch("--help"),
            switch("--json"),
        ],
        rest: Some("the agent's own arguments"),
        answers: &["agent"],
        refusals: &[
            "spawn_busy",
            "machine_unknown",
            "machine_not_permitted",
            "machine_unavailable",
            "repository_unavailable",
            "spawn_repo_required",
            "spawn_project_loading",
            "spawn_folder_unreadable",
            "agent_not_installed",
            "agent_hook_off",
            "agent_not_set_up",
            "agent_hook_untrusted",
            "agent_hook_missing",
            "agent_hook_unconfirmed",
            "agent_hook_unchecked",
            "intent_conflict",
            "invalid_launch_choice",
            "shell_starting",
            "holder_cap",
            "pane_busy",
            "agent_not_started",
            "worktree_unconfirmed",
            "native_identity_unavailable",
            "identity_waits_full",
            "session_too_long",
            "codex_capability_unknown",
            "invalid_brief",
            "brief_not_submitted",
            "mission_brief_missing",
            "mission_node_held",
            "mission_gate_before_start",
            "mission_machine_node_refused",
            "mission_machine_unavailable",
            "mission_projects_loading",
            "mission_machine_no_repository",
            "beside_other_checkout",
            "beside_other_machine",
            "beside_needs_agent_pane",
            "beside_with_parent",
            "beside_pane_unavailable",
            "plan_missing",
            "plan_node_unknown",
            "plan_node_finished",
            "plan_delegate_needs_parent",
            "plan_order_refused",
            "plan_review_pending",
            "plan_candidate_refused",
            "plan_delegate_full",
            "plan_delegate_not_permitted",
            "plan_unavailable",
            "plan_session_unknown",
            "profile_gone",
            "profiles_unsupported",
            "profile_refused",
            "profile_unanswered",
            "profile_unconfirmed",
            "launch_records_full",
        ],
    },
    // Without --json it prints the same answer as a tree for a person.
    Spec {
        words: &["ps"],
        arguments: &[],
        repeats: None,
        needs_one_of: &[],
        options: &[switch("--all"), switch("--here"), switch("--json")],
        rest: None,
        answers: &["ps"],
        refusals: &["here_unavailable", "ps_answer_unreadable"],
    },
    // An external tool's own connection (PRD hide-external-control D-25),
    // run outside Hide's panes on the core's machine. Approving, refusing,
    // the level and disconnecting are the operator's, in Settings ›
    // External tools; no command does them (B8).
    Spec {
        words: &["access", "request"],
        arguments: &[],
        repeats: None,
        needs_one_of: &[],
        options: &[required("--name", Key), switch("--json")],
        rest: None,
        answers: &["access_standing"],
        refusals: &[
            "access_from_pane",
            "access_refused",
            "access_request_expired",
            "access_revoked",
            "access_limit",
            "access_name_taken",
            "access_key_unreadable",
        ],
    },
    Spec {
        words: &["access", "status"],
        arguments: &[],
        repeats: None,
        needs_one_of: &[],
        options: &[switch("--json")],
        rest: None,
        answers: &["access_standing"],
        refusals: &[
            "access_from_pane",
            "access_ambiguous",
            "access_refused",
            "access_request_expired",
            "access_revoked",
            "access_key_unreadable",
        ],
    },
    Spec {
        words: &["access", "leave"],
        arguments: &[],
        repeats: None,
        needs_one_of: &[],
        options: &[switch("--json")],
        rest: None,
        answers: &["access_left"],
        refusals: &[
            "access_from_pane",
            "access_ambiguous",
            "access_not_connected",
            "access_key_unreadable",
        ],
    },
    Spec {
        words: &["access", "list"],
        arguments: &[],
        repeats: None,
        needs_one_of: &[],
        options: &[switch("--json")],
        rest: None,
        answers: &["access_list"],
        refusals: &[
            "access_not_connected",
            "access_ambiguous",
            "access_revoked",
            "access_key_unreadable",
        ],
    },
    // Only the core's own machine may run these. Add registers the device
    // without Hide's helper, which the operator allows on its row. A device
    // answers its state, what it waits on from the operator, its failure by
    // the row's action and the next step, never ssh's words.
    Spec {
        words: &["device", "list"],
        arguments: &[],
        repeats: None,
        needs_one_of: &[],
        options: &[switch("--json")],
        rest: None,
        answers: &["device_list"],
        refusals: &["machine_not_permitted"],
    },
    Spec {
        words: &["device", "add"],
        arguments: &[Arg {
            name: "ssh_alias",
            value: Key,
            required: true,
            value_with: None,
        }],
        repeats: None,
        needs_one_of: &[],
        options: &[optional("--name", Text), switch("--json")],
        rest: None,
        answers: &["device_added"],
        refusals: &[
            "machine_not_permitted",
            "device_invalid",
            "device_duplicate",
        ],
    },
    Spec {
        words: &["device", "remove"],
        arguments: &[Arg {
            name: "device_id",
            value: Key,
            required: true,
            value_with: None,
        }],
        repeats: None,
        needs_one_of: &[],
        options: &[switch("--json")],
        rest: None,
        answers: &["device_removed"],
        refusals: &[
            "machine_not_permitted",
            "device_unknown",
            "device_local_remove_denied",
            "device_files_open",
        ],
    },
    // Starts the row's Retry, an attempt that may ask the operator; with
    // --wait it answers the attempt's outcome. Nothing answers what the
    // attempt asks: that is the operator's, on the row.
    Spec {
        words: &["device", "retry"],
        arguments: &[Arg {
            name: "device_id",
            value: Key,
            required: true,
            value_with: None,
        }],
        repeats: None,
        needs_one_of: &[],
        options: &[switch("--wait"), switch("--json")],
        rest: None,
        answers: &["device_retried"],
        refusals: &[
            "machine_not_permitted",
            "device_unknown",
            "device_dials_in",
            "device_wait_timeout",
        ],
    },
    // An agent's reads of and input to the panes of its machine, and its
    // checkout's purpose, instead of Herdr's own commands (PRD
    // engine-bundle-1 D-5, D-13). A read prints the pane's text unless
    // --json asks for the answer line.
    Spec {
        words: &["pane", "read"],
        arguments: &[Arg {
            name: "pane",
            value: Key,
            required: true,
            value_with: None,
        }],
        repeats: None,
        needs_one_of: &[],
        options: &[
            Opt {
                conflicts: &["--source", "--format"],
                ..optional("--lines", Unsigned)
            },
            Opt {
                requires: &["--format"],
                conflicts: &["--lines"],
                ..optional("--source", OneOf(&["visible"]))
            },
            Opt {
                requires: &["--source"],
                conflicts: &["--lines"],
                ..optional("--format", OneOf(&["ansi"]))
            },
            switch("--json"),
        ],
        rest: None,
        answers: &["pane_read"],
        refusals: &["pane_required", "lines_out_of_range", "pane_unavailable"],
    },
    Spec {
        words: &["pane", "send"],
        arguments: &[
            Arg {
                name: "pane",
                value: Key,
                required: true,
                value_with: None,
            },
            // No control character, as a key the caller did not name; with
            // `--paste`, line breaks and tabs too.
            Arg {
                name: "text",
                value: Text,
                required: false,
                value_with: Some(("--paste", PasteText)),
            },
        ],
        repeats: None,
        // Text, keys or Enter: each key is pressed in order after the text,
        // then Enter.
        needs_one_of: &["text", "--key", "--enter"],
        options: &[
            repeated(
                "--key",
                OneOf(&hide_core::engine::keys::PaneKey::NAMES),
                hide_core::pane_command::KEYS_LIMIT,
            ),
            switch("--enter"),
            switch("--paste"),
        ],
        rest: None,
        answers: &["pane_sent"],
        refusals: &[
            "pane_required",
            "nothing_to_send",
            "text_empty",
            "text_invalid",
            "text_too_long",
            "too_many_keys",
            "paste_unsupported",
            "pane_unavailable",
            "pane_asleep",
            "composer_not_empty",
            "keys_not_sent",
            "send_unconfirmed",
            "operator_typing",
        ],
    },
    Spec {
        words: &["pane", "open"],
        arguments: &[],
        repeats: None,
        needs_one_of: &[],
        options: &[
            Opt {
                name: "--beside",
                value: None,
                required: true,
                requires: &[],
                conflicts: &[],
                repeats: None,
            },
            switch("--focus"),
        ],
        rest: Some(
            "the command the new pane's shell runs, its words joined by spaces into one line",
        ),
        answers: &["pane_opened"],
        refusals: &[
            "pane_required",
            "command_invalid",
            "command_too_long",
            "pane_unavailable",
            "pane_open_unconfirmed",
            "shell_not_ready",
        ],
    },
    Spec {
        words: &["agent", "read"],
        arguments: &[Arg {
            name: "agent",
            value: Key,
            required: true,
            value_with: None,
        }],
        repeats: None,
        needs_one_of: &[],
        options: &[optional("--lines", Unsigned), switch("--json")],
        rest: None,
        answers: &["pane_read"],
        refusals: &[
            "pane_required",
            "lines_out_of_range",
            "agent_unavailable",
            "target_ambiguous",
        ],
    },
    Spec {
        words: &["workspace", "purpose"],
        arguments: &[Arg {
            name: "text",
            value: Text,
            required: true,
            value_with: None,
        }],
        repeats: None,
        needs_one_of: &[],
        options: &[],
        rest: None,
        answers: &["purpose_set"],
        refusals: &[
            "pane_required",
            "purpose_invalid",
            "purpose_too_long",
            "purpose_no_workspace",
        ],
    },
    // The delete dialog's removal: --force accepts every loss the deletion
    // gate names, --delete-branch takes the local branch with the folder.
    Spec {
        words: &["workspace", "remove"],
        arguments: &[Arg {
            name: "path",
            value: Text,
            required: true,
            value_with: None,
        }],
        repeats: None,
        needs_one_of: &[],
        options: &[switch("--delete-branch"), switch("--force")],
        rest: None,
        answers: &["worktree_removed"],
        refusals: &[
            "worktree_remove_unknown",
            "worktree_remove_blocked",
            "worktree_remove_needs_force",
            "worktree_remove_branch_unmerged",
            "worktree_remove_branch_kept",
            "worktree_remove_agent_working",
            "worktree_remove_own_checkout",
            "worktree_remove_busy",
            "worktree_remove_unavailable",
            "worktree_remove_stopped",
            "worktree_remove_still_running",
            "worktree_remove_pane_appeared",
            "worktree_remove_changed",
            "worktree_remove_close_failed",
            "worktree_remove_host_failed",
            "worktree_remove_result_unknown",
            "worktree_remove_pending",
        ],
    },
    // --force types the letter into the recipient's input now, even while
    // it works, and is refused, with nothing recorded or typed, while
    // someone's text shows there (PRD engine-bundle-1 D-14). The target
    // `parent` is refused `no_parent` for a sender no agent answers for, and
    // `participant_registering` for the seconds the spawn that started the
    // sender takes to register it.
    Spec {
        words: &["request", "send"],
        arguments: TARGET,
        repeats: None,
        needs_one_of: &[],
        options: &[
            required("--intent", Key),
            required("--body", Body),
            optional("--kind", OneOf(&["request", "block", "report", "progress"])),
            clearing("--now", &["--force"]),
            clearing("--force", &["--now"]),
        ],
        rest: None,
        answers: &["letter"],
        refusals: &[
            "composer_not_empty",
            "composer_unconfirmed",
            "force_target_asleep",
            "intent_conflict",
            "no_parent",
            "participant_registering",
        ],
    },
    spec(
        &["request", "reply"],
        ID,
        &[required("--intent", Key), required("--body", Body)],
        &["letter"],
    ),
    spec(&["request", "ack"], ID, &[], &["letter"]),
    spec(&["request", "cancel"], ID, &[], &["letter"]),
    spec(&["request", "show"], ID, &[], &["letter"]),
    spec(&["inbox"], &[], &[], &["inbox"]),
    spec(
        &["inbox", "--hook"],
        &[],
        &[
            optional("--prompt-digest", Sha256),
            optional("--session", Session),
            Opt {
                name: "--tool",
                value: None,
                required: false,
                requires: &["--session"],
                conflicts: &["--turn-end"],
                repeats: None,
            },
            Opt {
                name: "--turn-end",
                value: None,
                required: false,
                requires: &["--session"],
                conflicts: &["--tool"],
                repeats: None,
            },
        ],
        &["intake"],
    ),
    Spec {
        words: &["inbox", "--confirm"],
        arguments: ID,
        repeats: Some(BUNDLE_LETTERS),
        needs_one_of: &[],
        options: &[],
        rest: None,
        answers: &["confirmed"],
        refusals: &[],
    },
    spec(&["inbox", "--ack"], ID, &[], &["acknowledged"]),
    spec(
        &["watch", "start"],
        TARGET,
        &[optional("--observer", Key), optional("--actor", Key)],
        &["watch"],
    ),
    spec(
        &["watch", "assign"],
        ID,
        &[
            required("--observer", Key),
            optional("--actor", Key),
            optional("--expected-generation", Unsigned),
            Opt {
                name: "--approval",
                value: Some(Approval),
                required: false,
                requires: &["--expected-generation"],
                conflicts: &[],
                repeats: None,
            },
        ],
        &["watch"],
    ),
    spec(&["watch", "stop"], ID, &[], &["stopped"]),
    spec(&["watch", "list"], &[], &[], &["watch_list"]),
    spec(
        &["agent", "catalog"],
        &[],
        &[optional("--machine", Text), switch("--json")],
        &["agent_catalog"],
    ),
    Spec {
        words: &["mission", "create"],
        arguments: &[],
        repeats: None,
        needs_one_of: &[],
        options: &[
            required("--title", Text),
            optional("--scope", OneOf(&["pr", "merge"])),
            optional("--cap", Unsigned),
            optional("--github", Text),
        ],
        rest: None,
        answers: &["mission_created"],
        refusals: &[
            "mission_already_conducting",
            "mission_full",
            "mission_projects_loading",
            "mission_project_unknown",
            "mission_cap_out_of_range",
        ],
    },
    Spec {
        words: &["mission", "plan"],
        arguments: &[],
        repeats: None,
        needs_one_of: &[],
        options: &[
            optional("--mission", Key),
            Opt {
                name: "--file",
                value: Some(Text),
                required: false,
                requires: &[],
                conflicts: &["--json"],
                repeats: None,
            },
            Opt {
                name: "--json",
                value: Some(Json),
                required: false,
                requires: &[],
                conflicts: &["--file"],
                repeats: None,
            },
        ],
        rest: None,
        answers: &["mission_planned"],
        refusals: &[
            "mission_plan_invalid",
            "mission_not_conductor",
            "mission_closed",
            "mission_cancelled",
            "mission_too_many_nodes",
            "mission_node_in_progress",
            "mission_dependency_unknown",
            "mission_dependency_cycle",
            "mission_criteria_missing",
            "mission_criteria_too_many",
            "mission_steps_invalid",
            "mission_checkers_invalid",
            "mission_external_invalid",
            "mission_destination_invalid",
            "mission_needs_git",
            "mission_operator_merge_invalid",
            "mission_preset_invalid",
            "mission_gate_node_started",
            "mission_machine_node_refused",
            "mission_profile_unknown",
            "mission_candidate_refused",
        ],
    },
    Spec {
        words: &["mission", "node"],
        arguments: &[Arg {
            name: "node",
            value: Key,
            required: true,
            value_with: None,
        }],
        repeats: None,
        needs_one_of: &[],
        options: &[
            optional("--mission", Key),
            optional("--status", OneOf(crate::mission_cli::STATUSES)),
            cleared("--agent-id", Key, &["--no-agent-id"]),
            cleared("--pr", Unsigned, &["--no-pr"]),
            cleared("--base-pr", Unsigned, &["--no-base-pr"]),
            cleared("--branch", Text, &["--no-branch"]),
            optional("--attempts", Unsigned),
            cleared("--line", Text, &["--no-line"]),
            cleared("--change", Text, &["--no-change"]),
            cleared("--review", Text, &["--no-review"]),
            cleared("--commit", Commit, &["--no-commit"]),
            cleared("--verdict", OneOf(&["pass", "fail"]), &["--no-verdict"]),
            cleared("--checks", Ratio, &["--no-checks"]),
            clearing("--no-agent-id", &["--agent-id"]),
            clearing("--no-pr", &["--pr"]),
            clearing("--no-base-pr", &["--base-pr"]),
            clearing("--no-branch", &["--branch"]),
            clearing("--no-line", &["--line"]),
            clearing("--no-change", &["--change"]),
            clearing("--no-review", &["--review"]),
            clearing("--no-commit", &["--commit"]),
            clearing("--no-verdict", &["--verdict"]),
            clearing("--no-checks", &["--checks"]),
            optional("--reviewer", Key),
            optional("--verifier", Key),
            Opt {
                name: "--hold",
                value: Some(Text),
                required: false,
                requires: &["--for"],
                conflicts: &["--no-hold"],
                repeats: None,
            },
            Opt {
                name: "--for",
                value: Some(Span),
                required: false,
                requires: &["--hold"],
                conflicts: &["--no-hold"],
                repeats: None,
            },
            clearing("--no-hold", &["--hold", "--for"]),
        ],
        rest: None,
        answers: &["mission"],
        refusals: &[
            "mission_node_held",
            "mission_node_started",
            "mission_gate_before_start",
            "mission_node_unknown",
            "mission_not_conductor",
            "mission_ambiguous",
            "mission_closed",
            "mission_criteria_unmet",
            "mission_checker_is_worker",
            "mission_checker_unknown",
            "mission_external_invalid",
            "mission_machine_node_refused",
            "mission_machine_unavailable",
            "mission_projects_loading",
            "mission_machine_no_repository",
        ],
    },
    Spec {
        words: &["mission", "record"],
        arguments: &[
            Arg {
                name: "node",
                value: Key,
                required: true,
                value_with: None,
            },
            Arg {
                name: "line",
                value: Key,
                required: true,
                value_with: None,
            },
        ],
        repeats: None,
        needs_one_of: &[],
        options: &[
            optional("--mission", Key),
            Opt {
                name: "--pass",
                value: None,
                required: false,
                requires: &[],
                conflicts: &["--fail"],
                repeats: None,
            },
            Opt {
                name: "--fail",
                value: None,
                required: false,
                requires: &[],
                conflicts: &["--pass"],
                repeats: None,
            },
            required("--evidence", Text),
            optional("--summary", Text),
            optional("--commit", Commit),
            switch("--rerun"),
        ],
        rest: None,
        answers: &["mission"],
        refusals: &[
            "mission_node_unknown",
            "mission_criterion_unknown",
            "mission_criterion_not_yours",
            "participant_session_changed",
            "participant_registering",
            "participant_unavailable",
            "ledger_unavailable",
            "mission_ambiguous",
            "mission_commit_required",
            "mission_rerun_spent",
            "mission_evidence_missing",
            "mission_summary_invalid",
            "mission_checkout_unknown",
            "mission_closed",
        ],
    },
    Spec {
        words: &["mission", "skip"],
        arguments: &[
            Arg {
                name: "node",
                value: Key,
                required: true,
                value_with: None,
            },
            Arg {
                name: "line",
                value: Key,
                required: true,
                value_with: None,
            },
        ],
        repeats: None,
        needs_one_of: &[],
        options: &[
            optional("--mission", Key),
            required("--reason", Text),
            optional("--decision", Unsigned),
        ],
        rest: None,
        answers: &["mission"],
        refusals: &[
            "mission_not_conductor",
            "mission_node_unknown",
            "mission_criterion_unknown",
            "mission_ambiguous",
            "mission_skip_reason_missing",
            "mission_criteria_protected",
        ],
    },
    Spec {
        words: &["mission", "defect"],
        arguments: &[Arg {
            name: "node",
            value: Key,
            required: true,
            value_with: None,
        }],
        repeats: None,
        needs_one_of: &[],
        options: &[
            optional("--mission", Key),
            required("--found-by", Key),
            required("--line", Text),
        ],
        rest: None,
        answers: &["mission"],
        refusals: &[
            "mission_not_conductor",
            "mission_node_unknown",
            "mission_external_invalid",
            "mission_defects_too_many",
        ],
    },
    Spec {
        words: &["mission", "gate"],
        arguments: &[Arg {
            name: "node",
            value: Key,
            required: true,
            value_with: None,
        }],
        repeats: None,
        needs_one_of: &[],
        options: &[
            optional("--mission", Key),
            Opt {
                name: "--pass",
                value: None,
                required: false,
                requires: &[],
                conflicts: &["--fix"],
                repeats: None,
            },
            Opt {
                name: "--fix",
                value: None,
                required: false,
                requires: &[],
                conflicts: &["--pass"],
                repeats: None,
            },
            required("--said", Text),
        ],
        rest: None,
        answers: &["mission"],
        refusals: &[
            "mission_not_conductor",
            "mission_node_unknown",
            "mission_gate_none",
            "mission_gate_not_open",
            "mission_gate_words_missing",
            "mission_closed",
        ],
    },
    spec(
        &["mission", "start"],
        &[],
        &[optional("--mission", Key)],
        &["mission"],
    ),
    spec(
        &["mission", "decide"],
        &[],
        &[
            optional("--mission", Key),
            cleared("--line", Text, &["--clear"]),
            clearing("--clear", &["--line", "--show"]),
            optional("--why", Text),
            optional("--option", Text),
            optional("--recommend", Unsigned),
            // Given more than once like `--option`: the first names the node
            // the decision stopped, the rest the other nodes it is about.
            optional("--node", Key),
            // A View id or a path the CLI opens as one; given more than once
            // like `--option`, up to the core's `missions::VIEW_LINK_LIMIT`.
            optional("--show", Text),
        ],
        &["mission"],
    ),
    spec(
        &["mission", "delay"],
        &[],
        &[
            optional("--mission", Key),
            cleared("--line", Text, &["--clear"]),
            cleared("--for", Span, &["--clear"]),
            clearing("--clear", &["--line", "--for"]),
        ],
        &["mission"],
    ),
    Spec {
        words: &["mission", "close"],
        arguments: &[],
        repeats: None,
        needs_one_of: &[],
        options: &[
            optional("--mission", Key),
            required("--as", OneOf(&["done", "merge_wait", "cancelled"])),
            cleared("--report", Text, &["--report-file"]),
            cleared("--report-file", Text, &["--report"]),
        ],
        rest: None,
        answers: &["mission"],
        refusals: &[
            "mission_nodes_unfinished",
            "mission_criteria_unmet",
            "mission_not_started",
            "mission_cancelled",
        ],
    },
    spec(
        &["mission", "reopen"],
        &[],
        &[optional("--mission", Key)],
        &["mission"],
    ),
    Spec {
        words: &["mission", "take"],
        arguments: ID,
        repeats: None,
        needs_one_of: &[],
        options: &[optional("--handover", Text)],
        rest: None,
        answers: &["mission"],
        refusals: &[
            "mission_conducted",
            "mission_cancelled",
            "mission_other_project",
        ],
    },
    // The mission as text, or one node of it with its worker's newest
    // letters; --json prints the answer line (with-hide/hide-private#552).
    Spec {
        words: &["mission", "show"],
        arguments: &[Arg {
            name: "node",
            value: Key,
            required: false,
            value_with: None,
        }],
        repeats: None,
        needs_one_of: &[],
        options: &[optional("--mission", Key), switch("--json")],
        rest: None,
        answers: &["mission", "mission_node"],
        refusals: &["mission_node_unknown", "mission_ambiguous"],
    },
    spec(&["mission", "list"], &[], &[], &["mission_list"]),
    spec(&["mission", "settings"], &[], &[], &["mission_settings"]),
    Spec {
        words: &["task", "set"],
        arguments: &[],
        repeats: None,
        needs_one_of: &[],
        options: &[
            Opt {
                name: "--file",
                value: Some(Text),
                required: false,
                requires: &[],
                conflicts: &["--json"],
                repeats: None,
            },
            Opt {
                name: "--json",
                value: Some(Json),
                required: false,
                requires: &[],
                conflicts: &["--file"],
                repeats: None,
            },
            // `--ask [why]`: the reason may follow as the next word.
            switch("--ask"),
            optional("--reason", Text),
        ],
        rest: None,
        answers: &["plan"],
        refusals: &[
            "plan_invalid",
            "plan_order_refused",
            "plan_too_large",
            "plan_full",
            "plan_unavailable",
            "plan_session_unknown",
            "plan_evidence_missing",
            "plan_pr_not_yours",
            "plan_pr_unconfirmed",
            "plan_file_outside",
            "plan_file_missing",
            "plan_file_unconfirmed",
            "plan_report_missing",
        ],
    },
    Spec {
        words: &["task", "node"],
        arguments: &[Arg {
            name: "key",
            value: Key,
            required: true,
            value_with: None,
        }],
        repeats: None,
        needs_one_of: &[],
        options: &[
            // `done` first: the sample takes the first value, and --pr and
            // --file go with --status done only.
            optional(
                "--status",
                OneOf(&["done", "planned", "working", "dropped"]),
            ),
            optional("--line", Text),
            cleared("--pr", Unsigned, &["--file"]),
            cleared("--file", Text, &["--pr"]),
        ],
        rest: None,
        answers: &["plan"],
        refusals: &[
            "plan_missing",
            "plan_node_unknown",
            "plan_order_refused",
            "plan_evidence_missing",
            "plan_pr_not_yours",
            "plan_pr_unconfirmed",
            "plan_file_outside",
            "plan_file_missing",
            "plan_file_unconfirmed",
            "plan_report_missing",
        ],
    },
    Spec {
        words: &["task", "approve"],
        arguments: &[],
        repeats: None,
        needs_one_of: &[],
        options: &[],
        rest: None,
        answers: &["plan"],
        refusals: &["plan_missing", "plan_not_yours", "plan_agent_unknown"],
    },
    Spec {
        words: &["task", "drop"],
        arguments: &[],
        repeats: None,
        needs_one_of: &[],
        options: &[],
        rest: None,
        answers: &["plan"],
        refusals: &["plan_not_yours", "plan_agent_unknown"],
    },
    Spec {
        words: &["task", "show"],
        arguments: &[],
        repeats: None,
        needs_one_of: &[],
        options: &[],
        rest: None,
        answers: &["plan"],
        refusals: &["plan_agent_unknown"],
    },
];

/// The command `args` (the words after `hide`) name, by its longest words.
/// Whether `topic` is one whose every command the table names.
pub fn topic(topic: &str) -> bool {
    TOPICS.contains(&topic)
}

/// Whether the table names the command `args` start with.
pub fn names(args: &[String]) -> bool {
    find(args).is_some()
}

fn find(args: &[String]) -> Option<&'static Spec> {
    COMMANDS
        .iter()
        .filter(|spec| {
            spec.words.len() <= args.len()
                && spec.words.iter().zip(args).all(|(word, arg)| word == arg)
        })
        .max_by_key(|spec| spec.words.len())
}

/// Refuses a command of the contract's topics that the table does not name,
/// and a flag the table does not give the command, before its parser reads
/// either. The parser still checks the values.
pub fn admit(args: &[String]) -> Result<(), String> {
    let Some(spec) = find(args) else {
        return match args {
            [topic] if TOPICS.contains(&topic.as_str()) => {
                Err(format!("hide {topic} needs a subcommand"))
            }
            [topic, verb, ..] if TOPICS.contains(&topic.as_str()) => {
                Err(format!("unknown command: hide {topic} {verb}"))
            }
            _ => Ok(()),
        };
    };
    let mut rest = args[spec.words.len()..].iter();
    while let Some(arg) = rest.next() {
        if arg == "--" && spec.rest.is_some() {
            break;
        }
        if !arg.starts_with("--") {
            continue;
        }
        let option = spec
            .options
            .iter()
            .find(|option| option.name == arg)
            .ok_or_else(|| format!("unknown flag for hide {}: {arg}", spec.words.join(" ")))?;
        if option.value.is_some() {
            rest.next();
        }
    }
    Ok(())
}

fn value_json(value: Input) -> Value {
    match value {
        OneOf(values) => json!({"type": value.name(), "values": values}),
        _ => json!({"type": value.name()}),
    }
}

fn command_json(spec: &Spec) -> Value {
    let mut command = json!({
        "command": spec.words.join(" "),
        "arguments": spec.arguments.iter().map(|argument| {
            let mut value = json!({
                "name": argument.name,
                "value": value_json(argument.value),
            });
            if !argument.required {
                value["required"] = json!(false);
            }
            if let Some((option, widened)) = argument.value_with {
                value["value_with"] = json!({option: value_json(widened)});
            }
            value
        }).collect::<Vec<_>>(),
        "repeats_at_most": spec.repeats,
        "options": spec.options.iter().map(|option| {
            let mut value = json!({
            "name": option.name,
            "value": option.value.map(value_json),
            "required": option.required,
            "requires": option.requires,
            });
            if !option.conflicts.is_empty() {
                value["conflicts"] = json!(option.conflicts);
            }
            if let Some(most) = option.repeats {
                value["repeats_at_most"] = json!(most);
            }
            value
        }).collect::<Vec<_>>(),
        "rest": spec.rest,
        "answers": spec.answers,
    });
    if !spec.refusals.is_empty() {
        command["refusals"] = json!(spec.refusals);
    }
    if !spec.needs_one_of.is_empty() {
        command["needs_one_of"] = json!(spec.needs_one_of);
    }
    command
}

/// The contract without its digest.
fn body() -> Value {
    json!({
        "format": FORMAT,
        "commands": COMMANDS.iter().map(command_json).collect::<Vec<_>>(),
        "value_types": {
            "text": "non-empty, no control characters, does not start with --",
            "key": "non-empty, at most 256 bytes, no control characters, does not start with -",
            "session": format!(
                "non-empty, at most {} bytes, no control characters, does not start with -",
                hide_core::delivery::SESSION_LIMIT
            ),
            "body": format!("not blank, at most {BODY_LIMIT} bytes"),
            "approval": "not blank, at most 256 bytes, no control characters",
            "commit": "7 to 64 hexadecimal digits",
            "ratio": "two decimal integers as <passed>/<total>, such as 6/6",
            "span": "whole minutes or hours, such as 30m or 2h",
            "unsigned": "a decimal integer from 0 to 18446744073709551615",
            "json": "a JSON document of the shape hide mission help or hide task help shows",
            "paste_text": format!(
                "non-empty, at most {} bytes, no control characters but line breaks (LF, CR) and tabs",
                hide_core::pane_command::TEXT_LIMIT
            ),
            "one_of": "one of the listed values",
        },
        // Where each topic's printed line carries the answer and, when the
        // command fails (a non-zero exit), the failure's code.
        "envelopes": {
            "agent": {"ok": "ok", "answer": "value", "code": "error.code"},
            "ps": {"ok": "ok", "answer": "value", "code": "error.code"},
            "request": {"ok": "ok", "answer": "result", "code": "reason"},
            "inbox": {"ok": "ok", "answer": "result", "code": "reason"},
            "watch": {"ok": "ok", "answer": "result", "code": "reason"},
            "mission": {"ok": "ok", "answer": "result", "code": "reason"},
            "plan": {"ok": "ok", "answer": "result", "code": "reason"},
            "device": {"ok": "ok", "answer": "value", "code": "error.code"},
            "pane": {"ok": "ok", "answer": "value", "code": "error.code"},
            "workspace": {"ok": "ok", "answer": "value", "code": "error.code"},
            "access": {"ok": "ok", "answer": "value", "code": "error.code"},
        },
        // An external tool runs the commands above as its connection, at
        // the level the operator chose; any of them may answer one of
        // these, with its next_action (PRD hide-external-control B20-B23).
        "access": {
            "choose": crate::env::HIDE_ACCESS,
            "levels": ["view", "act"],
            "refusals": crate::access::REFUSALS
                .iter()
                .map(|code| (
                    (*code).to_owned(),
                    json!(crate::access::next_action(code).unwrap_or_default()),
                ))
                .collect::<Map<String, Value>>(),
        },
        "answers": answers(),
    })
}

/// The answer schemas: the delivery answers' own, and the mission
/// commands' envelopes, whose mission object is documented by `hide
/// mission help` and the snapshot it is drawn from.
fn answers() -> Value {
    let mut answers = hide_core::delivery::answer::answer_schemas();
    let object = json!({"type": "object"});
    let shape = |fields: &[&str]| {
        let properties: Map<String, Value> = fields
            .iter()
            .map(|field| ((*field).to_owned(), object.clone()))
            .collect();
        json!({"type": "object", "properties": properties, "required": fields})
    };
    let map = answers.as_object_mut().expect("the answers are an object");
    map.insert("mission".into(), shape(&["mission"]));
    map.insert("mission_created".into(), shape(&["mission", "settings"]));
    map.insert("mission_planned".into(), shape(&["mission", "overlaps"]));
    map.insert("mission_list".into(), shape(&["items"]));
    map.insert("mission_settings".into(), shape(&["settings"]));
    // One node of the mission and its worker's newest letters, null while
    // the ledger cannot be read.
    map.insert(
        "mission_node".into(),
        json!({
            "type": "object",
            "properties": {
                "mission_id": {"type": "string"},
                "node": object,
                "letters": {"type": ["array", "null"]},
            },
            "required": ["mission_id", "node", "letters"],
        }),
    );
    // The plan's own record and, for the caller's own plan, the summary its
    // hook keeps (`contracts/delivery-envelope.json`, `plan_context`).
    map.insert("plan".into(), shape(&["session", "plan"]));
    map.insert(
        "agent_catalog".into(),
        shape(&["machine", "installed_known", "items"]),
    );
    map.insert("agent_archived".into(), shape(&["archived", "pane_id"]));
    map.insert("agent_profiles".into(), shape(&["machine", "profiles"]));
    map.insert("agent_profile".into(), shape(&["machine", "profile"]));
    map.insert(
        "agent_profile_removed".into(),
        shape(&["machine", "removed", "projects_back_to_default"]),
    );
    // Where a tool stands: not_connected, pending (with next_action) or
    // connected (with level); its leave; and the tools and requests.
    map.insert("access_standing".into(), shape(&["state"]));
    map.insert("access_left".into(), shape(&["state", "name"]));
    map.insert(
        "access_list".into(),
        shape(&["machine", "limit", "pending", "connections"]),
    );
    map.insert("device_list".into(), shape(&["devices"]));
    map.insert("device_added".into(), shape(&["device", "next_action"]));
    map.insert("device_removed".into(), shape(&["device_id"]));
    map.insert(
        "device_retried".into(),
        hide_core::device_command::retried_schema(),
    );
    for (name, schema) in hide_core::pane_command::answer_schemas() {
        map.insert(name.into(), schema);
    }
    for (name, schema) in hide_core::worktree_command::answer_schemas() {
        map.insert(name.into(), schema);
    }
    answers
}

/// `value` with every object's keys in order, so the digest does not depend
/// on whether a build orders JSON maps by insertion.
fn canonical(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, canonical(value)))
                    .collect::<Map<_, _>>(),
            )
        }
        Value::Array(items) => Value::Array(items.into_iter().map(canonical).collect()),
        other => other,
    }
}

/// `sha256:<hex>` of the canonical contract body.
pub fn digest() -> String {
    digest_of(&canonical(body()))
}

fn digest_of(body: &Value) -> String {
    let bytes = serde_json::to_vec(body).expect("a JSON value serializes");
    format!(
        "sha256:{}",
        hex::encode(ring::digest::digest(&ring::digest::SHA256, &bytes))
    )
}

/// The document `hide contract --json` prints: the body and its digest.
pub fn document() -> Value {
    let body = canonical(body());
    let digest = digest_of(&body);
    let Value::Object(mut map) = body else {
        unreachable!("the body is an object")
    };
    map.insert("digest".to_owned(), Value::String(digest));
    canonical(Value::Object(map))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{CommandKind, parse_args};

    /// A value the parser takes, and one the value type rules out.
    fn sample(value: Input) -> (String, String) {
        let (taken, refused) = match value {
            Text => ("some text", "--x"),
            Key => ("k1", "-x"),
            Session => ("/sessions/a.jsonl", "-x"),
            Body => ("multi\nline body", " "),
            Sha256 => (
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
                "BA7816BF",
            ),
            Commit => ("abc1234", "xyz"),
            Ratio => ("3/4", "x/4"),
            Span => ("30m", "30"),
            Approval => ("approved by the lead", " "),
            Unsigned => ("1", "-1"),
            Json => ("{\"nodes\": []}", "{nodes"),
            PasteText => ("line one\nline two\tend", "\u{1b}[201~"),
            OneOf(values) => (values[0], "other"),
        };
        (taken.to_owned(), refused.to_owned())
    }

    /// The command line with every argument and option the contract names,
    /// less `leave_out`, with `refuse`'s value replaced by a refused one.
    fn argv(spec: &Spec, leave_out: &[&str], refuse: Option<&str>, repeat: usize) -> Vec<String> {
        let mut argv = vec!["hide".to_owned()];
        argv.extend(spec.words.iter().map(|word| (*word).to_owned()));
        for (index, argument) in spec.arguments.iter().enumerate() {
            let (taken, refused) = sample(argument.value);
            let value = if refuse == Some(argument.name) {
                refused
            } else {
                taken
            };
            let last = index + 1 == spec.arguments.len();
            for copy in 0..if last { repeat } else { 1 } {
                argv.push(format!(
                    "{value}{}",
                    if copy == 0 {
                        String::new()
                    } else {
                        copy.to_string()
                    }
                ));
            }
        }
        let mut selected = Vec::new();
        for option in spec.options {
            // Help exits before delivery and has its own standalone argv.
            if option.name == "--help"
                || leave_out.contains(&option.name)
                || option.conflicts.iter().any(|name| selected.contains(name))
            {
                continue;
            }
            selected.push(option.name);
            argv.push(option.name.to_owned());
            if let Some(value) = option.value {
                let (taken, refused) = sample(value);
                argv.push(if refuse == Some(option.name) {
                    refused
                } else {
                    taken
                });
            }
        }
        if spec.rest.is_some() {
            argv.extend(["--".to_owned(), "--model".to_owned()]);
        }
        argv
    }

    fn takes(argv: &[String]) -> bool {
        // A pane command's own check runs before anything is asked, as
        // `pane_cli::run` does: it refuses what the parser leaves to it.
        if let Ok(CommandKind::Pane(cli)) = parse_args(argv) {
            return cli.command.check().is_ok();
        }
        matches!(
            parse_args(argv),
            Ok(CommandKind::Delivery(_)
                | CommandKind::AgentSpawnBriefFile { .. }
                | CommandKind::Mission(_)
                | CommandKind::Plan(_)
                | CommandKind::AgentCatalog { .. }
                | CommandKind::AgentProfile(_)
                | CommandKind::Ps(_)
                | CommandKind::AgentSleep { .. }
                | CommandKind::AgentArchive { .. }
                | CommandKind::Device(_)
                | CommandKind::Access(_)
                | CommandKind::Pane(_)
                | CommandKind::WorkspaceRemove(_))
        )
    }

    /// Each command the contract names parses with everything it names, and
    /// refuses what the contract rules out: a required option left out, an
    /// option without the one it requires, a value outside its type, one
    /// repetition too many. A parser that drifts from the table fails here.
    #[test]
    fn each_command_takes_what_the_contract_names_and_nothing_it_rules_out() {
        for spec in COMMANDS {
            let command = spec.words.join(" ");
            if spec.options.iter().any(|option| option.name == "--help") {
                let mut help = vec!["hide".to_owned()];
                help.extend(spec.words.iter().map(|word| (*word).to_owned()));
                help.push("--help".to_owned());
                assert_eq!(parse_args(&help).unwrap(), CommandKind::AgentSpawnHelp);
            }
            let most = spec.repeats.unwrap_or(1);
            let full = argv(spec, &[], None, most);
            assert!(takes(&full), "hide {command}: {:?}", parse_args(&full));
            if spec.repeats.is_some() {
                assert!(
                    !takes(&argv(spec, &[], None, most + 1)),
                    "hide {command} repeated"
                );
            }
            for option in spec.options {
                // Exercise each side of an exclusive choice, not just the
                // first one the full sample selected.
                let allowed = argv(spec, option.conflicts, None, most);
                assert!(
                    takes(&allowed),
                    "hide {command} {}: {:?}",
                    option.name,
                    parse_args(&allowed)
                );
                for conflict in option.conflicts {
                    let other = spec
                        .options
                        .iter()
                        .find(|other| other.name == *conflict)
                        .unwrap();
                    // The conflict goes before `--`: after it, every word
                    // is the agent's own, not an option of the command.
                    let mut both = allowed.clone();
                    let at = both
                        .iter()
                        .position(|word| word == "--")
                        .unwrap_or(both.len());
                    both.insert(at, other.name.to_owned());
                    if let Some(value) = other.value {
                        both.insert(at + 1, sample(value).0);
                    }
                    assert!(
                        !takes(&both),
                        "hide {command} {} with {conflict}",
                        option.name
                    );
                }
                if option.required {
                    assert!(
                        !takes(&argv(spec, &[option.name], None, most)),
                        "hide {command} without {}",
                        option.name
                    );
                }
                if !option.requires.is_empty() {
                    let mut omitted = option.conflicts.to_vec();
                    omitted.extend_from_slice(option.requires);
                    assert!(
                        !takes(&argv(spec, &omitted, None, most)),
                        "hide {command} {} alone",
                        option.name
                    );
                }
                if option.value.is_some() {
                    assert!(
                        !takes(&argv(spec, option.conflicts, Some(option.name), most)),
                        "hide {command} {} refused value",
                        option.name
                    );
                }
            }
            for argument in spec.arguments {
                assert!(
                    !takes(&argv(spec, &[], Some(argument.name), most)),
                    "hide {command} refused {}",
                    argument.name
                );
            }
        }
    }

    fn arguments_of(command: &Value) -> &Vec<Value> {
        command["arguments"].as_array().unwrap()
    }

    /// A value the exported grammar's value type admits.
    fn exported_sample(value: &Value) -> String {
        match value["type"].as_str().unwrap() {
            "one_of" => value["values"][0].as_str().unwrap().to_owned(),
            "key" => "k1".to_owned(),
            "text" => "some text".to_owned(),
            "body" => "multi\nline body".to_owned(),
            "paste_text" => "line one\nline two\tend".to_owned(),
            other => panic!("no sample for {other}"),
        }
    }

    /// The forms the exported grammar declares are the ones the CLI takes:
    /// each of a command's `needs_one_of` alone with its required
    /// arguments, and none of them refused; an argument marked not required
    /// left out; a repeated option up to its `repeats_at_most`, and not once
    /// more; an argument's `value_with` value taken with its option and
    /// refused without it.
    #[test]
    fn the_forms_the_exported_grammar_declares_are_the_ones_the_cli_takes() {
        let document = document();
        let mut checked = Vec::new();
        for command in document["commands"].as_array().unwrap() {
            let name = command["command"].as_str().unwrap();
            let forms = command["needs_one_of"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let options = command["options"].as_array().unwrap();
            let repeated: Vec<&Value> = options
                .iter()
                .filter(|option| option["repeats_at_most"].is_u64())
                .collect();
            let widened = arguments_of(command)
                .iter()
                .any(|argument| argument["value_with"].is_object());
            if forms.is_empty() && repeated.is_empty() && !widened {
                continue;
            }
            let mut base = vec!["hide".to_owned()];
            base.extend(name.split(' ').map(str::to_owned));
            let arguments = command["arguments"].as_array().unwrap();
            for argument in arguments
                .iter()
                .take_while(|argument| argument["required"] != false)
            {
                base.push(exported_sample(&argument["value"]));
            }
            let with = |argv: &[String], form: &str| {
                let mut argv = argv.to_vec();
                if let Some(option) = options.iter().find(|option| option["name"] == form) {
                    argv.push(form.to_owned());
                    if !option["value"].is_null() {
                        argv.push(exported_sample(&option["value"]));
                    }
                } else {
                    let argument = arguments
                        .iter()
                        .find(|argument| argument["name"] == form)
                        .unwrap_or_else(|| panic!("hide {name}: {form} is no argument or option"));
                    argv.push(exported_sample(&argument["value"]));
                }
                argv
            };
            if !forms.is_empty() {
                assert!(!takes(&base), "hide {name} with none of {forms:?}");
            }
            for form in &forms {
                let argv = with(&base, form.as_str().unwrap());
                assert!(takes(&argv), "hide {name} {form}: {:?}", parse_args(&argv));
            }
            for option in repeated {
                let flag = option["name"].as_str().unwrap();
                let most = option["repeats_at_most"].as_u64().unwrap() as usize;
                let mut argv = base.clone();
                for _ in 0..most {
                    argv = with(&argv, flag);
                }
                assert!(takes(&argv), "hide {name} {flag} x{most}");
                assert!(
                    !takes(&with(&argv, flag)),
                    "hide {name} {flag} x{}",
                    most + 1
                );
            }
            for argument in arguments {
                let Some(widening) = argument["value_with"].as_object() else {
                    continue;
                };
                for (flag, value) in widening {
                    let mut plain = base.clone();
                    plain.push(exported_sample(value));
                    assert!(
                        !takes(&plain),
                        "hide {name} {} without {flag}",
                        argument["name"]
                    );
                    let widened = with(&plain, flag);
                    assert!(
                        takes(&widened),
                        "hide {name} {} with {flag}: {:?}",
                        argument["name"],
                        parse_args(&widened)
                    );
                }
            }
            checked.push(name.to_owned());
        }
        assert!(checked.contains(&"pane send".to_owned()), "{checked:?}");
    }

    /// The flags `hide mission`'s parser reads for each verb are the ones
    /// the contract names for it, values and switches alike, and every flag
    /// its usage and guides show for a mission command is among them: a flag
    /// added to one and not the other is refused before the parser sees it.
    #[test]
    fn each_mission_flag_the_parser_reads_is_one_the_contract_names() {
        let missions: Vec<&Spec> = COMMANDS
            .iter()
            .filter(|spec| spec.words.first() == Some(&"mission"))
            .collect();
        let spec_of = |verb: &str| {
            missions
                .iter()
                .find(|spec| spec.words.get(1) == Some(&verb))
                .copied()
        };
        for (verb, valued, switches) in crate::mission_cli::FLAGS {
            let spec = spec_of(verb).unwrap_or_else(|| panic!("hide mission {verb} has no spec"));
            let read: std::collections::BTreeSet<(&str, bool)> = valued
                .iter()
                .map(|flag| (*flag, true))
                .chain(switches.iter().map(|flag| (*flag, false)))
                .collect();
            let named: std::collections::BTreeSet<(&str, bool)> = spec
                .options
                .iter()
                .map(|option| (option.name, option.value.is_some()))
                .collect();
            assert_eq!(read, named, "hide mission {verb}");
        }
        for spec in &missions {
            assert!(
                crate::mission_cli::FLAGS
                    .iter()
                    .any(|(verb, ..)| spec.words.get(1) == Some(verb)),
                "hide {} is not read by the parser",
                spec.words.join(" ")
            );
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut documents = vec![("usage".to_owned(), crate::mission_cli::USAGE.to_owned())];
        for path in [
            "assets/mission/help.md",
            "assets/guide.md",
            "../docs/delivery.md",
            "../docs/agent-hooks.md",
        ] {
            documents.push((
                path.to_owned(),
                std::fs::read_to_string(root.join(path)).unwrap(),
            ));
        }
        let mut shown = 0;
        for (name, text) in &documents {
            // A usage line the help wraps goes on indented, from `[--`.
            let mut last: Option<&str> = None;
            for line in text.lines() {
                let mut commands: Vec<(&str, &str)> = Vec::new();
                for (at, _) in line.match_indices("hide mission ") {
                    let command = &line[at + "hide mission ".len()..];
                    let command = command.split('`').next().unwrap_or_default();
                    if let Some(verb) = command.split_whitespace().next() {
                        commands.push((verb, command));
                    }
                }
                match (commands.last(), last) {
                    (Some((verb, _)), _) => last = Some(verb),
                    (None, Some(verb)) if line.trim_start().starts_with("[--") => {
                        commands.push((verb, line));
                    }
                    (None, _) => last = None,
                }
                for (verb, command) in commands {
                    if verb == "help" {
                        continue;
                    }
                    let spec = spec_of(verb)
                        .unwrap_or_else(|| panic!("{name} shows hide mission {verb}: {line}"));
                    for word in command.split(|c: char| !(c.is_ascii_lowercase() || c == '-')) {
                        if word.starts_with("--") && word.len() > 2 {
                            shown += 1;
                            assert!(
                                spec.options.iter().any(|option| option.name == word),
                                "{name} shows hide mission {verb} {word}, which the contract refuses: {line}"
                            );
                        }
                    }
                }
            }
        }
        assert!(shown > 50, "the documents show {shown} mission flags");
    }

    /// The commands the mission guide teaches pass the flag check and the
    /// parser together, as `hide` runs them.
    #[test]
    fn the_mission_commands_the_guide_teaches_parse_whole() {
        let line = |words: &[&str]| {
            std::iter::once("hide")
                .chain(words.iter().copied())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        for words in [
            &[
                "mission",
                "decide",
                "--line",
                "Skip the check?",
                "--why",
                "fails on main",
                "--option",
                "skip",
                "--option",
                "fix",
                "--recommend",
                "1",
                "--node",
                "fix",
                "--show",
                "view-1",
                "--show",
                "report.html",
            ][..],
            &["mission", "decide", "--clear"],
            &[
                "mission",
                "node",
                "branch",
                "--branch",
                "x",
                "--commit",
                "abc1234",
                "--verdict",
                "pass",
                "--checks",
                "3/4",
            ],
            &[
                "mission",
                "node",
                "branch",
                "--no-commit",
                "--no-verdict",
                "--no-checks",
            ],
            &[
                "mission",
                "record",
                "core",
                "check",
                "--pass",
                "--evidence",
                "cargo test passed",
                "--commit",
                "abc1234",
            ],
        ] {
            assert!(
                takes(&line(words)),
                "{words:?}: {:?}",
                parse_args(&line(words))
            );
        }
        assert!(!takes(&line(&[
            "mission", "node", "branch", "--commit", "main"
        ])));
    }

    #[test]
    fn a_command_or_flag_the_contract_does_not_name_is_refused() {
        let args = |line: &[&str]| {
            line.iter()
                .map(|word| (*word).to_owned())
                .collect::<Vec<_>>()
        };
        assert!(admit(&args(&["agent", "resume", "a1"])).is_err());
        assert!(admit(&args(&["watch", "pause", "w1"])).is_err());
        assert!(admit(&args(&["factory", "status"])).is_ok());
        assert!(admit(&args(&["request", "send", "t", "--wait"])).is_err());
        assert!(parse_args(&args(&["hide", "watch", "stop", "w1", "--observer", "o"])).is_err());
        // A value that looks like a flag is a value.
        assert!(admit(&args(&["request", "send", "t", "--body", "--wait"])).is_ok());
        // What `--` passes through is the agent's own.
        assert!(admit(&args(&["agent", "spawn", "--", "--model", "x"])).is_ok());
        assert!(
            admit(&args(&[
                "inbox",
                "--hook",
                "--session",
                "s",
                "--prompt-digest",
                "d"
            ]))
            .is_ok()
        );
        assert!(admit(&args(&["inbox", "--hook", "--bell"])).is_err());
        assert!(admit(&args(&["inbox", "--prompt-digest", "d"])).is_err());
    }

    /// The committed copy is the contract this build exports, so a change to
    /// a command's options or answer fields shows in review as a change to
    /// `contracts/hide-cli.json`. Regenerate it with
    /// `target/debug/hide contract --json | python3 -m json.tool > contracts/hide-cli.json`.
    #[test]
    fn the_committed_contract_is_the_exported_one() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../contracts/hide-cli.json");
        let committed: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(
            committed == document(),
            "contracts/hide-cli.json differs from the exported contract; regenerate it as this test's comment says"
        );
    }
}
