use regex::{Regex, RegexBuilder};
use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RedactedText {
    pub text: String,
    pub redactions: usize,
    pub contains_secret_candidate: bool,
}

#[derive(Deserialize)]
struct Rule {
    pattern: String,
    flags: String,
    #[serde(default)]
    diagnostic_pattern: Option<String>,
    #[serde(default)]
    replacement: String,
}

struct Policy {
    secrets: Vec<Regex>,
    diagnostic_secrets: Vec<Regex>,
    secret_field: Regex,
    diagnostic: Vec<(Regex, String)>,
}

fn policy() -> &'static Policy {
    static POLICY: OnceLock<Policy> = OnceLock::new();
    POLICY.get_or_init(|| {
        #[derive(Deserialize)]
        struct Source {
            secrets: Vec<Rule>,
            secret_field: Rule,
            diagnostic: Vec<Rule>,
        }
        let source: Source =
            serde_json::from_str(include_str!("../../contracts/diagnostic-redaction.json"))
                .expect("static redaction policy");
        let compile = |pattern: &str, flags: &str| {
            RegexBuilder::new(pattern)
                .case_insensitive(flags.contains('i'))
                .dot_matches_new_line(flags.contains('s'))
                .build()
                .expect("static redaction pattern")
        };
        Policy {
            secrets: source
                .secrets
                .iter()
                .map(|rule| compile(&rule.pattern, &rule.flags))
                .collect(),
            diagnostic_secrets: source
                .secrets
                .iter()
                .map(|rule| {
                    compile(
                        rule.diagnostic_pattern.as_deref().unwrap_or(&rule.pattern),
                        &rule.flags,
                    )
                })
                .collect(),
            secret_field: compile(&source.secret_field.pattern, &source.secret_field.flags),
            diagnostic: source
                .diagnostic
                .into_iter()
                .map(|rule| (compile(&rule.pattern, &rule.flags), rule.replacement))
                .collect(),
        }
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
    for pattern in &policy().secrets {
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

/// A field whose name supplies credential context for an otherwise opaque value.
pub fn is_secret_field(name: &str) -> bool {
    policy().secret_field.is_match(name)
}

/// Text leaving the process through diagnostic logs or Copy diagnostics.
/// The shell and desktop host consume the same policy. Memory keeps its
/// credential-only policy, so paths in project knowledge stay useful.
pub fn redact_diagnostic(input: &str) -> String {
    let mut text = input.to_owned();
    for pattern in &policy().diagnostic_secrets {
        text = pattern.replace_all(&text, "[redacted]").into_owned();
    }
    for (pattern, replacement) in &policy().diagnostic {
        text = pattern
            .replace_all(&text, replacement.as_str())
            .into_owned();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_follow_the_same_privacy_contract_as_the_shell() {
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../../contracts/fixtures/diagnostic-redaction.json"
        ))
        .unwrap();
        for case in cases.as_array().unwrap() {
            assert_eq!(
                redact_diagnostic(&case["input"].as_str().map(str::to_owned).unwrap_or_else(
                    || {
                        case["input"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|part| part.as_str().unwrap())
                            .collect::<String>()
                    }
                )),
                case["expected"].as_str().unwrap(),
                "{}",
                case["name"]
            );
        }
        // Project knowledge retains paths and addresses: the extra privacy
        // rules apply to diagnostic exports, not Memory's content.
        assert_eq!(
            redact("/home/example/repo someone@example.invalid").text,
            "/home/example/repo someone@example.invalid"
        );
    }

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
            ("JWT_SECRET=supersecretvalue1", "supersecretvalue1"),
            ("SECRET_KEY=abcdef123456", "abcdef123456"),
            ("NPM_TOKEN=abcdef123456789", "abcdef123456789"),
            ("export TOKEN=abcdef123456", "abcdef123456"),
            (r#"{"password":"hunter2hunter2"}"#, "hunter2hunter2"),
            (r#""DB_PASSWORD": "hunter2hunter2""#, "hunter2hunter2"),
        ] {
            let result = redact(source);
            assert!(!result.text.contains(secret), "{source} -> {}", result.text);
        }
        // Code that names a field, and prose, are not credentials.
        for ordinary in [
            "pub token: String,",
            "see https://example.com:8080/path and git@github.com:org/repo.git",
            "the secret is to keep it short",
        ] {
            assert_eq!(redact(ordinary).text, ordinary);
        }
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
