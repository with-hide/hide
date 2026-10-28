//! The operator's repeated requests (PRD memory-self-improve D-14, D-37,
//! D-46): what the operator asks for every time, named by its meaning by the
//! same analysis call that writes cards, kept as the operator's words that
//! said it, by session and event. Hide AI 제안 scores them; Memory only keeps
//! them, bounded.

use super::*;

/// The most rows a Project keeps, and one meaning keeps; a write past either
/// keeps nothing more and is reported (B33).
pub const REPEAT_LIMIT: u64 = 5_000;
pub const REPEAT_MEANING_LIMIT: u64 = 200;
/// A meaning's name, in characters (D-44).
pub const MEANING_CHARS: usize = 40;
/// The proposed instruction line, in characters.
pub const LINE_CHARS: usize = 200;
/// The most repeat groups one analysis answer carries (D-44).
pub const REPEATS_PER_ANSWER: usize = 5;

/// Where a repeated request belongs (D-06): the operator's personal taste,
/// for the Hide global instruction, or this Project's way of working, for its
/// documents, skills and checks.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepeatScope {
    Personal,
    Project,
}

impl RepeatScope {
    fn as_str(self) -> &'static str {
        match self {
            Self::Personal => "personal",
            Self::Project => "project",
        }
    }

    fn parse(value: &str) -> Self {
        if value == "personal" {
            Self::Personal
        } else {
            Self::Project
        }
    }
}

/// One repeated request the analysis found in a batch, with the operator's
/// words that said it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RepeatRequest {
    pub meaning: String,
    pub scope: RepeatScope,
    /// A procedure, which a skill holds, rather than a rule (D-20).
    pub procedure: bool,
    /// The rule or preference card that says the same, whose own sessions
    /// count too (D-46).
    pub target_id: Option<String>,
    /// The one line a fix would write.
    pub line: String,
    /// The operator's events that said it: offset, when, and the words'
    /// digest. Only `human` events reach here; the caller drops the rest.
    pub sources: Vec<(u64, u64, Option<String>)>,
}

/// One time the operator said a repeated request, as scoring reads it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RepeatSource {
    pub meaning: String,
    pub scope: RepeatScope,
    pub procedure: bool,
    pub line: String,
    /// The card that says the same, by number.
    pub card: Option<u64>,
    /// `provider:session`.
    pub session: String,
    pub event_offset: u64,
    pub at_unix_ms: u64,
    /// The words' digest, so the same words said again (a re-read, a fork's
    /// copy) count once (D-37).
    pub words: Option<String>,
    pub session_started_unix_ms: Option<u64>,
    /// The source is the card's, not the analysis's repeat (D-46).
    pub from_card: bool,
}

fn clean(text: &str, limit: usize) -> Option<String> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!text.is_empty()
        && text.chars().count() <= limit
        && !text.contains('<')
        && !redact(&text).contains_secret_candidate)
        .then_some(text)
}

/// Writes the batch's repeated requests in its transaction; answers how many
/// sources were left out at a cap.
pub(super) fn record(
    transaction: &Transaction<'_>,
    batch: &AnalysisBatch,
    repeats: &[RepeatRequest],
) -> Result<usize, MemoryError> {
    let mut left_out = 0;
    for repeat in repeats.iter().take(REPEATS_PER_ANSWER) {
        let (Some(meaning), Some(line)) = (
            clean(&repeat.meaning, MEANING_CHARS),
            clean(&repeat.line, LINE_CHARS),
        ) else {
            left_out += repeat.sources.len();
            continue;
        };
        let target = match &repeat.target_id {
            Some(id) => transaction
                .query_row(
                    "SELECT i.id FROM memory_items i JOIN memory_revisions r ON r.item_id=i.id AND r.revision=i.current_revision WHERE i.id=?1 AND i.project_id=?2 AND r.kind IN('rule','preference')",
                    params![id, batch.project_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?,
            None => None,
        };
        for (offset, at, words) in &repeat.sources {
            let (all, this): (u64, u64) = transaction.query_row(
                "SELECT COUNT(*),COUNT(*) FILTER (WHERE meaning=?2) FROM repeat_requests WHERE project_id=?1",
                params![batch.project_id, meaning],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            if all >= REPEAT_LIMIT || this >= REPEAT_MEANING_LIMIT {
                left_out += 1;
                continue;
            }
            transaction.execute(
                "INSERT INTO repeat_requests(project_id,meaning,scope,procedure,item_id,line,provider,session_id,event_offset,words,at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11) \
                 ON CONFLICT(project_id,meaning,provider,session_id,event_offset) DO UPDATE SET scope=excluded.scope,procedure=excluded.procedure,item_id=COALESCE(excluded.item_id,item_id),line=excluded.line",
                params![
                    batch.project_id,
                    meaning,
                    repeat.scope.as_str(),
                    repeat.procedure,
                    target,
                    line,
                    batch.provider,
                    batch.session_id,
                    offset,
                    words,
                    at
                ],
            )?;
        }
    }
    Ok(left_out)
}

impl MemoryStore {
    /// Every repeated request of a Project, with the operator's words behind
    /// the cards that say the same (D-46), oldest first. Sources of a card
    /// count only for its current text, as everywhere else.
    pub fn repeat_sources(&self, project_id: &str) -> Result<Vec<RepeatSource>, MemoryError> {
        let mut statement = self.connection.prepare(
            "SELECT q.meaning,q.scope,q.procedure,q.line,i.number,q.provider||':'||q.session_id,q.event_offset,q.at_ms,q.words,ss.started_at_ms FROM repeat_requests q \
             LEFT JOIN memory_items i ON i.id=q.item_id AND i.lifecycle='active' \
             LEFT JOIN session_sources ss ON ss.project_id=q.project_id AND ss.provider=q.provider AND ss.id=q.session_id \
             WHERE q.project_id=?1 ORDER BY q.at_ms,q.provider,q.session_id,q.event_offset",
        )?;
        let mut sources = statement
            .query_map([project_id], |row| {
                Ok(RepeatSource {
                    meaning: row.get(0)?,
                    scope: RepeatScope::parse(&row.get::<_, String>(1)?),
                    procedure: row.get(2)?,
                    line: row.get(3)?,
                    card: row.get(4)?,
                    session: row.get(5)?,
                    event_offset: row.get(6)?,
                    at_unix_ms: row.get(7)?,
                    words: row.get(8)?,
                    session_started_unix_ms: row.get(9)?,
                    from_card: false,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        // The card's own operator sources, under the same meaning as the
        // repeat that named it.
        let named = sources
            .iter()
            .filter_map(|source| source.card.map(|card| (card, source.clone())))
            .collect::<BTreeMap<_, _>>();
        let mut statement = self.connection.prepare(
            "SELECT s.provider||':'||s.session_id,s.event_offset,COALESCE(s.at_ms,r.created_at_ms),s.words,ss.started_at_ms FROM memory_items i JOIN memory_revisions r ON r.item_id=i.id AND r.revision=i.current_revision \
             JOIN memory_sources s ON s.item_id=i.id AND s.human=1 JOIN memory_revisions v ON v.item_id=s.item_id AND v.revision=s.revision AND v.body=r.body \
             LEFT JOIN session_sources ss ON ss.project_id=i.project_id AND ss.provider=s.provider AND ss.id=s.session_id \
             WHERE i.project_id=?1 AND i.number=?2 AND i.lifecycle='active'",
        )?;
        for (card, repeat) in named {
            let rows = statement
                .query_map(params![project_id, card], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, u64>(1)?,
                        row.get::<_, u64>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<u64>>(4)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for (session, offset, at, words, started) in rows {
                sources.push(RepeatSource {
                    session,
                    event_offset: offset,
                    at_unix_ms: at,
                    words,
                    session_started_unix_ms: started,
                    from_card: true,
                    ..repeat.clone()
                });
            }
        }
        Ok(sources)
    }

    /// The meanings a Project already names, most recent first, for the
    /// analysis to name the same request the same way.
    pub fn known_meanings(
        &self,
        project_id: &str,
        limit: usize,
    ) -> Result<Vec<String>, MemoryError> {
        let mut statement = self.connection.prepare(
            "SELECT meaning FROM repeat_requests WHERE project_id=?1 GROUP BY meaning ORDER BY MAX(at_ms) DESC,meaning LIMIT ?2",
        )?;
        Ok(statement
            .query_map(params![project_id, limit as u64], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::super::upkeep::tests::{DAY, card, id_of, said_again, store};
    use super::*;
    use crate::CandidateKind;

    #[test]
    fn a_repeat_keeps_the_operators_words_and_brings_its_cards_sessions() {
        let (_dir, mut store) = store();
        let preference = card(
            &mut store,
            "a",
            "s1",
            CandidateKind::Preference,
            "Answer in Korean",
            999 * DAY,
        );
        let other = card(
            &mut store,
            "b",
            "s0",
            CandidateKind::Lesson,
            "Unrelated",
            999 * DAY,
        );
        let options = ApplyOptions {
            repeats: vec![
                RepeatRequest {
                    meaning: "한국어로 답하기".into(),
                    scope: RepeatScope::Personal,
                    procedure: false,
                    target_id: Some(id_of(&store, preference)),
                    line: "Answer in Korean.".into(),
                    sources: vec![(7, 1_000 * DAY, Some("w1".into()))],
                },
                RepeatRequest {
                    meaning: "x".repeat(MEANING_CHARS + 1),
                    scope: RepeatScope::Project,
                    procedure: false,
                    target_id: None,
                    line: "Too long a meaning".into(),
                    sources: vec![(8, 1_000 * DAY, None)],
                },
            ],
            ..ApplyOptions::default()
        };
        let summary = said_again(&mut store, "r1", "s2", other, 1, &[], &options);
        assert_eq!(summary.repeats_left_out, 1);
        let sources = store.repeat_sources("p").unwrap();
        let sessions = sources
            .iter()
            .map(|source| (source.session.as_str(), source.from_card))
            .collect::<Vec<_>>();
        assert_eq!(sessions, [("claude:s2", false), ("claude:s1", true)]);
        assert!(sources.iter().all(|source| source.card == Some(preference)));
        assert_eq!(store.known_meanings("p", 10).unwrap(), ["한국어로 답하기"]);
    }
}
