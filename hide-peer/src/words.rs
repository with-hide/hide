//! The four words both operators compare (PRD B2).
//!
//! Both machines derive them from what only an untampered pairing has in
//! common: the two keys the transport proved and the offer's secret. A
//! machine in the middle that swapped the link holds another key, so the
//! words on the guest's screen are not the words on the real host's.
//!
//! The derivation is part of what two builds agree on, like a message of
//! `contracts/peer-link.json`: the first 44 bits of
//! BLAKE3(`hide-pair/v1/sas` | host key | guest key | secret), cut into
//! four 11-bit numbers, each one word of a 2048-word list. The lists are
//! BIP-39's English and Korean ones, unchanged (see NOTICE); the host says
//! which one both screens read from.

use std::sync::OnceLock;

use hide_peer_protocol::PeerKey;

use crate::link::Secret;

const DOMAIN: &[u8] = b"hide-pair/v1/sas";

/// How many words are compared, and how many a list holds.
pub const COUNT: usize = 4;
pub const LIST_WORDS: usize = 2048;

/// A list both screens read the words from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WordList {
    English,
    Korean,
}

impl WordList {
    /// The name the host sends.
    pub const fn name(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Korean => "ko",
        }
    }

    /// `None` for a list this build does not carry.
    pub fn from_name(name: &str) -> Option<Self> {
        [Self::English, Self::Korean]
            .into_iter()
            .find(|list| list.name() == name)
    }

    /// The list for an operator who reads `language` (a tag such as `ko` or
    /// `ko-KR`). English for every language without a list of its own.
    pub fn for_language(language: &str) -> Self {
        let primary = language.split(['-', '_']).next().unwrap_or_default();
        if primary.eq_ignore_ascii_case("ko") {
            Self::Korean
        } else {
            Self::English
        }
    }

    fn words(self) -> &'static [&'static str] {
        static ENGLISH: OnceLock<Vec<&'static str>> = OnceLock::new();
        static KOREAN: OnceLock<Vec<&'static str>> = OnceLock::new();
        let (cell, text) = match self {
            Self::English => (&ENGLISH, include_str!("words/en.txt")),
            Self::Korean => (&KOREAN, include_str!("words/ko.txt")),
        };
        cell.get_or_init(|| text.lines().collect())
    }
}

/// The four numbers the words stand for.
pub fn indexes(host: &PeerKey, guest: &PeerKey, secret: &Secret) -> [u16; COUNT] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(DOMAIN);
    hasher.update(host.as_bytes());
    hasher.update(guest.as_bytes());
    hasher.update(secret.bytes());
    let hash = hasher.finalize();
    let mut first = [0u8; 8];
    first[2..].copy_from_slice(&hash.as_bytes()[..6]);
    // 48 bits read, the last 4 dropped: 44 bits, eleven to a word.
    let bits = u64::from_be_bytes(first) >> 4;
    std::array::from_fn(|index| ((bits >> (33 - 11 * index)) & 0x7ff) as u16)
}

/// The words of a pairing between `host` and `guest` under `secret`.
pub fn of(
    list: WordList,
    host: &PeerKey,
    guest: &PeerKey,
    secret: &Secret,
) -> [&'static str; COUNT] {
    let words = list.words();
    indexes(host, guest, secret).map(|index| words[usize::from(index)])
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn key(byte: u8) -> PeerKey {
        PeerKey::from_bytes([byte; 32])
    }

    #[test]
    fn two_builds_read_the_same_words_from_the_same_pairing() {
        // Worked out with another BLAKE3 implementation (Python's `blake3`)
        // over the same bytes and the same two files.
        let secret = Secret::from_bytes([0xa5; 32]);
        assert_eq!(indexes(&key(7), &key(9), &secret), [595, 175, 1164, 942]);
        assert_eq!(
            of(WordList::English, &key(7), &key(9), &secret),
            ["engine", "bicycle", "muscle", "into"]
        );
        assert_eq!(
            of(WordList::Korean, &key(7), &key(9), &secret),
            ["방금", "교양", "열정", "스승"]
        );
    }

    #[test]
    fn another_key_or_another_secret_gives_other_words() {
        let secret = Secret::from_bytes([0xa5; 32]);
        let words = indexes(&key(7), &key(9), &secret);
        // A machine in the middle holds another key, on either side.
        assert_ne!(indexes(&key(8), &key(9), &secret), words);
        assert_ne!(indexes(&key(7), &key(8), &secret), words);
        // Host and guest are not interchangeable.
        assert_ne!(indexes(&key(9), &key(7), &secret), words);
        assert_ne!(
            indexes(&key(7), &key(9), &Secret::from_bytes([0xa4; 32])),
            words
        );
    }

    #[test]
    fn each_list_holds_2048_different_words_an_operator_can_read() {
        for list in [WordList::English, WordList::Korean] {
            let words = list.words();
            assert_eq!(words.len(), LIST_WORDS, "{list:?}");
            assert_eq!(words.iter().collect::<HashSet<_>>().len(), LIST_WORDS);
            assert!(
                words
                    .iter()
                    .all(|word| !word.is_empty() && word.chars().all(char::is_alphabetic)),
                "{list:?}"
            );
            assert_eq!(WordList::from_name(list.name()), Some(list));
        }
        assert_eq!(WordList::from_name("ja"), None);
        assert_eq!(WordList::for_language("ko-KR"), WordList::Korean);
        assert_eq!(WordList::for_language("zh-CN"), WordList::English);
    }
}
