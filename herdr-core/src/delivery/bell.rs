//! The line the doorbell types into an idle agent pane: who wrote, what kind
//! of letter it is and the letter's first line, so the operator watching the
//! pane can tell what arrived (`docs/delivery.md`, The bell line).
//!
//! The line is typed into an agent's composer and submitted with Enter, so it
//! is one line and carries nothing a composer acts on while it is typed: no
//! control or invisible character, and none of the characters that open a
//! picker or continue the line in the runtimes the doorbell rings.

/// The longest sender name and letter summary the line carries, in
/// characters; a longer one is cut and ends in `…`.
const NAME_CHARS: usize = 32;
const SUMMARY_CHARS: usize = 60;

/// The longest line a letter may keep, in bytes. A line `line` makes is far
/// shorter; a ledger holding a longer one was not written by this code.
const LINE_LIMIT: usize = 512;

/// What one bell says. `sender` is the name the operator knows the writer
/// by, `others` the letters waiting for the same recipient besides this one,
/// and `inbox` whether the line names where the letter is, for a recipient
/// whose prompt can arrive without its hook having run.
pub(crate) struct Ring<'a> {
    pub sender: &'a str,
    pub kind: &'a str,
    pub body: &'a str,
    pub others: usize,
    pub inbox: bool,
}

/// `🔔 <sender> <kind>: <first line of the letter>`, then ` · 외 N통` when
/// other letters wait for the same recipient and ` · hide inbox` when the
/// ring asks for it.
pub(crate) fn line(ring: &Ring) -> String {
    let mut line = format!("🔔 {} {}", inert(ring.sender, NAME_CHARS), word(ring.kind));
    let summary = ring
        .body
        .lines()
        .map(|line| inert(line, SUMMARY_CHARS))
        .find(|line| !line.is_empty());
    if let Some(summary) = summary {
        line.push_str(": ");
        line.push_str(&summary);
    }
    if ring.others > 0 {
        line.push_str(&format!(" · 외 {}통", ring.others));
    }
    if ring.inbox {
        line.push_str(" · hide inbox");
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

fn word(kind: &str) -> &'static str {
    match kind {
        "request" => "요청",
        "block" => "막힘",
        "report" => "보고",
        "progress" => "진행",
        "reply" => "답장",
        "watch" => "감시",
        _ => "편지",
    }
}

/// `text` as one line of at most `max` characters, cut with `…`, with the
/// characters a composer acts on replaced by their full-width forms, which
/// read the same and do nothing.
fn inert(text: &str, max: usize) -> String {
    let Some(line) = crate::display_text::one_line(text, max + 1) else {
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
            c => c,
        })
        .collect();
    if line.chars().count() > max {
        kept = kept.trim_end().to_owned();
        kept.push('…');
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A line the bell types, which the agent live check
    /// (`scripts/agent_live_check`) types into each scene in place of a
    /// letter's own: a cut summary carrying characters a composer acts on.
    const SAMPLE: &str = "🔔 live-check 보고: ＠src/main.rs ＄HOME /review !ls #881 `cargo test` 50% 끝나면 진행… · 외 1통";

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
        })
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
            }),
            "🔔 label-end-fix 보고: PR #881을 열었습니다 · 외 1통 · hide inbox"
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
            "🔔 label-end-fix 보고: label end 수정 PR #881을 열었습니다"
        );
        assert_eq!(
            ring(
                "hierarchy-pen-scratch",
                "report",
                "\n  Pen 시안이 준비됐습니다\n",
                1
            ),
            "🔔 hierarchy-pen-scratch 보고: Pen 시안이 준비됐습니다 · 외 1통"
        );
        assert_eq!(ring("a", "request", "x", 0), "🔔 a 요청: x");
        assert_eq!(ring("a", "block", "x", 0), "🔔 a 막힘: x");
        assert_eq!(ring("a", "reply", "x", 0), "🔔 a 답장: x");
        assert_eq!(ring("a", "watch", "x", 2), "🔔 a 감시: x · 외 2통");
    }

    #[test]
    fn a_long_first_line_is_cut_at_sixty_characters() {
        let body = "가".repeat(61);
        assert_eq!(
            ring("a", "report", &body, 0),
            format!("🔔 a 보고: {}…", "가".repeat(60))
        );
        assert_eq!(
            ring("a", "report", &"가".repeat(60), 0),
            format!("🔔 a 보고: {}", "가".repeat(60))
        );
        assert_eq!(
            ring(&"n".repeat(40), "report", "x", 0),
            format!("🔔 {}… 보고: x", "n".repeat(32))
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
            "🔔 ci-lead 보고: [2J fix ＠src/main.rs, ＄skill and C:＼＼"
        );
        assert!(valid_line(&typed));
        // A body with no visible first line still names who rang.
        assert_eq!(ring("a", "report", "\u{200B}\n\u{3164}", 0), "🔔 a 보고");
        assert_eq!(ring("a", "progress", "plan", 0), "🔔 a 진행: plan");
        assert!(!valid_line("🔔 a\n보고"));
        assert!(!valid_line(""));
        assert!(!valid_line(&"x".repeat(LINE_LIMIT + 1)));
    }
}
