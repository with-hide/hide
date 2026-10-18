use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};

pub const RECENT_CLOSED_LIMIT: usize = 20;

pub fn push_bounded(stack: &mut VecDeque<ClosedItem>, mut item: ClosedItem) {
    item.clear_start_claims();
    if stack.len() == RECENT_CLOSED_LIMIT {
        stack.pop_front();
    }
    stack.push_back(item);
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClosedContext {
    pub workspace_id: String,
    pub workspace_label: String,
    /// Herdr identities present before the close. A retry inspects only later
    /// identities for Hide's exact reopen-intent marker; being new, having the
    /// same label, or sharing a cwd never proves ownership by itself.
    pub workspace_ids_before_close: Vec<String>,
    pub tab_ids_before_close: Vec<String>,
    pub pane_ids_before_close: Vec<String>,
    pub checkout_id: String,
    pub checkout_path: String,
    pub tab_id: String,
    pub tab_label: String,
    pub tab_index: usize,
    /// Hide-owned area and insertion position, independent of Herdr order.
    pub agent_area: Option<(String, usize)>,
    #[serde(default)]
    pub agent_group: Option<String>,
    /// A shell is made before the close: a new tab when the primary
    /// workspace's last tab closes, or a pane in the closing tab when
    /// `replacement_in_tab` says so.
    pub replacement_shell: bool,
    /// The shell goes into the closing tab beside its last agent, so the tab
    /// and the name the operator gave it stay (docs/UI_BEHAVIOR.md, Agent
    /// areas). Set only together with `replacement_shell`.
    #[serde(default)]
    pub replacement_in_tab: bool,
    /// The name the operator gave the tab, the only label a reopen or a
    /// wake sends Herdr: Herdr keeps any label it is sent as the tab's name,
    /// so sending `tab_label`, the label Hide displays, would name a tab the
    /// operator never named.
    #[serde(default)]
    pub tab_name: ClosedTabName,
}

/// What a closed tab was called (`ClosedContext::tab_name`).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClosedTabName {
    /// A record written before the name was kept, which sends `tab_label`
    /// as it always did, and is not the operator's.
    #[default]
    Unrecorded,
    Unnamed,
    /// The name the operator gave it through Rename; the tab it comes back
    /// as is named again.
    Named(String),
    /// A name someone else gave it, such as `hide agent spawn`: it comes
    /// back under that name but stays unnamed.
    Labeled(String),
}

impl ClosedContext {
    /// The label a reopen or a wake sends Herdr; none for a tab nobody
    /// named, so Herdr keeps or picks the tab's own.
    pub(crate) fn herdr_tab_label(&self) -> Option<&str> {
        match &self.tab_name {
            ClosedTabName::Named(name) | ClosedTabName::Labeled(name) => Some(name),
            ClosedTabName::Unnamed => None,
            ClosedTabName::Unrecorded => Some(&self.tab_label),
        }
    }

    /// The name the operator gave the tab, which the tab a reopen or a wake
    /// opens is named by again.
    pub(crate) fn operator_tab_name(&self) -> Option<&str> {
        match &self.tab_name {
            ClosedTabName::Named(name) => Some(name),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ClosedAgent {
    pub kind: String,
    pub session_id: Option<String>,
    /// The model and effort it was started with, which its reopen starts it
    /// with again after the resume arguments.
    pub launch: hide_agent_adapter::LaunchChoice,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClosedPane {
    pub pane_id: String,
    pub label: Option<String>,
    pub cwd: String,
    pub agent: Option<ClosedAgent>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClosedLayout {
    pub workspace_id: String,
    pub tab_id: String,
    pub zoomed: bool,
    pub focused_pane_id: String,
    pub root: ClosedLayoutNode,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClosedLayoutNode {
    Pane {
        #[serde(default)]
        pane_id: Option<String>,
        #[serde(default)]
        label: Option<String>,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        command: Option<Vec<String>>,
        #[serde(default)]
        env: BTreeMap<String, String>,
    },
    Split {
        direction: ClosedSplitDirection,
        ratio: f32,
        first: Box<ClosedLayoutNode>,
        second: Box<ClosedLayoutNode>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClosedSplitDirection {
    Right,
    Down,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PanePlacement {
    pub neighbor_pane_id: Option<String>,
    /// Path from the tab root to the split that directly owned the closed
    /// pane. A split sibling has no single pane that `pane.split` can target,
    /// so reopen wraps the surviving subtree at this location instead.
    pub parent_path: Vec<ClosedLayoutBranch>,
    pub direction: ClosedSplitDirection,
    pub ratio: f32,
    pub target_was_first: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClosedLayoutBranch {
    First,
    Second,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ClosedItem {
    Draft {
        key: String,
        device_id: String,
        checkout_path: String,
        group_id: Option<String>,
        area_id: String,
        index: usize,
        tab: crate::agent_layout::Tab,
    },
    Pane {
        key: String,
        context: ClosedContext,
        pane: ClosedPane,
        placement: PanePlacement,
    },
    Tab {
        key: String,
        context: ClosedContext,
        layout: ClosedLayout,
        panes: Vec<ClosedPane>,
    },
    File {
        key: String,
        /// The device the file is on; it reopens only while that device is
        /// the one in front, and only through that device's host.
        device_id: String,
        workspace_id: String,
        checkout_id: String,
        checkout_path: String,
        path: String,
        label: String,
    },
}

impl ClosedItem {
    fn clear_start_claims(&mut self) {
        match self {
            Self::Pane { pane, .. } => {
                pane.label = crate::start_claim::public_label(pane.label.as_deref());
            }
            Self::Tab { layout, panes, .. } => {
                layout.root.clear_start_claims();
                for pane in panes {
                    pane.label = crate::start_claim::public_label(pane.label.as_deref());
                }
            }
            Self::Draft { .. } | Self::File { .. } => {}
        }
    }

    pub fn key(&self) -> &str {
        match self {
            Self::Pane { key, .. }
            | Self::Tab { key, .. }
            | Self::File { key, .. }
            | Self::Draft { key, .. } => key,
        }
    }

    /// The device whose surface can reopen this item. Hide records a pane or
    /// tab close only on this machine's Herdr, the core's `node`.
    pub fn device_id<'a>(&'a self, node: &'a str) -> &'a str {
        match self {
            Self::Pane { .. } | Self::Tab { .. } => node,
            Self::File { device_id, .. } | Self::Draft { device_id, .. } => device_id,
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Pane { pane, .. } => pane
                .label
                .as_deref()
                .filter(|label| !crate::start_claim::is_reserved(label))
                .unwrap_or("Pane"),
            Self::Tab { context, .. } => &context.tab_label,
            Self::File { label, .. } => label,
            Self::Draft { .. } => "New tab",
        }
    }
}

impl ClosedLayoutNode {
    fn clear_start_claims(&mut self) {
        match self {
            Self::Pane { label, .. } => {
                *label = crate::start_claim::public_label(label.as_deref());
            }
            Self::Split { first, second, .. } => {
                first.clear_start_claims();
                second.clear_start_claims();
            }
        }
    }

    pub fn pane_count(&self) -> usize {
        match self {
            Self::Pane { .. } => 1,
            Self::Split { first, second, .. } => first.pane_count() + second.pane_count(),
        }
    }

    pub fn pane_ids(&self, output: &mut Vec<String>) {
        match self {
            Self::Pane { pane_id, .. } => output.extend(pane_id.iter().cloned()),
            Self::Split { first, second, .. } => {
                first.pane_ids(output);
                second.pane_ids(output);
            }
        }
    }

    pub fn known_pane_ids(&self, panes: &BTreeMap<String, ClosedPane>, output: &mut Vec<String>) {
        match self {
            Self::Pane {
                pane_id: Some(pane_id),
                ..
            } if panes.contains_key(pane_id) => output.push(pane_id.clone()),
            Self::Pane { .. } => {}
            Self::Split { first, second, .. } => {
                first.known_pane_ids(panes, output);
                second.known_pane_ids(panes, output);
            }
        }
    }

    pub fn placement_for(&self, target: &str) -> Option<PanePlacement> {
        self.placement_for_at_path(target, &mut Vec::new())
    }

    fn placement_for_at_path(
        &self,
        target: &str,
        path: &mut Vec<ClosedLayoutBranch>,
    ) -> Option<PanePlacement> {
        match self {
            Self::Pane { .. } => None,
            Self::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                if first.is_pane(target) {
                    return Some(PanePlacement {
                        neighbor_pane_id: second.direct_pane_id(),
                        parent_path: path.clone(),
                        direction: *direction,
                        ratio: *ratio,
                        target_was_first: true,
                    });
                }
                if second.is_pane(target) {
                    return Some(PanePlacement {
                        neighbor_pane_id: first.direct_pane_id(),
                        parent_path: path.clone(),
                        direction: *direction,
                        // Herdr's pane.split ratio always describes the first
                        // child's share. The later swap restores which side the
                        // reopened pane occupied, so complementing here would
                        // invert the original geometry.
                        ratio: *ratio,
                        target_was_first: false,
                    });
                }
                path.push(ClosedLayoutBranch::First);
                let first_placement = first.placement_for_at_path(target, path);
                path.pop();
                if first_placement.is_some() {
                    return first_placement;
                }
                path.push(ClosedLayoutBranch::Second);
                let second_placement = second.placement_for_at_path(target, path);
                path.pop();
                second_placement
            }
        }
    }

    fn is_pane(&self, target: &str) -> bool {
        matches!(self, Self::Pane { pane_id: Some(pane_id), .. } if pane_id == target)
    }

    fn direct_pane_id(&self) -> Option<String> {
        match self {
            Self::Pane { pane_id, .. } => pane_id.clone(),
            Self::Split { .. } => None,
        }
    }

    /// `folders` says whether a pane's folder is still a directory on its node.
    pub fn resolve_panes(
        &self,
        panes: &BTreeMap<String, ClosedPane>,
        checkout_root: &str,
        folders: &dyn Fn(&str) -> bool,
        notices: &mut Vec<String>,
    ) -> Option<Self> {
        match self {
            Self::Pane {
                pane_id,
                label,
                cwd: _,
                command: _,
                env,
            } => {
                let pane = pane_id.as_ref().and_then(|id| panes.get(id))?;
                let restored_cwd = if folders(&pane.cwd) {
                    pane.cwd.clone()
                } else {
                    notices.push(format!(
                        "{} no longer exists; reopened in the checkout root",
                        pane.cwd
                    ));
                    checkout_root.to_owned()
                };
                Some(Self::Pane {
                    pane_id: None,
                    label: crate::start_claim::public_label(pane.label.as_deref())
                        .or_else(|| crate::start_claim::public_label(label.as_deref())),
                    cwd: Some(restored_cwd),
                    command: None,
                    env: env.clone(),
                })
            }
            Self::Split {
                direction,
                ratio,
                first,
                second,
            } => match (
                first.resolve_panes(panes, checkout_root, folders, notices),
                second.resolve_panes(panes, checkout_root, folders, notices),
            ) {
                (Some(first), Some(second)) => Some(Self::Split {
                    direction: *direction,
                    ratio: *ratio,
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (Some(node), None) | (None, Some(node)) => Some(node),
                (None, None) => None,
            },
        }
    }
}

pub fn resume_arguments(agent: &ClosedAgent) -> Option<Vec<String>> {
    let session_id = agent.session_id.as_ref()?;
    if !hide_session::valid_native_id(session_id) {
        return None;
    }
    let dialect = hide_agent_adapter::adapter(&agent.kind)?.resume?;
    Some(vec![dialect.resume_flag().into(), session_id.clone()])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(id: &str) -> ClosedLayoutNode {
        ClosedLayoutNode::Pane {
            pane_id: Some(id.into()),
            label: None,
            cwd: Some("/tmp".into()),
            command: None,
            env: BTreeMap::new(),
        }
    }

    fn file(key: &str) -> ClosedItem {
        ClosedItem::File {
            key: key.into(),
            device_id: "local".into(),
            workspace_id: "workspace".into(),
            checkout_id: "checkout".into(),
            checkout_path: "/tmp".into(),
            path: format!("/tmp/{key}"),
            label: key.into(),
        }
    }

    fn context() -> ClosedContext {
        ClosedContext {
            workspace_id: "w1".into(),
            workspace_label: "Project".into(),
            workspace_ids_before_close: vec!["w1".into()],
            tab_ids_before_close: vec!["w1:t1".into()],
            pane_ids_before_close: vec!["terminal".into()],
            checkout_id: "checkout".into(),
            checkout_path: "/repo".into(),
            tab_id: "w1:t1".into(),
            tab_label: "Work".into(),
            tab_index: 0,
            agent_area: None,
            agent_group: None,
            replacement_shell: false,
            replacement_in_tab: false,
            tab_name: ClosedTabName::Unnamed,
        }
    }

    #[test]
    fn recently_closed_items_discard_creation_claims_but_keep_human_names() {
        let claim = crate::start_claim::label(&"c".repeat(64));
        let closed = ClosedPane {
            pane_id: "terminal".into(),
            label: Some(claim.clone()),
            cwd: "/repo".into(),
            agent: None,
        };
        let mut leaf = pane("terminal");
        if let ClosedLayoutNode::Pane { label, .. } = &mut leaf {
            *label = Some(claim.clone());
        }
        let single = ClosedItem::Pane {
            key: "pane".into(),
            context: context(),
            pane: closed.clone(),
            placement: PanePlacement {
                neighbor_pane_id: None,
                parent_path: vec![],
                direction: ClosedSplitDirection::Right,
                ratio: 0.5,
                target_was_first: true,
            },
        };
        assert_eq!(single.label(), "Pane", "legacy records never show a claim");
        let human = ClosedPane {
            pane_id: "human".into(),
            label: Some(" 작업 메모 ".into()),
            ..closed.clone()
        };
        let mut human_leaf = pane("human");
        if let ClosedLayoutNode::Pane { label, .. } = &mut human_leaf {
            *label = human.label.clone();
        }
        let tab = ClosedItem::Tab {
            key: "tab".into(),
            context: context(),
            layout: ClosedLayout {
                workspace_id: "w1".into(),
                tab_id: "w1:t1".into(),
                zoomed: false,
                focused_pane_id: "terminal".into(),
                root: ClosedLayoutNode::Split {
                    direction: ClosedSplitDirection::Right,
                    ratio: 0.4,
                    first: Box::new(leaf),
                    second: Box::new(human_leaf),
                },
            },
            panes: vec![closed, human],
        };
        let mut stack = VecDeque::new();
        push_bounded(&mut stack, single);
        push_bounded(&mut stack, tab);
        let ClosedItem::Tab { layout, panes, .. } = stack.pop_back().unwrap() else {
            panic!("tab retained")
        };
        assert_eq!(panes[0].label, None);
        assert_eq!(panes[1].label.as_deref(), Some(" 작업 메모 "));
        let saved_layout = serde_json::to_string(&layout).unwrap();
        assert!(!saved_layout.contains(&claim));
        assert!(saved_layout.contains(" 작업 메모 "));
        let ClosedItem::Pane { pane, .. } = stack.pop_back().unwrap() else {
            panic!("pane retained")
        };
        assert_eq!(pane.label, None);
    }

    #[test]
    fn reopening_legacy_records_never_copies_a_creation_claim() {
        let claim = crate::start_claim::label(&"d".repeat(64));
        for (recorded, layout_label, expected) in [
            (Some(claim.as_str()), Some(claim.as_str()), None),
            (
                Some(claim.as_str()),
                Some("Layout name"),
                Some("Layout name"),
            ),
            (Some("Pane name"), Some(claim.as_str()), Some("Pane name")),
            (None, Some(claim.as_str()), None),
            (
                Some("hide-start:v1:human notes"),
                None,
                Some("hide-start:v1:human notes"),
            ),
        ] {
            let layout: ClosedLayoutNode = serde_json::from_value(serde_json::json!({
                "type": "pane", "pane_id": "terminal", "label": layout_label,
                "cwd": "/repo", "env": {"PROJECT": "work"}
            }))
            .unwrap();
            let panes = BTreeMap::from([(
                "terminal".into(),
                ClosedPane {
                    pane_id: "terminal".into(),
                    label: recorded.map(str::to_owned),
                    cwd: "/repo".into(),
                    agent: None,
                },
            )]);
            let restored = layout
                .resolve_panes(&panes, "/repo", &|_| true, &mut vec![])
                .unwrap();
            let ClosedLayoutNode::Pane {
                label, cwd, env, ..
            } = &restored
            else {
                panic!("pane restored")
            };
            assert_eq!(label.as_deref(), expected);
            assert_eq!(cwd.as_deref(), Some("/repo"));
            assert_eq!(env.get("PROJECT").map(String::as_str), Some("work"));
            assert!(!serde_json::to_string(&restored).unwrap().contains(&claim));
        }
    }

    /// Herdr keeps a label it is sent as the tab's name, so a reopen or a
    /// wake sends only the operator's, and the choice survives being saved;
    /// a record from before the name was kept sends what it always did.
    #[test]
    fn a_reopen_names_the_tab_only_with_the_operators_name() {
        let saved_before = serde_json::json!({
            "workspace_id": "w1", "workspace_label": "repo",
            "workspace_ids_before_close": ["w1"], "tab_ids_before_close": ["w1:t1"],
            "pane_ids_before_close": ["w1:p1"], "checkout_id": "c1", "checkout_path": "/repo",
            "tab_id": "w1:t1", "tab_label": "Planning", "tab_index": 0,
            "agent_area": null, "replacement_shell": false
        });
        let old: ClosedContext = serde_json::from_value(saved_before).unwrap();
        assert_eq!(old.herdr_tab_label(), Some("Planning"));
        for (tab_name, sent) in [
            (ClosedTabName::Unnamed, None),
            (ClosedTabName::Named("Planning".into()), Some("Planning")),
        ] {
            let context = ClosedContext {
                tab_label: "Agent one".into(),
                tab_name,
                ..old.clone()
            };
            let saved: ClosedContext =
                serde_json::from_value(serde_json::to_value(&context).unwrap()).unwrap();
            assert_eq!(saved.herdr_tab_label(), sent);
        }
    }

    #[test]
    fn bounded_stack_drops_only_the_oldest_and_remains_lifo() {
        let mut stack = VecDeque::new();
        for index in 0..=RECENT_CLOSED_LIMIT {
            push_bounded(&mut stack, file(&format!("file-{index}")));
        }
        assert_eq!(stack.len(), RECENT_CLOSED_LIMIT);
        assert_eq!(stack.front().unwrap().label(), "file-1");
        assert_eq!(stack.pop_back().unwrap().label(), "file-20");
        assert_eq!(stack.pop_back().unwrap().label(), "file-19");
    }

    #[test]
    fn placement_preserves_original_split_ratio_for_either_side() {
        let layout = ClosedLayoutNode::Split {
            direction: ClosedSplitDirection::Right,
            ratio: 0.35,
            first: Box::new(pane("left")),
            second: Box::new(ClosedLayoutNode::Split {
                direction: ClosedSplitDirection::Down,
                ratio: 0.6,
                first: Box::new(pane("top")),
                second: Box::new(pane("bottom")),
            }),
        };
        assert_eq!(
            layout.placement_for("left"),
            Some(PanePlacement {
                neighbor_pane_id: None,
                parent_path: vec![],
                direction: ClosedSplitDirection::Right,
                ratio: 0.35,
                target_was_first: true,
            })
        );
        let bottom = layout.placement_for("bottom").unwrap();
        assert_eq!(bottom.neighbor_pane_id.as_deref(), Some("top"));
        assert_eq!(bottom.parent_path, [ClosedLayoutBranch::Second]);
        assert_eq!(bottom.direction, ClosedSplitDirection::Down);
        assert!((bottom.ratio - 0.6).abs() < f32::EPSILON * 2.0);
        assert!(!bottom.target_was_first);
    }

    #[test]
    fn resume_arguments_resume_without_forking() {
        assert_eq!(
            resume_arguments(&ClosedAgent {
                kind: "grok".into(),
                session_id: Some("0199b000-0000-7000-8000-000000000003".into()),
                ..Default::default()
            }),
            Some(vec![
                "--resume".to_owned(),
                "0199b000-0000-7000-8000-000000000003".to_owned()
            ])
        );
        for kind in ["grok", "pi", "omp"] {
            assert!(
                resume_arguments(&ClosedAgent {
                    kind: kind.into(),
                    session_id: Some("--yolo".into()),
                    ..Default::default()
                })
                .is_none()
            );
        }
        assert_eq!(
            resume_arguments(&ClosedAgent {
                kind: "opencode".into(),
                session_id: Some("ses_native".into()),
                ..Default::default()
            }),
            Some(vec!["-s".to_owned(), "ses_native".to_owned()])
        );
        assert!(
            resume_arguments(&ClosedAgent {
                kind: "opencode".into(),
                session_id: Some("--fork".into()),
                ..Default::default()
            })
            .is_none(),
            "a flag-shaped id never reaches the command line"
        );
        assert_eq!(
            resume_arguments(&ClosedAgent {
                kind: "claude".into(),
                session_id: Some("session-c".into()),
                ..Default::default()
            }),
            Some(
                vec!["--resume", "session-c"]
                    .into_iter()
                    .map(String::from)
                    .collect()
            )
        );
        assert_eq!(
            resume_arguments(&ClosedAgent {
                kind: "codex".into(),
                session_id: Some("session-x".into()),
                ..Default::default()
            }),
            Some(
                vec!["resume", "session-x"]
                    .into_iter()
                    .map(String::from)
                    .collect()
            )
        );
    }

    #[test]
    fn resolved_panes_restore_the_recorded_label_and_cwd() {
        let layout = pane("terminal");
        let panes = BTreeMap::from([(
            "terminal".into(),
            ClosedPane {
                pane_id: "terminal".into(),
                label: None,
                cwd: "/tmp".into(),
                agent: None,
            },
        )]);
        assert!(matches!(
            layout.resolve_panes(
                &panes,
                "/tmp",
                &|path| std::path::Path::new(path).is_dir(),
                &mut Vec::new()
            ),
            Some(ClosedLayoutNode::Pane { pane_id: None, .. })
        ));
    }

    #[test]
    fn an_unknown_pane_id_resolves_to_nothing() {
        let layout = pane("missing");
        assert_eq!(
            layout.resolve_panes(
                &BTreeMap::new(),
                "/tmp",
                &|path| std::path::Path::new(path).is_dir(),
                &mut Vec::new()
            ),
            None
        );
    }
}
