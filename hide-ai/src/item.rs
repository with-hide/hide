//! An agent item: one agent and the profile it runs as (PRD
//! agent-profiles-phase2 D-01, D-07).
//!
//! Hide AI's Runs on, its fallback list, each item's model and machine, and
//! everything the router keeps about who answered are keyed by an item, so
//! two profiles of one agent are two places a request can go. The built-in
//! default profile is `profile: None`, which every file and wire written
//! before items reads as.

use std::fmt;

use hide_agent_adapter::ProfileId;

use crate::ProviderId;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Item {
    pub provider: ProviderId,
    /// The machine's profile of the provider's agent; `None` is its built-in
    /// default. Never `Some` of the built-in default's own id.
    pub profile: Option<ProfileId>,
}

impl Item {
    /// The agent's built-in default item: `Claude Code`.
    pub fn agent(provider: ProviderId) -> Self {
        Self {
            provider,
            profile: None,
        }
    }

    /// The item of `profile`, the built-in default for `None` or for the
    /// default's own id.
    pub fn of(provider: ProviderId, profile: Option<ProfileId>) -> Self {
        Self {
            provider,
            profile: profile.filter(|profile| !profile.is_default()),
        }
    }

    pub fn is_default(&self) -> bool {
        self.profile.is_none()
    }

    /// The profile's id as the file and the wire write it, `None` for the
    /// built-in default.
    pub fn profile_id(&self) -> Option<&str> {
        self.profile.as_ref().map(|profile| profile.0.as_str())
    }
}

impl From<ProviderId> for Item {
    fn from(provider: ProviderId) -> Self {
        Self::agent(provider)
    }
}

/// `claude` for a built-in default, `claude:p-work` for a profile: a log
/// line's value, never a name a person reads.
impl fmt::Display for Item {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.profile {
            None => write!(formatter, "{}", self.provider),
            Some(profile) => write!(formatter, "{}:{}", self.provider, profile.0),
        }
    }
}
