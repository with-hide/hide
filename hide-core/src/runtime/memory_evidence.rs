//! What a card rests on, read from the node that holds the checkout (PRD
//! memory-card-quality D-05, D-06).
//!
//! At write time the files a card cites get a content fingerprint and the
//! repository paths and script names its text writes are kept when they
//! exist. Later a pass asks the node for the same facts and a card whose
//! facts moved is unverified until it is checked again. Every read is a call
//! on the analysis worker; nothing here runs under the runtime lock, and a
//! node that cannot answer leaves a card as it was.

use std::collections::HashSet;

use hide_memory::{
    EVIDENCE_ROW_LIMIT, Evidence, EvidenceFact, EvidenceKind, MemoryStore, still_holds,
    written_names,
};
use hide_node_link::cleanup::PathState;
use hide_node_link::protocol::{Call, RevisionNow};
use hide_node_link::{ErrorCode, LinkError};
use serde_json::json;

use super::memory::SESSION_CALL_TIMEOUT;
use crate::node_access::{NodeLink, call_as, index_root, pinned_root};

/// Cards whose evidence one Project's walk compares with the node.
pub(super) const CARD_CHECKS_PER_WALK: usize = 200;
/// File fingerprints one Project's walk asks the node for when it compares
/// cards, over all cards; a card whose file is past it keeps its state until
/// a later walk.
const REVISION_READS_PER_WALK: usize = 300;
/// File fingerprints one Project's walk asks the node for when it writes a
/// card's evidence. Apart from the comparison's, so a long comparison never
/// leaves a new card without the fingerprints its check rests on.
const WRITE_READS_PER_WALK: usize = 200;
/// Cards the evidence comparison looks at again no sooner than this.
pub(super) const EVIDENCE_RECHECK_MS: u64 = 10 * 60 * 1000;

/// The node's answer for one file's content revision.
enum Revision {
    Found(String),
    Missing,
    /// The node could not read it (a folder, too large, a lost link).
    Unreadable,
}

/// The script names of the checkout, as the node's file index lists them.
enum Names {
    Complete(HashSet<String>),
    /// The index stopped at its cap: a name it lacks may still exist.
    Truncated(HashSet<String>),
    Unreadable,
}

/// Facts about one checkout, read once and reused through a walk.
pub(crate) struct CheckoutFacts<'a> {
    node: &'a dyn NodeLink,
    root: &'a str,
    names: Option<Names>,
    revisions_left: usize,
    writes_left: usize,
}

impl<'a> CheckoutFacts<'a> {
    pub(crate) fn new(node: &'a dyn NodeLink, root: &'a str) -> Self {
        Self {
            node,
            root,
            names: None,
            revisions_left: REVISION_READS_PER_WALK,
            writes_left: WRITE_READS_PER_WALK,
        }
    }

    /// The evidence a card with this text and these cited paths rests on:
    /// each cited file that exists with its fingerprint, each cited folder
    /// and written path that exists, and each written script name the index
    /// lists. A name found nowhere is not evidence, so only what held when
    /// the card was written can break later.
    pub(crate) fn gather(&mut self, body: &str, cited: &[String]) -> Vec<Evidence> {
        let written = written_names(body);
        let mut wanted: Vec<String> = cited.to_vec();
        for path in &written.paths {
            if !wanted.contains(path) {
                wanted.push(path.clone());
            }
        }
        wanted.truncate(EVIDENCE_ROW_LIMIT);
        let states = self.real_paths(&wanted);
        let mut rows = Vec::new();
        for (path, state) in wanted.iter().zip(states) {
            if matches!(state, Some(PathState::Missing)) {
                // A name that looks like a file (it has an extension) and is
                // not there is kept as not found, so `show` can say so; prose
                // such as `Pin/Unpin` has none and is not kept.
                if path
                    .rsplit('/')
                    .next()
                    .is_some_and(|name| name.contains('.'))
                    && rows.len() < EVIDENCE_ROW_LIMIT
                {
                    rows.push(Evidence {
                        kind: EvidenceKind::Path,
                        value: path.clone(),
                        fingerprint: None,
                        present: false,
                    });
                }
                continue;
            }
            if !matches!(state, Some(PathState::Real { .. })) {
                continue;
            }
            // A file is fingerprinted; a folder, or a file the node cannot
            // read, is kept as a path that exists.
            let fingerprint = match cited.contains(path) {
                true => match self.revision(path, true) {
                    Revision::Found(revision) => Some(revision),
                    Revision::Missing | Revision::Unreadable => None,
                },
                false => None,
            };
            rows.push(Evidence {
                kind: if fingerprint.is_some() {
                    EvidenceKind::File
                } else {
                    EvidenceKind::Path
                },
                value: path.clone(),
                fingerprint,
                present: true,
            });
        }
        for name in &written.commands {
            if rows.len() >= EVIDENCE_ROW_LIMIT {
                break;
            }
            match self.name_fact(name) {
                EvidenceFact::Found { .. } => rows.push(Evidence {
                    kind: EvidenceKind::Command,
                    value: name.clone(),
                    fingerprint: None,
                    present: true,
                }),
                EvidenceFact::Missing => rows.push(Evidence {
                    kind: EvidenceKind::Command,
                    value: name.clone(),
                    fingerprint: None,
                    present: false,
                }),
                EvidenceFact::Unknown => {}
            }
        }
        rows.truncate(EVIDENCE_ROW_LIMIT);
        rows
    }

    /// Which of `paths` exist inside the root, in order.
    pub(crate) fn existing(&self, paths: &[String]) -> Vec<String> {
        paths
            .iter()
            .zip(self.real_paths(paths))
            .filter(|(_, state)| matches!(state, Some(PathState::Real { .. })))
            .map(|(path, _)| path.clone())
            .collect()
    }

    /// What the node answers now for each of `rows`, in order.
    pub(crate) fn facts(&mut self, rows: &[Evidence]) -> Vec<EvidenceFact> {
        let paths = rows
            .iter()
            .filter(|row| row.kind != EvidenceKind::Command)
            .map(|row| row.value.clone())
            .collect::<Vec<_>>();
        let mut states = self.real_paths(&paths).into_iter();
        rows.iter()
            .map(|row| match row.kind {
                EvidenceKind::Command => self.name_fact(&row.value),
                EvidenceKind::Path | EvidenceKind::File => match states.next().flatten() {
                    None | Some(PathState::Unreadable { .. }) => EvidenceFact::Unknown,
                    Some(PathState::Missing) => EvidenceFact::Missing,
                    Some(PathState::Real { .. }) if row.kind == EvidenceKind::Path => {
                        EvidenceFact::Found { fingerprint: None }
                    }
                    Some(PathState::Real { .. }) => match self.revision(&row.value, false) {
                        Revision::Found(revision) => EvidenceFact::Found {
                            fingerprint: Some(revision),
                        },
                        Revision::Missing => EvidenceFact::Missing,
                        Revision::Unreadable => EvidenceFact::Unknown,
                    },
                },
            })
            .collect()
    }

    /// Each of `relative` as the node resolves it inside the root: `None`
    /// when the node could not say, which keeps a card as it was. A path that
    /// leaves the root counts as missing.
    fn real_paths(&self, relative: &[String]) -> Vec<Option<PathState>> {
        if relative.is_empty() {
            return Vec::new();
        }
        let mut paths = vec![self.root.to_owned()];
        paths.extend(
            relative
                .iter()
                .map(|path| format!("{}/{path}", self.root.trim_end_matches('/'))),
        );
        let states =
            call_as::<Vec<PathState>>(self.node, Call::RealPaths { paths }, SESSION_CALL_TIMEOUT);
        let unknown = || vec![None; relative.len()];
        let Ok(states) = states else {
            return unknown();
        };
        if states.len() != relative.len() + 1 {
            return unknown();
        }
        let root = match &states[0] {
            PathState::Real { path } => format!("{}/", path.trim_end_matches('/')),
            _ => return unknown(),
        };
        states[1..]
            .iter()
            .map(|state| {
                Some(match state {
                    PathState::Real { path } if !path.starts_with(&root) => PathState::Missing,
                    other => other.clone(),
                })
            })
            .collect()
    }

    fn revision(&mut self, relative: &str, writing: bool) -> Revision {
        let left = if writing {
            &mut self.writes_left
        } else {
            &mut self.revisions_left
        };
        if *left == 0 {
            return Revision::Unreadable;
        }
        *left -= 1;
        let root = match pinned_root(self.node, self.root, SESSION_CALL_TIMEOUT) {
            Ok(root) => root,
            Err(_) => return Revision::Unreadable,
        };
        match call_as::<RevisionNow>(
            self.node,
            Call::Revision {
                root,
                path: relative.to_owned(),
            },
            SESSION_CALL_TIMEOUT,
        ) {
            Ok(now) => Revision::Found(now.revision),
            Err(LinkError::Refused(error)) if error.code == ErrorCode::NotFound => {
                Revision::Missing
            }
            Err(_) => Revision::Unreadable,
        }
    }

    /// Whether the checkout has a file with this name anywhere the index
    /// lists, one walk per `CheckoutFacts`.
    fn name_fact(&mut self, name: &str) -> EvidenceFact {
        let names = self
            .names
            .get_or_insert_with(|| match index_root(self.node, self.root) {
                Ok(walked) => {
                    let set = walked
                        .paths
                        .iter()
                        .filter_map(|path| path.rsplit('/').next())
                        .map(str::to_owned)
                        .collect::<HashSet<_>>();
                    if walked.truncated {
                        Names::Truncated(set)
                    } else {
                        Names::Complete(set)
                    }
                }
                Err(_) => Names::Unreadable,
            });
        match names {
            Names::Complete(set) if set.contains(name) => EvidenceFact::Found { fingerprint: None },
            Names::Complete(_) => EvidenceFact::Missing,
            Names::Truncated(set) if set.contains(name) => {
                EvidenceFact::Found { fingerprint: None }
            }
            Names::Truncated(_) | Names::Unreadable => EvidenceFact::Unknown,
        }
    }
}

/// Compares the evidence of the Project's verified cards with the node and
/// unverifies each card whose facts moved (D-06). Needs no model, so it runs
/// with Hide AI off. Answers how many cards it unverified.
pub(super) fn check_evidence(
    store: &mut MemoryStore,
    facts: &mut CheckoutFacts<'_>,
    project_id: &str,
    now_unix_ms: u64,
) -> Result<usize, String> {
    let cards = store
        .cards_to_check(
            project_id,
            now_unix_ms.saturating_sub(EVIDENCE_RECHECK_MS),
            CARD_CHECKS_PER_WALK,
        )
        .map_err(|error| error.to_string())?;
    let mut moved = 0;
    let mut examined = Vec::with_capacity(cards.len());
    for card in cards {
        let answers = facts.facts(&card.rows);
        let broken = card
            .rows
            .iter()
            .zip(&answers)
            .any(|(row, fact)| still_holds(row, fact) == Some(false));
        if broken
            && store
                .mark_unverified(&card.item_id, card.revision)
                .map_err(|error| error.to_string())?
        {
            moved += 1;
            crate::diagnostic!(json!({
                "component": "memory",
                "kind": "evidence.moved",
                "project_id": project_id,
                "card": card.number,
            }));
        }
        examined.push(card.item_id);
    }
    store
        .touch_checked(&examined, now_unix_ms)
        .map_err(|error| error.to_string())?;
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hide_node_link::{LinkAnswer, RootIdentity};
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::Duration;

    /// A node whose files are a name-to-content map; it answers the four
    /// calls evidence uses and counts them.
    struct Files {
        files: Mutex<HashMap<String, String>>,
        folders: Vec<String>,
        calls: Mutex<Vec<&'static str>>,
        truncated: bool,
    }

    impl Files {
        fn new(files: &[(&str, &str)], folders: &[&str]) -> Self {
            Self {
                files: Mutex::new(
                    files
                        .iter()
                        .map(|(path, text)| ((*path).to_owned(), (*text).to_owned()))
                        .collect(),
                ),
                folders: folders.iter().map(|folder| (*folder).to_owned()).collect(),
                calls: Mutex::new(Vec::new()),
                truncated: false,
            }
        }
    }

    impl NodeLink for Files {
        fn call(&self, call: Call, _: Duration) -> Result<LinkAnswer, LinkError> {
            let files = self.files.lock().unwrap();
            let value = match call {
                Call::RealPaths { paths } => {
                    self.calls.lock().unwrap().push("real_paths");
                    let states = paths
                        .iter()
                        .map(|path| match path.strip_prefix("/repo/") {
                            None if path == "/repo" => PathState::Real {
                                path: "/repo".into(),
                            },
                            Some(relative)
                                if files.contains_key(relative)
                                    || self.folders.iter().any(|f| f == relative) =>
                            {
                                PathState::Real { path: path.clone() }
                            }
                            _ => PathState::Missing,
                        })
                        .collect::<Vec<_>>();
                    serde_json::to_value(states).unwrap()
                }
                Call::Revision { path, .. } => {
                    self.calls.lock().unwrap().push("revision");
                    match files.get(&path) {
                        Some(text) => json!({"revision": format!("sha256:{text}")}),
                        None => {
                            return Err(LinkError::Refused(hide_node_link::HostError::new(
                                ErrorCode::NotFound,
                                "gone",
                            )));
                        }
                    }
                }
                Call::Index { .. } => {
                    self.calls.lock().unwrap().push("index");
                    json!({"paths": files.keys().collect::<Vec<_>>(), "truncated": self.truncated})
                }
                Call::RootOpen { .. } => {
                    json!({"identity": RootIdentity { device: 1, inode: 1 }})
                }
                _ => serde_json::Value::Null,
            };
            Ok(value.into())
        }

        fn pinned(&self, _: &str) -> Option<RootIdentity> {
            Some(RootIdentity {
                device: 1,
                inode: 1,
            })
        }

        fn pin(&self, _: &str, _: Option<RootIdentity>) {}
    }

    #[test]
    fn a_card_rests_on_the_files_paths_and_scripts_that_exist_and_breaks_when_one_moves() {
        let node = Files::new(
            &[
                ("docs/BUILD.md", "v1"),
                ("scripts/verify-web.sh", ""),
                ("scripts/gone.sh", ""),
            ],
            &["macos"],
        );
        node.files.lock().unwrap().remove("scripts/gone.sh");
        let mut facts = CheckoutFacts::new(&node, "/repo");

        let rows = facts.gather(
            "Run `scripts/verify-web.sh` not scripts/never.sh; macos/ is gone from docs/BUILD.md, \
             use build_dev_app.sh or verify-web.sh",
            &["docs/BUILD.md".to_owned()],
        );

        let described = rows
            .iter()
            .map(|row| {
                (
                    row.kind.as_str(),
                    row.value.as_str(),
                    row.fingerprint.is_some(),
                    row.present,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            described,
            [
                ("file", "docs/BUILD.md", true, true),
                ("path", "scripts/verify-web.sh", false, true),
                ("path", "scripts/never.sh", false, false),
                ("path", "macos", false, true),
                ("command", "build_dev_app.sh", false, false),
                ("command", "verify-web.sh", false, true),
            ],
            "a file-like name found nowhere is kept as not found, prose is not"
        );
        assert!(
            facts
                .facts(&rows)
                .iter()
                .zip(&rows)
                .all(|(fact, row)| still_holds(row, fact) == Some(true))
        );

        node.files
            .lock()
            .unwrap()
            .insert("docs/BUILD.md".into(), "v2".into());
        node.files.lock().unwrap().remove("scripts/verify-web.sh");
        let mut later = CheckoutFacts::new(&node, "/repo");
        let broken = later
            .facts(&rows)
            .iter()
            .zip(&rows)
            .filter(|(fact, row)| still_holds(row, fact) == Some(false))
            .map(|(_, row)| row.value.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            broken,
            ["docs/BUILD.md", "scripts/verify-web.sh", "verify-web.sh"]
        );
    }

    #[test]
    fn a_node_that_cannot_answer_leaves_every_card_as_it_was() {
        struct Down;
        impl NodeLink for Down {
            fn call(&self, _: Call, _: Duration) -> Result<LinkAnswer, LinkError> {
                Err(LinkError::NotConnected("down".into()))
            }
            fn pinned(&self, _: &str) -> Option<RootIdentity> {
                None
            }
            fn pin(&self, _: &str, _: Option<RootIdentity>) {}
        }
        let rows = [
            Evidence {
                kind: EvidenceKind::File,
                value: "docs/a.md".into(),
                fingerprint: Some("sha256:1".into()),
                present: true,
            },
            Evidence {
                kind: EvidenceKind::Command,
                value: "a.sh".into(),
                fingerprint: None,
                present: true,
            },
        ];
        let mut facts = CheckoutFacts::new(&Down, "/repo");
        let answers = facts.facts(&rows);
        assert_eq!(answers, [EvidenceFact::Unknown, EvidenceFact::Unknown]);
        assert!(facts.gather("see docs/a.md and a.sh", &[]).is_empty());
    }

    #[test]
    fn a_truncated_index_cannot_say_a_script_is_gone() {
        let mut node = Files::new(&[("scripts/a.sh", "")], &[]);
        node.truncated = true;
        let mut facts = CheckoutFacts::new(&node, "/repo");
        assert_eq!(
            facts.name_fact("a.sh"),
            EvidenceFact::Found { fingerprint: None }
        );
        assert_eq!(facts.name_fact("b.sh"), EvidenceFact::Unknown);
    }
}
