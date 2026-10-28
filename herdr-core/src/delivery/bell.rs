//! The line the doorbell submits through Herdr's `agent.prompt`: who wrote, what kind
//! of letter it is and the letter's first line, so the operator watching the
//! pane can tell what arrived (`docs/delivery.md`, The bell line). Its words
//! are in the operator's interface language; a letter's turn is matched by
//! the whole line its ledger kept. Its parts are `hide_session::bell_line`'s,
//! whose transcript readers tell the line from the operator's own words by
//! them.
//!
//! The line reaches an agent's composer through that API, so it is one line
//! and carries nothing a composer acts on while it is received: no
//! control or invisible character, and none of the characters that open a
//! picker or continue the line in the runtimes the doorbell rings.

use crate::model::InterfaceLanguage;
use hide_session::bell_line::{
    BELL_CUT, BELL_DOT, BELL_DOT_STAND_IN, BELL_ENGLISH, BELL_INBOX, BELL_JAPANESE, BELL_KOREAN,
    BELL_MARK, BELL_NAME_CHARS, BELL_SEPARATOR, BELL_SIMPLIFIED_CHINESE, BELL_SUMMARY_CHARS,
    BellWords,
};

/// The longest line a letter may keep, in bytes. A line `line` makes is far
/// shorter; a ledger holding a longer one was not written by this code.
const LINE_LIMIT: usize = 512;

/// What one bell says. `sender` is the name the operator knows the writer
/// by, `others` the letters waiting for the same recipient besides this one,
/// `inbox` whether the line names where the letter is, for a recipient
/// whose prompt can arrive without its hook having run, and `language` the
/// operator's interface language the line is worded in.
pub(crate) struct Ring<'a> {
    pub sender: &'a str,
    pub kind: &'a str,
    pub body: &'a str,
    pub others: usize,
    pub inbox: bool,
    pub language: InterfaceLanguage,
}

/// `🦉 <sender> <kind>: <first line of the letter>`, then ` · and N more`
/// (` · 외 N통`) when other letters wait for the same recipient and
/// ` · hide inbox` when the ring asks for it.
pub(crate) fn line(ring: &Ring) -> String {
    let words = words(ring.language);
    // A sender with nothing left to show is still a name, so the line keeps
    // the shape its readers know it by.
    let sender = Some(inert(ring.sender, BELL_NAME_CHARS))
        .filter(|sender| !sender.is_empty())
        .unwrap_or_else(|| BELL_CUT.to_string());
    let mut line = format!("{BELL_MARK} {sender} {}", words.kind(ring.kind));
    let summary = ring
        .body
        .lines()
        .map(|line| inert(line, BELL_SUMMARY_CHARS))
        .find(|line| !line.is_empty());
    if let Some(summary) = summary {
        line.push_str(": ");
        line.push_str(&summary);
    }
    if ring.others > 0 {
        line.push_str(BELL_SEPARATOR);
        line.push_str(&words.more(ring.others));
    }
    if ring.inbox {
        line.push_str(BELL_INBOX);
    }
    line
}

/// Whether a bell for an agent of `kind` names `hide inbox`. Codex runs a
/// user-level hook only once its own trust review approved that exact entry,
/// so another tool's hook change silently stops Hide's, and a Codex attached
/// to its shared app-server daemon runs hooks in the daemon's environment
/// rather than the pane's (`docs/agent-hooks.md`); either way its bell turn
/// arrives with neither the letter nor the session guidance that says what
/// the line means. Claude Code runs every user-level hook it is given.
pub(crate) fn names_inbox(kind: &str) -> bool {
    hide_agent_adapter::adapter(kind).is_some_and(|row| row.bell_names_inbox)
}

/// Whether a letter may keep `line` as its bell.
pub(crate) fn valid_line(line: &str) -> bool {
    !line.is_empty() && line.len() <= LINE_LIMIT && !line.chars().any(char::is_control)
}

/// The line's words in `language`.
fn words(language: InterfaceLanguage) -> &'static BellWords {
    match language {
        InterfaceLanguage::English => &BELL_ENGLISH,
        InterfaceLanguage::Korean => &BELL_KOREAN,
        InterfaceLanguage::SimplifiedChinese => &BELL_SIMPLIFIED_CHINESE,
        InterfaceLanguage::Japanese => &BELL_JAPANESE,
    }
}

/// `text` as one line of at most `max` characters, cut with `…`, with the
/// characters a composer acts on replaced by their full-width forms, which
/// read the same and do nothing (`BELL_INERT`). Characters above U+FFFF are
/// left out before the spaces are folded, since Codex CLI on Windows drops
/// them as it receives the line and its transcript then holds this same
/// line without the owl; and `·` becomes `∙`, so the line's own ` · ` only
/// ever opens one of its tails.
fn inert(text: &str, max: usize) -> String {
    let basic: String = text.chars().filter(|c| u32::from(*c) <= 0xFFFF).collect();
    let Some(line) = crate::display_text::one_line(&basic, max + 1) else {
        return String::new();
    };
    let mut kept: String = line
        .chars()
        .take(max)
        .map(|c| match c {
            // A file picker in both runtimes, which Enter would accept.
            '@' => '＠',
            // Codex's skill picker.
            '$' => '＄',
            // A backslash before Enter continues the line in Claude Code.
            '\\' => '＼',
            BELL_DOT => BELL_DOT_STAND_IN,
            c => c,
        })
        .collect();
    if line.chars().count() > max {
        kept = kept.trim_end().to_owned();
        kept.push(BELL_CUT);
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A line the bell types, which the agent live check
    /// (`scripts/agent_live_check`) types into each scene in place of a
    /// letter's own: a cut summary carrying characters a composer acts on.
    const SAMPLE: &str = "🦉 live-check 보고: ＠src/main.rs ＄HOME /review !ls #881 `cargo test` 50% 끝나면 진행… · 외 1통";

    #[test]
    fn the_live_checks_sample_is_a_line_the_bell_types() {
        assert_eq!(
            ring(
                "live-check",
                "report",
                "@src/main.rs $HOME /review !ls #881 `cargo test` 50% 끝나면 진행 상황을 짧게 알려 주세요",
                1
            ),
            SAMPLE
        );
    }

    fn ring<'a>(sender: &'a str, kind: &'a str, body: &'a str, others: usize) -> String {
        line(&Ring {
            sender,
            kind,
            body,
            others,
            inbox: false,
            language: InterfaceLanguage::Korean,
        })
    }

    #[test]
    fn the_line_keeps_its_shape_in_every_interface_language() {
        let worded = |language| {
            line(&Ring {
                sender: "ci-lead",
                kind: "report",
                body: "PR #881",
                others: 2,
                inbox: false,
                language,
            })
        };
        assert_eq!(
            worded(InterfaceLanguage::English),
            "🦉 ci-lead report: PR #881 · and 2 more"
        );
        assert_eq!(
            worded(InterfaceLanguage::Korean),
            "🦉 ci-lead 보고: PR #881 · 외 2통"
        );
        assert_eq!(
            worded(InterfaceLanguage::SimplifiedChinese),
            "🦉 ci-lead 报告: PR #881 · 另有 2 封"
        );
        assert_eq!(
            worded(InterfaceLanguage::Japanese),
            "🦉 ci-lead 報告: PR #881 · ほか 2 通"
        );
        assert_eq!(
            words(InterfaceLanguage::English).kind("a_newer_kind"),
            "letter"
        );
    }

    #[test]
    fn a_codex_bell_names_where_the_letter_is_and_a_claude_code_bell_does_not() {
        assert!(names_inbox("codex"));
        assert!(!names_inbox("claude"));
        assert!(!names_inbox("claude-code"));
        assert_eq!(
            line(&Ring {
                sender: "label-end-fix",
                kind: "report",
                body: "PR #881을 열었습니다",
                others: 1,
                inbox: true,
                language: InterfaceLanguage::Korean,
            }),
            "🦉 label-end-fix 보고: PR #881을 열었습니다 · 외 1통 · hide inbox"
        );
    }

    #[test]
    fn the_line_names_the_sender_the_kind_and_the_first_line_of_the_letter() {
        // The two lines the operator approved.
        assert_eq!(
            ring(
                "label-end-fix",
                "report",
                "label end 수정 PR #881을 열었습니다\n\n변경: ...",
                0
            ),
            "🦉 label-end-fix 보고: label end 수정 PR #881을 열었습니다"
        );
        assert_eq!(
            ring(
                "hierarchy-pen-scratch",
                "report",
                "\n  Pen 시안이 준비됐습니다\n",
                1
            ),
            "🦉 hierarchy-pen-scratch 보고: Pen 시안이 준비됐습니다 · 외 1통"
        );
        assert_eq!(ring("a", "request", "x", 0), "🦉 a 요청: x");
        assert_eq!(ring("a", "block", "x", 0), "🦉 a 막힘: x");
        assert_eq!(ring("a", "reply", "x", 0), "🦉 a 답장: x");
        assert_eq!(ring("a", "watch", "x", 2), "🦉 a 감시: x · 외 2통");
    }

    #[test]
    fn every_line_the_bell_types_reads_as_a_letter_and_not_the_operators_words() {
        let languages = [
            InterfaceLanguage::English,
            InterfaceLanguage::Korean,
            InterfaceLanguage::SimplifiedChinese,
            InterfaceLanguage::Japanese,
        ];
        let long_name = "n".repeat(40);
        let long_body = "가 나".repeat(30);
        let senders = [
            "Codex",
            "라벨 제목",
            long_name.as_str(),
            "a@b $c",
            "🐈",
            "a · b",
        ];
        let bodies = [
            "레이어 2도 1.0.1에 포함🐈\nmore",
            "",
            long_body.as_str(),
            "@src/main.rs $HOME C:\\ 보고: x · 외 2통",
            "Read 🐈 the letter",
            "🐈",
            "· hide inbox",
            "· and 2 more",
        ];
        for language in languages {
            for kind in [
                "request", "block", "report", "progress", "reply", "watch", "new",
            ] {
                for (sender, body) in senders.iter().cycle().zip(bodies.iter()) {
                    for (others, inbox) in [(0, false), (2, false), (0, true), (11, true)] {
                        let typed = line(&Ring {
                            sender,
                            kind,
                            body,
                            others,
                            inbox,
                            language,
                        });
                        assert!(hide_session::is_bell_line(&typed), "{typed}");
                        assert!(hide_session::is_letter(&typed), "{typed}");
                        if inbox {
                            // Codex CLI on Windows drops what is above U+FFFF.
                            let received: String =
                                typed.chars().filter(|c| u32::from(*c) <= 0xFFFF).collect();
                            assert!(hide_session::is_bell_line(&received), "{received}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_bell_with_a_non_bmp_summary_is_recognized_when_console_input_drops_it() {
        let bell = ring("sender", "report", "Read🐈the letter", 0);
        assert_eq!(bell, "🦉 sender 보고: Readthe letter");
        // SHA-256 of the received "sender 보고: Readthe letter", independently
        // calculated after the leading owl was lost.
        assert!(hide_agent_hooks::delivery::matches_prompt_digest(
            &bell,
            "25b30086ee8c322631a41392425753beb05ed84f46d8aeadc5a682ddea42018b"
        ));
    }

    #[test]
    fn a_long_first_line_is_cut_at_sixty_characters() {
        let body = "가".repeat(61);
        assert_eq!(
            ring("a", "report", &body, 0),
            format!("🦉 a 보고: {}…", "가".repeat(60))
        );
        assert_eq!(
            ring("a", "report", &"가".repeat(60), 0),
            format!("🦉 a 보고: {}", "가".repeat(60))
        );
        assert_eq!(
            ring(&"n".repeat(40), "report", "x", 0),
            format!("🦉 {}… 보고: x", "n".repeat(32))
        );
    }

    #[test]
    fn the_line_carries_nothing_a_composer_acts_on() {
        let typed = ring(
            "ci\u{202E}-lead\u{200B}",
            "report",
            "\u{1b}[2J\tfix @src/main.rs, $skill and C:\\\\\r\nsecond",
            0,
        );
        assert_eq!(
            typed,
            "🦉 ci-lead 보고: [2J fix ＠src/main.rs, ＄skill and C:＼＼"
        );
        assert!(valid_line(&typed));
        // A body with no visible first line still names who rang.
        assert_eq!(ring("a", "report", "\u{200B}\n\u{3164}", 0), "🦉 a 보고");
        assert_eq!(ring("a", "progress", "plan", 0), "🦉 a 진행: plan");
        assert!(!valid_line("🦉 a\n보고"));
        assert!(!valid_line(""));
        assert!(!valid_line(&"x".repeat(LINE_LIMIT + 1)));
    }
}
