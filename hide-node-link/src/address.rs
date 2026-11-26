//! The one spelling of an id the core scopes to a device.
//!
//! The core names a device's objects by prefixing the device's own id:
//! pane `w1:p2` of device `mini` is `remote:mini:pane:w1:p2`, its tab
//! `remote:mini:tab:w1:t1`, and so on. The core's own node keeps the engine's ids
//! as they are. Every place that builds or reads that form does it here, so
//! the device, the kind and the device's own id are split one way only; the
//! terminal router a node runs reads the same form, which is why this lives
//! in the link crate the core and the node both depend on.
//!
//! Two more forms name what is not this core's on a device, and are read
//! here too: `guest:<device>:pane:<id>`, another machine's pane running on
//! this one ([`guest_pane_id`]), and `core:<device>:agent|pane:<id>`, an
//! agent or a pane of another machine's own core ([`CoreAddress`]).
//!
//! The separator after the device is its first `:`, because the core
//! registers and connects no device whose id holds `:`
//! ([`device_id_is_unambiguous`]): an id holding `:pane:` would otherwise
//! read `remote:a:pane:pane:w1` as a pane of `a`, and one device could speak
//! for another's pane.

use std::fmt;

const PREFIX: &str = "remote:";

/// What a device-scoped id names.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Kind {
    Workspace,
    Checkout,
    Tab,
    Pane,
    Agent,
    Project,
}

impl Kind {
    pub const ALL: [Kind; 6] = [
        Kind::Workspace,
        Kind::Checkout,
        Kind::Tab,
        Kind::Pane,
        Kind::Agent,
        Kind::Project,
    ];

    pub fn word(self) -> &'static str {
        match self {
            Kind::Workspace => "workspace",
            Kind::Checkout => "checkout",
            Kind::Tab => "tab",
            Kind::Pane => "pane",
            Kind::Agent => "agent",
            Kind::Project => "project",
        }
    }

    fn from_word(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.word() == word)
    }
}

/// Whether `device` can name a device. An id holding `:` could hold `:pane:`
/// or end in `:pane`, and then `remote:a:pane:pane:w1` or
/// `remote:a:pane:b:pane:w1` would read as a pane of `a`, so one device
/// could speak for another's pane; the core registers and connects no such
/// device. Without `:` the first `:` after the prefix always ends the device.
pub fn device_id_is_unambiguous(device: &str) -> bool {
    !device.contains(':')
}

/// Whether `id` is spelled as one the core scoped to a device, of any kind.
/// No engine id starts with the prefix, so an id without it is the core's own
/// node's. This is the cheap test for a hot filter; [`Scoped::parse`] and
/// [`PaneAddress::parse`] say what the id names.
pub fn is_device_scoped(id: &str) -> bool {
    id.starts_with(PREFIX)
}

/// The device an id was scoped to, of any kind; `None` for the core's own
/// node's ids.
pub fn device_of(id: &str) -> Option<&str> {
    id.strip_prefix(PREFIX)?
        .split_once(':')
        .map(|(device, _)| device)
        .filter(|device| !device.is_empty())
}

const GUEST_PREFIX: &str = "guest:";
const GUEST_PANE: &str = ":pane:";

/// A pane of this machine that belongs to a guest: another machine's work
/// running here in that machine's own node (PRD device-connect-iroh D-61).
/// `device` is the guest machine's device id and `pane` its node's own id
/// for the pane: `guest:<device>:pane:<pane>`.
///
/// It is not `remote:<device>:pane:<pane>`, which names a pane on that
/// machine: two machines that open each other have both, and the two
/// engines name their panes apart from each other, so the same `<pane>`
/// can be either.
pub fn guest_pane_id(device: &str, pane: &str) -> String {
    format!("{GUEST_PREFIX}{device}{GUEST_PANE}{pane}")
}

/// The guest's device id and its node's own pane id, when `id` names a
/// guest's pane on this machine ([`PaneAddress::Guest`]).
pub fn guest_pane(id: &str) -> Option<(&str, &str)> {
    match PaneAddress::parse(id) {
        Ok(PaneAddress::Guest { device, pane }) => Some((device, pane)),
        _ => None,
    }
}

pub const CORE_PREFIX: &str = "core:";

/// What an address of another core names: one of its agents, by the id or
/// the name that core registered it under, or one of its panes, by the id
/// its engine gives it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum CoreItem {
    Agent,
    Pane,
}

impl CoreItem {
    pub const ALL: [CoreItem; 2] = [CoreItem::Agent, CoreItem::Pane];

    pub fn word(self) -> &'static str {
        match self {
            CoreItem::Agent => "agent",
            CoreItem::Pane => "pane",
        }
    }

    fn from_word(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|item| item.word() == word)
    }
}

/// An agent or a pane of another machine's own core, reached through the
/// grant that machine gave this one (PRD device-connect-iroh D-72):
/// `core:<device>:agent:<id>` or `core:<device>:pane:<id>`.
///
/// It is not `remote:<device>:...`, which names what this core keeps on
/// that machine, and not `guest:<device>:...`, which names that machine's
/// work running here: the item is that core's own, so this core's ledger
/// and engine hold nothing of it, and a command that names one is that
/// core's to run.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct CoreAddress<'a> {
    /// The other machine's device id, which is its core's node id.
    pub device: &'a str,
    pub item: CoreItem,
    /// That core's own id for the item.
    pub id: &'a str,
}

/// An id spelled as another core's (`core:`) that names nothing there: its
/// device, its kind or its own id is missing, or the kind is none a core
/// address names.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotACoreItem(pub String);

impl fmt::Display for NotACoreItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} names no agent or pane of another core", self.0)
    }
}

impl std::error::Error for NotACoreItem {}

impl<'a> CoreAddress<'a> {
    pub fn new(device: &'a str, item: CoreItem, id: &'a str) -> Self {
        Self { device, item, id }
    }

    /// What `id` names when it is spelled as another core's: `None` for any
    /// id without the prefix, which is this core's to read, and an error for
    /// one with it that names nothing.
    pub fn parse(id: &'a str) -> Option<Result<Self, NotACoreItem>> {
        let rest = id.strip_prefix(CORE_PREFIX)?;
        let parsed = rest.split_once(':').and_then(|(device, rest)| {
            let (item, own) = rest.split_once(':')?;
            let item = CoreItem::from_word(item)?;
            (!device.is_empty() && !own.is_empty()).then_some(Self {
                device,
                item,
                id: own,
            })
        });
        Some(parsed.ok_or_else(|| NotACoreItem(id.to_owned())))
    }
}

impl fmt::Display for CoreAddress<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{CORE_PREFIX}{}:{}:{}",
            self.device,
            self.item.word(),
            self.id
        )
    }
}

/// Whether `id` is spelled as another core's item, well formed or not
/// ([`CoreAddress::parse`] says which).
pub fn is_core_scoped(id: &str) -> bool {
    id.starts_with(CORE_PREFIX)
}

/// An id the core scoped to a device: `remote:<device>:<kind>:<id>`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Scoped<'a> {
    pub device: &'a str,
    pub kind: Kind,
    /// The device's own id for it, as its engine (or its catalog) names it.
    pub id: &'a str,
}

impl<'a> Scoped<'a> {
    pub fn new(device: &'a str, kind: Kind, id: &'a str) -> Self {
        Self { device, kind, id }
    }

    /// The parts of a device-scoped id; `None` for any other id, including
    /// one with the prefix whose device, kind or own id is missing.
    pub fn parse(scoped: &'a str) -> Option<Self> {
        let rest = scoped.strip_prefix(PREFIX)?;
        let (device, rest) = rest.split_once(':')?;
        let (kind, id) = rest.split_once(':')?;
        let kind = Kind::from_word(kind)?;
        (!device.is_empty() && !id.is_empty()).then_some(Self { device, kind, id })
    }

    /// The device's own id when `scoped` is an id of `kind` on `device`.
    pub fn strip(scoped: &'a str, device: &str, kind: Kind) -> Option<&'a str> {
        Self::parse(scoped)
            .filter(|parsed| parsed.device == device && parsed.kind == kind)
            .map(|parsed| parsed.id)
    }
}

impl fmt::Display for Scoped<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{PREFIX}{}:{}:{}",
            self.device,
            self.kind.word(),
            self.id
        )
    }
}

/// A pane as the core names it: a pane of its own node by the engine's id, a
/// pane of a device by that device and the id the device's engine gives it,
/// or a guest's pane on this machine by the guest's device and the id the
/// guest's node gives it ([`guest_pane_id`]).
///
/// A guest's pane is none of the core's: its engine runs no such pane, so
/// wherever an id is taken to the engine a guest's reads as a pane the
/// engine does not have.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum PaneAddress<'a> {
    Node(&'a str),
    Device { device: &'a str, pane: &'a str },
    Guest { device: &'a str, pane: &'a str },
}

/// A pane id spelled as a device's (`remote:`), a guest's (`guest:`) or
/// another core's (`core:`) that names no pane of this core: its device or
/// its own id is missing, it names another kind, or it is another core's.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotAPane(pub String);

impl fmt::Display for NotAPane {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} names no pane", self.0)
    }
}

impl std::error::Error for NotAPane {}

impl<'a> PaneAddress<'a> {
    /// Pane `pane` of `device`, where `node` is the core's own node: the
    /// node's pane keeps the engine's id.
    pub fn on(node: &str, device: &'a str, pane: &'a str) -> Self {
        if device == node {
            Self::Node(pane)
        } else {
            Self::Device { device, pane }
        }
    }

    /// The pane an id names. An id without the device prefix is the core's
    /// own node's; one with it must name a device's pane, and one spelled as
    /// another core's names none of this core's.
    pub fn parse(id: &'a str) -> Result<Self, NotAPane> {
        if let Some(rest) = id.strip_prefix(GUEST_PREFIX) {
            return match rest.split_once(GUEST_PANE) {
                Some((device, pane))
                    if !device.is_empty()
                        && !pane.is_empty()
                        && device_id_is_unambiguous(device) =>
                {
                    Ok(Self::Guest { device, pane })
                }
                _ => Err(NotAPane(id.to_owned())),
            };
        }
        // Another core's pane is none of this core's: a command that names
        // one is that core's to run ([`CoreAddress`]), so where an id is read
        // as a pane of this core it is no pane.
        if is_core_scoped(id) {
            return Err(NotAPane(id.to_owned()));
        }
        if !is_device_scoped(id) {
            return Ok(Self::Node(id));
        }
        match Scoped::parse(id) {
            Some(Scoped {
                device,
                kind: Kind::Pane,
                id: pane,
            }) => Ok(Self::Device { device, pane }),
            _ => Err(NotAPane(id.to_owned())),
        }
    }

    /// The pane an id names on `device`, where `node` is the core's own
    /// node: a full device address stands as it is, and a bare id is a pane
    /// of `device`.
    pub fn named(node: &str, device: &'a str, id: &'a str) -> Result<Self, NotAPane> {
        match Self::parse(id)? {
            Self::Node(pane) => Ok(Self::on(node, device, pane)),
            address => Ok(address),
        }
    }

    /// The device the pane is on, when it is not the core's own node. A
    /// guest's pane runs on this machine.
    pub fn device(self) -> Option<&'a str> {
        match self {
            Self::Node(_) | Self::Guest { .. } => None,
            Self::Device { device, .. } => Some(device),
        }
    }

    /// The id the pane's own engine gives it (`w1:p2`), on whichever machine.
    pub fn device_pane_id(self) -> &'a str {
        match self {
            Self::Node(pane) | Self::Device { pane, .. } | Self::Guest { pane, .. } => pane,
        }
    }

    pub fn is_device(self) -> bool {
        matches!(self, Self::Device { .. })
    }
}

impl fmt::Display for PaneAddress<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Node(pane) => f.write_str(pane),
            Self::Device { device, pane } => Scoped::new(device, Kind::Pane, pane).fmt(f),
            Self::Guest { device, pane } => f.write_str(&guest_pane_id(device, pane)),
        }
    }
}

/// The id `device`'s own engine gives the pane `id` names, when `id` is one
/// of `device`'s panes as the core scopes them.
pub fn pane_of<'a>(id: &'a str, device: &str) -> Option<&'a str> {
    Scoped::strip(id, device, Kind::Pane)
}

/// The id `device`'s own engine gives the tab `id` names, when `id` is one of
/// `device`'s tabs as the core scopes them.
pub fn tab_of<'a>(id: &'a str, device: &str) -> Option<&'a str> {
    Scoped::strip(id, device, Kind::Tab)
}

/// Whether `id` names a device's pane ([`PaneAddress::Device`]).
pub fn is_device_pane(id: &str) -> bool {
    PaneAddress::parse(id).is_ok_and(PaneAddress::is_device)
}

/// Whether `id` is spelled as one of `device`'s panes ([`pane_of`]).
pub fn is_pane_of(id: &str, device: &str) -> bool {
    pane_of(id, device).is_some()
}

/// The id the core gives pane `pane` of `device`.
pub fn scoped_pane_id(device: &str, pane: &str) -> String {
    Scoped::new(device, Kind::Pane, pane).to_string()
}

/// The id the core gives tab `tab` of `device`.
pub fn scoped_tab_id(device: &str, tab: &str) -> String {
    Scoped::new(device, Kind::Tab, tab).to_string()
}

/// The id the core gives `id` of `kind` on `device`.
pub fn scoped_id(device: &str, kind: Kind, id: &str) -> String {
    Scoped::new(device, kind, id).to_string()
}

/// The id the core gives a project of `device`, from the id its machine
/// derives for it (`hide_project::project_id`, `project:<hash>`): the same
/// kind and hash, scoped to the device.
///
/// # Panics
///
/// When `project` is not spelled `project:<hash>`, which no project id is.
pub fn scoped_project_id(device: &str, project: &str) -> String {
    let hash = project
        .strip_prefix(Kind::Project.word())
        .and_then(|rest| rest.strip_prefix(':'))
        .filter(|hash| !hash.is_empty())
        .unwrap_or_else(|| panic!("{project} is not a project id"));
    scoped_id(device, Kind::Project, hash)
}

/// Pane `pane` of `device` as the core names it, where `node` is the core's
/// own node.
pub fn pane_id_on(node: &str, device: &str, pane: &str) -> String {
    PaneAddress::on(node, device, pane).to_string()
}

/// Pane `pane` of `machine` as an answer names it to a caller on machine
/// `caller`: bare on the caller's own machine, which is how a bare id it
/// types reads ([`named_pane_id`]), and `remote:<machine>:pane:<pane>` on
/// any other, the core's own node included. Every answer that names a pane
/// to a caller spells it here, so the caller uses it as written and never
/// builds one from `machine` and `pane`. It is relative to the caller, not
/// to the core's node as [`pane_id_on`] is: a device's agent reads its own
/// pane bare.
pub fn pane_for_caller(caller: &str, machine: &str, pane: &str) -> String {
    PaneAddress::on(caller, machine, pane).to_string()
}

/// A pane another machine's own core `device` named in its answer, as a
/// caller of this core uses it: `core:<device>:pane:<pane>`. That core
/// names its panes relative to itself (bare on its own node,
/// `remote:<its device>:pane:<id>` on one of its devices), and `device`'s
/// core reads the id back the same way when this core passes it on.
pub fn core_pane_id(device: &str, pane: &str) -> String {
    CoreAddress::new(device, CoreItem::Pane, pane).to_string()
}

/// The core's id for the pane a caller named on `device`, where `node` is
/// the core's own node. The engine numbers panes per machine, so a bare id is a
/// pane of the caller's machine; an id already scoped to a device is a full
/// address and stands as it is.
pub fn named_pane_id(node: &str, device: &str, id: &str) -> String {
    if device == node || is_device_scoped(id) {
        id.to_owned()
    } else {
        scoped_pane_id(device, id)
    }
}

/// Whether two ids named on `device` are the same pane: a bare id is a pane
/// of `device` and a full address names its own device, so the same engine
/// id on two devices is never one pane.
pub fn same_pane(node: &str, device: &str, a: &str, b: &str) -> bool {
    a == b
        || matches!(
            (PaneAddress::named(node, device, a), PaneAddress::named(node, device, b)),
            (Ok(a), Ok(b)) if a == b
        )
}

/// The machine a pane id is on and the id that machine's engine gives it: a
/// device's pane splits into its device and own id, and any other id is a
/// pane of `machine` (the core's own node, or the caller's machine for a
/// bare id a caller typed).
pub fn split_pane<'a>(machine: &'a str, id: &'a str) -> (&'a str, &'a str) {
    match PaneAddress::parse(id) {
        Ok(PaneAddress::Device { device, pane }) => (device, pane),
        _ => (machine, id),
    }
}

/// The device a pane id names, when it is not the core's own node's.
pub fn pane_device(id: &str) -> Option<&str> {
    PaneAddress::parse(id).ok().and_then(PaneAddress::device)
}

/// `id` of `kind` on `device` as the core names it: the engine's own id on the
/// core's own `node`, the device-scoped id on any other device.
pub fn id_on(node: &str, device: &str, kind: Kind, id: &str) -> String {
    if device == node {
        id.to_owned()
    } else {
        scoped_id(device, kind, id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_names_each_pane_as_its_caller_uses_it() {
        // (caller's machine, pane's machine, engine id, address the caller is handed)
        for (caller, machine, pane, expected) in [
            // A caller on the core's own machine.
            ("core", "core", "h5:p2", "h5:p2"),
            ("core", "mini", "h5:p2", "remote:mini:pane:h5:p2"),
            // A caller on a device this core runs: its own pane bare, the
            // core's node and another device scoped.
            ("mini", "mini", "h5:p2", "h5:p2"),
            ("mini", "core", "w1:p1", "remote:core:pane:w1:p1"),
            ("mini", "studio", "h5:p2", "remote:studio:pane:h5:p2"),
        ] {
            assert_eq!(
                pane_for_caller(caller, machine, pane),
                expected,
                "{caller} {machine} {pane}"
            );
        }
        // A pane another machine's own core named, bare on its node or
        // scoped to one of its devices, as this core's caller is handed it.
        assert_eq!(core_pane_id("mini", "h5:p2"), "core:mini:pane:h5:p2");
        assert_eq!(
            core_pane_id("mini", "remote:studio:pane:w1:p1"),
            "core:mini:pane:remote:studio:pane:w1:p1"
        );
        assert_eq!(
            CoreAddress::parse("core:mini:pane:h5:p2"),
            Some(Ok(CoreAddress::new("mini", CoreItem::Pane, "h5:p2")))
        );
    }

    #[test]
    fn a_device_pane_round_trips_and_a_node_pane_keeps_the_engine_s_id() {
        let remote = pane_id_on("core", "mini", "w1:p2");
        assert_eq!(remote, "remote:mini:pane:w1:p2");
        assert_eq!(
            PaneAddress::parse(&remote),
            Ok(PaneAddress::Device {
                device: "mini",
                pane: "w1:p2"
            })
        );
        let local = pane_id_on("core", "core", "w1:p2");
        assert_eq!(local, "w1:p2");
        assert_eq!(PaneAddress::parse(&local), Ok(PaneAddress::Node("w1:p2")));
    }

    #[test]
    fn the_same_engine_pane_on_two_devices_is_two_addresses() {
        let mini = scoped_pane_id("mini", "w1:p1");
        let studio = scoped_pane_id("studio", "w1:p1");
        assert_ne!(mini, studio);
        assert_ne!(PaneAddress::parse(&mini), PaneAddress::parse(&studio));
        assert_ne!(
            PaneAddress::parse(&mini).unwrap(),
            PaneAddress::Node("w1:p1")
        );
        assert_eq!(
            PaneAddress::parse(&mini).unwrap().device_pane_id(),
            PaneAddress::parse(&studio).unwrap().device_pane_id()
        );
    }

    #[test]
    fn a_device_pane_splits_at_the_first_separator() {
        // An engine id that itself holds `:pane:` stays the device's own id.
        assert_eq!(
            PaneAddress::parse("remote:mini:pane:w1:pane:p2"),
            Ok(PaneAddress::Device {
                device: "mini",
                pane: "w1:pane:p2"
            })
        );
    }

    #[test]
    fn a_device_spelled_id_that_names_no_pane_is_refused() {
        for id in [
            "remote:",
            "remote:mini",
            "remote:mini:pane:",
            "remote::pane:w1:p1",
            "remote:mini:tab:w1:t1",
            "remote:mini:agent:w1:p1",
        ] {
            assert_eq!(PaneAddress::parse(id), Err(NotAPane(id.to_owned())), "{id}");
        }
    }

    #[test]
    fn a_named_pane_is_qualified_by_the_device_unless_it_already_is() {
        assert_eq!(
            named_pane_id("core", "mini", "w1:p1"),
            "remote:mini:pane:w1:p1"
        );
        assert_eq!(named_pane_id("core", "core", "w1:p1"), "w1:p1");
        assert_eq!(
            named_pane_id("core", "mini", "remote:studio:pane:w1:p1"),
            "remote:studio:pane:w1:p1"
        );
    }

    #[test]
    fn a_pane_is_the_same_only_on_the_same_device() {
        assert!(same_pane("core", "mini", "w1:p1", "remote:mini:pane:w1:p1"));
        assert!(same_pane("core", "core", "w1:p1", "w1:p1"));
        assert!(!same_pane(
            "core",
            "mini",
            "remote:studio:pane:w1:p1",
            "w1:p1"
        ));
        assert!(!same_pane(
            "core",
            "mini",
            "remote:studio:pane:w1:p1",
            "remote:mini:pane:w1:p1"
        ));
        assert!(!same_pane(
            "core",
            "core",
            "remote:mini:pane:w1:p1",
            "w1:p1"
        ));
        assert_eq!(pane_device("remote:mini:pane:w1:p1"), Some("mini"));
        assert_eq!(pane_device("w1:p1"), None);
        assert_eq!(split_pane("mini", "w1:p1"), ("mini", "w1:p1"));
        assert_eq!(
            split_pane("core", "remote:mini:pane:w1:p1"),
            ("mini", "w1:p1")
        );
        assert_eq!(pane_device("remote:mini:tab:w1:t1"), None);
    }

    #[test]
    fn every_kind_round_trips_and_names_its_device() {
        for kind in Kind::ALL {
            let id = scoped_id("mini", kind, "w1:x1");
            assert_eq!(Scoped::parse(&id), Some(Scoped::new("mini", kind, "w1:x1")));
            assert_eq!(Scoped::strip(&id, "mini", kind), Some("w1:x1"));
            assert_eq!(Scoped::strip(&id, "studio", kind), None);
            assert_eq!(device_of(&id), Some("mini"));
        }
        assert_eq!(device_of("w1:p1"), None);
        assert_eq!(
            scoped_project_id("mini", "project:ab12"),
            "remote:mini:project:ab12"
        );
        assert!(is_pane_of("remote:mini:pane:w1:p1", "mini"));
        assert!(!is_pane_of("remote:mini:pane:w1:p1", "min"));
        assert!(!is_pane_of("remote:mini:tab:w1:t1", "mini"));
        assert!(!is_pane_of("w1:p1", "mini"));
        assert!(!is_pane_of("remote:mini:pane:", "mini"));
        assert!(is_device_pane("remote:mini:pane:w1:p1"));
        assert!(!is_device_pane("remote:mini:tab:w1:t1"));
        assert!(!is_device_pane("w1:p1"));
        assert_eq!(
            id_on("core", "mini", Kind::Tab, "w1:t1"),
            "remote:mini:tab:w1:t1"
        );
        assert_eq!(id_on("core", "core", Kind::Tab, "w1:t1"), "w1:t1");
        assert!(!is_device_scoped("w1:p1"));
    }

    #[test]
    fn a_guests_pane_here_is_no_pane_of_that_machine() {
        let here = guest_pane_id("macbook", "w1:p1");
        assert_eq!(here, "guest:macbook:pane:w1:p1");
        assert_eq!(guest_pane(&here), Some(("macbook", "w1:p1")));
        // The same machine's own pane of the same engine id is another
        // address, and neither reads as the other.
        let there = scoped_pane_id("macbook", "w1:p1");
        assert_ne!(here, there);
        assert_eq!(guest_pane(&there), None);
        assert_eq!(device_of(&here), None);
        assert!(!is_device_scoped(&here));
        // A node id that itself holds `:pane:` stays the node's own id.
        assert_eq!(
            guest_pane("guest:macbook:pane:a:pane:b"),
            Some(("macbook", "a:pane:b"))
        );
        for id in [
            "guest:",
            "guest::pane:w1:p1",
            "guest:macbook:pane:",
            "w1:p1",
        ] {
            assert_eq!(guest_pane(id), None, "{id}");
        }
    }

    #[test]
    fn another_cores_agent_and_pane_round_trip_and_are_no_pane_of_this_core() {
        for (item, own) in [(CoreItem::Agent, "agent-12"), (CoreItem::Pane, "w1:p2")] {
            let address = CoreAddress::new("mini", item, own);
            let text = address.to_string();
            assert_eq!(text, format!("core:mini:{}:{own}", item.word()));
            assert_eq!(CoreAddress::parse(&text), Some(Ok(address)));
            assert!(is_core_scoped(&text));
            // Neither this core's own on that machine nor that machine's
            // work here.
            assert!(!is_device_scoped(&text));
            assert_eq!(device_of(&text), None);
            assert_eq!(guest_pane(&text), None);
            assert_eq!(PaneAddress::parse(&text), Err(NotAPane(text.clone())));
            assert_eq!(pane_device(&text), None);
        }
        // The other core's own id is everything after the kind, separators
        // included.
        assert_eq!(
            CoreAddress::parse("core:mini:pane:remote:studio:pane:w1:p1"),
            Some(Ok(CoreAddress::new(
                "mini",
                CoreItem::Pane,
                "remote:studio:pane:w1:p1"
            )))
        );
        // An id of this core's is not read here at all.
        for id in [
            "w1:p1",
            "agent-12",
            "remote:mini:pane:w1:p1",
            "guest:mini:pane:w1:p1",
        ] {
            assert_eq!(CoreAddress::parse(id), None, "{id}");
        }
        for id in [
            "core:",
            "core:mini",
            "core:mini:agent",
            "core:mini:agent:",
            "core::agent:agent-1",
            "core:mini:tab:w1:t1",
            "core:mini:workspace:w1",
        ] {
            assert_eq!(
                CoreAddress::parse(id),
                Some(Err(NotACoreItem(id.to_owned()))),
                "{id}"
            );
        }
    }

    #[test]
    fn only_a_device_id_without_a_separator_can_name_a_device() {
        for device in ["mini", "14ad0319-60d7-5a7c-ae5a-f71550dc8a60", "wsl-Ubuntu"] {
            assert!(device_id_is_unambiguous(device), "{device}");
        }
        for device in ["a:pane", "a:pane:b", "x:"] {
            assert!(!device_id_is_unambiguous(device), "{device}");
        }
    }
}
