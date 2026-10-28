//! The doorbell's line: what Hide types into a pane to wake it for a letter
//! (`docs/delivery.md`, The bell line), and how a transcript reader tells it
//! from the operator's own words.
//!
//! `herdr-core/src/delivery/bell.rs` writes the line from the parts named
//! here and [`is_bell_line`] reads it back by the same parts, so the two
//! cannot drift: `🦉 <sender> <kind word>`, then `: <summary>` when the letter
//! has a first line, ` · <N more>` when other letters wait and
//! [`BELL_INBOX`] when the line names where the letter is. The transcript
//! keeps no other mark of it: the line the ledger saved outlives neither the
//! letter's retention nor the session file it is read from.
//!
//! The writer keeps the shape unambiguous, so the reader need not guess: a
//! sender and summary carry no character above U+FFFF (Codex CLI on Windows
//! drops them as it receives the line, which then differs from what was
//! typed only by the missing owl) and no [`BELL_DOT`] (typed as
//! [`BELL_DOT_STAND_IN`]), so [`BELL_SEPARATOR`] only ever opens a tail.

/// What the line begins with, before a space and the sender.
pub const BELL_MARK: char = '🦉';

/// What comes before each tail of the line.
pub const BELL_SEPARATOR: &str = " · ";

/// What the line ends with when it names where the letter is.
pub const BELL_INBOX: &str = " · hide inbox";

/// The separator's dot, which a sender or summary carries as the stand-in.
pub const BELL_DOT: char = '·';
pub const BELL_DOT_STAND_IN: char = '∙';

/// The longest sender name and letter summary the line carries, in
/// characters; a longer one is cut and ends in [`BELL_CUT`].
pub const BELL_NAME_CHARS: usize = 32;
pub const BELL_SUMMARY_CHARS: usize = 60;
pub const BELL_CUT: char = '…';

/// The characters a composer acts on, which the line types as their
/// full-width forms (`＠`, `＄`, `＼`) and so never carries.
pub const BELL_INERT: [char; 3] = ['@', '$', '\\'];

/// The letter kinds the line names, in the order of [`BellWords::kinds`].
pub const BELL_KINDS: [&str; 6] = ["request", "block", "report", "progress", "reply", "watch"];

/// The words of the line in one interface language.
pub struct BellWords {
    /// The word for each of [`BELL_KINDS`], then the one for a kind this
    /// build does not know.
    pub kinds: [&'static str; 7],
    /// What goes before and after the count of other waiting letters.
    pub more: (&'static str, &'static str),
}

impl BellWords {
    /// The word for `kind`; a kind this build does not know is a letter.
    pub fn kind(&self, kind: &str) -> &'static str {
        let index = BELL_KINDS
            .iter()
            .position(|known| *known == kind)
            .unwrap_or(BELL_KINDS.len());
        self.kinds[index]
    }

    /// How many other letters wait.
    pub fn more(&self, others: usize) -> String {
        format!("{}{others}{}", self.more.0, self.more.1)
    }
}

pub const BELL_ENGLISH: BellWords = BellWords {
    kinds: [
        "request", "blocked", "report", "progress", "reply", "watch", "letter",
    ],
    more: ("and ", " more"),
};
pub const BELL_KOREAN: BellWords = BellWords {
    kinds: ["요청", "막힘", "보고", "진행", "답장", "감시", "편지"],
    more: ("외 ", "통"),
};
pub const BELL_SIMPLIFIED_CHINESE: BellWords = BellWords {
    kinds: ["请求", "受阻", "报告", "进展", "回复", "监视", "信"],
    more: ("另有 ", " 封"),
};
pub const BELL_JAPANESE: BellWords = BellWords {
    kinds: ["依頼", "ブロック", "報告", "進捗", "返信", "監視", "手紙"],
    more: ("ほか ", " 通"),
};

const LANGUAGES: [&BellWords; 4] = [
    &BELL_ENGLISH,
    &BELL_KOREAN,
    &BELL_SIMPLIFIED_CHINESE,
    &BELL_JAPANESE,
];

/// Whether `text` is a line the doorbell typed, by its whole shape: the mark
/// and a space, a sender of at most [`BELL_NAME_CHARS`] characters, a kind
/// word of one interface language, an optional summary of at most
/// [`BELL_SUMMARY_CHARS`] characters after `: `, and only the tails the line
/// adds. Codex CLI on Windows can record the line with every character above
/// U+FFFF dropped, the mark among them; a line Codex is rung with always
/// names the inbox, so the same shape without the mark counts only with
/// [`BELL_INBOX`] at its end.
///
/// A person who types that exact shape is read as a letter; an instruction
/// that only begins with the mark or ends with the inbox tail is not.
pub fn is_bell_line(text: &str) -> bool {
    let text = text.trim();
    if text.chars().any(char::is_control) {
        return false;
    }
    let (rest, inbox_required) = match text.strip_prefix(BELL_MARK) {
        Some(rest) => match rest.strip_prefix(' ') {
            Some(rest) => (rest, false),
            None => return false,
        },
        None => (text, true),
    };
    let rest = match rest.strip_suffix(BELL_INBOX) {
        Some(rest) => rest,
        None if inbox_required => return false,
        None => rest,
    };
    let rest = strip_more(rest).unwrap_or(rest);
    LANGUAGES
        .iter()
        .flat_map(|words| words.kinds)
        .any(|word| head_is(rest, word))
}

/// `rest` without a trailing ` · <N more>` in any language.
fn strip_more(rest: &str) -> Option<&str> {
    let (head, tail) = rest.rsplit_once(BELL_SEPARATOR)?;
    LANGUAGES
        .iter()
        .any(|words| {
            tail.strip_prefix(words.more.0)
                .and_then(|tail| tail.strip_suffix(words.more.1))
                .is_some_and(|count| {
                    !count.is_empty()
                        && count.bytes().all(|b| b.is_ascii_digit())
                        && !count.starts_with('0')
                })
        })
        .then_some(head)
}

/// Whether `head` is `<sender> <word>` or `<sender> <word>: <summary>`.
fn head_is(head: &str, word: &str) -> bool {
    let needle = format!(" {word}");
    head.match_indices(&needle).any(|(at, _)| {
        let sender = &head[..at];
        let after = &head[at + needle.len()..];
        let summary = match after {
            "" => None,
            after => match after.strip_prefix(": ") {
                Some(summary) => Some(summary),
                None => return false,
            },
        };
        fits(sender, BELL_NAME_CHARS)
            && summary.is_none_or(|summary| fits(summary, BELL_SUMMARY_CHARS))
    })
}

/// Whether `part` is a sender or summary the line could carry: nonempty, no
/// outer or doubled whitespace, none of [`BELL_INERT`], and at most `max`
/// characters or `max` and [`BELL_CUT`].
fn fits(part: &str, max: usize) -> bool {
    let count = part.chars().count();
    !part.is_empty()
        && part.trim() == part
        && !part.contains("  ")
        && !part.contains(BELL_INERT)
        && (count <= max || (count == max + 1 && part.ends_with(BELL_CUT)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_inbox_tail_opens_with_the_separator() {
        assert!(BELL_INBOX.starts_with(BELL_SEPARATOR));
        assert!(BELL_SEPARATOR.contains(BELL_DOT));
    }

    #[test]
    fn the_doorbells_line_is_read_with_or_without_its_owl() {
        for line in [
            "🦉 ci-lead 보고: PR #881 · 외 2통",
            "🦉 ci-lead report",
            "🦉 라벨 제목 보고: 리뷰 끝났습니다 · 외 1통 · hide inbox",
            "  🦉 Codex report: private · hide inbox",
            "🦉 a 감시: x · 외 2통",
            "🦉 ci-lead 报告: PR #881 · 另有 2 封",
            "🦉 ci-lead 報告: PR #881 · ほか 2 通",
            "🦉 a request: ＠src/main.rs ＄HOME",
            // Codex CLI on Windows dropped the owl and the summary's cat.
            "Codex 보고: Readthe letter · hide inbox",
            "Codex report · and 1 more · hide inbox",
        ] {
            assert!(is_bell_line(line), "{line}");
        }
    }

    #[test]
    fn an_instruction_that_only_starts_or_ends_like_the_line_is_not_it() {
        for line in [
            "🦉 아이콘은 항상 파란색으로",
            "🦉 icon stays blue in the sidebar",
            "🦉ci-lead report",
            "fix the 🦉 icon",
            "🦉 ci-lead report\nand also run the tests",
            "🦉 ci-lead 보고:",
            "🦉 ci-lead 보고:no space",
            "🦉 보고: 이름이 없음",
            "🦉 a request: @src/main.rs",
            "새 탭은 항상 옆에 열어 · hide inbox",
            "hide inbox",
            " · hide inbox",
            "please check the hide inbox",
            // Without the owl the inbox tail is required.
            "Codex 보고: Readthe letter",
        ] {
            assert!(!is_bell_line(line), "{line}");
        }
        let long = format!("🦉 a 보고: {}", "가".repeat(61));
        assert!(!is_bell_line(&long));
        let cut = format!("🦉 a 보고: {}…", "가".repeat(60));
        assert!(is_bell_line(&cut));
    }
}
