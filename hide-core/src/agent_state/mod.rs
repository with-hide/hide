//! The authority for agent state on every surface.
//! Axes and facts enter here; shells draw the resulting values.

pub(crate) mod axes;
pub mod escalation;
pub mod header;
pub mod sessions;
pub(crate) mod tally;
pub(crate) mod turn;
pub mod work;

pub(crate) use axes::*;
pub(crate) use tally::*;
pub(crate) use turn::*;

pub use tally::RowMark;
pub use tally::phone;
pub use turn::push;
pub use turn::{RowState, TabState, Tone};

pub(crate) use tally::scope::Cache as ScopeCache;
pub use tally::scope::Scope;
