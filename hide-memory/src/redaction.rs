use regex::Regex;
use std::sync::OnceLock;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RedactedText {
    pub text: String,
    pub redactions: usize,
    pub contains_secret_candidate: bool,
}

fn patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            r#"(?i)\b(?:api[_-]?key|access[_-]?token|client[_-]?secret|password)\s*[:=]\s*['"]?[^\s'"]{8,}"#,
            r"\bsk-[A-Za-z0-9_-]{16,}\b",
            r"\b(?:ghp|github_pat)_[A-Za-z0-9_]{20,}\b",
            r"(?s)-----BEGIN (?:RSA |EC |OPENSSH |ENCRYPTED )?PRIVATE KEY-----.*?-----END (?:RSA |EC |OPENSSH |ENCRYPTED )?PRIVATE KEY-----",
            r"\bAKIA[0-9A-Z]{16}\b",
            r"(?i)\bBearer\s+[A-Za-z0-9._~+/=-]{16,}\b",
        ]
        .into_iter()
        .map(|pattern| Regex::new(pattern).expect("static credential pattern"))
        .collect()
    })
}

/// Whether `character` changes how text reads without being seen: a
/// zero-width, joiner, direction, variation or tag character. Text an agent
/// will be given as someone's words keeps none of them, so what a person
/// read is what the agent gets.
pub fn is_invisible(character: char) -> bool {
    matches!(character, '\u{00AD}' | '\u{034F}' | '\u{061C}' | '\u{180E}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{206F}' | '\u{FE00}'..='\u{FE0F}' | '\u{FEFF}' | '\u{E0000}'..='\u{E007F}')
}

pub fn redact(input: &str) -> RedactedText {
    let mut text = input.to_owned();
    let mut redactions = 0;
    for pattern in patterns() {
        let count = pattern.find_iter(&text).count();
        if count > 0 {
            redactions += count;
            text = pattern.replace_all(&text, "[REDACTED]").into_owned();
        }
    }
    RedactedText {
        text,
        redactions,
        contains_secret_candidate: redactions > 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_credentials_are_removed_before_any_caller_can_store_them() {
        let source = "Use api_key=super-secret-value-123 and Bearer abcdefghijklmnopqrstuvwxyz";
        let result = redact(source);
        assert_eq!(result.redactions, 2);
        assert!(result.contains_secret_candidate);
        assert!(!result.text.contains("super-secret"));
        assert!(!result.text.contains("abcdefghijkl"));
    }

    #[test]
    fn an_entire_multiline_private_key_is_removed_in_plain_and_json_text() {
        for source in [
            "before\n-----BEGIN PRIVATE KEY-----\nYWJjZGVmZ2hpams=\n-----END PRIVATE KEY-----\nafter",
            r#"{\"text\":\"before\\n-----BEGIN OPENSSH PRIVATE KEY-----\\nYWJjZGVmZ2hpams=\\n-----END OPENSSH PRIVATE KEY-----\\nafter\"}"#,
        ] {
            let result = redact(source);
            assert!(result.contains_secret_candidate);
            assert!(!result.text.contains("YWJjZGVmZ2hpams"));
            assert!(!result.text.contains("END PRIVATE KEY"));
            assert!(!result.text.contains("END OPENSSH PRIVATE KEY"));
        }
    }

    #[test]
    fn ordinary_project_text_is_unchanged() {
        let source = "Keep one SQLite writer and fail open in the hook.";
        assert_eq!(
            redact(source),
            RedactedText {
                text: source.to_owned(),
                redactions: 0,
                contains_secret_candidate: false
            }
        );
    }
}
