//! What the operator chose on the Overview's Work map (PRD
//! overview-graph-remembered D-05..D-09): the filter, the projects hidden and
//! folded, the teams folded or opened by hand, and where projects and
//! sessions were dragged. It is `ui_state.overview_graph`, so it outlives the
//! view, the window and a relaunch, and every window reads the same value.
//!
//! Each record names its machine, project and pane in their own fields, so a
//! core move rekeys them through `node_migration::KEYS` like any other UI
//! state. Every list keeps the most recently changed record last and holds at
//! most [`OVERVIEW_GRAPH_LIST_LIMIT`]; past it the least recently changed goes.

use serde::{Deserialize, Serialize};

pub const OVERVIEW_GRAPH_LIST_LIMIT: usize = 512;

/// Which sessions the map lights (`contracts/snapshot-wire-enums.json`:
/// `overview_graph_focus`).
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OverviewGraphFocus {
    #[default]
    All,
    Turn,
    Working,
    Results,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GraphProjectKey {
    pub device_id: String,
    pub workspace_id: String,
}

/// A session as the map names it: its machine, its Herdr pane, and the
/// digest of its native session (or the native id where no digest is known),
/// so a pane Herdr reuses for another session is not taken for this one.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GraphSessionKey {
    pub device_id: String,
    pub pane_id: String,
    pub session: Option<String>,
}

/// Which map a place belongs to: one device's map of every project, or one
/// project's own map (D-07).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GraphScope {
    All {
        device_id: String,
    },
    Project {
        device_id: String,
        workspace_id: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GraphTeamFold {
    pub session: GraphSessionKey,
    pub folded: bool,
}

/// Where something was dropped, in map units at an interface scale of one
/// (D-08).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GraphPlace<K> {
    pub scope: GraphScope,
    pub key: K,
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct OverviewGraphChoices {
    #[serde(default)]
    pub focus: OverviewGraphFocus,
    #[serde(default)]
    pub include_resting: bool,
    #[serde(default)]
    pub hidden_projects: Vec<GraphProjectKey>,
    #[serde(default)]
    pub folded_projects: Vec<GraphProjectKey>,
    /// Only the teams the operator folded or opened; one nobody chose for
    /// opens folded when every descendant sleeps.
    #[serde(default)]
    pub team_folds: Vec<GraphTeamFold>,
    #[serde(default)]
    pub project_places: Vec<GraphPlace<GraphProjectKey>>,
    #[serde(default)]
    pub session_places: Vec<GraphPlace<GraphSessionKey>>,
}

/// A position the shell dropped something at, without its scope.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GraphDrop<K> {
    pub key: K,
    pub x: f64,
    pub y: f64,
}

/// One operator action on the map (D-10).
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OverviewGraphChange {
    /// A filter button, the resting-sessions box, or Show all sessions.
    Focus {
        focus: OverviewGraphFocus,
        include_resting: bool,
    },
    ProjectHidden {
        project: GraphProjectKey,
        hidden: bool,
    },
    ProjectFolded {
        project: GraphProjectKey,
        folded: bool,
    },
    /// The toolbar's Fold all: every project of the map shown.
    FoldAll { projects: Vec<GraphProjectKey> },
    /// The toolbar's Unfold all: the map's projects and teams open.
    UnfoldAll {
        projects: Vec<GraphProjectKey>,
        teams: Vec<GraphSessionKey>,
    },
    TeamFolded {
        session: GraphSessionKey,
        folded: bool,
    },
    /// One drop: a session, or a project with the pinned sessions it carried.
    Place {
        scope: GraphScope,
        #[serde(default)]
        projects: Vec<GraphDrop<GraphProjectKey>>,
        #[serde(default)]
        sessions: Vec<GraphDrop<GraphSessionKey>>,
    },
    /// Reset layout: the scope's places are forgotten.
    ResetPlaces { scope: GraphScope },
}

/// What a change did: whether anything moved, and how many records the
/// caps dropped.
#[derive(Debug, Default, Eq, PartialEq)]
pub struct Applied {
    pub changed: bool,
    pub forgotten: usize,
}

impl OverviewGraphChoices {
    /// Applies one change. A change naming an empty id or an unplaceable
    /// position is refused whole, so nothing half applies.
    pub fn apply(&mut self, change: OverviewGraphChange) -> Result<Applied, String> {
        change.validate()?;
        let before = self.clone();
        let mut forgotten = 0;
        match change {
            OverviewGraphChange::Focus {
                focus,
                include_resting,
            } => {
                self.focus = focus;
                self.include_resting = include_resting;
            }
            OverviewGraphChange::ProjectHidden { project, hidden } => {
                forgotten += set_member(&mut self.hidden_projects, project, hidden);
            }
            OverviewGraphChange::ProjectFolded { project, folded } => {
                forgotten += set_member(&mut self.folded_projects, project, folded);
            }
            OverviewGraphChange::FoldAll { projects } => {
                for project in projects {
                    forgotten += set_member(&mut self.folded_projects, project, true);
                }
            }
            OverviewGraphChange::UnfoldAll { projects, teams } => {
                for project in projects {
                    set_member(&mut self.folded_projects, project, false);
                }
                for session in teams {
                    forgotten += set_team(&mut self.team_folds, session, false);
                }
            }
            OverviewGraphChange::TeamFolded { session, folded } => {
                forgotten += set_team(&mut self.team_folds, session, folded);
            }
            OverviewGraphChange::Place {
                scope,
                projects,
                sessions,
            } => {
                for drop in projects {
                    forgotten += set_place(&mut self.project_places, &scope, drop);
                }
                for drop in sessions {
                    forgotten += set_place(&mut self.session_places, &scope, drop);
                }
            }
            OverviewGraphChange::ResetPlaces { scope } => {
                self.project_places.retain(|place| place.scope != scope);
                self.session_places.retain(|place| place.scope != scope);
            }
        }
        Ok(Applied {
            changed: *self != before,
            forgotten,
        })
    }

    /// A stored value is read record by record: a record this build cannot
    /// read, or one a core move left naming nothing, is left out and counted,
    /// and the rest load (D-15). A value that is not an object at all is
    /// unreadable whole.
    pub fn read_stored(value: serde_json::Value) -> Result<(Self, usize), String> {
        #[derive(Deserialize)]
        struct Stored {
            #[serde(default)]
            focus: Option<serde_json::Value>,
            #[serde(default)]
            include_resting: Option<serde_json::Value>,
            #[serde(default)]
            hidden_projects: Vec<serde_json::Value>,
            #[serde(default)]
            folded_projects: Vec<serde_json::Value>,
            #[serde(default)]
            team_folds: Vec<serde_json::Value>,
            #[serde(default)]
            project_places: Vec<serde_json::Value>,
            #[serde(default)]
            session_places: Vec<serde_json::Value>,
        }
        let stored: Stored = serde_json::from_value(value).map_err(|error| error.to_string())?;
        let mut skipped = 0;
        let field = |value: Option<serde_json::Value>| -> Option<serde_json::Value> {
            value.filter(|value| !value.is_null())
        };
        let focus = match field(stored.focus).map(serde_json::from_value) {
            None => OverviewGraphFocus::default(),
            Some(Ok(focus)) => focus,
            Some(Err(_)) => {
                skipped += 1;
                OverviewGraphFocus::default()
            }
        };
        let include_resting = match field(stored.include_resting).map(serde_json::from_value) {
            None => false,
            Some(Ok(value)) => value,
            Some(Err(_)) => {
                skipped += 1;
                false
            }
        };
        let mut choices = Self {
            focus,
            include_resting,
            hidden_projects: readable(
                stored.hidden_projects,
                &mut skipped,
                |key: &GraphProjectKey| key.valid(),
            ),
            folded_projects: readable(
                stored.folded_projects,
                &mut skipped,
                |key: &GraphProjectKey| key.valid(),
            ),
            team_folds: readable(stored.team_folds, &mut skipped, |fold: &GraphTeamFold| {
                fold.session.valid()
            }),
            project_places: readable(stored.project_places, &mut skipped, place_valid),
            session_places: readable(stored.session_places, &mut skipped, place_valid),
        };
        // A hand-edited store over the cap keeps its most recent records.
        for overflow in [
            trim(&mut choices.hidden_projects),
            trim(&mut choices.folded_projects),
            trim(&mut choices.team_folds),
            trim(&mut choices.project_places),
            trim(&mut choices.session_places),
        ] {
            skipped += overflow;
        }
        Ok((choices, skipped))
    }
}

fn readable<T: serde::de::DeserializeOwned>(
    values: Vec<serde_json::Value>,
    skipped: &mut usize,
    valid: impl Fn(&T) -> bool,
) -> Vec<T> {
    values
        .into_iter()
        .filter_map(|value| match serde_json::from_value::<T>(value) {
            Ok(record) if valid(&record) => Some(record),
            _ => {
                *skipped += 1;
                None
            }
        })
        .collect()
}

/// Past the cap the least recently changed records go; how many went.
fn trim<T>(list: &mut Vec<T>) -> usize {
    let over = list.len().saturating_sub(OVERVIEW_GRAPH_LIST_LIMIT);
    list.drain(..over);
    over
}

/// A set kept in order of change: a member added or added again moves last.
fn set_member(list: &mut Vec<GraphProjectKey>, key: GraphProjectKey, member: bool) -> usize {
    let at = list.iter().position(|held| *held == key);
    match (at, member) {
        (Some(index), true) if index + 1 == list.len() => 0,
        (Some(index), true) => {
            let key = list.remove(index);
            list.push(key);
            0
        }
        (None, true) => {
            list.push(key);
            trim(list)
        }
        (Some(index), false) => {
            list.remove(index);
            0
        }
        (None, false) => 0,
    }
}

fn set_team(list: &mut Vec<GraphTeamFold>, session: GraphSessionKey, folded: bool) -> usize {
    if list
        .last()
        .is_some_and(|last| last.session == session && last.folded == folded)
    {
        return 0;
    }
    list.retain(|held| held.session != session);
    list.push(GraphTeamFold { session, folded });
    trim(list)
}

fn set_place<K: PartialEq>(
    list: &mut Vec<GraphPlace<K>>,
    scope: &GraphScope,
    drop: GraphDrop<K>,
) -> usize {
    list.retain(|held| !(held.scope == *scope && held.key == drop.key));
    list.push(GraphPlace {
        scope: scope.clone(),
        key: drop.key,
        x: drop.x,
        y: drop.y,
    });
    trim(list)
}

fn named(text: &str) -> bool {
    !text.is_empty()
}

impl GraphScope {
    fn valid(&self) -> bool {
        match self {
            Self::All { device_id } => named(device_id),
            Self::Project {
                device_id,
                workspace_id,
            } => named(device_id) && named(workspace_id),
        }
    }
}

trait GraphKey {
    fn valid(&self) -> bool;
}

impl GraphKey for GraphProjectKey {
    fn valid(&self) -> bool {
        named(&self.device_id) && named(&self.workspace_id)
    }
}

impl GraphKey for GraphSessionKey {
    fn valid(&self) -> bool {
        named(&self.device_id) && named(&self.pane_id) && self.session.as_deref().is_none_or(named)
    }
}

fn place_valid<K: GraphKey>(place: &GraphPlace<K>) -> bool {
    place.scope.valid() && place.key.valid() && place.x.is_finite() && place.y.is_finite()
}

impl OverviewGraphChange {
    fn validate(&self) -> Result<(), String> {
        let projects_valid = |projects: &[GraphProjectKey]| projects.iter().all(GraphKey::valid);
        let drops_valid = |x: f64, y: f64| x.is_finite() && y.is_finite();
        let valid = match self {
            Self::Focus { .. } => true,
            Self::ProjectHidden { project, .. } | Self::ProjectFolded { project, .. } => {
                project.valid()
            }
            Self::FoldAll { projects } => projects_valid(projects),
            Self::UnfoldAll { projects, teams } => {
                projects_valid(projects) && teams.iter().all(GraphKey::valid)
            }
            Self::TeamFolded { session, .. } => session.valid(),
            Self::Place {
                scope,
                projects,
                sessions,
            } => {
                scope.valid()
                    && projects
                        .iter()
                        .all(|drop| drop.key.valid() && drops_valid(drop.x, drop.y))
                    && sessions
                        .iter()
                        .all(|drop| drop.key.valid() && drops_valid(drop.x, drop.y))
            }
            Self::ResetPlaces { scope } => scope.valid(),
        };
        if valid {
            Ok(())
        } else {
            Err("an Overview map change named an empty id or a position off the map".to_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn project(id: &str) -> GraphProjectKey {
        GraphProjectKey {
            device_id: "node-a".into(),
            workspace_id: format!("workspace:{id}"),
        }
    }

    fn session(pane: &str) -> GraphSessionKey {
        GraphSessionKey {
            device_id: "node-a".into(),
            pane_id: pane.into(),
            session: Some("digest".into()),
        }
    }

    fn all() -> GraphScope {
        GraphScope::All {
            device_id: "node-a".into(),
        }
    }

    fn scoped(id: &str) -> GraphScope {
        GraphScope::Project {
            device_id: "node-a".into(),
            workspace_id: format!("workspace:{id}"),
        }
    }

    fn change(value: serde_json::Value) -> OverviewGraphChange {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn a_change_that_moves_nothing_reports_no_change() {
        let mut choices = OverviewGraphChoices::default();
        let hide = OverviewGraphChange::ProjectHidden {
            project: project("a"),
            hidden: true,
        };
        assert!(choices.apply(hide.clone()).unwrap().changed);
        assert!(!choices.apply(hide).unwrap().changed);
        assert!(
            !choices
                .apply(OverviewGraphChange::Focus {
                    focus: OverviewGraphFocus::All,
                    include_resting: false
                })
                .unwrap()
                .changed
        );
    }

    #[test]
    fn past_the_cap_the_least_recently_changed_record_is_forgotten() {
        let mut choices = OverviewGraphChoices::default();
        for index in 0..OVERVIEW_GRAPH_LIST_LIMIT {
            choices
                .apply(OverviewGraphChange::ProjectFolded {
                    project: project(&index.to_string()),
                    folded: true,
                })
                .unwrap();
        }
        // Folding the first again makes it the most recent.
        choices
            .apply(OverviewGraphChange::ProjectFolded {
                project: project("0"),
                folded: true,
            })
            .unwrap();
        let applied = choices
            .apply(OverviewGraphChange::ProjectFolded {
                project: project("new"),
                folded: true,
            })
            .unwrap();
        assert_eq!(applied.forgotten, 1);
        assert_eq!(choices.folded_projects.len(), OVERVIEW_GRAPH_LIST_LIMIT);
        assert!(choices.folded_projects.contains(&project("0")));
        assert!(!choices.folded_projects.contains(&project("1")));
    }

    #[test]
    fn a_drop_replaces_its_place_in_its_scope_and_a_reset_forgets_only_that_scope() {
        let mut choices = OverviewGraphChoices::default();
        let place = |scope: GraphScope, x: f64| OverviewGraphChange::Place {
            scope,
            projects: vec![GraphDrop {
                key: project("a"),
                x,
                y: 2.0,
            }],
            sessions: vec![GraphDrop {
                key: session("w1:p1"),
                x,
                y: 3.0,
            }],
        };
        choices.apply(place(all(), 1.0)).unwrap();
        choices.apply(place(all(), 5.0)).unwrap();
        choices.apply(place(scoped("a"), 9.0)).unwrap();
        assert_eq!(choices.project_places.len(), 2);
        assert_eq!(choices.session_places.len(), 2);
        assert_eq!(choices.project_places[0].x, 5.0);
        choices.hidden_projects.push(project("b"));
        assert!(
            choices
                .apply(OverviewGraphChange::ResetPlaces { scope: all() })
                .unwrap()
                .changed
        );
        assert_eq!(choices.project_places.len(), 1);
        assert_eq!(choices.project_places[0].scope, scoped("a"));
        assert_eq!(choices.session_places[0].scope, scoped("a"));
        assert_eq!(choices.hidden_projects, vec![project("b")]);
    }

    #[test]
    fn unfold_all_opens_the_listed_projects_and_teams_and_keeps_the_rest() {
        let mut choices = OverviewGraphChoices::default();
        choices
            .apply(change(
                json!({"kind": "fold_all", "projects": [project("a"), project("b"), project("c")]}),
            ))
            .unwrap();
        choices
            .apply(change(
                json!({"kind": "team_folded", "session": session("w1:p1"), "folded": true}),
            ))
            .unwrap();
        choices
            .apply(change(json!({"kind": "unfold_all", "projects": [project("a"), project("b")], "teams": [session("w1:p1"), session("w1:p2")]})))
            .unwrap();
        assert_eq!(choices.folded_projects, vec![project("c")]);
        assert_eq!(
            choices.team_folds,
            vec![
                GraphTeamFold {
                    session: session("w1:p1"),
                    folded: false
                },
                GraphTeamFold {
                    session: session("w1:p2"),
                    folded: false
                },
            ]
        );
    }

    #[test]
    fn a_change_naming_an_empty_id_or_an_unplaceable_position_is_refused_whole() {
        let mut choices = OverviewGraphChoices::default();
        assert!(
            choices
                .apply(OverviewGraphChange::FoldAll {
                    projects: vec![
                        project("a"),
                        GraphProjectKey {
                            device_id: "node-a".into(),
                            workspace_id: String::new()
                        }
                    ],
                })
                .is_err()
        );
        assert!(
            choices
                .apply(OverviewGraphChange::Place {
                    scope: all(),
                    projects: vec![],
                    sessions: vec![GraphDrop {
                        key: session("w1:p1"),
                        x: f64::NAN,
                        y: 0.0
                    }],
                })
                .is_err()
        );
        assert_eq!(choices, OverviewGraphChoices::default());
        assert!(
            serde_json::from_value::<OverviewGraphChange>(
                json!({"kind": "focus", "focus": "later", "include_resting": false})
            )
            .is_err()
        );
    }

    #[test]
    fn a_stored_value_loads_the_records_it_can_read_and_counts_the_rest() {
        let (choices, skipped) = OverviewGraphChoices::read_stored(json!({
            "focus": "turn",
            "include_resting": true,
            "hidden_projects": [project("a"), {"device_id": "node-a", "workspace_id": null}, 7],
            "folded_projects": [project("b")],
            "team_folds": [{"session": session("w1:p1"), "folded": true}],
            "project_places": [{"scope": {"kind": "project", "device_id": "node-a", "workspace_id": null}, "key": project("a"), "x": 1.0, "y": 2.0}],
            "session_places": [{"scope": {"kind": "all", "device_id": "node-a"}, "key": session("w1:p1"), "x": 1.0, "y": 2.0}],
        }))
        .unwrap();
        assert_eq!(skipped, 3);
        assert_eq!(choices.focus, OverviewGraphFocus::Turn);
        assert!(choices.include_resting);
        assert_eq!(choices.hidden_projects, vec![project("a")]);
        assert_eq!(choices.folded_projects, vec![project("b")]);
        assert_eq!(choices.team_folds.len(), 1);
        assert!(choices.project_places.is_empty());
        assert_eq!(choices.session_places.len(), 1);

        let (defaults, skipped) =
            OverviewGraphChoices::read_stored(json!({"focus": "later"})).unwrap();
        assert_eq!((defaults, skipped), (OverviewGraphChoices::default(), 1));
        assert!(OverviewGraphChoices::read_stored(json!(["not", "an", "object"])).is_err());
    }
}
