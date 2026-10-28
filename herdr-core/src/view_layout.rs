//! One Workspace's View areas (PRD S7 D-04, D-06, D-11, D-12): a binary tree
//! of splits whose leaves are areas, each holding an ordered strip of
//! displays, and the pure operations that focus, move, split, resize and
//! close them.
//!
//! A display shows one document, a file or a diff, and names it by what it
//! shows (path, kind, Changes group) so the tree survives a restart that
//! mints new editor tab ids. The runtime binds each display to the editor tab
//! that holds its document (`tab_id`, never stored); several displays may
//! bind one tab, which is how two areas show one buffer (D-03).
//!
//! A browser display (issue 155) shows a page instead: it has no document
//! and no tab, and names its page by the address it holds. The desktop host
//! draws the page; this tree only says where it sits.
//!
//! An external display (PRD outside-path-in-hide) shows a file outside every
//! checkout that the operator clicked in the desktop window. It holds no path:
//! only the desktop host's grant id, the file's name and its viewer kind. The
//! host reads the file for its own window; this tree only says where it sits,
//! and it is never stored.
//!
//! Nothing here knows the runtime. Every operation applies completely or
//! returns why it cannot and leaves the tree as it was, so one operator
//! action is one change (B18). The caps are the core's; the shell enforces
//! pixel minimums on top of them because only it has the geometry.

use serde::{Deserialize, Serialize};

/// View areas one checkout's layout may hold at once.
pub const MAX_VIEW_AREAS: usize = 6;
/// Split nodes on the path from the root to any area.
pub const MAX_SPLIT_DEPTH: usize = 3;
/// Displays one checkout may hold at once; an open past it closes the view
/// looked at longest ago (PRD lane-files-checkout-views D-07).
pub const MAX_VIEW_DISPLAYS: usize = 32;
/// Sessions one View remembers as its openers; a further one forgets the
/// session that opened it longest ago (PRD session-files D-26).
pub const MAX_VIEW_OPENERS: usize = 8;
#[cfg(test)]
use crate::split_tree::successor;
use crate::split_tree::{AreaItem, TreeLimits};
/// The first child's share of a split; the bounds keep either side usable.
pub use crate::split_tree::{Edge, LayoutError, SplitAxis};
#[cfg(test)]
use crate::split_tree::{MAX_SPLIT_RATIO, MIN_SPLIT_RATIO};
#[cfg(test)]
pub type Area = crate::split_tree::Area<Display>;
pub type Node = crate::split_tree::Node<Display>;
#[cfg(test)]
pub type Split = crate::split_tree::Split<Display>;
pub type Layout = crate::split_tree::SplitTree<Display>;
/// The longest address a browser display holds, in bytes; the desktop host
/// drops a longer one too.
pub const MAX_BROWSER_URL_BYTES: usize = 8192;
/// A page title past this many characters is cut; a tab shows far fewer.
pub const MAX_BROWSER_TITLE_CHARS: usize = 512;
/// The page a browser display shows when it has nothing else to show.
pub const BLANK_PAGE: &str = "about:blank";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayKind {
    File,
    Diff,
    /// A page the desktop host draws; `path` is empty and `url` names it.
    /// A pull request's card was a kind of View once; it opens in the
    /// lane's 보기 slot now, and a stored card is dropped as the file loads
    /// (PRD sidebar-task-lens D-50, `workspace_views::drop_retired_cards`).
    Browser,
    /// A file outside every checkout the desktop host granted its window;
    /// `path` is empty and `external` names it. Never stored.
    External,
}

impl DisplayKind {
    /// Whether the display shows a document the editor holds a tab for.
    pub const fn is_document(self) -> bool {
        matches!(self, Self::File | Self::Diff)
    }
}

/// Whether a browser display may hold `url`: the web, a local file, or the
/// blank page, spelled on one line within the cap. The desktop host loads
/// only these too.
pub fn browser_address(url: &str) -> bool {
    if url.is_empty()
        || url.len() > MAX_BROWSER_URL_BYTES
        || url.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return false;
    }
    if url == BLANK_PAGE {
        return true;
    }
    let Some((scheme, rest)) = url.split_once(':') else {
        return false;
    };
    matches!(
        scheme.to_ascii_lowercase().as_str(),
        "http" | "https" | "file"
    ) && rest.starts_with("//")
        && rest.len() > 2
}

/// Whether `url` names a local file, which only a Workspace on this Mac can
/// show.
pub fn is_file_address(url: &str) -> bool {
    url.get(..5)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("file:"))
}

/// A page title as a browser display keeps it: one line, cut at the cap.
pub fn browser_title(title: &str) -> String {
    title
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(MAX_BROWSER_TITLE_CHARS)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// The grant id the desktop host mints is 128 random bits in hex; anything
/// else is not one of its grants.
pub const EXTERNAL_GRANT_LEN: usize = 32;
/// A file name past this many characters is cut; a tab shows far fewer.
pub const MAX_EXTERNAL_NAME_CHARS: usize = 255;

/// Which viewer an external file opens in, decided by the desktop host when
/// it granted the file.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalKind {
    Text,
    Image,
    Pdf,
    Video,
    Html,
}

/// An outside file as the core knows it: the desktop host's grant and what
/// the tab shows. The path stays in the desktop host.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ExternalFile {
    pub grant: String,
    pub name: String,
    pub kind: ExternalKind,
}

impl ExternalFile {
    /// The file as an event names it, or why it is not one: a grant id of the
    /// host's shape and a one-line name that is not a path.
    pub fn admitted(grant: &str, name: &str, kind: ExternalKind) -> Result<Self, &'static str> {
        if !external_grant(grant) {
            return Err("An outside file is named by the desktop host's grant id");
        }
        let name: String = name
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .take(MAX_EXTERNAL_NAME_CHARS)
            .collect::<String>()
            .trim()
            .to_owned();
        if name.is_empty() || name.contains(['/', '\\']) {
            return Err("An outside file's name is its file name alone");
        }
        Ok(Self {
            grant: grant.to_owned(),
            name,
            kind,
        })
    }
}

/// Whether `grant` has the shape of a desktop host's grant id.
pub fn external_grant(grant: &str) -> bool {
    grant.len() == EXTERNAL_GRANT_LEN
        && grant
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Display {
    pub id: String,
    pub path: String,
    pub kind: DisplayKind,
    /// The Changes group a diff compares against; absent for a file.
    #[serde(default)]
    pub committed: Option<bool>,
    #[serde(default)]
    pub preview: bool,
    #[serde(default)]
    pub last_focused_unix_ms: u64,
    /// The editor tab holding this display's document, bound at runtime and
    /// never stored: a restart mints new tab ids.
    #[serde(skip)]
    pub tab_id: Option<String>,
    /// A browser display's address: the one its page last reported, or the
    /// one the operator last asked for. Absent for a file or a diff.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The title its page last reported; absent until it reports one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Raised each time the operator asks a browser display to load `url`,
    /// which is how the host tells a request from the page's own
    /// navigation. Never stored: a restart loads every page once anyway.
    #[serde(skip)]
    pub load: u64,
    /// The agent that last opened this View, absent for one the operator
    /// opened (PRD view-opener-feedback D-02, D-03).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opener: Option<crate::view_feedback::Opener>,
    /// Every session that opened this View, the one that did longest ago
    /// first: the View is in each of their Files strips (PRD session-files
    /// D-10, D-26). The mark above is the last of them, unless the operator's
    /// own address took it off.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub openers: Vec<crate::view_feedback::Opener>,
    /// Whether the operator opened it, from the Explorer, ⌘K, the file tree,
    /// a new tab's address field or a caller Hide could not bind to a pane:
    /// such a View is in every session's strip (D-10).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub operator: bool,
    /// What an agent opened here that the operator has not looked at yet
    /// (PRD unseen-agent-views D-07, D-19); absent once they have, and on a
    /// View stored before this was kept (D-27).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::view_feedback::unseen_or_unreadable"
    )]
    pub unseen: Option<crate::view_feedback::Unseen>,
    /// An external display's grant, name and viewer; absent for every other
    /// kind. A stored file never carries one (`workspace_views::save`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<ExternalFile>,
}

impl Display {
    /// A shell-owned new-tab page; no native page exists until navigation.
    pub fn is_new_tab(&self) -> bool {
        self.kind == DisplayKind::Browser && self.url.is_none()
    }

    /// Records `opener` as the session that opened this View last: its mark,
    /// and one of the sessions whose strip shows it (D-26).
    pub fn opened_by(&mut self, opener: crate::view_feedback::Opener) {
        // One session once: its terminal, or its native session found again
        // in another pane (D-10).
        self.openers.retain(|known| {
            !(known.device_id == opener.device_id
                && (known.terminal_id == opener.terminal_id
                    || (known.session.is_some() && known.session == opener.session)))
        });
        if self.openers.len() >= MAX_VIEW_OPENERS {
            self.openers.remove(0);
        }
        self.openers.push(opener.clone());
        self.opener = Some(opener);
    }

    /// The View shows something else now, opened by whoever opens it.
    pub fn forget_openers(&mut self) {
        self.opener = None;
        self.openers.clear();
        self.operator = false;
        self.unseen = None;
    }

    /// Whether the Files strip of `scope` shows this View: the operator's
    /// Views show in every session's, and a session's only in its own
    /// (D-10). With no session, a plain shell has the keyboard and only the
    /// operator's show (D-28). A View no session opened is the operator's.
    pub fn shown_to(&self, scope: Option<&crate::view_feedback::SessionScope>) -> bool {
        self.operator
            || self.openers.is_empty()
            || scope.is_some_and(|scope| self.openers.iter().any(|o| o.of_session(scope)))
    }

    /// Whether this display shows the document named by `path`, `kind` and,
    /// for a diff, its Changes group. A browser display shows no document.
    pub fn shows(&self, path: &str, kind: DisplayKind, committed: Option<bool>) -> bool {
        self.kind.is_document()
            && self.path == path
            && self.kind == kind
            && (kind == DisplayKind::File || self.committed == committed)
    }
}

impl AreaItem for Display {
    const LIMITS: TreeLimits = TreeLimits {
        areas: MAX_VIEW_AREAS,
        depth: MAX_SPLIT_DEPTH,
        items: MAX_VIEW_DISPLAYS,
    };
    const MINTED_IDENTITY: bool = true;
    fn id(&self) -> &str {
        &self.id
    }
    fn id_mut(&mut self) -> &mut String {
        &mut self.id
    }
    fn focus_stamp(&self) -> u64 {
        self.last_focused_unix_ms
    }
    fn set_focus_stamp(&mut self, stamp: u64) {
        self.last_focused_unix_ms = stamp;
    }
    fn keep_open(&mut self) {
        self.preview = false;
    }
    fn same_content(&self, other: &Self) -> bool {
        self.shows(&other.path, other.kind, other.committed)
    }
    fn repair_area(items: &mut [Self], active: Option<&str>, notes: &mut Vec<String>) {
        let mut groups = 0;
        let mut addresses = 0;
        let mut unreadable = 0;
        for display in items.iter_mut() {
            // A seen record that does not read is unseen, never seen
            // (PRD unseen-agent-views D-12, B18).
            if let Some(unseen) = display.unseen.as_mut().filter(|unseen| unseen.unreadable) {
                unseen.unreadable = false;
                unreadable += 1;
            }
            // A View stored before Views belonged to sessions goes to the
            // session its mark names, else to the operator (PRD
            // session-files D-29). Expected once per View, so no note.
            if display.openers.is_empty() && !display.operator {
                match display.opener.clone() {
                    Some(opener) => display.openers.push(opener),
                    None => display.operator = true,
                }
            }
            // A diff shows one Changes group, the working one unless
            // stored otherwise, and a file none: a diff stored
            // without its group would never find its tab again.
            let group = match display.kind {
                DisplayKind::Diff => Some(display.committed.unwrap_or(false)),
                DisplayKind::File | DisplayKind::Browser | DisplayKind::External => None,
            };
            if display.committed != group {
                display.committed = group;
                groups += 1;
            }
            // A page has an address it may load and no path; a
            // document has neither address nor title.
            if display.kind == DisplayKind::Browser {
                let valid = display.url.as_deref().is_none_or(browser_address);
                let title = display
                    .title
                    .as_deref()
                    .map(browser_title)
                    .filter(|title| !title.is_empty());
                if !valid || !display.path.is_empty() || display.preview || title != display.title {
                    if !valid {
                        display.url = Some(BLANK_PAGE.to_owned());
                    }
                    display.path.clear();
                    display.preview = false;
                    display.title = title;
                    addresses += 1;
                }
            } else if display.url.is_some() || display.title.is_some() {
                display.url = None;
                display.title = None;
                addresses += 1;
            }
            // Only an external display names a grant, and it names nothing
            // else; the runtime drops one whose grant is gone.
            if (display.kind == DisplayKind::External) != display.external.is_some()
                || (display.kind == DisplayKind::External
                    && (!display.path.is_empty() || display.preview))
            {
                if display.kind != DisplayKind::External {
                    display.external = None;
                }
                display.path.clear();
                display.preview = false;
                addresses += 1;
            }
        }
        if groups > 0 {
            notes.push(format!(
                "{groups} views had a Changes group that did not fit their kind"
            ));
        }
        if addresses > 0 {
            notes.push(format!(
                "{addresses} views had an address or path that did not fit their kind"
            ));
        }
        if unreadable > 0 {
            notes.push(format!(
                "{unreadable} views had a seen record that did not read and are unseen"
            ));
        }
        let keep = items
            .iter()
            .find(|item| item.preview && active == Some(item.id.as_str()))
            .or_else(|| items.iter().find(|item| item.preview))
            .map(|item| item.id.clone());
        let mut previews = 0;
        for item in items {
            if item.preview && Some(&item.id) != keep.as_ref() {
                item.preview = false;
                previews += 1;
            }
        }
        if previews > 0 {
            notes.push(format!("{previews} extra preview views were kept open"));
        }
    }
}

impl LayoutError {
    /// The stable error kind the shell matches a refusal by.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::UnknownDisplay(_) => "view_layout.unknown_display",
            Self::UnknownArea(_) => "view_layout.unknown_area",
            Self::UnknownSplit(_) => "view_layout.unknown_split",
            Self::AreaLimit | Self::DepthLimit => "view_layout.limit",
            Self::DisplayLimit => "view_layout.display_limit",
            Self::NothingToSplit => "view_layout.nothing_to_split",
            Self::InvalidRatio => "view_layout.invalid_ratio",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::UnknownDisplay(id) => format!("View {id} is not open in this Workspace"),
            Self::UnknownArea(id) => format!("View area {id} is not in this Workspace"),
            Self::UnknownSplit(id) => format!("Divider {id} is not in this Workspace"),
            Self::AreaLimit => format!(
                "This Workspace already has {MAX_VIEW_AREAS} view areas. Close a view to split again."
            ),
            Self::DepthLimit => format!(
                "This view area is already split {MAX_SPLIT_DEPTH} levels deep. Split another area instead."
            ),
            Self::DisplayLimit => format!(
                "This Workspace has {MAX_VIEW_DISPLAYS} views open. Close a view to open another."
            ),
            Self::NothingToSplit => {
                "This view is the only one in its area, so splitting it there would change nothing"
                    .to_owned()
            }
            Self::InvalidRatio => "A view area's share must be a number".to_owned(),
        }
    }
}

impl Layout {
    /// A new display with a fresh id, not yet in any area.
    pub fn new_display(
        &mut self,
        path: &str,
        kind: DisplayKind,
        committed: Option<bool>,
        preview: bool,
    ) -> Display {
        Display {
            id: self.mint('d'),
            path: path.to_owned(),
            kind,
            committed: (kind == DisplayKind::Diff).then_some(committed.unwrap_or(false)),
            preview,
            last_focused_unix_ms: 0,
            tab_id: None,
            url: None,
            title: None,
            load: 0,
            opener: None,
            openers: Vec::new(),
            operator: false,
            unseen: None,
            external: None,
        }
    }

    /// A new browser display for `url` with a fresh id, not yet in any area.
    /// It is never a preview: a page is opened on purpose.
    pub fn new_browser_display(&mut self, url: &str, load: u64) -> Display {
        Display {
            url: (!url.is_empty()).then(|| url.to_owned()),
            load,
            ..self.new_display("", DisplayKind::Browser, None, false)
        }
    }

    /// A new external display for `file` with a fresh id, not yet in any
    /// area. Like a page, it is opened on purpose and never a preview.
    pub fn new_external_display(&mut self, file: ExternalFile, load: u64) -> Display {
        Display {
            external: Some(file),
            load,
            ..self.new_display("", DisplayKind::External, None, false)
        }
    }

    /// Takes every external display out, as the stored file and a host that
    /// lost its grants want the tree. Returns how many went.
    pub fn remove_external(&mut self, keep: impl Fn(&ExternalFile) -> bool) -> usize {
        let gone: Vec<String> = self
            .displays()
            .filter(|display| {
                display.kind == DisplayKind::External
                    && !display.external.as_ref().is_some_and(&keep)
            })
            .map(|display| display.id.clone())
            .collect();
        for id in &gone {
            self.remove(id);
        }
        gone.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A layout whose root area holds `names`, in order.
    fn with_files(names: &[&str]) -> Layout {
        let mut layout = Layout::default();
        for name in names {
            let display =
                layout.new_display(&format!("/repo/{name}"), DisplayKind::File, None, false);
            layout.insert("a1", display, 0).unwrap();
        }
        layout
    }

    fn id(layout: &Layout, name: &str) -> String {
        layout
            .displays()
            .find(|display| display.path == format!("/repo/{name}"))
            .unwrap()
            .id
            .clone()
    }

    fn area_of(layout: &Layout, name: &str) -> String {
        layout.area_of(&id(layout, name)).unwrap().id.clone()
    }

    fn names(layout: &Layout) -> Vec<Vec<String>> {
        layout
            .areas()
            .iter()
            .map(|area| {
                area.displays
                    .iter()
                    .map(|display| display.path.trim_start_matches("/repo/").to_owned())
                    .collect()
            })
            .collect()
    }

    fn split(layout: &mut Layout, name: &str, target: &str, edge: Edge) -> String {
        let display = id(layout, name);
        layout.split(&display, target, edge, 1).unwrap()
    }

    /// A bookmark restore shows a display in its area and leaves the area in
    /// use where it was.
    #[test]
    fn showing_a_display_fronts_it_without_moving_the_area_in_use() {
        let mut layout = with_files(&["a", "b", "c"]);
        let right = split(&mut layout, "c", "a1", Edge::Right);
        assert_eq!(layout.active_area, right);
        let a = id(&layout, "a");
        let b = id(&layout, "b");
        assert_eq!(
            layout.area("a1").unwrap().active.as_deref(),
            Some(b.as_str())
        );

        assert_eq!(layout.show(&a, 9), Ok(true));
        assert_eq!(
            layout.area("a1").unwrap().active.as_deref(),
            Some(a.as_str())
        );
        assert_eq!(layout.active_area, right, "the keyboard's area stays");
        assert_eq!(layout.show(&a, 10), Ok(false));
        assert_eq!(
            layout.show("nope", 11),
            Err(LayoutError::UnknownDisplay("nope".to_owned()))
        );
    }

    #[test]
    fn a_split_past_the_area_cap_is_refused_and_changes_nothing() {
        let mut layout = with_files(&["a", "b", "c", "d", "e", "f", "g"]);
        let b = split(&mut layout, "b", "a1", Edge::Right);
        split(&mut layout, "c", "a1", Edge::Down);
        split(&mut layout, "d", &b, Edge::Down);
        split(&mut layout, "e", "a1", Edge::Right);
        split(&mut layout, "f", &b, Edge::Right);
        assert_eq!(layout.area_count(), MAX_VIEW_AREAS);
        assert_eq!(layout.depth(), MAX_SPLIT_DEPTH);

        let before = layout.clone();
        let shallow = area_of(&layout, "c");
        assert_eq!(
            layout.split(&id(&layout, "g"), &shallow, Edge::Right, 2),
            Err(LayoutError::AreaLimit)
        );
        assert_eq!(layout, before);
    }

    /// B19: the 33rd display fits, and every way of adding one more is
    /// refused and changes nothing, whichever path asked.
    #[test]
    fn every_way_of_adding_a_display_past_the_cap_is_refused() {
        let names: Vec<String> = (1..MAX_VIEW_DISPLAYS).map(|n| format!("f{n}")).collect();
        let mut layout = with_files(&names.iter().map(String::as_str).collect::<Vec<_>>());
        let last = layout.new_display("/repo/last", DisplayKind::File, None, false);
        layout.insert("a1", last, 1).unwrap();
        assert_eq!(layout.display_count(), MAX_VIEW_DISPLAYS);

        let before = layout.clone();
        let mut scratch = layout.clone();
        let mut extra = || scratch.new_display("/repo/x", DisplayKind::File, None, false);
        assert_eq!(
            layout.insert("a1", extra(), 2),
            Err(LayoutError::DisplayLimit)
        );
        assert_eq!(layout.append("a1", extra()), Err(LayoutError::DisplayLimit));
        assert_eq!(
            layout.split_new("a1", Edge::Right, extra(), 2),
            Err(LayoutError::DisplayLimit)
        );
        assert_eq!(layout, before);
    }

    #[test]
    fn a_split_past_the_depth_cap_is_refused_and_changes_nothing() {
        // `a` stays behind in the root area, so no area collapses below.
        let mut layout = with_files(&["a", "b", "c", "d", "e"]);
        let b = split(&mut layout, "b", "a1", Edge::Right);
        let c = split(&mut layout, "c", &b, Edge::Down);
        let d = split(&mut layout, "d", &c, Edge::Right);
        assert_eq!(layout.depth(), MAX_SPLIT_DEPTH, "the cap itself fits");
        assert!(layout.area_count() < MAX_VIEW_AREAS);

        let before = layout.clone();
        assert_eq!(
            layout.split(&id(&layout, "e"), &d, Edge::Down, 2),
            Err(LayoutError::DepthLimit)
        );
        assert_eq!(layout, before);
    }

    #[test]
    fn splitting_an_areas_only_display_into_that_area_is_refused() {
        let mut layout = with_files(&["a"]);
        let before = layout.clone();
        assert_eq!(
            layout.split(&id(&layout, "a"), "a1", Edge::Right, 1),
            Err(LayoutError::NothingToSplit)
        );
        assert_eq!(layout, before);
    }

    #[test]
    fn moving_an_areas_last_display_away_collapses_it_and_the_neighbour_takes_the_space() {
        let mut layout = with_files(&["a", "b"]);
        let fresh = split(&mut layout, "b", "a1", Edge::Right);
        assert_eq!(names(&layout), vec![vec!["a"], vec!["b"]]);
        assert_eq!(layout.active_area, fresh);

        let b = id(&layout, "b");
        assert!(layout.move_display(&b, "a1", 0, 2).unwrap());
        assert_eq!(names(&layout), vec![vec!["b", "a"]]);
        assert!(matches!(layout.root, Node::Area(ref area) if area.id == "a1"));
        assert_eq!(layout.active_area, "a1");
        assert_eq!(layout.area("a1").unwrap().active, Some(b.clone()));
        assert!(
            !layout.move_display(&b, "a1", 0, 3).unwrap(),
            "the same move again is the state already reached"
        );
    }

    #[test]
    fn a_view_moved_onto_its_twin_lands_at_the_index_and_the_same_move_again_changes_nothing() {
        // [left of the twin, right of the twin, the twin first in its area]
        for (files, index, expected) in [
            (["x", "a", "y"], 1, 1),
            (["x", "y", "a"], 0, 0),
            (["a", "x", "y"], 1, 1),
        ] {
            let mut layout = with_files(&files);
            let kept = id(&layout, "a");
            let second = layout.new_display("/repo/a", DisplayKind::File, None, false);
            let moved = second.id.clone();
            layout.split_new("a1", Edge::Right, second, 1).unwrap();

            assert!(layout.move_display(&moved, "a1", index, 2).unwrap());
            let area = layout.area("a1").unwrap();
            let ids: Vec<&str> = area
                .displays
                .iter()
                .map(|display| display.id.as_str())
                .collect();
            assert_eq!(ids.len(), 3, "{files:?}: the twin gives way");
            assert_eq!(ids[expected], moved, "{files:?}: the view lands at {index}");
            assert!(!ids.contains(&kept.as_str()), "{files:?}");
            let after = layout.clone();
            assert!(
                !layout.move_display(&moved, "a1", index, 3).unwrap(),
                "{files:?}: the same move again is the state already reached"
            );
            assert_eq!(layout, after, "{files:?}");
        }
    }

    #[test]
    fn the_root_area_stays_when_its_last_display_closes() {
        let mut layout = with_files(&["a"]);
        layout.remove(&id(&layout, "a")).unwrap();
        assert_eq!(layout.area_count(), 1);
        let root = layout.area("a1").expect("the root area stays");
        assert!(root.displays.is_empty());
        assert_eq!(root.active, None);
        assert_eq!(layout.active_area, "a1");
    }

    #[test]
    fn a_ratio_is_clamped_into_bounds_and_a_non_number_is_refused() {
        let mut layout = with_files(&["a", "b"]);
        split(&mut layout, "b", "a1", Edge::Down);
        let Node::Split(root) = &layout.root else {
            panic!("a split root")
        };
        let split_id = root.id.clone();
        let ratio = |layout: &Layout| match &layout.root {
            Node::Split(split) => split.ratio,
            Node::Area(_) => panic!("a split root"),
        };
        assert!(layout.resize(&split_id, 0.01).unwrap());
        assert_eq!(ratio(&layout), MIN_SPLIT_RATIO);
        assert!(
            !layout.resize(&split_id, 0.0).unwrap(),
            "same clamped value"
        );
        layout.resize(&split_id, 0.99).unwrap();
        assert_eq!(ratio(&layout), MAX_SPLIT_RATIO);
        assert_eq!(
            layout.resize(&split_id, f32::NAN),
            Err(LayoutError::InvalidRatio)
        );
        assert_eq!(ratio(&layout), MAX_SPLIT_RATIO);
        assert_eq!(
            layout.resize("s404", 0.5),
            Err(LayoutError::UnknownSplit("s404".to_owned()))
        );
    }

    #[test]
    fn a_neighbour_is_found_across_the_nearest_split_along_each_direction() {
        // a | (b over (c | d)): a left, b top right, c and d bottom right.
        let mut layout = with_files(&["a", "b", "c", "d"]);
        let b = split(&mut layout, "b", "a1", Edge::Right);
        let c = split(&mut layout, "c", &b, Edge::Down);
        let d = split(&mut layout, "d", &c, Edge::Right);
        assert_eq!(
            names(&layout),
            vec![vec!["a"], vec!["b"], vec!["c"], vec!["d"]]
        );
        assert_eq!(layout.neighbour("a1", Edge::Right), Some(b.clone()));
        assert_eq!(layout.neighbour("a1", Edge::Left), None);
        assert_eq!(layout.neighbour(&c, Edge::Left), Some("a1".to_owned()));
        assert_eq!(layout.neighbour(&c, Edge::Right), Some(d.clone()));
        assert_eq!(layout.neighbour(&b, Edge::Down), Some(c.clone()));
        assert_eq!(layout.neighbour(&d, Edge::Up), Some(b.clone()));
        assert_eq!(layout.neighbour(&b, Edge::Up), None);
    }

    /// A crafted file can hold ids and focus stamps at the top of their
    /// numbers, where minting the next one overflows: loading renumbers
    /// them and keeps what the area shows and which view was focused last.
    #[test]
    fn a_tree_at_the_top_of_its_numbers_is_renumbered_on_load() {
        let mut layout = with_files(&["a", "b"]);
        let b = id(&layout, "b");
        for display in layout.displays_mut() {
            let last = display.id == b;
            display.last_focused_unix_ms = if last { u64::MAX } else { u64::MAX - 1 };
            if last {
                display.id = format!("d{}", u64::MAX);
            }
        }
        layout.area_mut("a1").unwrap().active = Some(format!("d{}", u64::MAX));
        layout.next_id = u64::MAX;

        let notes = layout.repair();

        assert_eq!(notes.len(), 2, "{notes:?}");
        assert_eq!(names(&layout), vec![vec!["a", "b"]]);
        let area = layout.active_area();
        assert_eq!(area.active, Some(id(&layout, "b")));
        let shown = successor(&area.displays, 0).unwrap();
        assert_eq!(shown, id(&layout, "b"), "b stays the one focused last");
        let area_id = area.id.clone();
        let fresh = layout.new_display("/repo/c", DisplayKind::File, None, false);
        assert!(layout.displays().all(|display| display.id != fresh.id));
        let stamp = layout.next_stamp(1_700_000_000_000);
        layout.insert(&area_id, fresh, stamp).unwrap();
        assert_eq!(names(&layout), vec![vec!["a", "b", "c"]]);
        assert!(
            layout.clone().repair().is_empty(),
            "a repaired tree is stable"
        );
    }

    /// A crafted file can hold thousands of empty areas: loading drops them
    /// in one pass over the tree and says so in one line.
    #[test]
    fn thousands_of_empty_areas_are_dropped_with_one_note() {
        fn empties(depth: u32, next: &mut u64) -> Node {
            *next += 1;
            if depth == 0 {
                return Node::Area(Area::empty(format!("a{next}")));
            }
            let id = format!("s{next}");
            Node::Split(Split {
                id,
                axis: SplitAxis::Row,
                ratio: 0.5,
                first: Box::new(empties(depth - 1, next)),
                second: Box::new(empties(depth - 1, next)),
            })
        }
        let mut layout = with_files(&["a"]);
        let mut next = layout.next_id;
        let second = empties(12, &mut next);
        layout.root = Node::Split(Split {
            id: format!("s{}", next + 1),
            axis: SplitAxis::Row,
            ratio: 0.5,
            first: Box::new(layout.root.clone()),
            second: Box::new(second),
        });
        layout.next_id = next + 2;

        let notes = layout.repair();

        assert_eq!(names(&layout), vec![vec!["a"]]);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("4096"), "{notes:?}");
    }

    #[test]
    fn repair_restores_the_invariants_of_a_damaged_tree() {
        let mut layout = with_files(&["a", "b", "c"]);
        let fresh = split(&mut layout, "c", "a1", Edge::Right);
        for display in layout.displays_mut() {
            display.preview = true;
        }
        layout.area_mut("a1").unwrap().active = Some("d404".to_owned());
        layout.area_mut(&fresh).unwrap().displays.clear();
        layout.active_area = "a404".to_owned();
        layout.next_id = 1;

        let notes = layout.repair();

        assert!(!notes.is_empty());
        assert_eq!(names(&layout), vec![vec!["a", "b"]]);
        let root = layout.area("a1").unwrap();
        assert!(root.active.is_some());
        assert_eq!(
            root.displays
                .iter()
                .filter(|display| display.preview)
                .count(),
            1
        );
        assert_eq!(layout.active_area, "a1");
        let minted = layout
            .new_display("/repo/e", DisplayKind::File, None, false)
            .id;
        assert!(
            layout.displays().all(|display| display.id != minted),
            "an id already stored is never minted again"
        );
        assert!(
            layout.clone().repair().is_empty(),
            "a repaired tree is stable"
        );
    }

    /// A page is not a document: a browser display never stands for a file
    /// or another page, so a move never takes one out as a twin, and a
    /// stored one keeps its address and title but no load stamp.
    #[test]
    fn a_browser_display_is_named_by_its_address_and_never_a_twin() {
        let mut layout = with_files(&["a"]);
        let first = layout.new_browser_display("https://a.test/", 7);
        let first_id = first.id.clone();
        layout.insert("a1", first, 1).unwrap();
        let second = layout.new_browser_display("https://a.test/", 8);
        let fresh = layout.split_new("a1", Edge::Right, second, 2).unwrap();
        assert!(
            !layout
                .display(&first_id)
                .unwrap()
                .shows("", DisplayKind::Browser, None)
        );
        assert!(layout.move_display(&first_id, &fresh, 0, 3).unwrap());
        assert_eq!(
            layout.area(&fresh).unwrap().displays.len(),
            2,
            "both pages stay"
        );

        let mut stored = layout.clone();
        stored.display_mut(&first_id).unwrap().title = Some("A page".to_owned());
        let text = serde_json::to_string(&stored).unwrap();
        let back: Layout = serde_json::from_str(&text).unwrap();
        let page = back.display(&first_id).unwrap();
        assert_eq!(
            (
                page.kind,
                page.url.as_deref(),
                page.title.as_deref(),
                page.load
            ),
            (
                DisplayKind::Browser,
                Some("https://a.test/"),
                Some("A page"),
                0
            )
        );
        assert!(
            !text.contains("\"url\":null"),
            "a document stores no address: {text}"
        );
    }

    #[test]
    fn a_browser_display_holds_only_an_address_a_page_may_load() {
        for url in [
            "https://a.test/x?y#z",
            "http://localhost:3000",
            "file:///Users/example/a.html",
            "about:blank",
        ] {
            assert!(browser_address(url), "{url}");
        }
        let long = format!("https://a.test/{}", "x".repeat(MAX_BROWSER_URL_BYTES));
        for url in [
            "",
            "javascript:alert(1)",
            "data:text/html,x",
            "about:config",
            "https://",
            "https://a b",
            "https://a\nb",
            "a.test",
            long.as_str(),
        ] {
            assert!(!browser_address(url), "{url}");
        }
        assert_eq!(browser_title(" A\ttab\n "), "A tab");
        assert_eq!(
            browser_title(&"가".repeat(600)).chars().count(),
            MAX_BROWSER_TITLE_CHARS
        );
    }

    /// A stored page with an address it may not load shows the blank page;
    /// a document that carries an address, or a page a path, loses it.
    #[test]
    fn repair_keeps_addresses_to_pages() {
        let mut layout = with_files(&["a"]);
        let mut page = layout.new_browser_display("https://a.test/", 0);
        page.url = Some("javascript:alert(1)".to_owned());
        page.path = "/repo/x".to_owned();
        page.preview = true;
        let page_id = page.id.clone();
        layout.insert("a1", page, 1).unwrap();
        let file = id(&layout, "a");
        layout.display_mut(&file).unwrap().url = Some("https://a.test/".to_owned());

        let notes = layout.repair();
        assert!(
            notes
                .iter()
                .any(|note| note.starts_with("2 views had an address")),
            "{notes:?}"
        );
        let page = layout.display(&page_id).unwrap();
        assert_eq!(
            (page.url.as_deref(), page.path.as_str(), page.preview),
            (Some(BLANK_PAGE), "", false)
        );
        assert_eq!(layout.display(&file).unwrap().url, None);
        assert!(layout.repair().is_empty(), "a repaired tree needs nothing");
    }

    /// PRD session-files D-29: a View stored before Views belonged to
    /// sessions goes to the session its mark names, else to the operator,
    /// silently and once.
    #[test]
    fn a_view_stored_before_sessions_goes_to_its_mark_or_the_operator() {
        let mut layout = with_files(&["marked", "plain"]);
        let marked = id(&layout, "marked");
        let opener = crate::view_feedback::Opener {
            device_id: "local".into(),
            terminal_id: "t1".into(),
            session: Some("s1".into()),
            resume: None,
            resume_path: None,
            provider: "codex".into(),
            name: "Agent".into(),
        };
        layout.display_mut(&marked).unwrap().opener = Some(opener.clone());
        assert!(layout.repair().is_empty(), "the move says nothing");
        let marked = layout.display(&marked).unwrap();
        assert_eq!(
            (marked.openers.as_slice(), marked.operator),
            (std::slice::from_ref(&opener), false)
        );
        let plain = layout.display(&id(&layout, "plain")).unwrap();
        assert!(plain.operator && plain.openers.is_empty());
    }
}
