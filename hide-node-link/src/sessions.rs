//! The shapes of a node's agent sessions, which Project Memory and the
//! right panel read through the core (PRD core-host-node D-03).

use serde::{Deserialize, Serialize};

/// Independently implemented reader operations, not provider installation
/// or permission to start an agent.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReaderFeature {
    Identity,
    Labels,
    Titles,
    Conversation,
    Memory,
    Links,
    Activity,
    Turns,
    UserTurnContent,
    Usage,
    /// The session read as ACP session updates (`Call::SessionThread`).
    Thread,
}

/// Limits on the existing Hello's reader field. The outer transport already
/// bounds a whole answer; this smaller bound is checked before materializing
/// any advertisement as JSON or retaining capability state.
pub const READER_ADVERTISEMENT_BYTES: usize = 8 * 1024;
pub const READER_ADVERTISEMENT_PROVIDERS: usize = 16;
pub const READER_ADVERTISEMENT_FEATURES: usize = 16;

/// Validated facts for exactly one authenticated link. Unknown/malformed
/// provider rows grant nothing and do not erase another valid provider.
/// Diagnostic codes contain no provider-supplied text.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReaderFeatures {
    providers: std::collections::BTreeMap<String, std::collections::BTreeSet<ReaderFeature>>,
    diagnostics: std::collections::BTreeSet<&'static str>,
}

impl ReaderFeatures {
    /// Facts from the session implementation this process links. Future
    /// readers cannot become available through a protocol/version guess.
    pub fn implemented() -> Self {
        use ReaderFeature::*;
        let mut result = Self::default();
        for agent in hide_session::Agent::supported() {
            let row = agent.format().adapter();
            let mut features = std::collections::BTreeSet::from([Identity, Labels]);
            if agent != hide_session::Agent::Cursor {
                features.insert(Links);
            }
            if matches!(
                row.usage.tokens,
                hide_agent_adapter::usage::Support::Supported(_)
            ) {
                features.insert(Usage);
            }
            // Memory reads a session through its conversation reader
            // (`Call::SessionConversation`): a JSONL file's, or OpenCode's
            // database read by id. Cursor's graph has none for Memory.
            if (agent.is_jsonl() || agent == hide_session::Agent::OpenCode)
                && row.conversation.is_some()
            {
                features.insert(Memory);
            }
            if agent.reports_activity() {
                features.insert(Activity);
            }
            if row.titles.is_some() {
                features.insert(Titles);
            }
            if row.conversation.is_some() {
                features.insert(Conversation);
            }
            if agent.reports_turns() {
                features.extend([Turns, UserTurnContent]);
            }
            if hide_session::thread::reads(agent.format()) {
                features.insert(Thread);
            }
            result.providers.insert(agent.as_str().to_owned(), features);
        }
        result
    }

    /// The audited protocol24 contract, including its partial OpenCode
    /// label/link reader. This transition contract deliberately never grows
    /// when another reader is added to the current build. It has no Memory
    /// reader: Memory reads through `Call::SessionConversation`, which came
    /// with protocol 26, and the line read a protocol24 node answered for it
    /// is gone, so such a node's sessions are ones Memory does not read.
    pub fn protocol24() -> Self {
        use ReaderFeature::*;
        Self {
            providers: [
                (
                    "claude",
                    vec![Identity, Labels, Titles, Conversation, Links, Activity],
                ),
                (
                    "codex",
                    vec![
                        Identity,
                        Labels,
                        Titles,
                        Conversation,
                        Links,
                        Activity,
                        Turns,
                    ],
                ),
                ("opencode", vec![Identity, Labels, Titles, Links]),
            ]
            .into_iter()
            .map(|(provider, features)| (provider.to_owned(), features.into_iter().collect()))
            .collect(),
            diagnostics: Default::default(),
        }
    }

    pub fn supports(&self, provider: &str, feature: ReaderFeature) -> bool {
        self.providers
            .get(provider)
            .is_some_and(|features| features.contains(&feature))
    }

    pub fn diagnostics(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.diagnostics.iter().copied()
    }

    pub fn unavailable(reason: &'static str) -> Self {
        Self {
            providers: Default::default(),
            diagnostics: std::collections::BTreeSet::from([reason]),
        }
    }

    fn parse(raw: &str) -> Self {
        if raw.len() > READER_ADVERTISEMENT_BYTES {
            return Self::unavailable("reader_advertisement_too_large");
        }
        let Ok(rows) = serde_json::from_str::<Vec<Box<serde_json::value::RawValue>>>(raw) else {
            return Self::unavailable("reader_advertisement_invalid");
        };
        if rows.len() > READER_ADVERTISEMENT_PROVIDERS {
            return Self::unavailable("reader_advertisement_too_many_providers");
        }
        let mut result = Self::default();
        let mut seen = std::collections::BTreeSet::new();
        #[derive(Deserialize)]
        struct Row {
            provider: String,
            features: Vec<Box<serde_json::value::RawValue>>,
        }
        for row in rows {
            let Ok(Row { provider, features }) = serde_json::from_str(row.get()) else {
                result.diagnostics.insert("reader_provider_invalid");
                continue;
            };
            let provider = provider.as_str();
            if provider.len() > 32
                || hide_session::Agent::from_kind(provider)
                    .is_none_or(|agent| agent.as_str() != provider)
            {
                result.diagnostics.insert("reader_provider_unknown");
                continue;
            }
            if !seen.insert(provider.to_owned()) {
                result.providers.remove(provider);
                result.diagnostics.insert("reader_provider_duplicate");
                continue;
            }
            if features.len() > READER_ADVERTISEMENT_FEATURES {
                result.diagnostics.insert("reader_features_too_many");
                continue;
            }
            let mut parsed = std::collections::BTreeSet::new();
            let mut names = std::collections::BTreeSet::new();
            let mut valid = true;
            for feature in features {
                let name = serde_json::from_str::<String>(feature.get());
                let Ok(name) = name else {
                    result.diagnostics.insert("reader_feature_invalid");
                    valid = false;
                    break;
                };
                if name.len() > 32 {
                    result.diagnostics.insert("reader_feature_invalid");
                    valid = false;
                    break;
                }
                if !names.insert(name) {
                    result.diagnostics.insert("reader_feature_duplicate");
                    valid = false;
                    break;
                }
                match serde_json::from_str::<ReaderFeature>(feature.get()) {
                    Ok(feature) => {
                        parsed.insert(feature);
                    }
                    Err(_) => {
                        result.diagnostics.insert("reader_feature_unknown");
                    }
                }
            }
            if valid {
                result.providers.insert(provider.to_owned(), parsed);
            }
        }
        result
    }
}

impl Serialize for ReaderFeatures {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Row<'a> {
            provider: &'a str,
            features: &'a std::collections::BTreeSet<ReaderFeature>,
        }
        serializer.collect_seq(
            self.providers
                .iter()
                .map(|(provider, features)| Row { provider, features }),
        )
    }
}

impl<'de> Deserialize<'de> for ReaderFeatures {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = Box::<serde_json::value::RawValue>::deserialize(deserializer)?;
        Ok(Self::parse(raw.get()))
    }
}

#[cfg(test)]
mod reader_tests {
    use super::*;

    #[test]
    fn malformed_provider_facts_do_not_disable_another_reader() {
        let facts: ReaderFeatures = serde_json::from_str(
            r#"[
                {"provider":"claude","features":["labels","conversation"]},
                {"provider":"codex","features":["labels","labels"]},
                {"provider":"future","features":["memory"]},
                {"provider":"opencode","features":["labels","future_feature"]}
            ]"#,
        )
        .unwrap();
        assert!(facts.supports("claude", ReaderFeature::Conversation));
        assert!(facts.supports("opencode", ReaderFeature::Labels));
        assert!(!facts.supports("codex", ReaderFeature::Labels));
        assert!(!facts.supports("future", ReaderFeature::Memory));
        assert!(!facts.supports("opencode", ReaderFeature::Memory));
        assert_eq!(facts.diagnostics().count(), 3);
    }

    #[test]
    fn duplicate_provider_never_regains_support_from_a_third_row() {
        let facts: ReaderFeatures = serde_json::from_str(
            r#"[
                {"provider":"codex","features":["memory"]},
                {"provider":"codex","features":[]},
                {"provider":"codex","features":["labels"]},
                {"provider":"claude","features":["labels"]}
            ]"#,
        )
        .unwrap();
        assert!(!facts.supports("codex", ReaderFeature::Labels));
        assert!(!facts.supports("codex", ReaderFeature::Memory));
        assert!(facts.supports("claude", ReaderFeature::Labels));
    }

    #[test]
    fn advertisement_bounds_refuse_facts_without_rejecting_hello() {
        let row = r#"{"provider":"claude","features":["labels"]}"#;
        let at = format!("[{}]", vec![row; READER_ADVERTISEMENT_PROVIDERS].join(","));
        let beyond = format!(
            "[{}]",
            vec![row; READER_ADVERTISEMENT_PROVIDERS + 1].join(",")
        );
        assert!(
            !ReaderFeatures::parse(&at)
                .diagnostics()
                .any(|code| code == "reader_advertisement_too_many_providers")
        );
        assert!(
            ReaderFeatures::parse(&beyond)
                .diagnostics()
                .any(|code| code == "reader_advertisement_too_many_providers")
        );
        let oversized = format!("[{}]", " ".repeat(READER_ADVERTISEMENT_BYTES));
        assert_eq!(
            ReaderFeatures::parse(&oversized).diagnostics().next(),
            Some("reader_advertisement_too_large")
        );
        for malformed in ["null", "{}", "42"] {
            let wire = format!(
                r#"{{"protocol":{},"version":"fixture","os":"linux","arch":"x86_64","home":null,"machine_identity":{{"state":"unavailable","reason":"fixture"}},"reader_features":{malformed}}}"#,
                crate::protocol::PROTOCOL_VERSION
            );
            let hello: crate::protocol::Hello = serde_json::from_str(&wire).unwrap();
            assert!(!hello.readers().supports("claude", ReaderFeature::Labels));
        }
    }

    #[test]
    fn protocol24_keeps_only_its_audited_partial_readers() {
        let facts = ReaderFeatures::protocol24();
        assert!(facts.supports("claude", ReaderFeature::Conversation));
        for provider in ["claude", "codex"] {
            assert!(
                !facts.supports(provider, ReaderFeature::Memory),
                "{provider}"
            );
        }
        assert!(facts.supports("codex", ReaderFeature::Turns));
        assert!(facts.supports("opencode", ReaderFeature::Links));
        assert!(!facts.supports("opencode", ReaderFeature::Conversation));
        assert!(!facts.supports("claude", ReaderFeature::Turns));
        assert!(!facts.supports("codex", ReaderFeature::UserTurnContent));
        for provider in ["pi", "omp", "grok", "cursor"] {
            assert!(!facts.supports(provider, ReaderFeature::Identity));
        }
    }

    #[test]
    fn pi_has_its_complete_reader_but_no_structured_wait_or_content() {
        let facts = ReaderFeatures::implemented();
        for feature in [
            ReaderFeature::Identity,
            ReaderFeature::Labels,
            ReaderFeature::Titles,
            ReaderFeature::Conversation,
            ReaderFeature::Memory,
            ReaderFeature::Links,
            ReaderFeature::Activity,
        ] {
            assert!(facts.supports("pi", feature), "{feature:?}");
            assert!(!ReaderFeatures::protocol24().supports("pi", feature));
        }
        for feature in [ReaderFeature::Turns, ReaderFeature::UserTurnContent] {
            assert!(!facts.supports("pi", feature));
        }
        assert!(facts.supports("cursor", ReaderFeature::Identity));
    }

    #[test]
    fn cursor_advertises_only_its_proven_graph_features_on_current_helpers() {
        for feature in [
            ReaderFeature::Identity,
            ReaderFeature::Labels,
            ReaderFeature::Conversation,
            ReaderFeature::Activity,
        ] {
            assert!(ReaderFeatures::implemented().supports("cursor", feature));
            assert!(!ReaderFeatures::protocol24().supports("cursor", feature));
        }
        for feature in [
            ReaderFeature::Titles,
            ReaderFeature::Memory,
            ReaderFeature::Links,
            ReaderFeature::Turns,
            ReaderFeature::UserTurnContent,
        ] {
            assert!(!ReaderFeatures::implemented().supports("cursor", feature));
        }
        let earlier: ReaderFeatures =
            serde_json::from_str(r#"[{"provider":"grok","features":["identity","conversation"]}]"#)
                .unwrap();
        assert!(!earlier.supports("cursor", ReaderFeature::Conversation));
    }

    #[test]
    fn omp_has_its_native_reader_and_question_content_only_on_a_current_helper() {
        let facts = ReaderFeatures::implemented();
        for feature in [
            ReaderFeature::Identity,
            ReaderFeature::Labels,
            ReaderFeature::Titles,
            ReaderFeature::Conversation,
            ReaderFeature::Memory,
            ReaderFeature::Links,
            ReaderFeature::Activity,
            ReaderFeature::Turns,
            ReaderFeature::UserTurnContent,
        ] {
            assert!(facts.supports("omp", feature), "{feature:?}");
            assert!(!ReaderFeatures::protocol24().supports("omp", feature));
        }
        // A Pi-only intermediate protocol25 helper grants no omp operation.
        let earlier: ReaderFeatures = serde_json::from_str(
            r#"[{"provider":"pi","features":["identity","activity","conversation"]}]"#,
        )
        .unwrap();
        assert!(earlier.supports("pi", ReaderFeature::Identity));
        assert!(!earlier.supports("omp", ReaderFeature::Identity));
    }

    #[test]
    fn grok_has_its_native_reader_and_waits_only_on_a_helper_that_implements_it() {
        let facts = ReaderFeatures::implemented();
        for feature in [
            ReaderFeature::Identity,
            ReaderFeature::Labels,
            ReaderFeature::Titles,
            ReaderFeature::Conversation,
            ReaderFeature::Memory,
            ReaderFeature::Links,
            ReaderFeature::Activity,
            ReaderFeature::Turns,
            ReaderFeature::UserTurnContent,
        ] {
            assert!(facts.supports("grok", feature), "{feature:?}");
            assert!(!ReaderFeatures::protocol24().supports("grok", feature));
        }
        // A protocol25 helper built before this reader grants no Grok read.
        let earlier: ReaderFeatures = serde_json::from_str(
            r#"[{"provider":"pi","features":["identity"]},{"provider":"omp","features":["identity","turns"]}]"#,
        )
        .unwrap();
        assert!(earlier.supports("omp", ReaderFeature::Identity));
        assert!(!earlier.supports("grok", ReaderFeature::Identity));
    }

    #[test]
    fn opencode_has_its_complete_database_reader_while_protocol24_keeps_its_partial_one() {
        let facts = ReaderFeatures::implemented();
        for feature in [
            ReaderFeature::Identity,
            ReaderFeature::Labels,
            ReaderFeature::Titles,
            ReaderFeature::Conversation,
            ReaderFeature::Memory,
            ReaderFeature::Links,
            ReaderFeature::Activity,
            ReaderFeature::Turns,
            ReaderFeature::UserTurnContent,
        ] {
            assert!(facts.supports("opencode", feature), "{feature:?}");
        }
        let retained = ReaderFeatures::protocol24();
        for feature in [
            ReaderFeature::Conversation,
            ReaderFeature::Memory,
            ReaderFeature::Activity,
            ReaderFeature::Turns,
            ReaderFeature::UserTurnContent,
        ] {
            assert!(!retained.supports("opencode", feature), "{feature:?}");
        }
        assert!(retained.supports("opencode", ReaderFeature::Titles));
    }
}

/// A session file's size and modification time, read when asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionStat {
    pub size: u64,
    /// `None` where the system keeps no modification time.
    pub modified_unix_ms: Option<u64>,
}
