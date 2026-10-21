//! Revision-bound authority decisions for a card and one repository document.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentAuthority {
    Outdated,
    CardNewer,
    Ask,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DocumentComparison {
    pub authority: DocumentAuthority,
    pub document: String,
    pub document_text: String,
    pub document_at_unix_ms: Option<u64>,
    pub operator_text: String,
    pub operator_at_unix_ms: Option<u64>,
    pub source_session: Option<String>,
}

pub const COMPARISON_EXCERPT_BYTES: usize = 3 * 1024;
pub const PROMOTION_BATCH_LIMIT: usize = 5;
