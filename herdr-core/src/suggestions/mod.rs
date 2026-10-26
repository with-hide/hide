//! Hide AI 제안 (PRD hide-ai-suggestions): the core's suggestion area.
//!
//! - [`ledger`]: the records of what went wrong in a Project, counted once
//!   per session or pull request from facts the core already has.
//! - [`notebook`]: the list of problems each day's run goes on editing.
//! - [`analysis`]: the one boundary that turns records into notebook edits,
//!   with the deterministic [`analysis::TagGrouping`] behind it today.
//! - [`investigation`]: the analysis that edits the notebook with Hide AI
//!   over several turns, reading through Hide's own read tools.
//! - [`scoring`]: the counted pain of a line and which lines are raised.
//! - [`policy`]: every score, threshold, cap, budget and schedule value.
//! - [`settings`]: a Project's switches, quantity and apply mode.
//! - [`lifecycle`]: a raised line's attempt, watch and escalation.
//! - [`store`]: `suggestions.sqlite3`, written by the suggestion worker alone.
//! - [`worker`]: the thread that owns the store, records, runs and raises.
//! - [`read`]: the 제안 tab as the worker reads it for the screen.
//!
//! Everything a run decides is read back from the notebook it leaves, so
//! the way problems are found can change without the ledger, the lifecycle,
//! apply, the watch or the screen changing (D-49).

pub mod analysis;
pub mod investigation;
pub mod ledger;
pub mod lifecycle;
pub mod notebook;
pub mod policy;
pub mod read;
pub mod scoring;
pub mod settings;
pub mod store;
pub(crate) mod worker;
