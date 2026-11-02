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
            // A name may end an environment variable's (`DB_PASSWORD=`), so
            // it starts at a word boundary or after `_`.
            r#"(?i)(?:\b|_)(?:api[_-]?key|access[_-]?token|auth[_-]?token|client[_-]?secret|secret[_-]?access[_-]?key|private[_-]?key|password|passwd)\s*[:=]\s*['"]?[^\s'"]{8,}"#,
            // The password in an address's user part (`postgres://app:pw@db`).
            r"(?i)\b[a-z][a-z0-9+.-]*://[^\s:/@]+:[^\s@/]+@",
            r"\bsk-[A-Za-z0-9_-]{16,}\b",
            r"\b(?:ghp|gho|ghs|ghu|ghr|github_pat)_[A-Za-z0-9_]{20,}\b",
            r"\b(?:xox[abprs]-[A-Za-z0-9-]{10,}|npm_[A-Za-z0-9]{20,}|glpat-[A-Za-z0-9_-]{20,})",
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
    fn credentials_named_like_environment_variables_and_tokens_are_removed() {
        for (source, secret) in [
            ("DB_PASSWORD=hunter2hunter2", "hunter2hunter2"),
            ("export STRIPE_API_KEY=abcd1234efgh5678", "abcd1234efgh5678"),
            ("GITHUB_ACCESS_TOKEN=abcdefgh1234", "abcdefgh1234"),
            (
                "AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMIK7MDENG",
                "wJalrXUtnFEMIK7MDENG",
            ),
            ("PRIVATE_KEY=abcdefgh12345678", "abcdefgh12345678"),
            ("DATABASE_URL=postgres://app:pw1234@db:5432/x", "pw1234"),
            ("slack xoxb-1234567890-abcdefghij", "xoxb-1234567890"),
            ("gho_abcdefghijklmnopqrstuvwxyz12", "gho_abcdefghij"),
            ("npm_abcdefghijklmnopqrstuvwxyz12", "npm_abcdefghij"),
        ] {
            let result = redact(source);
            assert!(!result.text.contains(secret), "{source} -> {}", result.text);
        }
        // Code that names a field is not a credential.
        assert_eq!(redact("pub token: String,").text, "pub token: String,");
    }

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
