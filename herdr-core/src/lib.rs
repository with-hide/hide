#[cfg(test)]
mod agent_adapter_contract_tests;
mod agent_find;
pub mod agent_hooks;
mod agent_sleep;
mod agent_sleep_herdr;
mod agent_start;
pub mod agent_state;
mod ai;
mod changes;
mod checkout_git;
mod checkout_owner;
mod codex_launch;
mod composer_checks;
mod composer_store;
pub mod coordination;
pub mod delivery;
mod device_catalog;
pub mod diagnostics;
pub mod direct_observation;
mod disk;
mod display_text;
pub mod domain;
mod effects;
mod environment;
#[cfg(test)]
mod fake_herdr;
mod files;
pub use files::{FileRoots, inode_text};
mod agent_groups;
mod agent_layout;
pub mod find;
pub mod fixture;
mod fork;
mod github;
mod github_poll;
mod github_store;
mod handle;
pub mod herdr_contract;
pub mod herdr_takeover;
pub mod input_gate;
pub mod issues;
mod kit;
mod labels;
mod latest_file;
pub mod links;
pub mod live;
pub mod local_issues;
pub mod memory_command;
pub mod memory_hook;
pub mod missions;
mod model;
pub mod node;
pub mod node_access;
pub mod node_migration;
mod pane_reopen;
mod persistence;
pub mod pet;
mod ports;
pub mod product_analytics;
mod project_context;
mod project_settings;
mod reader;
mod recent_closed;
pub mod remote;
mod request_view;
mod runtime;
pub mod schema;
mod session_archive;
mod session_sync;
mod sidebar;
mod split_tree;
mod start_claim;
pub mod tasks;
mod terminal_attachments;
pub mod terminal_reports;
mod usage;
pub mod usage_ledger;
pub mod view_feedback;
mod view_layout;
pub mod wire;
pub mod workspace;
pub mod workspace_control;
pub mod workspace_views;
mod worktrees;
mod zoneinfo;

pub use ai::{
    hide_ai_asks, hide_ai_asks_of_stored, hide_ai_ready_here, hide_ai_settings_path,
    stored_hide_ai_settings,
};
pub use handle::Core;
pub use model::{
    CoreOptions, DeviceRegistration, LinkOrigin, SCHEMA_VERSION, Snapshot, SnapshotDeltaPayload,
};
pub use model::{
    RemoteFileListSnapshot, RemoteSessionSnapshot, RemoteStatusSnapshot, RestSections,
    RightPanelSection, SnapshotDeltaWire,
};
pub use runtime::WorkspaceRemoteRoute;
pub use runtime::core_move::{MoveProject, MoveSource, MoveSourceRefusal, ReleaseSource};
pub use runtime::serialize_snapshot_delta;
