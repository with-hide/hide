//! The one spelling of an id the core scopes to a device.
//!
//! The core names a device's Herdr objects by prefixing the device's own id:
//! pane `w1:p2` of device `mini` is `remote:mini:pane:w1:p2`, its tab
//! `remote:mini:tab:w1:t1`, and so on. The core's own node keeps Herdr's ids
//! as they are. Every place that builds or reads that form does it here, so
//! the device, the kind and the device's own id are split one way only; the
//! terminal router a node runs reads the same form, which is why this lives
//! in the link crate the core and the node both depend on.
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
/// No Herdr id starts with the prefix, so an id without it is the core's own
/// node's. This is the cheap test for a hot filter; [`Scoped::parse`] and
/// [`PaneAddress::parse`] say what the id names.
pub fn is_device_scoped(id: &str) -> bool {
    id.starts_with(PREFIX)
}

/// What every id the core scoped to `device` starts with, whatever its kind:
/// the scope a device's records are kept and dropped under.
pub fn device_scope(device: &str) -> String {
    format!("{PREFIX}{device}:")
}

/// What every id of `kind` the core scoped to `device` starts with.
pub fn kind_scope(device: &str, kind: Kind) -> String {
    format!("{PREFIX}{device}:{}:", kind.word())
}

/// The device an id was scoped to, of any kind; `None` for the core's own
/// node's ids.
pub fn device_of(id: &str) -> Option<&str> {
    id.strip_prefix(PREFIX)?
        .split_once(':')
        .map(|(device, _)| device)
        .filter(|device| !device.is_empty())
}

/// An id the core scoped to a device: `remote:<device>:<kind>:<id>`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Scoped<'a> {
    pub device: &'a str,
    pub kind: Kind,
    /// The device's own id for it, as its Herdr (or its catalog) names it.
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
        write!(f, "{PREFIX}{}:{}:{}", self.device, self.kind.word(), self.id)
    }
}

/// A pane as the core names it: a pane of its own node by Herdr's id, or a
/// pane of a device by that device and the id the device's Herdr gives it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum PaneAddress<'a> {
    Node(&'a str),
    Device { device: &'a str, pane: &'a str },
}

/// A pane id spelled as a device's (`remote:`) that names no device's pane:
/// its device or its own id is missing, or it names another kind.
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
    /// node's pane keeps Herdr's id.
    pub fn on(node: &str, device: &'a str, pane: &'a str) -> Self {
        if device == node {
            Self::Node(pane)
        } else {
            Self::Device { device, pane }
        }
    }

    /// The pane an id names. An id without the device prefix is the core's
    /// own node's; one with it must name a device's pane.
    pub fn parse(id: &'a str) -> Result<Self, NotAPane> {
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

    /// The pane a caller named on `device`: a full device address stands as
    /// it is, and a bare id is a pane of `device` (the core's own node's when
    /// `device` is `node`).
    pub fn named(node: &str, device: &'a str, id: &'a str) -> Result<Self, NotAPane> {
        match Self::parse(id)? {
            Self::Node(pane) => Ok(Self::on(node, device, pane)),
            address => Ok(address),
        }
    }

    /// The device the pane is on, with `node` standing for the core's own.
    pub fn device_or(self, node: &'a str) -> &'a str {
        match self {
            Self::Node(_) => node,
            Self::Device { device, .. } => device,
        }
    }

    /// The device the pane is on, when it is not the core's own node.
    pub fn device(self) -> Option<&'a str> {
        match self {
            Self::Node(_) => None,
            Self::Device { device, .. } => Some(device),
        }
    }

    /// The id the pane's own Herdr gives it (`w1:p2`), on whichever machine.
    pub fn device_pane_id(self) -> &'a str {
        match self {
            Self::Node(pane) | Self::Device { pane, .. } => pane,
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
        }
    }
}

/// The id `device`'s own Herdr gives the pane `id` names, when `id` is one
/// of `device`'s panes as the core scopes them.
pub fn pane_of<'a>(id: &'a str, device: &str) -> Option<&'a str> {
    Scoped::strip(id, device, Kind::Pane)
}

/// The id `device`'s own Herdr gives the tab `id` names, when `id` is one of
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

/// Pane `pane` of `device` as the core names it, where `node` is the core's
/// own node.
pub fn pane_id_on(node: &str, device: &str, pane: &str) -> String {
    PaneAddress::on(node, device, pane).to_string()
}

/// `id` of `kind` on `device` as the core names it: Herdr's own id on the
/// core's own `node`, the device-scoped id on any other device.
pub fn id_on(node: &str, device: &str, kind: Kind, id: &str) -> String {
    if device == node {
        id.to_owned()
    } else {
        scoped_id(device, kind, id)
    }
}

/// The id `device`'s own Herdr gives what `id` names, when `id` names one of
/// `kind` on `device` as [`id_on`] spells it; `None` when it names another
/// device's or another kind.
pub fn own_id<'a>(node: &str, device: &str, kind: Kind, id: &'a str) -> Option<&'a str> {
    if device == node {
        (!is_device_scoped(id)).then_some(id)
    } else {
        Scoped::strip(id, device, kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_pane_round_trips_and_a_node_pane_keeps_herdrs_id() {
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
        assert_eq!(PaneAddress::parse(&local).unwrap().device_or("core"), "core");
    }

    #[test]
    fn the_same_herdr_pane_on_two_devices_is_two_addresses() {
        let mini = scoped_pane_id("mini", "w1:p1");
        let studio = scoped_pane_id("studio", "w1:p1");
        assert_ne!(mini, studio);
        assert_ne!(PaneAddress::parse(&mini), PaneAddress::parse(&studio));
        assert_ne!(PaneAddress::parse(&mini).unwrap(), PaneAddress::Node("w1:p1"));
        assert_eq!(
            PaneAddress::parse(&mini).unwrap().device_pane_id(),
            PaneAddress::parse(&studio).unwrap().device_pane_id()
        );
    }

    #[test]
    fn a_device_pane_splits_at_the_first_separator() {
        // A Herdr id that itself holds `:pane:` stays the device's own id.
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
            PaneAddress::named("core", "mini", "w1:p1").unwrap().to_string(),
            "remote:mini:pane:w1:p1"
        );
        assert_eq!(
            PaneAddress::named("core", "core", "w1:p1").unwrap().to_string(),
            "w1:p1"
        );
        assert_eq!(
            PaneAddress::named("core", "core", "remote:studio:pane:w1:p1")
                .unwrap()
                .to_string(),
            "remote:studio:pane:w1:p1"
        );
    }

    #[test]
    fn every_kind_round_trips_and_names_its_device() {
        for kind in Kind::ALL {
            let id = scoped_id("mini", kind, "w1:x1");
            assert_eq!(Scoped::parse(&id), Some(Scoped::new("mini", kind, "w1:x1")));
            assert_eq!(Scoped::strip(&id, "mini", kind), Some("w1:x1"));
            assert_eq!(Scoped::strip(&id, "studio", kind), None);
            assert_eq!(device_of(&id), Some("mini"));
            assert!(id.starts_with(&device_scope("mini")));
            assert!(id.starts_with(&kind_scope("mini", kind)));
        }
        assert_eq!(device_of("w1:p1"), None);
        assert!(is_pane_of("remote:mini:pane:w1:p1", "mini"));
        assert!(!is_pane_of("remote:mini:pane:w1:p1", "min"));
        assert!(!is_pane_of("remote:mini:tab:w1:t1", "mini"));
        assert!(!is_pane_of("w1:p1", "mini"));
        assert!(!is_pane_of("remote:mini:pane:", "mini"));
        assert!(is_device_pane("remote:mini:pane:w1:p1"));
        assert!(!is_device_pane("remote:mini:tab:w1:t1"));
        assert!(!is_device_pane("w1:p1"));
        assert_eq!(id_on("core", "mini", Kind::Tab, "w1:t1"), "remote:mini:tab:w1:t1");
        assert_eq!(id_on("core", "core", Kind::Tab, "w1:t1"), "w1:t1");
        assert_eq!(own_id("core", "mini", Kind::Tab, "remote:mini:tab:w1:t1"), Some("w1:t1"));
        assert_eq!(own_id("core", "mini", Kind::Tab, "remote:mini:pane:w1:p1"), None);
        assert_eq!(own_id("core", "core", Kind::Tab, "w1:t1"), Some("w1:t1"));
        assert_eq!(own_id("core", "core", Kind::Tab, "remote:mini:tab:w1:t1"), None);
        assert!(!is_device_scoped("w1:p1"));
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
