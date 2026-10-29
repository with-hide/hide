//! The two checks a card gets after it was written (PRD memory-card-quality
//! D-07, D-11): a card whose evidence moved is read against the repository
//! again, and a card about to be offered for the repository's documents is
//! read against the documents agents already have.
//!
//! Both are provider calls on the analysis worker under the analysis call
//! cap and weekly deferral (D-08): a deferral leaves the card as it was, and
//! a card is asked about at most once a day (B10). Nothing here runs under
//! the runtime lock; the files come from the node, the operator's own
//! instruction files from the account this process runs as.

use std::collections::BTreeSet;
use std::path::Path;

use hide_ai::{AiError, AiRequest, AiRouter, CancelToken};
use hide_memory::{
    DOCUMENT_EXCERPT_BYTES, DOCUMENT_LIMIT, EXCERPT_SOURCE_BYTES, Excerpt, FILE_EXCERPT_BYTES,
    GLOBAL_EXCERPT_BYTES, GLOBAL_LIMIT, MemoryStore, PROMOTION_CHECKS_PER_PASS, PendingCard,
    PromoteCheck, PromoteCheckCard, Promotion, PromotionState, REVERIFY_PER_PASS, Verification,
    cited_files, parse_promote_check, parse_verification, pointer_target, promote_check_request,
    query_words, relevant_excerpt, sensitive_path, verify_request, written_names,
};
use serde_json::{Value, json};

use super::memory::{digest, now_ms};
use super::memory_analysis::{AnalysisFailure, FailureScope, count_input_failure};
use super::memory_evidence::CheckoutFacts;
use crate::node_access::{NodeLink, read_bytes};

/// What a Project's checks changed.
#[derive(Default)]
pub(super) struct Checked {
    pub(super) changed: usize,
    /// Calls the provider answered usably: what recovers a Project from an
    /// earlier provider failure.
    pub(super) answered: usize,
    /// Calls answered with something unusable: spent, but no sign of health.
    pub(super) unusable: usize,
}

/// What a check reports as it goes, for the card shown as being checked.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CheckProgress {
    Checking(u64),
    Answered,
    Unusable,
}

/// Completed reconciliation work survives every later stop in the same pass.
#[derive(Default)]
pub(super) struct Reconciled {
    pub(super) checked: Checked,
    pub(super) stopped: Option<AnalysisFailure>,
}

type Admit<'a> = &'a mut dyn FnMut(&MemoryStore) -> Result<(), AnalysisFailure>;

fn provider_failure(error: AiError) -> AnalysisFailure {
    match error {
        AiError::Cancelled => AnalysisFailure::Cancelled,
        error => AnalysisFailure::Provider(error),
    }
}

/// A file as the node gave it, up to [`EXCERPT_SOURCE_BYTES`], before an
/// excerpt for one card is chosen from it.
pub(super) struct Source {
    pub(super) path: String,
    pub(super) text: String,
    pub(super) truncated: bool,
}

/// An answer that arrived but cannot be used (not the schema, a correction
/// with no text, a document never shown): the card's day is spent, so a card
/// the model keeps answering badly does not hold the queue behind it or
/// back the Project off at every pass. Nothing of the answer is logged.
fn spend_unusable(
    store: &mut MemoryStore,
    item_id: &str,
    project_id: &str,
    number: u64,
) -> Result<(), AnalysisFailure> {
    store
        .begin_verification(item_id, now_ms())
        .map_err(|error| AnalysisFailure::Local(error.to_string()))?;
    crate::diagnostic!(json!({
        "component": "memory",
        "kind": "verification.unusable",
        "project_id": project_id,
        "card": number,
    }));
    Ok(())
}

/// Reads a checkout's file through the node.
fn source(node: &dyn NodeLink, root: &str, relative: &str) -> Option<Source> {
    if sensitive_path(relative) {
        return None;
    }
    let range = read_bytes(node, root, relative, 0, EXCERPT_SOURCE_BYTES as u64).ok()?;
    let bytes = range.bytes().ok()?;
    Some(Source {
        path: relative.to_owned(),
        text: String::from_utf8_lossy(&bytes).into_owned(),
        truncated: range.total > bytes.len() as u64,
    })
}

/// The part of each source about the card, each cut to `limit`.
fn excerpts(sources: &[Source], words: &[String], limit: usize) -> Vec<Excerpt> {
    sources
        .iter()
        .map(|source| relevant_excerpt(&source.path, &source.text, source.truncated, words, limit))
        .collect()
}

/// Only text that survived the request's redaction and whole-input cap can
/// support a repository answer. Loaded files omitted by the cap are unseen.
fn sent_documents(request: &AiRequest, key: &str, sources: &[Source]) -> Vec<Excerpt> {
    let input: serde_json::Value =
        serde_json::from_str(&request.input).expect("the request builder serialized its own input");
    input[key]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let path = entry["path"].as_str()?;
            sources
                .iter()
                .any(|source| source.path == path)
                .then(|| Excerpt {
                    path: path.into(),
                    text: entry["text"].as_str().unwrap_or_default().into(),
                    truncated: entry["truncated"].as_bool().unwrap_or(true),
                })
        })
        .collect()
}

/// Asks about the Project's unverified cards one by one, at most
/// [`REVERIFY_PER_PASS`] a pass, and applies each answer: a card that holds
/// goes back to the index with its evidence taken again, an outdated one gets
/// its corrected text as a new revision its Undo takes back, and one that no
/// longer applies is forgotten, which Undo takes back too (B7-B9).
#[allow(clippy::too_many_arguments)] // one Project's store, node, provider and the pass's limits
pub(super) fn reverify_pending(
    store: &mut MemoryStore,
    node: &dyn NodeLink,
    facts: &mut CheckoutFacts<'_>,
    router: &AiRouter,
    language: hide_ai::OutputLanguage,
    project_id: &str,
    root: &str,
    cancel: &CancelToken,
    admit: Admit<'_>,
    progress: &mut dyn FnMut(CheckProgress),
) -> Result<Checked, AnalysisFailure> {
    let local = |error: String| AnalysisFailure::Local(error);
    let mut checked = Checked::default();
    let cards = store
        .pending_cards(project_id, now_ms(), REVERIFY_PER_PASS)
        .map_err(|error| local(error.to_string()))?;
    let repository = repository_sources(node, facts, root);
    for card in cards {
        if cancel.is_cancelled() {
            return Err(AnalysisFailure::Cancelled);
        }
        // The deferral comes first, so a held-back card keeps its day.
        admit(store)?;
        progress(CheckProgress::Checking(card.number));
        // A card from before evidence was kept has none; what its text names
        // is read now, so the check is not asked blind.
        let mut card = card;
        if card.evidence.is_empty() {
            card.evidence = facts.gather(&card.body, &card.cited_paths);
        }
        let answers = facts.facts(&card.evidence);
        let words = query_words(&[&card.title, &card.body, &card.aliases.join(" ")]);
        let mut files = excerpts(
            &cited_files(&card.evidence)
                .into_iter()
                .filter_map(|path| source(node, root, &path))
                .collect::<Vec<_>>(),
            &words,
            FILE_EXCERPT_BYTES,
        );
        if card.kind != hide_memory::CandidateKind::Preference {
            files.extend(excerpts(&repository, &words, DOCUMENT_EXCERPT_BYTES));
        }
        let request = verify_request(
            digest(&[
                "memory-verify",
                project_id,
                &card.item_id,
                &card.revision.to_string(),
                &now_ms().to_string(),
            ]),
            project_id,
            &card,
            &answers,
            &files,
            language,
        )
        .map_err(|error| local(error.to_string()))?;
        let sent_repository = sent_documents(&request, "files", &repository);
        let verdict = router.execute(&request, cancel).and_then(|answer| {
            parse_verification(&answer.value)
                .map_err(|error| AiError::InvalidOutput(error.to_string()))
        });
        let verdict = match verdict {
            Ok(verdict) => verdict,
            Err(error) if FailureScope::of(&error) == FailureScope::Input => {
                checked.unusable += 1;
                spend_unusable(store, &card.item_id, project_id, card.number)?;
                progress(CheckProgress::Unusable);
                continue;
            }
            Err(error) => {
                let subject = format!("verify:{}", card.item_id);
                if count_input_failure(
                    store,
                    project_id,
                    &subject,
                    &card.revision.to_string(),
                    &error,
                )
                .map_err(local)?
                {
                    spend_unusable(store, &card.item_id, project_id, card.number)?;
                }
                return Err(provider_failure(error));
            }
        };
        // An answer ends the run of requests the provider never answered.
        store
            .input_answered(project_id, &format!("verify:{}", card.item_id), now_ms())
            .map_err(|error| local(error.to_string()))?;
        checked.answered += 1;
        // The day is spent by an answer, not by a call: a provider that
        // failed leaves the card due for the next pass.
        if !store
            .begin_verification(&card.item_id, now_ms())
            .map_err(|error| local(error.to_string()))?
        {
            progress(CheckProgress::Answered);
            continue;
        }
        let applied = match &verdict {
            Verification::Valid => {
                let evidence = facts.gather(&card.body, &card.cited_paths);
                store.confirm_card(
                    &card.item_id,
                    card.revision,
                    card.outdated_reason.as_deref(),
                    &evidence,
                    now_ms(),
                )
            }
            Verification::Outdated(correction) => revise(
                store,
                facts,
                project_id,
                &card,
                correction.clone(),
                root,
                node,
            ),
            Verification::Gone => store.forget_checked(project_id, &card.item_id, card.revision),
            Verification::InDocs { document } => {
                if card.kind != hide_memory::CandidateKind::Preference
                    && sent_repository
                        .iter()
                        .any(|source| source.path == *document)
                {
                    let item = store
                        .item_by_number(project_id, card.number)
                        .map_err(|error| local(error.to_string()))?
                        .ok_or_else(|| local("card_missing".into()))?;
                    if item.revision == card.revision {
                        point_at(store, project_id, &item, document).map(|()| true)
                    } else {
                        Ok(false)
                    }
                } else {
                    Ok(false)
                }
            }
            Verification::DocsDisagree { document, quote } => apply_disagreement(
                store,
                node,
                project_id,
                card.number,
                card.revision,
                root,
                &sent_repository,
                document,
                quote,
            ),
        }
        .map_err(|error| local(error.to_string()))?;
        if applied {
            checked.changed += 1;
        }
        progress(CheckProgress::Answered);
        crate::diagnostic!(json!({
            "component": "memory",
            "kind": "verification.applied",
            "project_id": project_id,
            "card": card.number,
            "verdict": match &verdict {
                Verification::Valid => "valid",
                Verification::Outdated(_) => "outdated",
                Verification::Gone => "gone",
                Verification::InDocs { .. } => "in_docs",
                Verification::DocsDisagree { .. } => "docs_disagree",
            },
            "applied": applied,
        }));
    }
    Ok(checked)
}

fn revise(
    store: &mut MemoryStore,
    facts: &mut CheckoutFacts<'_>,
    project_id: &str,
    card: &PendingCard,
    correction: hide_memory::Correction,
    root: &str,
    node: &dyn NodeLink,
) -> Result<bool, hide_memory::MemoryError> {
    let cited = super::memory_analysis::paths_inside(node, root, &card.cited_paths);
    let cited = card
        .cited_paths
        .iter()
        .filter(|path| cited.contains(*path))
        .cloned()
        .collect::<Vec<_>>();
    let evidence = facts.gather(&correction.text, &cited);
    let title = if correction.title.is_empty() {
        card.title.as_str()
    } else {
        correction.title.as_str()
    };
    let aliases = if correction.aliases.is_empty() {
        card.aliases.as_slice()
    } else {
        correction.aliases.as_slice()
    };
    store.revise_card(
        project_id,
        &card.item_id,
        card.revision,
        card.outdated_reason.as_deref(),
        &correction.text,
        title,
        aliases,
        &cited,
        &evidence,
        now_ms(),
    )
}

/// The documents agents read at the start of work here: `AGENTS.md`,
/// `CLAUDE.md` when it says something else, then the repository documents
/// they point to, at most [`DOCUMENT_LIMIT`] and each cut to a bound.
pub(super) fn repository_sources(
    node: &dyn NodeLink,
    facts: &CheckoutFacts<'_>,
    root: &str,
) -> Vec<Source> {
    let mut documents: Vec<Source> = Vec::new();
    for name in ["AGENTS.md", "CLAUDE.md"] {
        if let Some(found) = source(node, root, name)
            && !documents.iter().any(|known| known.text == found.text)
        {
            documents.push(found);
        }
    }
    let pointed = documents
        .iter()
        .flat_map(|document| written_names(&document.text).paths)
        .filter(|path| path.ends_with(".md"))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    for path in facts.existing(&pointed) {
        if documents.len() >= DOCUMENT_LIMIT {
            break;
        }
        if let Some(found) = source(node, root, &path)
            && !documents.iter().any(|known| known.path == found.path)
        {
            documents.push(found);
        }
    }
    documents.truncate(DOCUMENT_LIMIT);
    documents
}

/// The operator's own instruction files, which agents read in every
/// repository, through the node of the account this core runs as. A file that
/// is not there is skipped, and one with the text of a file already read (a
/// link to it) is not read twice.
fn global_sources(node: &dyn NodeLink, home: Option<&Path>) -> Vec<Source> {
    let Some(home) = home else {
        return Vec::new();
    };
    let mut documents: Vec<Source> = Vec::new();
    for (folder, name) in [
        (".claude", "CLAUDE.md"),
        (".codex", "AGENTS.md"),
        (".config/agents", "AGENTS.md"),
    ] {
        if documents.len() >= GLOBAL_LIMIT {
            break;
        }
        let root = home.join(folder).to_string_lossy().into_owned();
        let Some(mut found) = source(node, &root, name) else {
            continue;
        };
        if documents.iter().any(|known| known.text == found.text) {
            continue;
        }
        // The name an agent knows it by, never the account's folder.
        found.path = format!("~/{folder}/{name}");
        documents.push(found);
    }
    documents
}

/// Reads each card about to be suggested against the evidence now and the
/// documents agents already read, at most [`PROMOTION_CHECKS_PER_PASS`] a
/// pass (D-11). A card to propose is marked checked at its current text so
/// the tab asks about it; an outdated one is corrected like a re-verification
/// (B16); one a repository document already says becomes a pointer at that
/// file with no pull request (B15), and one the operator's global files
/// already say is forgotten, which Undo takes back.
#[allow(clippy::too_many_arguments)] // one Project's store, node, provider and the pass's limits
pub(super) fn check_before_suggestion(
    store: &mut MemoryStore,
    node: &dyn NodeLink,
    facts: &mut CheckoutFacts<'_>,
    router: &AiRouter,
    language: hide_ai::OutputLanguage,
    project_id: &str,
    root: &str,
    home: Option<&Path>,
    cancel: &CancelToken,
    admit: Admit<'_>,
    progress: &mut dyn FnMut(CheckProgress),
) -> Result<Checked, AnalysisFailure> {
    let local = |error: String| AnalysisFailure::Local(error);
    let mut checked = Checked::default();
    let due = store
        .promotion_unchecked(project_id, now_ms(), PROMOTION_CHECKS_PER_PASS)
        .map_err(|error| local(error.to_string()))?;
    if due.is_empty() {
        return Ok(checked);
    }
    let repository = repository_sources(node, facts, root);
    let global = global_sources(node, home);
    for suggestion in due {
        if cancel.is_cancelled() {
            return Err(AnalysisFailure::Cancelled);
        }
        let Some(item) = store
            .item_by_number(project_id, suggestion.number)
            .map_err(|error| local(error.to_string()))?
        else {
            continue;
        };
        let detail = store
            .detail(project_id, &item.id)
            .map_err(|error| local(error.to_string()))?;
        let answers = facts.facts(&detail.evidence);
        let words = query_words(&[&item.title, &item.body, &item.aliases.join(" ")]);
        admit(store)?;
        progress(CheckProgress::Checking(item.number));
        let request = promote_check_request(
            digest(&[
                "memory-promote-check",
                project_id,
                &item.id,
                &item.revision.to_string(),
            ]),
            project_id,
            &PromoteCheckCard {
                number: item.number,
                kind: item.kind,
                title: &item.title,
                body: &item.body,
                evidence: &detail.evidence,
                facts: &answers,
            },
            &excerpts(&repository, &words, DOCUMENT_EXCERPT_BYTES),
            &excerpts(&global, &words, GLOBAL_EXCERPT_BYTES),
            language,
        )
        .map_err(|error| local(error.to_string()))?;
        let sent_repository = sent_documents(&request, "documents", &repository);
        let known = sent_repository
            .iter()
            .chain(&sent_documents(&request, "global", &global))
            .map(|document| document.path.clone())
            .collect::<Vec<_>>();
        let verdict = router.execute(&request, cancel).and_then(|answer| {
            parse_promote_check(&answer.value, &known)
                .map_err(|error| AiError::InvalidOutput(error.to_string()))
        });
        let verdict = match verdict {
            Ok(verdict) => verdict,
            Err(error) if FailureScope::of(&error) == FailureScope::Input => {
                checked.unusable += 1;
                spend_unusable(store, &item.id, project_id, item.number)?;
                progress(CheckProgress::Unusable);
                continue;
            }
            Err(error) => {
                let subject = format!("promote:{}", item.id);
                if count_input_failure(
                    store,
                    project_id,
                    &subject,
                    &item.revision.to_string(),
                    &error,
                )
                .map_err(local)?
                {
                    spend_unusable(store, &item.id, project_id, item.number)?;
                }
                return Err(provider_failure(error));
            }
        };
        // An answer ends the run of requests the provider never answered.
        store
            .input_answered(project_id, &format!("promote:{}", item.id), now_ms())
            .map_err(|error| local(error.to_string()))?;
        checked.answered += 1;
        // The day's one answer about the card, so a card the check keeps
        // correcting costs one call a day (B10).
        if !store
            .begin_verification(&item.id, now_ms())
            .map_err(|error| local(error.to_string()))?
        {
            progress(CheckProgress::Answered);
            continue;
        }
        match verdict {
            PromoteCheck::DocsDisagree { document, quote } => {
                if apply_disagreement(
                    store,
                    node,
                    project_id,
                    item.number,
                    item.revision,
                    root,
                    &sent_repository,
                    &document,
                    &quote,
                )
                .map_err(|error| local(error.to_string()))?
                {
                    checked.changed += 1;
                }
            }
            PromoteCheck::Propose => {
                store
                    .mark_promotion_checked(project_id, item.number, item.revision)
                    .map_err(|error| local(error.to_string()))?;
            }
            PromoteCheck::Outdated(correction) => {
                let pending = PendingCard {
                    item_id: item.id.clone(),
                    number: item.number,
                    revision: item.revision,
                    kind: item.kind,
                    title: item.title.clone(),
                    body: item.body.clone(),
                    aliases: item.aliases.clone(),
                    cited_paths: item.cited_paths.clone(),
                    evidence: detail.evidence.clone(),
                    outdated_reason: None,
                };
                if revise(store, facts, project_id, &pending, correction, root, node)
                    .map_err(|error| local(error.to_string()))?
                {
                    checked.changed += 1;
                }
            }
            PromoteCheck::InDocs { document } => {
                if pointer_target(&document) && !document.starts_with("~/") {
                    point_at(store, project_id, &item, &document)
                        .map_err(|error| local(error.to_string()))?;
                } else {
                    store
                        .forget_checked(project_id, &item.id, item.revision)
                        .map_err(|error| local(error.to_string()))?;
                }
                checked.changed += 1;
            }
        }
        progress(CheckProgress::Answered);
        crate::diagnostic!(json!({
            "component": "memory",
            "kind": "promotion.checked",
            "project_id": project_id,
            "card": item.number,
        }));
    }
    Ok(checked)
}

/// Reconcile a confirmed correction when its owning document changes, including
/// edits made directly by a terminal agent. The existing writer owns the pointer.
#[allow(clippy::too_many_arguments)]
pub(super) fn reconcile_corrections(
    store: &mut MemoryStore,
    node: &dyn NodeLink,
    router: &AiRouter,
    language: hide_ai::OutputLanguage,
    project: &str,
    root: &str,
    cancel: &CancelToken,
    admit: Admit<'_>,
) -> Reconciled {
    let local = |error: hide_memory::MemoryError| AnalysisFailure::Local(error.to_string());
    let mut result = Reconciled::default();
    let stopped = (|| {
        for hide_memory::CorrectionCheck {
            number,
            revision,
            comparison,
            fingerprint: previous,
        } in store.correction_checks(project, 4).map_err(local)?
        {
            if cancel.is_cancelled() {
                return Err(AnalysisFailure::Cancelled);
            }
            let Some(document) = source(node, root, &comparison.document) else {
                store
                    .record_correction_check(project, number, revision, "unavailable", now_ms())
                    .map_err(local)?;
                continue;
            };
            let fingerprint = digest(&[
                &document.text,
                if document.truncated {
                    "truncated"
                } else {
                    "whole"
                },
            ]);
            // A correction with no stored sentence pair asks again once a day
            // (the same call, the same daily cap) until its answer brings one.
            let wanted = if comparison.replacement.is_none() {
                format!("{fingerprint}:pair:{}", now_ms() / 86_400_000)
            } else {
                fingerprint.clone()
            };
            if previous.as_deref() == Some(&wanted) {
                store
                    .record_correction_check(project, number, revision, &wanted, now_ms())
                    .map_err(local)?;
                continue;
            }
            let Some(item) = store.item_by_number(project, number).map_err(local)? else {
                continue;
            };
            let words = query_words(&[&item.title, &item.body]);
            let excerpt = relevant_excerpt(
                &document.path,
                &document.text,
                document.truncated,
                &words,
                DOCUMENT_EXCERPT_BYTES,
            );
            let claim =
                json!({"claim":item.body,"human_evidence":comparison.operator_text}).to_string();
            let Some(request) = hide_memory::creation_docs_request(
                digest(&["reconcile", &item.id, &revision.to_string(), &wanted]),
                project,
                &[(0, claim, excerpt)],
                hide_memory::ANALYSIS_INPUT_LIMIT_BYTES,
                language,
            ) else {
                store
                    .record_correction_check(project, number, revision, &wanted, now_ms())
                    .map_err(local)?;
                continue;
            };
            let sent_excerpt = serde_json::from_str::<Value>(&request.input)
                .ok()
                .and_then(|input| {
                    input["candidates"][0]["excerpt"]
                        .as_str()
                        .map(str::to_owned)
                })
                .unwrap_or_default();
            admit(store)?;
            // A failed attempt rotates this card without certifying an unseen
            // document fingerprint. It remains eligible on the next retry.
            store
                .record_correction_check(
                    project,
                    number,
                    revision,
                    previous.as_deref().unwrap_or(""),
                    now_ms(),
                )
                .map_err(local)?;
            let verdict = (|| {
                let answer = router.execute(&request, cancel).map_err(provider_failure)?;
                let results = answer.value["results"]
                    .as_array()
                    .filter(|rows| rows.len() == 1)
                    .ok_or_else(|| {
                        AnalysisFailure::Provider(AiError::InvalidOutput(
                            "correction_reconcile_shape".into(),
                        ))
                    })?;
                if results[0]["candidate"] != 0
                    || !matches!(
                        results[0]["verdict"].as_str(),
                        Some("same" | "differs" | "correction" | "unrelated" | "exception")
                    )
                {
                    return Err(AnalysisFailure::Provider(AiError::InvalidOutput(
                        "correction_reconcile_verdict".into(),
                    )));
                }
                Ok((
                    results[0]["verdict"]
                        .as_str()
                        .expect("validated verdict")
                        .to_owned(),
                    accepted_pair(&results[0], &sent_excerpt),
                ))
            })();
            let subject = format!("correction:{}", item.id);
            let (verdict, pair) = match verdict {
                Ok(answer) => answer,
                // Counted against this revision and document version; set
                // aside, it is asked about again only when either changes.
                // An unusable answer stops nothing else; any other failure
                // stops the pass's provider work.
                Err(AnalysisFailure::Provider(error)) => {
                    let input = format!("{revision}:{fingerprint}");
                    if count_input_failure(store, project, &subject, &input, &error)
                        .map_err(AnalysisFailure::Local)?
                    {
                        store
                            .record_correction_check(project, number, revision, &wanted, now_ms())
                            .map_err(local)?;
                    }
                    if FailureScope::of(&error) != FailureScope::Input {
                        return Err(AnalysisFailure::Provider(error));
                    }
                    result.checked.unusable += 1;
                    continue;
                }
                Err(error) => return Err(error),
            };
            store
                .input_answered(project, &subject, now_ms())
                .map_err(local)?;
            result.checked.answered += 1;
            if verdict == "same"
                && store
                    .item_by_number(project, number)
                    .map_err(local)?
                    .is_some_and(|current| current.revision == revision)
            {
                point_at(store, project, &item, &comparison.document).map_err(local)?;
                result.checked.changed += 1;
            }
            let mut certified = &wanted;
            if matches!(verdict.as_str(), "correction" | "differs") {
                match pair {
                    Some((document_sentence, replacement)) => {
                        // The authority stays whatever the operator and the
                        // first comparison settled; only the pair is added.
                        if let Some(mut current) =
                            store.document_comparison(project, number).map_err(local)?
                            && current.document == comparison.document
                            && current.authority == comparison.authority
                            && store
                                .item_by_number(project, number)
                                .map_err(local)?
                                .is_some_and(|item| item.revision == revision)
                        {
                            current.document_sentence = Some(document_sentence);
                            current.replacement = Some(replacement);
                            if store
                                .set_document_comparison(project, number, revision, &current)
                                .map_err(local)?
                            {
                                certified = &fingerprint;
                            }
                        }
                    }
                    None => crate::diagnostic!(json!({
                        "component": "memory",
                        "kind": "comparison.sentence_rejected",
                        "project_id": project,
                        "card": number,
                    })),
                }
            }
            store
                .record_correction_check(project, number, revision, certified, now_ms())
                .map_err(local)?;
        }
        Ok(())
    })();
    result.stopped = stopped.err();
    result
}

/// The sentence pair one comparison result carries, when it passes
/// [`hide_memory::accepted_sentence_pair`] against the excerpt that was sent.
pub(super) fn accepted_pair(result: &Value, sent_excerpt: &str) -> Option<(String, String)> {
    hide_memory::accepted_sentence_pair(
        sent_excerpt,
        result["document_sentence"].as_str().unwrap_or_default(),
        result["replacement"].as_str().unwrap_or_default(),
    )
}

/// The card becomes a line pointing at the document that already says it:
/// the merged state without a pull request, left out of every prompt.
fn point_at(
    store: &mut MemoryStore,
    project_id: &str,
    item: &hide_memory::MemoryItem,
    document: &str,
) -> Result<(), hide_memory::MemoryError> {
    let attempt = store
        .promotions(project_id)?
        .into_iter()
        .find(|(number, _)| *number == item.number)
        .map_or(0, |(_, promotion)| promotion.attempt);
    let now = now_ms();
    store.set_promotion(
        project_id,
        item.number,
        &Promotion {
            state: PromotionState::Merged,
            revision: item.revision,
            attempt,
            agent_id: None,
            branch: None,
            pr_number: None,
            repository: None,
            target_path: Some(document.to_owned()),
            started_at_unix_ms: now,
            updated_at_unix_ms: now,
            failure: None,
            building_branch: None,
        },
    )
}

/// One deterministic authority path used by both existing comparison calls.
#[allow(clippy::too_many_arguments)]
fn apply_disagreement(
    store: &mut MemoryStore,
    node: &dyn NodeLink,
    project_id: &str,
    number: u64,
    revision: u64,
    root: &str,
    documents: &[Excerpt],
    document: &str,
    quote: &str,
) -> Result<bool, hide_memory::MemoryError> {
    let Some(source) = documents.iter().find(|source| {
        source.path == document && source.text.contains(quote) && !quote.trim().is_empty()
    }) else {
        return Ok(false);
    };
    let Some(item) = store.item_by_number(project_id, number)? else {
        return Ok(false);
    };
    if item.revision != revision || item.kind == hide_memory::CandidateKind::Preference {
        return Ok(false);
    }
    // A settled correction is revision-bound. A later unrelated file commit
    // cannot reopen its authority; reconciliation may only reflect its meaning.
    if store
        .document_comparison(project_id, number)?
        .is_some_and(|comparison| comparison.authority == hide_memory::DocumentAuthority::CardNewer)
    {
        return Ok(false);
    }
    let human_source = store.current_human_source(project_id, number)?;
    let human = human_source.as_ref();
    let operator_at = human.and_then(|source| source.at_unix_ms);
    let operator_text = human
        .and_then(|human| original_words(store, node, project_id, human))
        .unwrap_or_default();
    store.set_document_comparison(
        project_id,
        number,
        revision,
        &compare_document(
            node,
            root,
            &source.path,
            quote,
            operator_at,
            operator_text,
            human.map(|source| source.session_id.clone()),
        ),
    )
}

/// Dates come from the node, off the runtime lock. Creation and established
/// rechecks use the same authority decision without another provider call.
#[allow(clippy::too_many_arguments)]
pub(super) fn compare_document(
    node: &dyn NodeLink,
    root: &str,
    document: &str,
    quote: &str,
    operator_at: Option<u64>,
    operator_text: String,
    source_session: Option<String>,
) -> hide_memory::DocumentComparison {
    let document_at = crate::node_access::call_as::<String>(
        node,
        hide_node_link::protocol::Call::Git {
            root: root.into(),
            command: hide_node_link::git::GitCommand::DocumentTime {
                path: document.into(),
            },
        },
        std::time::Duration::from_secs(10),
    )
    .ok()
    .and_then(|date| date.trim().parse::<u64>().ok())
    .and_then(|date| date.checked_mul(1000));
    hide_memory::DocumentComparison {
        authority: hide_memory::DocumentAuthority::Ask,
        document: document.into(),
        document_text: quote.into(),
        document_at_unix_ms: document_at,
        operator_text,
        operator_at_unix_ms: operator_at,
        source_session,
        document_sentence: None,
        replacement: None,
    }
}

fn original_words(
    store: &MemoryStore,
    node: &dyn NodeLink,
    project_id: &str,
    human: &hide_memory::MemorySource,
) -> Option<String> {
    source_event(
        store,
        node,
        project_id,
        &human.provider,
        &human.session_id,
        human.event_offset,
    )
    .filter(|event| event.kind == hide_session::EventKind::Human)
    .map(|event| {
        hide_memory::redact(&event.text)
            .text
            .chars()
            .take(hide_memory::COMPARISON_EXCERPT_BYTES / 4)
            .collect()
    })
}

/// The conversation event a card's source names, read again from the
/// session the node holds, or none when it cannot be read.
pub(super) fn source_event(
    store: &MemoryStore,
    node: &dyn NodeLink,
    project_id: &str,
    provider: &str,
    session_id: &str,
    event_offset: u64,
) -> Option<hide_session::ConversationEvent> {
    let source = store
        .list_session_sources(project_id)
        .ok()?
        .into_iter()
        .find(|source| source.id == session_id && source.provider == provider)?;
    let agent = hide_session::Agent::from_kind(&source.provider)?;
    let scope = hide_session::SessionReadScope {
        id: source.id,
        cwd: source.checkout_path,
    };
    let mut checkpoint = None;
    let mut consumed = 0u64;
    let deadline = std::time::Instant::now() + super::memory::SESSION_CALL_TIMEOUT;
    for _ in 0..hide_session::cursor::ARCHIVE_PAGE_LIMIT {
        let read: hide_session::ConversationRead = hide_node_link::link::call_as_reader(
            node,
            agent,
            hide_node_link::sessions::ReaderFeature::Conversation,
            hide_node_link::protocol::Call::SessionConversation {
                agent,
                path: source.locator.clone(),
                scope: scope.clone(),
                checkpoint: checkpoint.clone(),
            },
            deadline.checked_duration_since(std::time::Instant::now())?,
        )
        .ok()?;
        consumed = consumed.checked_add(read.read_bytes)?;
        if read.rescanned
            || consumed > hide_session::SESSION_READ_LIMIT_BYTES
            || read.events.len() != read.event_offsets.len()
        {
            return None;
        }
        let next = read.checkpoint.clone();
        let more = read.has_more;
        let found = read
            .events
            .into_iter()
            .zip(read.event_offsets)
            .find(|(_, offset)| *offset == event_offset)
            .map(|(event, _)| event);
        if found.is_some() {
            return found;
        }
        if !more
            || read.read_bytes == 0
            || checkpoint
                .as_ref()
                .is_some_and(|previous: &hide_session::ConversationCheckpoint| {
                    next.offset() <= previous.offset()
                })
        {
            return None;
        }
        checkpoint = Some(next);
    }
    None
}

/// The most rechecked cards whose sources one pass reads again, and the most
/// sources read for one card; one past it counts as unreadable, so the card
/// is kept.
const RECHECKS_PER_PASS: usize = 4;
const RECHECK_SOURCES: usize = 16;

/// Reads again the human sources of cards waiting on a recheck (PRD
/// memory-self-improve D-34, B27): a source that is really a Hide letter
/// stops standing as the operator's words, and a card whose every human
/// source was a letter is forgotten. No model is asked; the repository check
/// that follows is the existing one. Answers whether a card changed.
pub(super) fn read_recheck_sources(
    store: &mut MemoryStore,
    node: &dyn NodeLink,
    project_id: &str,
) -> Result<bool, String> {
    let waiting = store
        .rechecks_to_read(project_id)
        .map_err(|error| error.to_string())?;
    let mut changed = false;
    for (number, sources) in waiting.into_iter().take(RECHECKS_PER_PASS) {
        let mut verdicts = hide_memory::SourceVerdicts::default();
        for (index, source) in sources.into_iter().enumerate() {
            let event = (index < RECHECK_SOURCES)
                .then(|| {
                    source_event(
                        store,
                        node,
                        project_id,
                        &source.provider,
                        &source.session_id,
                        source.event_offset,
                    )
                })
                .flatten();
            match event {
                None => verdicts.unreadable.push(source),
                Some(event) if !event.from_operator() => verdicts.letters.push(source),
                Some(_) => {}
            }
        }
        let outcome = store
            .settle_recheck_sources(project_id, number, &verdicts, super::memory::now_ms())
            .map_err(|error| error.to_string())?;
        crate::diagnostic!(serde_json::json!({
            "component": "memory",
            "kind": "recheck.sources_read",
            "project_id": project_id,
            "card": number,
            "letters": verdicts.letters.len(),
            "unreadable": verdicts.unreadable.len(),
            "forgotten": outcome == hide_memory::SourcesOutcome::Forgotten,
        }));
        changed |= !verdicts.letters.is_empty();
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_retired_source_replays_only_its_proven_native_words() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("checkout");
        std::fs::create_dir(&cwd).unwrap();
        let id = "a1b2c3d4-0000-4000-8000-000000000001";
        let bucket = format!("{:x}", md5::compute(cwd.to_str().unwrap().as_bytes()));
        let path = temp
            .path()
            .join(".cursor/chats")
            .join(bucket)
            .join(id)
            .join("store.db");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let graph: serde_json::Value = serde_json::from_str(include_str!(
            "../../../hide-session/tests/fixtures/cursor-2026.10.01/graph.json"
        ))
        .unwrap();
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("PRAGMA user_version=1; CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT); CREATE TABLE blobs(id TEXT PRIMARY KEY,data BLOB);").unwrap();
        let metadata = json!({"agentId":id,"latestRootBlobId":graph["roots"]["first"],"createdAt":1_790_989_200_000_u64});
        db.execute(
            "INSERT INTO meta VALUES ('0',?1)",
            [hex::encode(metadata.to_string())],
        )
        .unwrap();
        for (id, value) in graph["blobs"].as_object().unwrap() {
            db.execute(
                "INSERT INTO blobs VALUES (?1,?2)",
                rusqlite::params![id, hex::decode(value.as_str().unwrap()).unwrap()],
            )
            .unwrap();
        }
        drop(db);
        std::fs::write(path.with_file_name("meta.json"), json!({"schemaVersion":1,"createdAtMs":1_790_989_200_000_u64,"hasConversation":true,"cwd":cwd}).to_string()).unwrap();
        let store = MemoryStore::open(&temp.path().join("memory.sqlite3")).unwrap();
        store.ensure_project("project", &cwd, "local").unwrap();
        let mut source = hide_memory::SessionSourceRecord {
            id: id.into(),
            project_id: "project".into(),
            provider: "cursor".into(),
            locator: path.to_str().unwrap().into(),
            checkout_path: cwd.to_str().unwrap().into(),
            started_at_unix_ms: None,
            updated_at_unix_ms: 1,
            unavailable_reason: None,
        };
        store.upsert_session_source(&source).unwrap();
        let human = hide_memory::MemorySource {
            provider: "cursor".into(),
            session_id: id.into(),
            event_offset: 0,
            content_hash: "retained".into(),
            available: true,
            human: true,
            at_unix_ms: None,
        };
        let node = hide_node::Local::new(Some(temp.path().to_path_buf()));
        let before = std::fs::read(&path).unwrap();
        assert_eq!(
            original_words(&store, &node, "project", &human).as_deref(),
            Some("Read this conversation")
        );
        source.checkout_path = temp
            .path()
            .join("another-checkout")
            .to_str()
            .unwrap()
            .into();
        store.upsert_session_source(&source).unwrap();
        assert!(original_words(&store, &node, "project", &human).is_none());
        source.checkout_path = cwd.to_str().unwrap().into();
        store.upsert_session_source(&source).unwrap();
        std::fs::remove_file(path.with_file_name("meta.json")).unwrap();
        assert!(original_words(&store, &node, "project", &human).is_none());
        assert_eq!(std::fs::read(path).unwrap(), before);
    }

    #[test]
    fn bounded_reverification_accepts_only_paths_and_quotes_actually_sent() {
        let temp = tempfile::tempdir().unwrap();
        let node = hide_node::Local::new(Some(temp.path().join("home")));
        let mut store = MemoryStore::open(&temp.path().join("memory.sqlite3")).unwrap();
        store
            .ensure_project("project", temp.path(), "local")
            .unwrap();
        store.set_enabled("project", true, true).unwrap();
        store
            .apply_candidates(
                &hide_memory::AnalysisBatch {
                    id: "fixture".into(),
                    project_id: "project".into(),
                    provider: "codex".into(),
                    analysis_provider: "codex".into(),
                    session_id: "fixture".into(),
                    content_hash: "fixture".into(),
                    created_at_unix_ms: 1,
                },
                &[hide_memory::Candidate {
                    title: "Run tests".into(),
                    text: "Run tests".into(),
                    aliases: Vec::new(),
                    kind: hide_memory::CandidateKind::Rule,
                    confidence: 1.0,
                    salience: 1.0,
                    source_offsets: vec![1],
                    human_offsets: vec![1],
                    source_times: Default::default(),
                    source_words: Default::default(),
                    cited_paths: Vec::new(),
                    relation: hide_memory::CandidateRelation::New,
                }],
            )
            .unwrap();
        let card = store.item_by_number("project", 1).unwrap().unwrap();
        let pending = PendingCard {
            item_id: card.id,
            number: 1,
            revision: 1,
            kind: card.kind,
            title: card.title,
            body: card.body,
            aliases: Vec::new(),
            cited_paths: Vec::new(),
            evidence: Vec::new(),
            outdated_reason: None,
        };
        let repository = (0..5)
            .map(|number| Source {
                path: format!("docs/guide-{number}.md"),
                text: format!("{}unseen quote", "x".repeat(9000)),
                truncated: false,
            })
            .collect::<Vec<_>>();
        let mut files = (0..5)
            .map(|number| Excerpt {
                path: format!("src/file-{number}.rs"),
                text: "x".repeat(FILE_EXCERPT_BYTES),
                truncated: false,
            })
            .collect::<Vec<_>>();
        files.extend(excerpts(&repository, &[], DOCUMENT_EXCERPT_BYTES));
        let request = verify_request(
            "fixture",
            "project",
            &pending,
            &[],
            &files,
            hide_ai::OutputLanguage::English,
        )
        .unwrap();
        let sent = sent_documents(&request, "files", &repository);
        assert!(!sent.is_empty());
        assert!(
            sent.len() < repository.len(),
            "the full input cap omits trailing documents"
        );
        assert!(
            !sent
                .iter()
                .any(|document| document.path == "docs/guide-4.md")
        );
        assert!(
            !apply_disagreement(
                &mut store,
                &node,
                "project",
                1,
                1,
                "fixture",
                &sent,
                "docs/guide-4.md",
                "x"
            )
            .unwrap()
        );
        assert!(
            !apply_disagreement(
                &mut store,
                &node,
                "project",
                1,
                1,
                "fixture",
                &sent,
                &sent[0].path,
                "unseen quote"
            )
            .unwrap()
        );
        assert!(store.document_comparison("project", 1).unwrap().is_none());
        assert!(
            apply_disagreement(
                &mut store,
                &node,
                "project",
                1,
                1,
                "fixture",
                &sent,
                &sent[0].path,
                "xxxxx"
            )
            .unwrap()
        );
        assert!(store.document_comparison("project", 1).unwrap().is_some());
    }
}
