//! Hide's ownership of the agent runtimes' global hook configuration.
//!
//! This crate is the only code path in the product that writes to a file the
//! operator owns outside the app's own state. Everything it knows lives here:
//! where each runtime keeps its hook file, what an entry looks like, how to
//! append one without touching anybody else's, how to judge what is currently
//! installed, and how to report that judgement.
//!
//! It is a separate crate rather than a module of `herdr-core` because the
//! risk it carries is a file-system risk, and folding it into the crate that
//! owns `Mutex<Runtime>` would put a `settings.json` write behind the same
//! lock as the render snapshot and drag the FFI boundary into the tests that
//! matter most here (PRD D-47).
//!
//! The only Herdr traffic here is the hook helper's `pane.report_metadata`
//! request, sent through `hide-herdr-client` (see `report`), which Herdr
//! defines as display-only metadata; the core reads the tokens back out of
//! the pane tokens its ordinary snapshot already carries. Nothing subscribes
//! or polls.

pub mod antigravity;
pub mod antigravity_trust;
pub mod basic;
pub mod codex_daemon;
pub mod codex_trust;
pub mod counters;
pub mod delivery;
pub mod diagnosis;
pub mod folder_trust;
pub mod guardrails;
pub mod guidance;
pub mod install;
mod lossless_json;
pub mod memory;
pub mod observation;
pub mod opencode;
pub mod pi_extension;
pub mod plan_record;
pub mod plugin;
pub mod report;
pub mod route;
pub mod runtime;
pub mod spawn_guard;
#[cfg(all(test, unix))]
#[path = "../../hide-platform/tests/it/stand_ins.rs"]
mod stand_ins;

pub use counters::PaneCounters;
pub use diagnosis::{
    Diagnosis, MemoryCompatibility, PaneInstrumentation, RuntimeDiagnosis, UninstrumentedReason,
    cli_path, find_binary, runtime_compatibility, version_at_least,
};
pub use install::{
    HookStatus, InstallFailure, InstallOutcome, RemoveOutcome, install, installed_helper_path,
    remove,
};
pub use runtime::{
    AgentRuntime, HELPER_BINARY_NAME, HOOK_ENTRY_VERSION, HOOK_SOURCE_NAME, HOOK_VERSION,
    HookEvent, PURPOSE_CONTEXT, hook_source_id, hook_stdout,
};
