//! Project Memory domain boundary.
//!
//! Hide owns extraction, relation planning, durable SQLite rows, provenance,
//! lifecycle, query-dependent retrieval, token budgets, and the read-only hook
//! projection.

mod budget;
mod engine;
mod evidence;
pub mod hook;
mod matcher;
mod reconcile;
mod redaction;
mod store;
mod verify;

pub use budget::{MemoryCallDeferral, MemoryCallKind, memory_call_reset_at};
pub use engine::{
    AliasBackfill, AnalysisOutput, HideNativeAnalyzer, HideNativeOutputError,
    SYSTEM_PROMPT_LIMIT_BYTES,
};
pub use evidence::{
    EVIDENCE_ROW_LIMIT, Evidence, EvidenceFact, EvidenceKind, NAME_CANDIDATE_LIMIT, WrittenNames,
    still_holds, written_names,
};
pub use reconcile::{
    COMPARISON_EXCERPT_BYTES, DocumentAuthority, DocumentComparison, PROMOTION_BATCH_LIMIT,
};
pub use redaction::{RedactedText, redact};
pub use store::hold::{
    CardHold, EndedHold, ReleasedReport, StaleOutcome, StaleRefusal, StaleReporter,
};
pub use store::{
    AnalysisBatch, ApplyOptions, ApplySummary, Candidate, CandidateKind, CandidateOutcome,
    CandidateRelation, CardEvidence, ComparisonCard, Conversion, CorrectionCheck, CursorReader,
    DeleteProjectOutcome, InjectedItem, Injection, InjectionKind, InjectionOutcome, InputFailure,
    MemoryDetail, MemoryError, MemoryItem, MemoryLifecycle, MemorySource, MemoryStore, PendingCard,
    PreviousRevision, ProjectMemoryState, Promotion, PromotionState, PromotionSuggestion,
    ReceiptRecord, RetrievalQuery, SessionCursorRecord, SessionSourceRecord, StoreMode,
    branch_name, card_name, parse_card_name, repository_path, words_digest,
};
pub use verify::{
    Correction, DOCUMENT_EXCERPT_BYTES, DOCUMENT_LIMIT, EXCERPT_SOURCE_BYTES, Excerpt,
    FILE_EXCERPT_BYTES, FILE_EXCERPT_LIMIT, GLOBAL_EXCERPT_BYTES, GLOBAL_LIMIT,
    PROMOTE_CHECK_FEATURE, PromoteCheck, PromoteCheckCard, VERIFY_FEATURE, Verification,
    VerifyError, cited_files, creation_docs_request, parse_promote_check, parse_verification,
    pointer_target, promote_check_request, query_words, relevant_excerpt, sensitive_path,
    verify_request,
};

/// The Memory store's file name inside a state folder.
pub const DATABASE_FILE: &str = "project-memory.sqlite3";

/// The one place the store's location is spelled, in the core's state
/// folder; the core alone opens it, and a hook asks the core ([`hook`]).
pub fn database_path(state_dir: &std::path::Path) -> std::path::PathBuf {
    state_dir.join(DATABASE_FILE)
}

pub const ACTIVE_MEMORY_LIMIT: usize = 10_000;
/// The most cards a full Project keeps forgotten for new ones, for Undo;
/// the oldest past it are removed for good.
pub const EVICTED_CARD_LIMIT: usize = 1_000;
/// The source provider of a card the operator added by hand
/// (`hide memory add`), which a full Project never forgets for a new one.
pub const HAND_ADDED_PROVIDER: &str = "hide";
/// Unusable answers in a row one unchanged input gets before it is set
/// aside: a session group is passed over and a correction's document version
/// is not asked about again, each with a diagnostic, until that input changes.
pub const INPUT_FAILURE_LIMIT: u32 = 3;
/// Requests in a row the provider took for one unchanged input and never
/// answered before it is set aside the same way. One counts only when the
/// provider answered the Project since the last, so an outage, which the
/// provider's backoff waits out, sets nothing aside.
pub const INPUT_UNANSWERED_LIMIT: u32 = 5;
/// Failures further apart than this are not in a row.
pub const INPUT_FAILURE_WINDOW_MS: u64 = 24 * 60 * 60 * 1000;
pub const MEMORY_BODY_LIMIT_CHARS: usize = 4_000;
pub const ANALYSIS_INPUT_LIMIT_BYTES: usize = 64 * 1024;
pub const HOOK_INPUT_LIMIT_BYTES: usize = 256 * 1024;
pub const HOOK_CANDIDATE_LIMIT: usize = 60;
/// Index lines a session start gets (D-04).
pub const INDEX_LINE_LIMIT: usize = 20;
/// Lines `hide memory search` answers with (B15).
pub const SEARCH_RESULT_LIMIT: usize = 10;
/// Cards one analysis compares new text against (D-13).
pub const COMPARISON_LIMIT: usize = 30;
pub const PROMPT_ITEM_LIMIT: usize = 3;
pub const INJECTION_TOKEN_LIMIT: usize = 600;
/// A card's index line, in characters (D-23: an old card's is its first 60).
pub const TITLE_LIMIT_CHARS: usize = 60;
pub const ALIAS_LIMIT: usize = 12;
pub const ALIAS_LIMIT_CHARS: usize = 40;
/// Files one card cites, each a repository-relative path (D-26).
pub const CITED_PATH_LIMIT: usize = 8;
pub const CITED_PATH_LIMIT_BYTES: usize = 240;
/// Receipt rows a Project keeps (B26); past it, receipts of sessions quiet
/// for [`RECEIPT_STALE_AFTER_MS`] go first, and a receipt that still does not
/// fit is not recorded.
pub const RECEIPT_LIMIT: usize = 20_000;
pub const RECEIPT_STALE_AFTER_MS: u64 = 30 * 86_400_000;
/// Analysis calls one machine makes in a UTC day (D-10 a).
pub const DAILY_ANALYSIS_CALL_LIMIT: u64 = 1_000;
/// The most one retrieval spends in the store, inside the hook's wait.
pub const HOOK_DEADLINE_MS: u64 = 100;
/// The bar a rule or lesson card crosses to be suggested for a repository
/// file (PRD memory-card-quality D-09, D-10): the operator's words in this
/// many different sessions, or reads from this many different sessions
/// within the window, with its evidence unchanged since it was last verified.
pub const PROMOTION_HUMAN_SESSIONS: u64 = 2;
pub const PROMOTION_PULLED_SESSIONS: u64 = 3;
pub const PROMOTION_WINDOW_MS: u64 = 14 * 86_400_000;
/// The most suggestions open at once (B29).
pub const PROMOTION_OPEN_LIMIT: usize = 3;
/// Significant new cards one extraction group may admit. Later groups have
/// their own opportunity; corrections do not consume this allowance.
pub const NEW_CARDS_PER_GROUP: usize = 8;
/// Extraction groups one session may process in a background pass.
pub const ANALYSIS_GROUP_QUANTUM: usize = 4;
/// The most characters of the reason an agent gives when it calls a card
/// outdated.
pub const OUTDATED_REASON_LIMIT_CHARS: usize = 500;
/// A card is verified again at most once in this long (D-07, B10).
pub const REVERIFY_INTERVAL_MS: u64 = 86_400_000;
/// Cards one pass verifies again, and cards one pass checks before a
/// suggestion (D-11): each is one provider call under the daily cap.
pub const REVERIFY_PER_PASS: usize = 4;
/// The bounded share of the day's calls for existing-card checks. Due
/// re-verifications and promotion checks reserve their remaining share from
/// analysis; without due checks, analysis may use the whole remaining total.
pub const MAINTENANCE_CALL_LIMIT: u64 = DAILY_ANALYSIS_CALL_LIMIT / 2;
pub const PROMOTION_CHECKS_PER_PASS: usize = 3;
/// Cards one session may hold out of injection at once by reporting them
/// stale, and cards a Project may have held (PRD memory-stale-cards D-09).
pub const STALE_HOLDS_PER_SESSION: usize = 3;
pub const STALE_HOLDS_PER_PROJECT: usize = 10;
/// How long a hold lasts when the reporting session is not analyzed (D-07).
pub const STALE_HOLD_MS: u64 = 7 * 86_400_000;
/// The most characters kept of the reason an agent reports a card stale
/// with, after the redactor masked it (D-08).
pub const STALE_REASON_LIMIT_CHARS: usize = 500;
