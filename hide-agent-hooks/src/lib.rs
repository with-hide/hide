//! Hide's ownership of the agent runtimes' global hook configuration.
//!
//! This crate is the only code path in the product that writes to a file the
//! operator owns outside the app's own state. Everything it knows lives here:
//! where each runtime keeps its hook file, what an entry looks like, how to
//! append one without touching anybody else's, how to judge what is currently
//! installed, and how to report that judgement.
//!
//! It is a separate crate rather than a module of `hide-core` because the
//! risk it carries is a file-system risk, and folding it into the crate that
//! owns `Mutex<Runtime>` would put a `settings.json` write behind the same
//! lock as the render snapshot and drag the FFI boundary into the tests that
//! matter most here (PRD D-47).

pub mod antigravity;
pub mod antigravity_trust;
pub mod basic;
pub mod codex_daemon;
pub mod codex_trust;
pub mod counters;
pub mod delivery;
pub mod diagnosis;
pub mod folder_trust;
pub mod global_guard;
pub mod guardrails;
pub mod guidance;
mod hook_log;
pub mod hook_request;
pub mod install;
mod lossless_json;
pub mod memory;
pub mod opencode;
pub mod opener_guard;
pub mod pane;
pub mod pi_extension;
pub mod plan_record;
pub mod plugin;
pub mod pre_tool;
pub mod report;
pub mod route;
pub mod runtime;
#[cfg(all(test, unix))]
#[path = "../../hide-platform/tests/it/stand_ins.rs"]
mod stand_ins;
pub mod state_folder;

pub use counters::PaneCounters;
pub use diagnosis::{
    Diagnosis, MemoryCompatibility, PaneInstrumentation, RuntimeDiagnosis, UninstrumentedReason,
    program_version, runtime_compatibility, runtime_program, version_at_least,
};
pub use install::{
    HookStatus, InstallFailure, InstallOutcome, RemoveOutcome, install, installed_helper_path,
    remove,
};
pub use runtime::{
    AgentRuntime, HELPER_BINARY_NAME, HOOK_ENTRY_VERSION, HOOK_SOURCE_NAME, HOOK_VERSION,
    HookEvent, PURPOSE_CONTEXT, hook_source_id, hook_stdout,
};
