//! The link record's reads for `hide session` (PRD agent-session-read D-08,
//! D-09, D-11): which sessions belong to a set of Projects, one page of them
//! newest first, and where one session's file is. Every read is on the
//! read-only connection a reader opens, off the runtime's lock.

use super::{LinkStore, SessionKey, failed};
use crate::links::ledger::{LEDGER_DEPTH_LIMIT, RecordStatus, SessionIdentity};
use rusqlite::{OptionalExtension, params};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// One line of `hide session list`, as the record holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedSession {
    pub session: SessionIdentity,
    pub cwd: Option<String>,
    pub started_at_unix_ms: Option<u64>,
    pub ended_at_unix_ms: Option<u64>,
    /// The latest of its last request, its end and its start, as History
    /// orders sessions.
    pub activity_unix_ms: u64,
    pub native_title: Option<String>,
    pub manual_title: Option<String>,
    /// The flow the operator asked the most in, the later one on a tie, as
    /// the record names a session; none while it has no flow.
    pub main_flow: Option<String>,
    /// Its requests, letters left out.
    pub request_count: usize,
    /// The pull requests it made, newest first.
    pub prs: Vec<u64>,
    pub status: Option<RecordStatus>,
    pub file_gone: bool,
}

/// One page of a scope's sessions and how many more the period holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionPage {
    pub sessions: Vec<ListedSession>,
    pub more: usize,
}

/// Where one session's file is, as the device that holds it spells it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionFile {
    pub path: Option<String>,
    pub cwd: Option<String>,
    pub file_gone: bool,
}

/// What a scope may hold at most: past it the read fails instead of growing.
const MEMBER_LIMIT: usize = 100_000;

impl LinkStore {
    /// Every session of the Projects `keys` name: a session whose cwd is
    /// inside a Project's root or one of its recorded worktrees on the
    /// Project's own device, or whose recorded original project is one of
    /// them, and every descendant of those on any device.
    pub fn scope_members(&self, keys: &[String]) -> Result<BTreeSet<SessionIdentity>, String> {
        let mut owners = HashSet::new();
        for key in keys {
            let Some(project) = self.project(key)? else {
                continue;
            };
            let root = project.root.trim_end_matches('/').to_owned();
            let mut statement = self
                .connection
                .prepare_cached("SELECT path FROM worktrees WHERE project=?1")
                .map_err(failed)?;
            let worktrees = statement
                .query_map([key], |r| r.get::<_, String>(0))
                .map_err(failed)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(failed)?;
            owners.insert((project.device.clone(), root));
            for path in worktrees {
                owners.insert((
                    project.device.clone(),
                    path.trim_end_matches('/').to_owned(),
                ));
            }
        }
        owners.retain(|(_, path)| !path.is_empty());
        let mut statement = self
            .connection
            .prepare(
                "SELECT s.device,s.agent,s.id,s.cwd,o.project FROM sessions s \
                 LEFT JOIN ledger_session_origins o USING(device,agent,id)",
            )
            .map_err(failed)?;
        let rows = statement
            .query_map([], |r| {
                Ok((
                    SessionIdentity {
                        device_id: r.get(0)?,
                        agent: r.get(1)?,
                        session_id: r.get(2)?,
                    },
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                ))
            })
            .map_err(failed)?;
        let owned = |device: &str, path: &str| {
            let mut path = path.trim_end_matches('/');
            loop {
                if owners.contains(&(device.to_owned(), path.to_owned())) {
                    return true;
                }
                match path.rfind('/') {
                    Some(at) if at > 0 => path = &path[..at],
                    _ => return false,
                }
            }
        };
        let mut members = BTreeSet::new();
        for row in rows {
            let (session, cwd, origin) = row.map_err(failed)?;
            let device = session.device_id.as_str();
            let mine = origin.as_deref().is_some_and(|root| {
                owners.contains(&(device.to_owned(), root.trim_end_matches('/').to_owned()))
            }) || cwd.as_deref().is_some_and(|cwd| owned(device, cwd));
            if mine {
                members.insert(session);
                if members.len() > MEMBER_LIMIT {
                    return Err("session_scope_capacity".into());
                }
            }
        }
        let mut statement = self
            .connection
            .prepare(
                "SELECT device,agent,id,parent_device,parent_agent,parent_id FROM session_parents",
            )
            .map_err(failed)?;
        let mut children: HashMap<SessionIdentity, Vec<SessionIdentity>> = HashMap::new();
        for row in statement
            .query_map([], |r| {
                Ok((
                    SessionIdentity {
                        device_id: r.get(0)?,
                        agent: r.get(1)?,
                        session_id: r.get(2)?,
                    },
                    SessionIdentity {
                        device_id: r.get(3)?,
                        agent: r.get(4)?,
                        session_id: r.get(5)?,
                    },
                ))
            })
            .map_err(failed)?
        {
            let (child, parent) = row.map_err(failed)?;
            children.entry(parent).or_default().push(child);
        }
        let mut level: Vec<SessionIdentity> = members.iter().cloned().collect();
        for _ in 0..LEDGER_DEPTH_LIMIT {
            let mut next = Vec::new();
            for parent in &level {
                for child in children.get(parent).into_iter().flatten() {
                    if members.insert(child.clone()) {
                        next.push(child.clone());
                    }
                }
            }
            if members.len() > MEMBER_LIMIT {
                return Err("session_scope_capacity".into());
            }
            if next.is_empty() {
                break;
            }
            level = next;
        }
        Ok(members)
    }

    /// The sessions of `members` active at or after `since`, newest first,
    /// at most `limit`, and how many more the period holds.
    pub fn session_page(
        &self,
        members: &BTreeSet<SessionIdentity>,
        since_unix_ms: u64,
        limit: usize,
    ) -> Result<SessionPage, String> {
        // A request the ledger holds is activity even before the file
        // reader dated the session.
        let mut statement = self
            .connection
            .prepare("SELECT device,agent,id,MAX(at) FROM ledger_requests GROUP BY device,agent,id")
            .map_err(failed)?;
        let mut asked: HashMap<SessionIdentity, u64> = HashMap::new();
        for row in statement
            .query_map([], |r| {
                Ok((
                    SessionIdentity {
                        device_id: r.get(0)?,
                        agent: r.get(1)?,
                        session_id: r.get(2)?,
                    },
                    r.get::<_, u64>(3)?,
                ))
            })
            .map_err(failed)?
        {
            let (session, at) = row.map_err(failed)?;
            if members.contains(&session) {
                asked.insert(session, at);
            }
        }
        let mut statement = self
            .connection
            .prepare(
                "SELECT device,agent,id,cwd,started_at,ended_at,last_request_at,file_gone FROM sessions",
            )
            .map_err(failed)?;
        let mut found = Vec::new();
        for row in statement
            .query_map([], |r| {
                Ok((
                    SessionIdentity {
                        device_id: r.get(0)?,
                        agent: r.get(1)?,
                        session_id: r.get(2)?,
                    },
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<u64>>(4)?,
                    r.get::<_, Option<u64>>(5)?,
                    r.get::<_, Option<u64>>(6)?,
                    r.get::<_, bool>(7)?,
                ))
            })
            .map_err(failed)?
        {
            let (session, cwd, started, ended, requested, file_gone) = row.map_err(failed)?;
            if !members.contains(&session) {
                continue;
            }
            let activity = [started, ended, requested, asked.get(&session).copied()]
                .into_iter()
                .flatten()
                .max()
                .unwrap_or(0);
            if activity < since_unix_ms {
                continue;
            }
            found.push((activity, session, cwd, started, ended, file_gone));
        }
        found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        let more = found.len().saturating_sub(limit);
        found.truncate(limit);
        let mut sessions = Vec::with_capacity(found.len());
        for (activity, session, cwd, started, ended, file_gone) in found {
            sessions.push(self.listed(session, cwd, started, ended, activity, file_gone)?);
        }
        Ok(SessionPage { sessions, more })
    }

    fn listed(
        &self,
        session: SessionIdentity,
        cwd: Option<String>,
        started: Option<u64>,
        ended: Option<u64>,
        activity: u64,
        file_gone: bool,
    ) -> Result<ListedSession, String> {
        let args = params![session.device_id, session.agent, session.session_id];
        let (native_title, manual_title) = self
            .connection
            .query_row(
                "SELECT native_title,manual_title FROM ledger_titles WHERE device=?1 AND agent=?2 AND id=?3",
                args,
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(failed)?
            .unwrap_or((None, None));
        let mut statement = self
            .connection
            .prepare_cached(
                "SELECT goal,at FROM ledger_chapters WHERE device=?1 AND agent=?2 AND id=?3 ORDER BY at,rowid",
            )
            .map_err(failed)?;
        let chapters = statement
            .query_map(args, |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))
            .map_err(failed)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(failed)?;
        let mut statement = self
            .connection
            .prepare_cached(
                "SELECT at FROM ledger_requests WHERE device=?1 AND agent=?2 AND id=?3 \
                 AND author='\"operator\"' AND letter_from IS NULL ORDER BY at",
            )
            .map_err(failed)?;
        let requests = statement
            .query_map(args, |r| r.get::<_, u64>(0))
            .map_err(failed)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(failed)?;
        let main_flow = main_flow(&chapters, &requests);
        let mut statement = self
            .connection
            .prepare_cached(
                "SELECT number FROM ledger_outputs WHERE device=?1 AND agent=?2 AND id=?3 AND kind='pr' \
                 ORDER BY created_at DESC LIMIT 20",
            )
            .map_err(failed)?;
        let prs = statement
            .query_map(args, |r| r.get::<_, u64>(0))
            .map_err(failed)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(failed)?;
        let status = self.ledger_status(&session)?;
        Ok(ListedSession {
            session,
            cwd,
            started_at_unix_ms: started,
            ended_at_unix_ms: ended,
            activity_unix_ms: activity,
            native_title,
            manual_title,
            main_flow,
            request_count: requests.len(),
            prs,
            status,
            file_gone,
        })
    }

    /// Every recorded session whose native id is `id`, on any device.
    pub fn sessions_named(&self, id: &str) -> Result<Vec<SessionIdentity>, String> {
        let mut statement = self
            .connection
            .prepare_cached(
                "SELECT device,agent,id FROM sessions WHERE id=?1 ORDER BY device,agent",
            )
            .map_err(failed)?;
        statement
            .query_map([id], |r| {
                Ok(SessionKey {
                    device: r.get(0)?,
                    agent: r.get(1)?,
                    id: r.get(2)?,
                }
                .identity())
            })
            .map_err(failed)?
            .collect::<Result<_, _>>()
            .map_err(failed)
    }

    /// Where `session`'s file is, when the record holds the session.
    pub fn session_file(&self, session: &SessionIdentity) -> Result<Option<SessionFile>, String> {
        self.connection
            .query_row(
                "SELECT path,cwd,file_gone FROM sessions WHERE device=?1 AND agent=?2 AND id=?3",
                params![session.device_id, session.agent, session.session_id],
                |r| {
                    Ok(SessionFile {
                        path: r.get(0)?,
                        cwd: r.get(1)?,
                        file_gone: r.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(failed)
    }
}

/// The flow with the most requests, the later one on a tie; a request before
/// the first flow counts toward the first, as the record places it.
fn main_flow(chapters: &[(String, u64)], requests: &[u64]) -> Option<String> {
    let mut counts: BTreeMap<usize, usize> = BTreeMap::new();
    for at in requests {
        let index = chapters
            .iter()
            .rposition(|(_, start)| start <= at)
            .unwrap_or(0);
        *counts.entry(index).or_default() += 1;
    }
    let best = (0..chapters.len())
        .max_by_key(|index| (counts.get(index).copied().unwrap_or(0), *index))?;
    chapters.get(best).map(|(goal, _)| goal.clone())
}

#[cfg(test)]
mod tests {
    use super::main_flow;

    #[test]
    fn the_main_flow_is_the_one_asked_most_the_later_on_a_tie() {
        let chapters = vec![
            ("a".to_owned(), 10),
            ("b".to_owned(), 20),
            ("c".to_owned(), 30),
        ];
        assert_eq!(main_flow(&chapters, &[5, 11, 21, 31]).as_deref(), Some("a"));
        assert_eq!(main_flow(&chapters, &[11, 21]).as_deref(), Some("b"));
        assert_eq!(main_flow(&chapters, &[]).as_deref(), Some("c"));
        assert_eq!(main_flow(&[], &[1, 2]), None);
    }
}
