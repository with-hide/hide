//! Who sent a Hide letter in a pane's transcript.
//!
//! The first nonempty line is `Hide letter <id> from <name> (<agent>)
//! [<kind>]`. A batch keeps that first letter's attribution; body text and
//! later headers never select a different sender. The independent examples
//! in `contracts/delivery-envelope.json` bind this reader and mailbox::pull.
//!
//! The doorbell's line (`herdr-core/src/delivery/bell.rs`) wakes a pane for a
//! letter and carries no envelope; it is written from [`BELL_MARK`] and
//! [`BELL_INBOX`] here, so the line and [`is_letter_bell`] cannot drift.

/// The most characters of a sender's name a message's facts keep.
const MAX_SENDER_CHARS: usize = 128;

/// The sender named by the first Hide letter header, or `None` without one.
pub fn envelope_sender(text: &str) -> Option<String> {
    envelope_header(text).map(|(sender, _)| sender)
}

/// Whether an envelope's sender names the agent called `name`: the envelope
/// keeps a trimmed, bounded copy of the name, so the name is read the same way.
pub fn envelope_names(sender: &str, name: &str) -> bool {
    name.trim()
        .chars()
        .take(MAX_SENDER_CHARS)
        .eq(sender.chars())
}

/// Whether the first Hide letter header is the operator's own: the letter a
/// View feedback Send makes (PRD view-opener-feedback D-11). The core alone
/// writes the `operator` sender kind; an agent's kind is the provider Herdr
/// detected.
pub fn envelope_from_operator(text: &str) -> bool {
    text.trim_start()
        .lines()
        .next()
        .and_then(|first| first.trim().strip_prefix("Hide letter "))
        .and_then(|header| header.rsplit_once(" [").map(|(envelope, _)| envelope))
        .and_then(|envelope| envelope.split_once(" from ").map(|(_, from)| from))
        .is_some_and(|from| from == "operator (operator)")
}

/// The canonical envelope sender and kind, for work-giver attribution.
pub fn envelope_header(text: &str) -> Option<(String, String)> {
    let first = text.trim_start().lines().next()?.trim();
    let header = first.strip_prefix("Hide letter ")?;
    let (envelope, kind) = header.rsplit_once(" [")?;
    let kind = kind.strip_suffix(']')?;
    if !matches!(
        kind,
        "request" | "block" | "report" | "progress" | "reply" | "watch"
    ) {
        return None;
    }
    let (id, from) = envelope.split_once(" from ")?;
    if id.is_empty() || id.len() > 256 || id.chars().any(char::is_whitespace) {
        return None;
    }
    let (name, agent) = from.strip_suffix(')')?.rsplit_once(" (")?;
    if agent.trim().is_empty() {
        return None;
    }
    // The name is the sender's own choice; the facts keep a bounded copy and
    // the core decides how it is shown.
    let name = name.trim();
    (!name.is_empty()).then(|| {
        (
            name.chars().take(MAX_SENDER_CHARS).collect(),
            kind.to_owned(),
        )
    })
}

/// What the doorbell's line begins with, before a space and the sender.
pub const BELL_MARK: char = '🦉';

/// What the doorbell's line ends with when it names where the letter is.
pub const BELL_INBOX: &str = " · hide inbox";

/// Whether `text` is the doorbell's line: one line that begins with
/// [`BELL_MARK`], or, as Codex CLI on Windows can record it with every
/// character above U+FFFF dropped, one that ends with [`BELL_INBOX`] (a line
/// Codex is rung with always names the inbox). The transcript keeps no other
/// mark of it: the line the ledger saved outlives neither the letter's
/// retention nor the session file it is read from.
pub fn is_letter_bell(text: &str) -> bool {
    let text = text.trim();
    if text.contains('\n') {
        return false;
    }
    text.strip_prefix(BELL_MARK)
        .is_some_and(|rest| rest.starts_with(' '))
        || text.ends_with(BELL_INBOX)
}

/// Whether a person's message in a transcript is a Hide letter rather than
/// the operator's own words: another agent's envelope or the doorbell's line.
/// A letter the operator's Send made is the operator's.
pub fn is_letter(text: &str) -> bool {
    is_letter_bell(text) || (envelope_header(text).is_some() && !envelope_from_operator(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_contract_example_reads_its_sender() {
        let contract: serde_json::Value =
            serde_json::from_str(include_str!("../../contracts/delivery-envelope.json")).unwrap();
        for example in contract["examples"].as_array().unwrap() {
            let line = example["first_line"].as_str().unwrap();
            let body = format!("\n{line}\nPlease look\nFull letter: hide request show letter-1\n");
            assert_eq!(
                envelope_sender(&body).as_deref(),
                example["sender"].as_str(),
                "{line}"
            );
            assert_eq!(
                envelope_from_operator(&body),
                example["operator"].as_bool().unwrap_or(false),
                "{line}"
            );
        }
    }

    #[test]
    fn a_batch_keeps_the_first_letters_sender() {
        let contract: serde_json::Value =
            serde_json::from_str(include_str!("../../contracts/delivery-envelope.json")).unwrap();
        assert_eq!(
            envelope_sender(contract["batch"]["context"].as_str().unwrap()).as_deref(),
            contract["batch"]["sender"].as_str(),
        );
    }

    #[test]
    fn a_senders_name_is_bounded_at_a_character_boundary() {
        for chars in [128, 129] {
            let name = "배".repeat(chars);
            assert_eq!(
                envelope_sender(&format!(
                    "Hide letter letter-1 from {name} (claude) [request]"
                )),
                Some("배".repeat(128)),
            );
            // The name the envelope was written from still names its sender.
            assert!(envelope_names(
                &"배".repeat(128),
                &format!(" {} ", "배".repeat(chars))
            ));
        }
        assert!(!envelope_names("배", "배배"));
    }

    #[test]
    fn a_message_without_a_header_names_no_sender() {
        for text in [
            "fix the tests",
            "Hide letter letter-1 from sender",
            "hide letter letter-1 from sender (claude) [request]",
            "Hide letter  from sender (claude) [request]",
            "Hide letter letter-1 from  (claude) [request]",
            "Hide letter letter-1 from sender () [request]",
            "Hide letter letter-1 from sender (claude) [unknown]",
            "Hide letter letter-1 from sender (claude) [request] extra",
            "",
            "Please read Hide letter letter-1 from sender (claude) [request]",
            "ordinary request\nHide letter letter-1 from sender (claude) [request]",
        ] {
            assert_eq!(envelope_sender(text), None, "{text}");
        }
    }

    #[test]
    fn the_doorbells_line_is_a_letter_with_or_without_its_owl() {
        for line in [
            "🦉 ci-lead 보고: PR #881 · 외 2통",
            "🦉 ci-lead report",
            "  🦉 Codex report: private · hide inbox\n",
            // Codex CLI on Windows dropped the owl and the summary's cat.
            "Codex 보고: Readthe letter · hide inbox",
        ] {
            assert!(is_letter_bell(line), "{line}");
            assert!(is_letter(line), "{line}");
        }
        for line in [
            "fix the 🦉 icon",
            "🦉ci-lead report",
            "🦉 ci-lead report\nand also run the tests",
            "hide inbox",
            "please check the hide inbox",
        ] {
            assert!(!is_letter_bell(line), "{line}");
        }
    }

    #[test]
    fn another_agents_envelope_is_a_letter_and_the_operators_send_is_not() {
        assert!(is_letter(
            "Hide letter letter-1 from ci-lead (claude) [report]\nDone"
        ));
        assert!(!is_letter(
            "Hide letter letter-1 from operator (operator) [request]\nLook here"
        ));
        assert!(!is_letter("레이어 2도 1.0.1에 포함"));
    }
}
