//! The four words both operators compare (PRD B2).
//!
//! Both machines derive them from what only an untampered pairing has in
//! common: the two keys the transport proved, and one random value of each
//! side. A machine in the middle that swapped the link holds another key,
//! so the words on the guest's screen are not the words on the real host's.
//!
//! It cannot search for a value that makes them agree either, because no
//! side picks its value knowing the other's: the guest first sends only a
//! commitment to its value ([`commitment`]), the host then sends its own,
//! and only then does the guest open the commitment ([`opens`]). Whoever
//! stands between the two has to send a value to one side before it learns
//! the other side's, so the four words are out of its hands, and matching
//! them is one chance in 2^44 for each attempt an operator would see fail.
//! The link's secret is not an input: a secret the middle machine wrote
//! itself was the one thing it could have searched over.
//!
//! The derivation is part of what two builds agree on, like a message of
//! `contracts/peer-link.json`: the commitment is
//! BLAKE3(`hide-pair/v1/commit` | value) in hex, and the words are the
//! first 44 bits of
//! BLAKE3(`hide-pair/v1/sas` | host key | guest key | guest value | host value),
//! cut into four 11-bit numbers, each one word of a 2048-word list. The
//! lists are BIP-39's English and Korean ones, unchanged (see NOTICE); the
//! host says which one both screens read from.

use std::sync::OnceLock;

use hide_peer_protocol::PeerKey;

use crate::link::Secret;

const DOMAIN: &[u8] = b"hide-pair/v1/sas";
const COMMIT_DOMAIN: &[u8] = b"hide-pair/v1/commit";

/// One side's random value for the words, drawn for one attempt: 32 bytes,
/// sent as 64 hex characters like a link's secret.
pub type Nonce = Secret;

/// What binds a side to `nonce` before it shows it.
pub fn commitment(nonce: &Nonce) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(COMMIT_DOMAIN);
    hasher.update(nonce.bytes());
    hasher.finalize().to_hex().to_string()
}

/// Whether `nonce` is the value `commit` was made from.
pub fn opens(commit: &str, nonce: &Nonce) -> bool {
    commitment(nonce) == commit
}

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
pub fn indexes(
    host: &PeerKey,
    guest: &PeerKey,
    guest_nonce: &Nonce,
    host_nonce: &Nonce,
) -> [u16; COUNT] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(DOMAIN);
    hasher.update(host.as_bytes());
    hasher.update(guest.as_bytes());
    hasher.update(guest_nonce.bytes());
    hasher.update(host_nonce.bytes());
    let hash = hasher.finalize();
    let mut first = [0u8; 8];
    first[2..].copy_from_slice(&hash.as_bytes()[..6]);
    // 48 bits read, the last 4 dropped: 44 bits, eleven to a word.
    let bits = u64::from_be_bytes(first) >> 4;
    std::array::from_fn(|index| ((bits >> (33 - 11 * index)) & 0x7ff) as u16)
}

/// The words of a pairing between `host` and `guest`, from the value each
/// of them drew for it.
pub fn of(
    list: WordList,
    host: &PeerKey,
    guest: &PeerKey,
    guest_nonce: &Nonce,
    host_nonce: &Nonce,
) -> [&'static str; COUNT] {
    let words = list.words();
    indexes(host, guest, guest_nonce, host_nonce).map(|index| words[usize::from(index)])
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn key(byte: u8) -> PeerKey {
        PeerKey::from_bytes([byte; 32])
    }

    fn nonce(byte: u8) -> Nonce {
        Nonce::from_bytes([byte; 32])
    }

    #[test]
    fn two_builds_read_the_same_words_from_the_same_pairing() {
        // Worked out with another BLAKE3 implementation (Python's `blake3`)
        // over the same bytes and the same two files.
        let (guest, host) = (nonce(0xa5), nonce(0x3c));
        assert_eq!(
            commitment(&guest),
            "27d2a30d1b20a66c64109f1e73ccadc3d737b3abde60c6629c4e5ee775feadea"
        );
        assert_eq!(
            indexes(&key(7), &key(9), &guest, &host),
            [1780, 1723, 565, 253]
        );
        assert_eq!(
            of(WordList::English, &key(7), &key(9), &guest, &host),
            ["teach", "student", "effort", "cabbage"]
        );
        assert_eq!(
            of(WordList::Korean, &key(7), &key(9), &guest, &host),
            ["콩나물", "최근", "박수", "기독교"]
        );
    }

    #[test]
    fn another_key_or_another_value_of_either_side_gives_other_words() {
        let (guest, host) = (nonce(0xa5), nonce(0x3c));
        let words = indexes(&key(7), &key(9), &guest, &host);
        // A machine in the middle holds another key, on either side.
        assert_ne!(indexes(&key(8), &key(9), &guest, &host), words);
        assert_ne!(indexes(&key(7), &key(8), &guest, &host), words);
        // Host and guest are not interchangeable, and neither are their values.
        assert_ne!(indexes(&key(9), &key(7), &guest, &host), words);
        assert_ne!(indexes(&key(7), &key(9), &host, &guest), words);
        // Each side's value moves the words, so neither side alone sets them.
        assert_ne!(indexes(&key(7), &key(9), &nonce(0xa4), &host), words);
        assert_ne!(indexes(&key(7), &key(9), &guest, &nonce(0x3d)), words);
    }

    #[test]
    fn a_commitment_is_opened_only_by_the_value_it_was_made_from() {
        let commit = commitment(&nonce(0xa5));
        assert!(opens(&commit, &nonce(0xa5)));
        assert!(!opens(&commit, &nonce(0xa4)));
        // Not the bare hash of the value: a commitment is no other hash a
        // machine might be made to compute over the same bytes.
        assert_ne!(commit, blake3::hash(&[0xa5; 32]).to_hex().to_string());
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
