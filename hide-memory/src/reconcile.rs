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
    /// The one sentence of the document the operator's words contradict,
    /// copied from the excerpt the comparison was given; absent until a
    /// comparison answer carried it (older rows never have it).
    #[serde(default)]
    pub document_sentence: Option<String>,
    /// The sentence that replaces `document_sentence`; set and cleared with it.
    #[serde(default)]
    pub replacement: Option<String>,
}

pub const COMPARISON_EXCERPT_BYTES: usize = 3 * 1024;
pub const PROMOTION_BATCH_LIMIT: usize = 5;
/// The longest sentence a comparison keeps, document side or replacement.
pub const COMPARISON_SENTENCE_CHARS: usize = 400;

/// The document sentence and its replacement from one comparison answer, or
/// none unless both are present, within the cap, and the document sentence
/// occurs word for word (whitespace aside) in the excerpt that was sent.
pub fn accepted_sentence_pair(
    excerpt: &str,
    document_sentence: &str,
    replacement: &str,
) -> Option<(String, String)> {
    let squash = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
    let document = squash(document_sentence);
    let replacement = squash(&crate::redact(replacement).text);
    let within = |text: &str| (1..=COMPARISON_SENTENCE_CHARS).contains(&text.chars().count());
    (within(&document) && within(&replacement) && squash(excerpt).contains(&document))
        .then_some((document, replacement))
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXCERPT: &str = "## Tests\nRun the suite with\n  the ordinary runner.\nKeep it fast.";

    #[test]
    fn a_sentence_inside_the_excerpt_is_accepted_with_its_whitespace_squashed() {
        assert_eq!(
            accepted_sentence_pair(EXCERPT, "Run the suite with the ordinary runner.", " Use  the repository script. "),
            Some((
                "Run the suite with the ordinary runner.".into(),
                "Use the repository script.".into()
            ))
        );
    }

    #[test]
    fn a_sentence_outside_the_excerpt_or_past_the_cap_is_dropped() {
        assert_eq!(accepted_sentence_pair(EXCERPT, "Run it nightly.", "Use the script."), None);
        assert_eq!(accepted_sentence_pair(EXCERPT, "Keep it fast.", ""), None);
        assert_eq!(accepted_sentence_pair(EXCERPT, "", "Use the script."), None);
        let long = "x".repeat(COMPARISON_SENTENCE_CHARS + 1);
        assert_eq!(accepted_sentence_pair(&long, &long, "Use the script."), None);
    }

    #[test]
    fn a_comparison_stored_before_the_pair_existed_still_reads() {
        let old = r#"{"authority":"card_newer","document":"AGENTS.md","document_text":"a","document_at_unix_ms":null,"operator_text":"b","operator_at_unix_ms":1,"source_session":null}"#;
        let read: DocumentComparison = serde_json::from_str(old).unwrap();
        assert_eq!((read.document_sentence, read.replacement), (None, None));
    }
}
