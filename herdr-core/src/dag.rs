//! The one dependency graph a Conductor's mission and a session's own plan
//! share (PRD hide-plan D-30): node keys that name other nodes of the same
//! list, checked for unknown keys and cycles under one set of caps, and the
//! walks both records make over it. A rule about the graph lives here once,
//! so the map a mission draws and the map a plan draws cannot disagree.

use std::collections::{BTreeMap, BTreeSet};

/// The longest node key.
const KEY_LIMIT: usize = 40;
/// Nodes in one graph: a mission's plan or a session's plan.
pub const NODE_LIMIT: usize = 64;

/// A node as the graph sees it: its key and the keys it waits on.
pub(crate) trait Node {
    fn key(&self) -> &str;
    fn depends_on(&self) -> &[String];
}

/// Why a node list is not a graph, with the node it was found at so the
/// caller can say where to fix it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Refusal {
    /// A key that is empty, too long, of other characters or given twice.
    Key(String),
    /// A dependency that names no other node of the list, or the node itself.
    Unknown { node: String, dependency: String },
    /// A dependency loop through this node.
    Cycle(String),
}

/// A node key: ASCII letters, digits, `-`, `_` and `.`, at most 40 bytes.
pub fn valid_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= KEY_LIMIT
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
}

/// Whether `key` may be one more key of a list whose keys so far are
/// `keys`, adding it when it may.
pub(crate) fn admit_key<'a>(keys: &mut BTreeSet<&'a str>, key: &'a str) -> Result<(), Refusal> {
    if valid_key(key) && keys.insert(key) {
        Ok(())
    } else {
        Err(Refusal::Key(key.to_owned()))
    }
}

/// Checks that every dependency names another node of `nodes` and that no
/// dependency loops back. The node count, the keys and each node's
/// dependency count are the caller's to have checked, with its own words.
pub(crate) fn check_edges<N: Node>(nodes: &[N]) -> Result<(), Refusal> {
    let keys: BTreeSet<&str> = nodes.iter().map(Node::key).collect();
    for node in nodes {
        if let Some(dependency) = node
            .depends_on()
            .iter()
            .find(|key| key.as_str() == node.key() || !keys.contains(key.as_str()))
        {
            return Err(Refusal::Unknown {
                node: node.key().to_owned(),
                dependency: dependency.clone(),
            });
        }
    }
    match cycle(nodes) {
        Some(key) => Err(Refusal::Cycle(key.to_owned())),
        None => Ok(()),
    }
}

/// A node on a dependency loop, if there is one.
fn cycle<N: Node>(nodes: &[N]) -> Option<&str> {
    let index: BTreeMap<&str, &N> = nodes.iter().map(|node| (node.key(), node)).collect();
    // 1 on the path, 2 done.
    let mut marks: BTreeMap<&str, u8> = BTreeMap::new();
    fn visit<'a, N: Node>(
        key: &'a str,
        index: &BTreeMap<&'a str, &'a N>,
        marks: &mut BTreeMap<&'a str, u8>,
    ) -> bool {
        match marks.get(key) {
            Some(1) => return true,
            Some(2) => return false,
            _ => {}
        }
        marks.insert(key, 1);
        if let Some(node) = index.get(key) {
            for dependency in node.depends_on() {
                if visit(dependency, index, marks) {
                    return true;
                }
            }
        }
        marks.insert(key, 2);
        false
    }
    nodes
        .iter()
        .find(|node| visit(node.key(), &index, &mut marks))
        .map(Node::key)
}

/// What a walk does at an upstream node.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Step {
    /// Go on to the nodes it waits on.
    Continue,
    /// Leave the nodes it waits on out of this walk.
    Skip,
    /// End the walk.
    Stop,
}

/// Visits each node upstream of `start` once, nearest first along each
/// branch, until `visit` stops the walk. A key no node has is passed over.
pub(crate) fn walk_upstream<'a, N: Node>(
    nodes: &'a [N],
    start: &'a N,
    mut visit: impl FnMut(&'a N) -> Step,
) {
    let by_key: BTreeMap<&str, &N> = nodes.iter().map(|node| (node.key(), node)).collect();
    let mut seen = BTreeSet::new();
    let mut stack: Vec<&str> = start.depends_on().iter().map(String::as_str).collect();
    while let Some(key) = stack.pop() {
        if !seen.insert(key) {
            continue;
        }
        let Some(upstream) = by_key.get(key) else {
            continue;
        };
        match visit(upstream) {
            Step::Continue => stack.extend(upstream.depends_on().iter().map(String::as_str)),
            Step::Skip => {}
            Step::Stop => return,
        }
    }
}

/// Each node's depth: 0 for one that waits on nothing, otherwise one more
/// than the deepest node it waits on. A key no node has counts as nothing.
/// The graph must have passed [`check_edges`].
pub(crate) fn depths<N: Node>(nodes: &[N]) -> BTreeMap<&str, usize> {
    let by_key: BTreeMap<&str, &N> = nodes.iter().map(|node| (node.key(), node)).collect();
    let mut out: BTreeMap<&str, usize> = BTreeMap::new();
    fn depth<'a, N: Node>(
        key: &'a str,
        by_key: &BTreeMap<&'a str, &'a N>,
        out: &mut BTreeMap<&'a str, usize>,
        guard: usize,
    ) -> usize {
        if let Some(depth) = out.get(key) {
            return *depth;
        }
        let Some(node) = by_key.get(key) else {
            return 0;
        };
        // A checked graph has no loop; the guard keeps an unchecked one from
        // recursing forever.
        let value = if guard > by_key.len() {
            0
        } else {
            node.depends_on()
                .iter()
                .filter(|dependency| by_key.contains_key(dependency.as_str()))
                .map(|dependency| depth(dependency, by_key, out, guard + 1) + 1)
                .max()
                .unwrap_or(0)
        };
        out.insert(key, value);
        value
    }
    for node in nodes {
        depth(node.key(), &by_key, &mut out, 0);
    }
    out
}

/// The topmost node `pick` accepts: the shallowest, and of those the first
/// in the list.
pub(crate) fn topmost<N: Node>(nodes: &[N], pick: impl Fn(&N) -> bool) -> Option<&N> {
    let depths = depths(nodes);
    nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| pick(node))
        .min_by_key(|(index, node)| (depths.get(node.key()).copied().unwrap_or(0), *index))
        .map(|(_, node)| node)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct N(&'static str, Vec<String>);

    impl Node for N {
        fn key(&self) -> &str {
            self.0
        }
        fn depends_on(&self) -> &[String] {
            &self.1
        }
    }

    fn n(key: &'static str, depends: &[&str]) -> N {
        N(key, depends.iter().map(|key| (*key).to_owned()).collect())
    }

    #[test]
    fn a_dependency_on_no_node_or_on_itself_is_unknown_and_a_loop_is_a_cycle() {
        assert_eq!(
            check_edges(&[n("a", &["b"])]),
            Err(Refusal::Unknown {
                node: "a".into(),
                dependency: "b".into()
            })
        );
        assert!(matches!(
            check_edges(&[n("a", &["a"])]),
            Err(Refusal::Unknown { .. })
        ));
        assert!(matches!(
            check_edges(&[n("a", &["c"]), n("b", &["a"]), n("c", &["b"])]),
            Err(Refusal::Cycle(_))
        ));
        assert_eq!(check_edges(&[n("a", &[]), n("b", &["a"])]), Ok(()));
    }

    #[test]
    fn a_key_given_twice_or_of_other_characters_is_refused() {
        let mut keys = BTreeSet::new();
        assert_eq!(admit_key(&mut keys, "spec"), Ok(()));
        assert_eq!(
            admit_key(&mut keys, "spec"),
            Err(Refusal::Key("spec".into()))
        );
        assert_eq!(admit_key(&mut keys, "a b"), Err(Refusal::Key("a b".into())));
        assert_eq!(
            admit_key(&mut keys, &"k".repeat(41)),
            Err(Refusal::Key("k".repeat(41)))
        );
    }

    #[test]
    fn the_walk_visits_each_upstream_node_once_and_skips_below_a_skipped_one() {
        let nodes = [
            n("root", &[]),
            n("left", &["root"]),
            n("right", &["root"]),
            n("join", &["left", "right"]),
        ];
        let mut seen = Vec::new();
        walk_upstream(&nodes, &nodes[3], |node| {
            seen.push(node.key().to_owned());
            if node.key() == "left" {
                Step::Skip
            } else {
                Step::Continue
            }
        });
        seen.sort();
        assert_eq!(seen, ["left", "right", "root"]);
    }

    #[test]
    fn the_topmost_node_is_the_shallowest_then_the_first_listed() {
        let nodes = [n("late", &["early"]), n("early", &[]), n("other", &[])];
        assert_eq!(topmost(&nodes, |_| true).map(Node::key), Some("early"));
        assert_eq!(
            topmost(&nodes, |node| node.key() != "early").map(Node::key),
            Some("other")
        );
        assert_eq!(depths(&nodes).get("late"), Some(&1));
    }
}
