//! The one model-free matcher Project Memory uses to find cards for a piece of
//! text: the prompt hook, `hide memory search`, and the comparison set a new
//! card is checked against when it is written (PRD project-memory-v2 D-05,
//! D-13).
//!
//! A card is indexed by its words, the words with a Korean particle or ending
//! taken off, its aliases, and every two-syllable piece of its Hangul words,
//! so `테스트를 돌려줘` finds a card about `테스트` and a run of Hangul without
//! spaces still shares pieces with a spaced one. SQLite's FTS5 ranks the
//! candidates by BM25; each one is then scored here and kept only above
//! [`MINIMUM_SCORE`], so a prompt that shares no word, stem, alias or piece
//! with a card never receives it (B11, B12).

use unicode_normalization::UnicodeNormalization;

/// The most query terms one prompt or search uses; the rest is ignored.
pub(crate) const QUERY_TERM_LIMIT: usize = 24;
/// The most terms a comparison takes from a conversation it analyzes.
pub(crate) const COMPARISON_TERM_LIMIT: usize = 96;
/// A card needs one whole word, stem or alias, or two strong pieces.
pub(crate) const MINIMUM_SCORE: f64 = 0.8;
/// The lower bar a card clears to be compared with new words, so the
/// analysis sees a same-meaning card said in other words (PRD
/// memory-self-improve D-38).
pub(crate) const COMPARISON_MINIMUM_SCORE: f64 = 0.4;
/// The share of a Hangul word's pieces a card must hold, starting with its
/// first, for the word to count when neither it nor its stem is found whole.
const GRAM_SHARE: f64 = 0.6;

/// Korean particles and verb endings taken off a Hangul word's end, longest
/// first. A stem shorter than two syllables keeps the whole word.
const KOREAN_SUFFIXES: &[&str] = &[
    "해주세요",
    "입니다",
    "합니다",
    "습니다",
    "에서는",
    "에서도",
    "으로는",
    "이라는",
    "에게서",
    "해줘",
    "해라",
    "세요",
    "니다",
    "에서",
    "으로",
    "에게",
    "한테",
    "까지",
    "부터",
    "처럼",
    "보다",
    "이랑",
    "하고",
    "이나",
    "이고",
    "이다",
    "라는",
    "하기",
    "하는",
    "했다",
    "한다",
    "된다",
    "되는",
    "줘",
    "를",
    "을",
    "이",
    "가",
    "은",
    "는",
    "의",
    "에",
    "로",
    "와",
    "과",
    "도",
    "만",
    "랑",
    "나",
    "고",
    "다",
];

/// Words that say nothing about a card's subject; a prompt term in this list
/// is not matched, so `하나` in `리뷰어 하나 띄워줘` finds nothing on its own.
const STOPWORDS: &[&str] = &[
    "하나",
    "이거",
    "그거",
    "저거",
    "이것",
    "그것",
    "여기",
    "거기",
    "지금",
    "그냥",
    "다시",
    "좀",
    "해줘",
    "해주세요",
    "주세요",
    "그리고",
    "근데",
    "그럼",
    "어떻게",
    "뭐",
    "the",
    "and",
    "for",
    "with",
    "this",
    "that",
    "from",
    "into",
    "please",
    "can",
    "you",
    "are",
    "was",
    "will",
    "not",
    "but",
    "have",
    "has",
    "what",
    "how",
    "why",
    "let",
    "lets",
    "now",
    "then",
    "just",
    "also",
    "is",
    "in",
    "it",
    "to",
    "of",
    "on",
    "do",
    "be",
    "as",
    "or",
    "if",
    "at",
    "by",
    "we",
    "an",
    "my",
    "me",
    "so",
    "no",
    "up",
    "ok",
];

/// The shortest non-Hangul stem a card word may merely start with: `test`
/// finds `tests`, but `is` does not find `issue` (B12).
const PREFIX_MINIMUM_CHARS: usize = 4;

/// A text's normalized form: NFC, lowercase, single spaces.
pub(crate) fn normalize(value: &str) -> String {
    value
        .nfc()
        .collect::<String>()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn words(value: &str) -> impl Iterator<Item = String> + '_ {
    value
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
        .map(|word| word.trim_matches('-').to_owned())
        .filter(|word| word.chars().count() >= 2)
}

fn is_hangul(character: char) -> bool {
    matches!(character, '\u{AC00}'..='\u{D7A3}')
}

/// The word with one Korean particle or ending removed, or the word itself.
fn stem(word: &str) -> String {
    if !word.chars().last().is_some_and(is_hangul) {
        return word.to_owned();
    }
    for suffix in KOREAN_SUFFIXES {
        if let Some(stem) = word.strip_suffix(suffix)
            && stem.chars().count() >= 2
        {
            return stem.to_owned();
        }
    }
    word.to_owned()
}

/// Every two-syllable piece of each Hangul run in `word`.
fn grams(word: &str) -> Vec<String> {
    let mut grams = Vec::new();
    let mut run = Vec::new();
    for character in word.chars().chain(std::iter::once(' ')) {
        if is_hangul(character) {
            run.push(character);
            continue;
        }
        for pair in run.windows(2) {
            grams.push(pair.iter().collect());
        }
        run.clear();
    }
    grams
}

/// What a card is matched by, from its body and aliases.
#[derive(Debug, Default)]
pub(crate) struct CardTerms {
    words: Vec<String>,
    grams: std::collections::HashSet<String>,
}

impl CardTerms {
    pub(crate) fn new(body: &str, aliases: &[String]) -> Self {
        let mut terms = Self::default();
        for text in std::iter::once(body).chain(aliases.iter().map(String::as_str)) {
            for word in words(&normalize(text)) {
                terms.grams.extend(grams(&word));
                let stemmed = stem(&word);
                if stemmed != word {
                    terms.words.push(stemmed);
                }
                terms.words.push(word);
            }
        }
        terms.words.sort();
        terms.words.dedup();
        terms
    }

    /// The text stored in the search index's `normalized_terms` column: the
    /// normalized body, the aliases, the stems and the pieces, so BM25 sees
    /// every form a query can arrive in.
    pub(crate) fn index_text(body: &str, aliases: &[String]) -> String {
        let terms = Self::new(body, aliases);
        let mut grams = terms.grams.into_iter().collect::<Vec<_>>();
        grams.sort();
        let mut text = normalize(body);
        for part in aliases.iter().map(|alias| normalize(alias)) {
            text.push(' ');
            text.push_str(&part);
        }
        for part in terms.words.iter().chain(grams.iter()) {
            text.push(' ');
            text.push_str(part);
        }
        text
    }
}

/// One query word with the forms it is matched in.
#[derive(Debug)]
struct QueryTerm {
    word: String,
    stem: String,
    grams: Vec<String>,
}

impl QueryTerm {
    fn new(word: String) -> Self {
        Self {
            stem: stem(&word),
            grams: grams(&word),
            word,
        }
    }

    /// Whether a card word that only starts with this term's stem counts:
    /// a Hangul stem has its particle off already, and any other stem must
    /// be long enough to name something.
    fn prefixes(&self) -> bool {
        self.stem.chars().any(is_hangul) || self.stem.chars().count() >= PREFIX_MINIMUM_CHARS
    }
}

/// A text a lookup starts from: a prompt, a search, or new card text.
#[derive(Debug)]
pub(crate) struct Query {
    terms: Vec<QueryTerm>,
}

impl Query {
    pub(crate) fn new(text: &str) -> Self {
        Self::with_limit(text, QUERY_TERM_LIMIT)
    }

    pub(crate) fn with_limit(text: &str, limit: usize) -> Self {
        let mut seen = std::collections::HashSet::new();
        let terms = words(&normalize(text))
            .filter(|word| !STOPWORDS.contains(&word.as_str()))
            .filter(|word| seen.insert(word.clone()))
            .take(limit)
            .map(QueryTerm::new)
            .collect();
        Self { terms }
    }

    /// The `limit` words a long text repeats most, earlier first among
    /// equals: what a conversation is about, wherever in it that is, rather
    /// than its first words (D-13, B17).
    pub(crate) fn salient(text: &str, limit: usize) -> Self {
        let mut counts: Vec<(String, usize)> = Vec::new();
        let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for word in words(&normalize(text)).filter(|word| !STOPWORDS.contains(&word.as_str())) {
            match index.get(&word) {
                Some(&position) => counts[position].1 += 1,
                None => {
                    index.insert(word.clone(), counts.len());
                    counts.push((word, 1));
                }
            }
        }
        // A stable sort keeps first occurrence as the tie-break.
        counts.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        Self {
            terms: counts
                .into_iter()
                .take(limit)
                .map(|(word, _)| QueryTerm::new(word))
                .collect(),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// The FTS5 expression that finds every card sharing a word, stem prefix
    /// or piece with the query; scoring decides which of them count.
    pub(crate) fn fts_expression(&self) -> String {
        let mut parts = Vec::new();
        for term in &self.terms {
            let star = if term.prefixes() { "*" } else { "" };
            parts.push(format!("\"{}\"{star}", term.stem.replace('"', "\"\"")));
            if term.word != term.stem {
                parts.push(format!("\"{}\"", term.word.replace('"', "\"\"")));
            }
            for gram in &term.grams {
                parts.push(format!("\"{gram}\""));
            }
        }
        parts.sort();
        parts.dedup();
        parts.join(" OR ")
    }

    /// How strongly `card` matches: one per whole word or stem, less for a
    /// prefix, and the share of a Hangul word's pieces the card holds when
    /// the word is not found whole.
    pub(crate) fn score(&self, card: &CardTerms) -> f64 {
        self.terms
            .iter()
            .map(|term| {
                if card
                    .words
                    .binary_search(&term.word)
                    .or_else(|_| card.words.binary_search(&term.stem))
                    .is_ok()
                {
                    1.0
                } else if term.prefixes()
                    && card.words.iter().any(|word| word.starts_with(&term.stem))
                {
                    0.85
                } else if term
                    .grams
                    .first()
                    .is_some_and(|first| card.grams.contains(first))
                {
                    let found = term
                        .grams
                        .iter()
                        .filter(|gram| card.grams.contains(*gram))
                        .count();
                    let share = found as f64 / term.grams.len() as f64;
                    if share >= GRAM_SHARE { share } else { 0.0 }
                } else {
                    0.0
                }
            })
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(query: &str, body: &str, aliases: &[&str]) -> f64 {
        let aliases = aliases
            .iter()
            .map(|alias| (*alias).to_owned())
            .collect::<Vec<_>>();
        Query::new(query).score(&CardTerms::new(body, &aliases))
    }

    #[test]
    fn a_korean_prompt_with_particles_finds_the_card_about_its_stem() {
        // B11: "테스트를 돌려줘" receives a card whose body says "테스트".
        assert!(score("테스트를 돌려줘", "테스트는 nextest로 돌린다", &[]) >= MINIMUM_SCORE);
    }

    #[test]
    fn an_alias_alone_is_enough() {
        // B11: "리뷰어 하나 띄워줘" receives the spawn card through its alias.
        let body = "Spawn delegated agents with hide agent spawn --parent here";
        assert!(score("리뷰어 하나 띄워줘", body, &["리뷰어", "reviewer"]) >= MINIMUM_SCORE);
        assert!(score("리뷰어 하나 띄워줘", body, &[]) < MINIMUM_SCORE);
    }

    #[test]
    fn a_prompt_sharing_no_word_stem_alias_or_piece_gets_nothing() {
        // B12
        assert_eq!(
            score(
                "오늘 점심 메뉴 추천",
                "테스트는 nextest로 돌린다",
                &["test runner"]
            ),
            0.0
        );
        assert_eq!(score("hook retrieval", "Prefer blue buttons", &[]), 0.0);
        assert!(score("하나 해줘", "하나의 규칙만 둔다", &[]) < MINIMUM_SCORE);
    }

    #[test]
    fn short_english_words_find_no_card_by_prefix() {
        // B12: "is it ok to push" shares no subject with these cards.
        let card = "Track issues, items and tokens in the tools";
        assert!(score("is it ok to push", card, &[]) < MINIMUM_SCORE);
        assert!(score("go", "Run gofmt before a commit", &[]) < MINIMUM_SCORE);
        assert!(
            Query::new("is it ok to push")
                .fts_expression()
                .split(" OR ")
                .all(|part| !part.ends_with('*') || part.len() > 6)
        );
    }

    #[test]
    fn english_words_match_whole_or_by_prefix() {
        assert_eq!(
            score("hook retrieval", "Keep hook retrieval local", &[]),
            2.0
        );
        assert!(score("migrations", "Run migration tests", &[]) < MINIMUM_SCORE);
        assert!(score("migration", "Run migrations first", &[]) >= MINIMUM_SCORE);
    }

    #[test]
    fn hangul_without_spaces_shares_pieces_with_spaced_text() {
        assert!(score("프로젝트기억", "프로젝트 기억은 로컬이다", &[]) >= MINIMUM_SCORE);
        assert!(score("기억", "프로젝트기억은 로컬이다", &[]) >= MINIMUM_SCORE);
        assert_eq!(score("기억력", "프로젝트기억은 로컬이다", &[]), 0.0);
    }

    #[test]
    fn index_text_carries_stems_aliases_and_pieces() {
        let text = CardTerms::index_text("테스트는 먼저", &["리뷰어".to_owned()]);
        for part in ["테스트는", "테스트", "리뷰어", "테스", "스트", "리뷰"] {
            assert!(
                text.split(' ').any(|word| word == part),
                "{part} missing from {text}"
            );
        }
    }

    #[test]
    fn a_long_texts_comparison_terms_are_what_it_repeats_not_its_first_words() {
        // B17: the subject named late but often still finds its card.
        let mut text = (0..200).map(|n| format!("word{n} ")).collect::<String>();
        text.push_str("nextest nextest nextest");
        let card = CardTerms::new("Run tests with nextest", &[]);
        assert_eq!(
            Query::with_limit(&text, COMPARISON_TERM_LIMIT).score(&card),
            0.0
        );
        assert!(Query::salient(&text, COMPARISON_TERM_LIMIT).score(&card) >= MINIMUM_SCORE);
    }

    #[test]
    fn the_fts_expression_quotes_terms_and_skips_stopwords() {
        let expression = Query::new("리뷰어 하나 \"띄워줘\"").fts_expression();
        assert!(expression.contains("\"리뷰어\"*"));
        assert!(expression.contains("\"띄워\"*"));
        assert!(!expression.contains("하나"));
        assert!(Query::new("하나 좀").is_empty());
    }
}
