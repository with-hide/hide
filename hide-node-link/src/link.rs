//! The one interface the core reaches a node through.

use std::fmt;
use std::time::Duration;

use serde_json::Value;

use crate::RootIdentity;
use crate::error::HostError;
use crate::protocol::Call;
use crate::sessions::{ReaderFeature, ReaderFeatures};

#[derive(Debug)]
pub enum LinkError {
    /// No connection to the node; nothing was sent.
    NotConnected(String),
    /// Four requests run and thirty-two wait; nothing was sent.
    Busy,
    /// The node answered and refused, or the operation failed there.
    Refused(HostError),
    /// The request may have reached the node and its effect is unknown.
    Unknown(String),
}

impl fmt::Display for LinkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotConnected(reason) => write!(formatter, "{reason}"),
            Self::Busy => formatter.write_str(
                "The device is busy with other file work; nothing was sent. Try again in a moment",
            ),
            Self::Refused(error) => write!(formatter, "{}", error.message),
            Self::Unknown(reason) => write!(formatter, "{reason}"),
        }
    }
}

/// A node's answer before it is decoded into the type the call expects.
/// Another machine's answer stays the raw JSON text it sent: materializing an
/// untrusted line as a generic `Value` costs tens of times its size, so it is
/// decoded once, straight into the typed answer (`call_as`).
pub enum LinkAnswer {
    Parsed(Value),
    Raw(Box<serde_json::value::RawValue>),
}

/// An answer can carry what must never reach a log (a provider's
/// credentials, a file's contents), so its Debug form names its size only.
impl fmt::Debug for LinkAnswer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parsed(value) => write!(
                formatter,
                "LinkAnswer::Parsed({} bytes)",
                value.to_string().len()
            ),
            Self::Raw(raw) => write!(formatter, "LinkAnswer::Raw({} bytes)", raw.get().len()),
        }
    }
}

impl From<Value> for LinkAnswer {
    fn from(value: Value) -> Self {
        LinkAnswer::Parsed(value)
    }
}

/// One node's answerer, addressed by the core through that node's id.
pub trait NodeLink: Send + Sync {
    /// Sends one request and waits at most `timeout` for its answer, its
    /// wait to be sent included, so the call's work ends when its caller
    /// stops waiting. Blocks, on this machine's disk as on another; never
    /// call it under the runtime lock.
    fn call(&self, call: Call, timeout: Duration) -> Result<LinkAnswer, LinkError>;

    /// Immutable reader facts belonging to this live connection only, never
    /// a durable cache or an inference from its package version. A link
    /// without facts grants no reader.
    fn reader_features(&self) -> Option<&ReaderFeatures> {
        None
    }

    /// Whether the node at the other end speaks an older protocol this core
    /// still admits (the audited 24), so a request it predates is refused
    /// as one it cannot read rather than one it misread.
    fn predates_current_protocol(&self) -> bool {
        false
    }

    /// A call whose node reports progress before it answers: `progress`
    /// hears each report and answers whether the call should go on, and
    /// answering `false` asks the node to stop the work, which it answers
    /// as stopped. A link that carries no reports answers as
    /// [`NodeLink::call`] does.
    fn call_with_progress(
        &self,
        call: Call,
        timeout: Duration,
        progress: &mut dyn FnMut(Value) -> bool,
    ) -> Result<LinkAnswer, LinkError> {
        let _ = progress;
        self.call(call, timeout)
    }

    /// Whether the answer is computed in this process, so a path the
    /// operator spelled through a link to the checkout can be resolved on
    /// this machine's filesystem.
    fn in_process(&self) -> bool {
        false
    }

    /// Why the connection ended, once it has; `None` while it takes requests.
    fn closed_reason(&self) -> Option<String> {
        None
    }

    /// Ends the connection; requests still waiting become `Unknown`.
    fn close(&self, _reason: &str) {}

    /// Ends the connection once the requests already admitted have answered,
    /// so work in flight settles to its real result (B52). The caller has
    /// already stopped handing the link out.
    fn close_when_idle(&self, reason: &str) {
        self.close(reason);
    }

    /// The identity this link pinned for `root`, if any. A node pins a
    /// checkout root the first time a connection touches it, so a folder
    /// swapped in at that path later is refused rather than listed.
    fn pinned(&self, _root: &str) -> Option<RootIdentity> {
        None
    }

    fn pin(&self, _root: &str, _identity: Option<RootIdentity>) {}
}

/// [`call_as`] for a call that reports progress: each report is decoded as
/// `P`, and one that does not decode stops the call.
pub fn call_as_with_progress<T: serde::de::DeserializeOwned, P: serde::de::DeserializeOwned>(
    link: &(impl NodeLink + ?Sized),
    mut call: Call,
    timeout: Duration,
    mut progress: impl FnMut(P) -> bool,
) -> Result<T, LinkError> {
    let readers = reader_requirements(&call);
    check_readers(link, &readers)?;
    let label = prepare_label_call(link, &mut call);
    let mut undecoded = None;
    let answer = link.call_with_progress(
        call,
        timeout,
        &mut |report| match serde_json::from_value(report) {
            Ok(report) => progress(report),
            Err(error) => {
                undecoded = Some(error);
                false
            }
        },
    );
    if let Some(error) = undecoded {
        return Err(LinkError::Unknown(format!(
            "The node reported progress in an unexpected shape: {error}"
        )));
    }
    let answer = answer?;
    check_readers(link, &readers)?;
    decode(label_answer(link, label, answer)?)
}

pub fn call_as<T: serde::de::DeserializeOwned>(
    link: &(impl NodeLink + ?Sized),
    mut call: Call,
    timeout: Duration,
) -> Result<T, LinkError> {
    let readers = reader_requirements(&call);
    check_readers(link, &readers)?;
    let label = prepare_label_call(link, &mut call);
    let answer = link.call(call, timeout)?;
    check_readers(link, &readers)?;
    decode(label_answer(link, label, answer)?)
}

fn prepare_label_call(
    link: &(impl NodeLink + ?Sized),
    call: &mut Call,
) -> Option<hide_session::Agent> {
    let Call::LabelTranscript { request } = call else {
        return None;
    };
    if !link
        .reader_features()
        .is_some_and(|features| features.supports(request.agent.as_str(), ReaderFeature::Turns))
    {
        request.turns = None;
    } else if !link.reader_features().is_some_and(|features| {
        features.supports(request.agent.as_str(), ReaderFeature::UserTurnContent)
    }) && let Some(turns) = &mut request.turns
    {
        turns.clear_user_turn_content();
    }
    Some(request.agent)
}

/// Optional reader fields are discarded as raw JSON before their native
/// enum/checkpoint is decoded. A newer helper's unsupported field therefore
/// cannot disable the labels that this connection actually implements.
fn label_answer(
    link: &(impl NodeLink + ?Sized),
    agent: Option<hide_session::Agent>,
    answer: LinkAnswer,
) -> Result<LinkAnswer, LinkError> {
    let Some(agent) = agent else {
        return Ok(answer);
    };
    let features = link.reader_features().ok_or_else(|| {
        LinkError::Refused(HostError::new(
            crate::ErrorCode::Unsupported,
            "session_reader_support_unavailable",
        ))
    })?;
    let titles = features.supports(agent.as_str(), ReaderFeature::Titles);
    let identity = features.supports(agent.as_str(), ReaderFeature::Identity);
    let turns = features.supports(agent.as_str(), ReaderFeature::Turns);
    let content = features.supports(agent.as_str(), ReaderFeature::UserTurnContent);
    if identity && titles && turns && content {
        return Ok(answer);
    }
    let raw = match answer {
        LinkAnswer::Raw(raw) => raw,
        LinkAnswer::Parsed(value) => {
            serde_json::value::to_raw_value(&value).map_err(|_| invalid_label_answer())?
        }
    };
    let mut fields: RawReaderFields =
        serde_json::from_str(raw.get()).map_err(|_| invalid_label_answer())?;
    let mut refused = Vec::new();
    if !identity
        && let Some(raw) = fields.0.get_mut("confirmed")
        && raw.get() != "null"
    {
        let mut confirmed: RawReaderFields =
            serde_json::from_str(raw.get()).map_err(|_| invalid_label_answer())?;
        let mut supplied = false;
        for name in ["native_session_id", "source_path"] {
            supplied |= confirmed
                .0
                .remove(name)
                .is_some_and(|raw| raw.get() != "null");
        }
        if supplied {
            refused.push("reader_identity_unsupported");
        }
        *raw = serde_json::value::to_raw_value(&confirmed.0).map_err(|_| invalid_label_answer())?;
    }
    if !titles {
        for name in ["title", "custom_title"] {
            if fields.0.remove(name).is_some_and(|raw| raw.get() != "null") {
                refused.push("reader_titles_unsupported");
            }
        }
    }
    if !turns
        && fields
            .0
            .remove("turns")
            .is_some_and(|raw| raw.get() != "null")
    {
        refused.push("reader_turns_unsupported");
    }
    if turns
        && !content
        && let Some(raw) = fields.0.get_mut("turns")
        && raw.get() != "null"
    {
        let (filtered, changed) = without_turn_content(raw)?;
        *raw = filtered;
        if changed {
            refused.push("reader_user_turn_content_unsupported");
        }
    }
    if !refused.is_empty() {
        let mut reasons: std::collections::BTreeMap<String, usize> = fields
            .0
            .get("skipped_reasons")
            .map(|raw| serde_json::from_str(raw.get()))
            .transpose()
            .map_err(|_| invalid_label_answer())?
            .unwrap_or_default();
        for reason in refused {
            reasons.insert(reason.to_owned(), 1);
        }
        fields.0.insert(
            "skipped_reasons".to_owned(),
            serde_json::value::to_raw_value(&reasons).map_err(|_| invalid_label_answer())?,
        );
    }
    serde_json::value::to_raw_value(&fields.0)
        .map(LinkAnswer::Raw)
        .map_err(|_| invalid_label_answer())
}

/// Remove unsupported nested bodies before decoding their provider format.
/// Native wait markers remain readable on a peer with only `Turns`.
fn without_turn_content(
    raw: &serde_json::value::RawValue,
) -> Result<(Box<serde_json::value::RawValue>, bool), LinkError> {
    let mut tracker: RawReaderFields =
        serde_json::from_str(raw.get()).map_err(|_| invalid_label_answer())?;
    let mut changed = false;
    if let Some(last) = tracker.0.get_mut("last")
        && last.get() != "null"
    {
        let mut fields: RawReaderFields =
            serde_json::from_str(last.get()).map_err(|_| invalid_label_answer())?;
        changed |= fields
            .0
            .remove("plan_content")
            .is_some_and(|raw| raw.get() != "null");
        *last = serde_json::value::to_raw_value(&fields.0).map_err(|_| invalid_label_answer())?;
    }
    if let Some(questions) = tracker.0.get_mut("questions") {
        struct Questions;
        impl<'de> serde::de::Visitor<'de> for Questions {
            type Value = Vec<RawReaderFields>;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("bounded native question calls")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                let mut calls = Vec::new();
                while calls.len() < hide_session::turns::QUESTION_CALL_LIMIT {
                    let Some(call) = seq.next_element()? else {
                        return Ok(calls);
                    };
                    calls.push(call);
                }
                if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::custom("user_turn_capacity"));
                }
                Ok(calls)
            }
        }
        use serde::de::Deserializer;
        let mut decoder = serde_json::Deserializer::from_str(questions.get());
        let mut calls = (&mut decoder)
            .deserialize_seq(Questions)
            .map_err(|_| invalid_label_answer())?;
        decoder.end().map_err(|_| invalid_label_answer())?;
        for call in &mut calls {
            changed |= call
                .0
                .remove("content")
                .is_some_and(|raw| raw.get() != "null");
            call.0.insert(
                "content".into(),
                serde_json::value::to_raw_value(&Option::<()>::None)
                    .map_err(|_| invalid_label_answer())?,
            );
        }
        *questions = serde_json::value::to_raw_value(
            &calls.into_iter().map(|call| call.0).collect::<Vec<_>>(),
        )
        .map_err(|_| invalid_label_answer())?;
    }
    Ok((
        serde_json::value::to_raw_value(&tracker.0).map_err(|_| invalid_label_answer())?,
        changed,
    ))
}

fn invalid_label_answer() -> LinkError {
    LinkError::Unknown("session_reader_answer_invalid".to_owned())
}

pub(crate) struct RawReaderFields(
    pub std::collections::BTreeMap<String, Box<serde_json::value::RawValue>>,
);

impl<'de> serde::Deserialize<'de> for RawReaderFields {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Fields;
        impl<'de> serde::de::Visitor<'de> for Fields {
            type Value = RawReaderFields;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a bounded reader answer")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut fields = std::collections::BTreeMap::new();
                while let Some((name, value)) =
                    map.next_entry::<String, Box<serde_json::value::RawValue>>()?
                {
                    if name.len() > 64 || fields.len() == 32 || fields.insert(name, value).is_some()
                    {
                        return Err(serde::de::Error::custom("invalid label answer fields"));
                    }
                }
                Ok(RawReaderFields(fields))
            }
        }
        deserializer.deserialize_map(Fields)
    }
}

/// Path-only session operations name their provider at the worker boundary,
/// before the locator is sent. A replaced/draining link cannot lend its old
/// facts to a returned body or checkpoint.
pub fn call_as_reader<T: serde::de::DeserializeOwned>(
    link: &(impl NodeLink + ?Sized),
    agent: hide_session::Agent,
    feature: ReaderFeature,
    call: Call,
    timeout: Duration,
) -> Result<T, LinkError> {
    let readers = [(agent, feature)];
    check_readers(link, &readers)?;
    let answer = link.call(call, timeout)?;
    check_readers(link, &readers)?;
    decode(answer)
}

pub fn check_reader_call(link: &(impl NodeLink + ?Sized), call: &Call) -> Result<(), LinkError> {
    check_readers(link, &reader_requirements(call))
}

pub fn check_reader_support(
    link: &(impl NodeLink + ?Sized),
    agent: hide_session::Agent,
    feature: ReaderFeature,
) -> Result<(), LinkError> {
    check_readers(link, &[(agent, feature)])
}

/// The node repeats the sender's feature check before dispatching a native
/// parser. This uses compiled implementation facts, never caller claims.
pub fn check_reader_features(features: &ReaderFeatures, call: &Call) -> crate::HostResult<()> {
    if reader_requirements(call)
        .iter()
        .any(|(agent, feature)| !features.supports(agent.as_str(), *feature))
    {
        return Err(HostError::new(
            crate::ErrorCode::Unsupported,
            "session_reader_unsupported",
        ));
    }
    Ok(())
}

fn reader_requirements(call: &Call) -> Vec<(hide_session::Agent, ReaderFeature)> {
    match call {
        Call::LabelTranscript { request } => vec![(request.agent, ReaderFeature::Labels)],
        Call::SessionWorkTurn { request } => vec![(request.agent, ReaderFeature::Labels)],
        Call::SessionActivity { request } => vec![(request.agent, ReaderFeature::Activity)],
        Call::SessionConversation { agent, .. } => vec![(*agent, ReaderFeature::Conversation)],
        Call::UsageFiles { agent, .. } | Call::UsageRead { agent, .. } => {
            vec![(*agent, ReaderFeature::Usage)]
        }
        Call::LinkRead { requests } => requests
            .iter()
            .map(|request| (request.agent, ReaderFeature::Links))
            .collect(),
        _ => Vec::new(),
    }
}

fn check_readers(
    link: &(impl NodeLink + ?Sized),
    readers: &[(hide_session::Agent, ReaderFeature)],
) -> Result<(), LinkError> {
    if readers.is_empty() {
        return Ok(());
    }
    if let Some(reason) = link.closed_reason() {
        return Err(LinkError::NotConnected(reason));
    }
    let Some(features) = link.reader_features() else {
        return Err(LinkError::Refused(HostError::new(
            crate::ErrorCode::Unsupported,
            "session_reader_support_unavailable",
        )));
    };
    if readers
        .iter()
        .any(|(agent, feature)| !features.supports(agent.as_str(), *feature))
    {
        return Err(LinkError::Refused(HostError::new(
            crate::ErrorCode::Unsupported,
            "session_reader_unsupported",
        )));
    }
    Ok(())
}

fn decode<T: serde::de::DeserializeOwned>(answer: LinkAnswer) -> Result<T, LinkError> {
    let decoded = match answer {
        LinkAnswer::Parsed(value) => serde_json::from_value(value),
        LinkAnswer::Raw(raw) => serde_json::from_str(raw.get()),
    };
    decoded.map_err(|error| {
        LinkError::Unknown(format!(
            "The device helper answered in an unexpected shape: {error}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct LabelPeer {
        features: ReaderFeatures,
        response: &'static str,
    }

    impl NodeLink for LabelPeer {
        fn call(&self, call: Call, _timeout: Duration) -> Result<LinkAnswer, LinkError> {
            let Call::LabelTranscript { request } = call else {
                panic!("expected label request")
            };
            if !self
                .features
                .supports(request.agent.as_str(), ReaderFeature::Turns)
                && request.turns.is_some()
            {
                return Err(LinkError::Refused(HostError::new(
                    crate::ErrorCode::Unsupported,
                    "checkpoint format unsupported",
                )));
            }
            Ok(LinkAnswer::Raw(
                serde_json::value::RawValue::from_string(self.response.to_owned()).unwrap(),
            ))
        }

        fn reader_features(&self) -> Option<&ReaderFeatures> {
            Some(&self.features)
        }
    }

    #[derive(serde::Deserialize)]
    struct LabelProbe {
        events: Vec<hide_session::label_transcript::LabelEvent>,
        #[serde(default)]
        confirmed: Option<hide_session::ConfirmedLabelSession>,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        turns: Option<hide_session::turns::TurnTracker>,
        skipped_reasons: std::collections::BTreeMap<String, usize>,
    }

    fn label_call(agent: hide_session::Agent) -> Call {
        Call::LabelTranscript {
            request: hide_session::label_transcript::LabelTranscriptRequest {
                agent,
                reference_kind: "id".to_owned(),
                reference_value: "native-fixture".to_owned(),
                cwd: None,
                checkpoint: None,
                subagents: Default::default(),
                turns: Some(Default::default()),
                tools: None,
            },
        }
    }

    #[test]
    fn unadvertised_optional_formats_do_not_block_working_labels() {
        let peer = LabelPeer {
            features: serde_json::from_str(r#"[{"provider":"claude","features":["labels"]}]"#)
                .unwrap(),
            // These optional fields deliberately cannot decode as the current
            // native title or turn format. The label event is independently valid.
            response: r#"{"title":{"future":true},"custom_title":[],"turns":{"through":"future-checkpoint"},"events":[{"kind":"human","at_unix_ms":1,"text":"working request","offset":0}],"skipped_reasons":{}}"#,
        };
        let answer: LabelProbe = call_as(
            &peer,
            label_call(hide_session::Agent::Claude),
            Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(answer.events[0].text, "working request");
        assert!(answer.title.is_none());
        assert!(answer.turns.is_none());
        assert_eq!(
            answer.skipped_reasons.get("reader_titles_unsupported"),
            Some(&1)
        );
        assert_eq!(
            answer.skipped_reasons.get("reader_turns_unsupported"),
            Some(&1)
        );
    }

    #[test]
    fn labels_keep_owner_proof_without_decoding_unadvertised_native_identity() {
        let peer = LabelPeer {
            features: serde_json::from_str(r#"[{"provider":"claude","features":["labels"]}]"#)
                .unwrap(),
            response: r#"{"confirmed":{"owner":"v1:fixture","native_session_id":{"future":true},"source_path":{"future":true},"incarnation":"1:2","bytes":10},"events":[{"kind":"human","at_unix_ms":1,"text":"working request","offset":0}],"skipped_reasons":{}}"#,
        };
        let answer: LabelProbe = call_as(
            &peer,
            label_call(hide_session::Agent::Claude),
            Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(answer.events[0].text, "working request");
        let confirmed = answer.confirmed.unwrap();
        assert_eq!(confirmed.owner, "v1:fixture");
        assert!(confirmed.native_session_id.is_none());
        assert!(confirmed.source_path.is_none());
        assert_eq!(
            answer.skipped_reasons.get("reader_identity_unsupported"),
            Some(&1)
        );
    }

    #[test]
    fn advertised_native_identity_is_retained_and_validated() {
        for (response, valid) in [
            (
                r#"{"confirmed":{"owner":"v1:fixture","native_session_id":"native-fixture","incarnation":"1:2","bytes":10},"events":[],"skipped_reasons":{}}"#,
                true,
            ),
            (
                r#"{"confirmed":{"owner":"v1:fixture","native_session_id":"outside/session","incarnation":"1:2","bytes":10},"events":[],"skipped_reasons":{}}"#,
                false,
            ),
        ] {
            let peer = LabelPeer {
                features: serde_json::from_str(
                    r#"[{"provider":"claude","features":["labels","identity"]}]"#,
                )
                .unwrap(),
                response,
            };
            let answer: Result<LabelProbe, _> = call_as(
                &peer,
                label_call(hide_session::Agent::Claude),
                Duration::from_secs(1),
            );
            if valid {
                assert_eq!(
                    answer
                        .unwrap()
                        .confirmed
                        .unwrap()
                        .native_session_id
                        .as_deref(),
                    Some("native-fixture")
                );
            } else {
                assert!(answer.is_err());
            }
        }
    }

    #[test]
    fn protocol24_codex_keeps_its_existing_plan_hold() {
        let peer = LabelPeer {
            features: ReaderFeatures::protocol24(),
            response: r#"{"title":"Native title","events":[],"turns":{"through":31,"last":{"id":"turn","mode":"plan","plan":true,"end":"completed","answered":false}},"skipped_reasons":{}}"#,
        };
        let answer: LabelProbe = call_as(
            &peer,
            label_call(hide_session::Agent::Codex),
            Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(answer.title.as_deref(), Some("Native title"));
        assert_eq!(
            answer.turns.unwrap().waiting(),
            Some(hide_session::turns::Waiting::PlanApproval)
        );
    }

    #[test]
    fn a_peer_without_user_turn_content_keeps_native_waits_without_decoding_future_bodies() {
        for response in [
            r#"{"events":[],"turns":{"through":31,"last":{"id":"turn","mode":"plan","plan":true,"plan_content":{"future":42},"end":"completed","answered":false}},"skipped_reasons":{}}"#,
            r#"{"events":[],"turns":{"through":31,"questions":[{"call":"ask-a","content":{"future":42},"answered":false}]},"skipped_reasons":{}}"#,
        ] {
            let peer = LabelPeer {
                features: ReaderFeatures::protocol24(),
                response,
            };
            let answer: LabelProbe = call_as(
                &peer,
                label_call(hide_session::Agent::Codex),
                Duration::from_secs(1),
            )
            .unwrap();
            let turn = answer.turns.unwrap().user_turn().unwrap();
            assert!(turn.content.is_none());
            assert_eq!(
                answer
                    .skipped_reasons
                    .get("reader_user_turn_content_unsupported"),
                Some(&1)
            );
        }
    }

    #[test]
    fn request_checkpoint_content_is_removed_independently_from_the_native_plan_hold() {
        let peer = LabelPeer {
            features: ReaderFeatures::protocol24(),
            response: "{}",
        };
        let mut call = label_call(hide_session::Agent::Codex);
        let Call::LabelTranscript { request } = &mut call else {
            unreachable!()
        };
        request.turns = Some(
            serde_json::from_value(serde_json::json!({
                "through":31,"last":{"id":"turn","mode":"plan","plan":true,
                "plan_content":{"text":"review this plan","choices":[],"truncated":false},
                "end":"completed","answered":false}
            }))
            .unwrap(),
        );
        prepare_label_call(&peer, &mut call);
        let Call::LabelTranscript { request } = call else {
            unreachable!()
        };
        let turn = request.turns.unwrap().user_turn().unwrap();
        assert_eq!(turn.kind, hide_session::turns::UserTurnKind::PlanApproval);
        assert!(turn.content.is_none());
    }

    #[test]
    fn an_answer_s_debug_form_names_its_size_never_its_contents() {
        let secret = serde_json::json!({"access_token": "sk-secret"});
        let parsed = format!("{:?}", LinkAnswer::Parsed(secret.clone()));
        let raw = format!(
            "{:?}",
            LinkAnswer::Raw(serde_json::value::to_raw_value(&secret).unwrap())
        );
        for shown in [parsed, raw] {
            assert!(!shown.contains("sk-secret"), "{shown}");
            assert!(shown.contains("bytes"), "{shown}");
        }
    }

    #[test]
    fn usage_calls_require_the_live_usage_reader_not_a_protocol_guess() {
        for agent in hide_session::Agent::supported() {
            for call in [
                Call::UsageFiles {
                    agent,
                    since_unix_ms: 0,
                },
                Call::UsageRead {
                    agent,
                    path: "/fixture/native.jsonl".into(),
                    checkpoint: None,
                },
            ] {
                assert!(check_reader_features(&ReaderFeatures::protocol24(), &call).is_err());
                let peer = LabelPeer {
                    features: ReaderFeatures::protocol24(),
                    response: "null",
                };
                assert!(check_reader_call(&peer, &call).is_err());
                assert_eq!(
                    check_reader_features(&ReaderFeatures::implemented(), &call).is_ok(),
                    agent != hide_session::Agent::Cursor
                );
            }
        }
    }

    #[test]
    fn cursor_graph_calls_and_saved_progress_require_current_reader_facts() {
        let call = Call::SessionConversation {
            agent: hide_session::Agent::Cursor,
            path: "/private-fixture/store.db".into(),
            scope: hide_session::SessionReadScope {
                id: "fixture".into(),
                cwd: "/private-fixture".into(),
            },
            checkpoint: Some(Default::default()),
        };
        for features in [
            ReaderFeatures::protocol24(),
            serde_json::from_str::<ReaderFeatures>(
                r#"[{"provider":"grok","features":["conversation"]}]"#,
            )
            .unwrap(),
        ] {
            assert!(check_reader_features(&features, &call).is_err());
            let peer = LabelPeer {
                features,
                response: "null",
            };
            assert!(check_reader_call(&peer, &call).is_err());
            let answer: Result<serde_json::Value, _> = call_as_reader(
                &peer,
                hide_session::Agent::Cursor,
                ReaderFeature::Conversation,
                call.clone(),
                Duration::from_secs(1),
            );
            assert!(answer.is_err());
        }
        assert!(check_reader_features(&ReaderFeatures::implemented(), &call).is_ok());
    }
}
