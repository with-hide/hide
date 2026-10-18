//! Every node's terminals behind one [`TerminalRoutes`], as the core and the
//! screen on this machine reach them (PRD core-host-node-terminal D-02,
//! D-11). This machine's panes go to its own [`super::Service`] by their own
//! ids; a device's panes, which the core names `remote:<device>:pane:<id>`,
//! go to the device's node inside its link, by the device's own ids.
//!
//! Nothing here holds a byte for long: a key, a control or a frame takes
//! the routing lock for a map lookup and goes on.
//!
//! A device's node is another machine's program, so what it sends up is
//! taken only where the core asked for it: output for a pane the core
//! attached on that device, a report about such a pane or one on screen, and
//! a paste's outcome for a paste held on that device. Its word that a key
//! moved the keyboard counts only for the pane the last screen key went
//! to, within [`KEY_FOCUS_WINDOW`] of that key. What it says is bounded
//! before the core reads it: every name is a short plain name, a message is
//! cut to [`DEVICE_MESSAGE_CHARS`], and at most
//! [`DEVICE_REPORTS_PER_WINDOW`] reports a window are taken.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use hide_node_link::terminal::{
    GridSize, KeyTarget, PaneTerminalState, ReportSink, TerminalControl, TerminalNode,
    TerminalReport, TerminalRoutes, device_pane_id, split_device_pane_id,
};
use serde_json::json;

use super::OutputSink;
use super::device::DeviceSink;
use super::input::{INPUT_REPORT_WINDOW, Refusal, Refusals, Unreported};

/// How long after a screen key a device's word that the key moved the
/// keyboard is believed; its node reports that at once.
const KEY_FOCUS_WINDOW: Duration = Duration::from_secs(2);
/// The longest message a device's report carries to the core, in
/// characters.
const DEVICE_MESSAGE_CHARS: usize = 512;
/// The longest name (a state, a mode, a decision, a kind) a device's report
/// may carry.
const DEVICE_NAME_BYTES: usize = 64;
/// Reports one device's node may send the core in one
/// [`INPUT_REPORT_WINDOW`]; past it they are refused, far above what its
/// panes' attaches and keys make.
const DEVICE_REPORTS_PER_WINDOW: u32 = 1024;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Where one pane's terminal is.
enum Route {
    Local,
    Device {
        device: String,
        node: Arc<dyn TerminalNode>,
        pane: String,
    },
    /// A device pane whose device has no link now.
    Unrouted,
}

#[derive(Default)]
struct Routes {
    devices: HashMap<String, Arc<dyn TerminalNode>>,
    /// Devices whose link is up without terminals, and why.
    unstarted: HashMap<String, String>,
    /// The panes on screen, as the core last said, by the core's ids.
    shown: Vec<String>,
    /// The pane the core's keyboard is in, as it last said, by its id.
    focus: Option<String>,
    /// The device panes the core said hold a sleeping agent, and those it
    /// said are closing, by its ids: a device's next link starts a fresh
    /// node, which is told them. Each mirrors a set of the core's.
    asleep: BTreeSet<String>,
    closing: BTreeSet<String>,
    /// Which device a paste's held input is on; a paste on this machine
    /// has none.
    intents: HashMap<String, Option<String>>,
    /// The device panes the core attached, by device and by the core's ids:
    /// only their output reaches the screen's hub, and a device that leaves
    /// takes their output with it.
    attached: HashMap<String, HashSet<String>>,
    /// The pane the last screen key went to, by the core's ids, and when.
    keyed: Option<(String, Instant)>,
    /// Each device's reports in its current window.
    rates: HashMap<String, ReportRate>,
    /// Keys refused for a device with no link, and lines a device sent
    /// that were not taken, logged once a window and counted after that.
    refusals: Refusals,
    /// Whether a screen of this machine draws device panes, as
    /// [`Router::mirror`] last said; every node is told on its link.
    mirror: bool,
}

struct ReportRate {
    opened: Instant,
    reports: u32,
}

impl Routes {
    fn attached(&self, device: &str, pane: &str) -> bool {
        self.attached
            .get(device)
            .is_some_and(|panes| panes.contains(pane))
    }

    /// Whether `device` may speak for `pane`: the pane is that device's, by
    /// the router's own reading of the id, and the core attached it there
    /// or shows it.
    fn speaks_for(&self, device: &str, pane: &str) -> bool {
        Router::device_of(pane).is_some_and(|(owner, _)| owner == device)
            && (self.attached(device, pane) || self.shown.iter().any(|shown| shown == pane))
    }

    /// Whether the screen's last key went to `pane` recently enough for a
    /// device's word that it moved the keyboard.
    fn keyed_recently(&self, pane: &str, now: Instant) -> bool {
        self.keyed.as_ref().is_some_and(|(keyed, at)| {
            keyed == pane && now.saturating_duration_since(*at) < KEY_FOCUS_WINDOW
        })
    }

    /// Whether `device` may send one more report in its window.
    fn within_rate(&mut self, device: &str, now: Instant) -> bool {
        let rate = self.rates.entry(device.to_owned()).or_insert(ReportRate {
            opened: now,
            reports: 0,
        });
        if now.saturating_duration_since(rate.opened) >= INPUT_REPORT_WINDOW {
            *rate = ReportRate {
                opened: now,
                reports: 0,
            };
        }
        rate.reports += 1;
        rate.reports <= DEVICE_REPORTS_PER_WINDOW
    }

    /// Whether a line `device` sent that was not taken, for `kind`, is
    /// logged now; the rest of its window's are counted.
    fn refused_line(&mut self, device: &str, kind: &'static str) -> bool {
        self.refusals.refuse(device, kind, Instant::now()) == Refusal::Report
    }

    /// The counts of every refusal window that has passed. The router has
    /// no clock, so they are taken on its next key or device line.
    fn due(&mut self) -> Vec<Unreported> {
        if self.refusals.is_empty() {
            return Vec::new();
        }
        self.refusals.due(Instant::now())
    }
}

/// Why a device's line was not taken, as the log names it.
const LINE_UNADMITTED: &str = "terminal.device_line_unadmitted";
const REPORT_MALFORMED: &str = "terminal.device_report_malformed";
const REPORTS_OVER_RATE: &str = "terminal.device_reports_over_rate";

fn log_due(due: Vec<Unreported>) {
    for unreported in due {
        unreported.log("terminal_router");
    }
}

fn log_refused_line(device: &str, kind: &str, what: &str) {
    crate::diagnostic!(json!({
        "component": "terminal_router",
        "kind": kind,
        "device": device,
        "line": what,
    }));
}

/// A name a device's report may carry: short, lowercase, digits, `_`, `.`.
fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= DEVICE_NAME_BYTES
        && name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'.')
        })
}

fn cut(message: &mut String) {
    if let Some((at, _)) = message.char_indices().nth(DEVICE_MESSAGE_CHARS) {
        message.truncate(at);
    }
}

/// Bounds a device's report before the core reads it: false when a name in
/// it is not a plain name, else its messages cut to size.
fn bound_device_report(report: &mut TerminalReport) -> bool {
    match report {
        TerminalReport::State { state, .. } => {
            let names = [
                Some(&state.state),
                state.mode.as_ref(),
                Some(&state.retry_decision),
            ]
            .into_iter()
            .chain([state.exit_category.as_ref()])
            .flatten();
            if !names.into_iter().all(|name| plain_name(name)) {
                return false;
            }
            if let Some(message) = &mut state.message {
                cut(message);
            }
            true
        }
        TerminalReport::Error { kind, message, .. }
        | TerminalReport::Note { kind, message, .. } => {
            if !plain_name(kind) {
                return false;
            }
            cut(message);
            true
        }
        _ => true,
    }
}

/// This machine's terminals and its devices', routed by pane.
pub struct Router {
    local: Arc<dyn TerminalNode>,
    hub: Arc<dyn OutputSink>,
    reports: Arc<dyn ReportSink>,
    routes: Mutex<Routes>,
}

impl Router {
    /// `local` is this machine's service, which writes to `hub` and reports
    /// to `reports` itself; a device's output and reports reach them through
    /// the router ([`DeviceSink`]).
    pub fn new(
        local: Arc<dyn TerminalNode>,
        hub: Arc<dyn OutputSink>,
        reports: Arc<dyn ReportSink>,
    ) -> Self {
        Self {
            local,
            hub,
            reports,
            routes: Mutex::default(),
        }
    }

    /// The device a scoped pane id names, and the device's own id for it.
    fn device_of(pane: &str) -> Option<(&str, &str)> {
        split_device_pane_id(pane)
    }

    fn route(&self, pane: &str) -> Route {
        Self::route_in(&lock(&self.routes), pane)
    }

    fn route_in(routes: &Routes, pane: &str) -> Route {
        match Self::device_of(pane) {
            None => Route::Local,
            Some((device, source)) => match routes.devices.get(device) {
                Some(node) => Route::Device {
                    node: Arc::clone(node),
                    pane: source.to_owned(),
                    device: device.to_owned(),
                },
                None => Route::Unrouted,
            },
        }
    }

    fn unrouted_attach(&self, pane: String) {
        let unstarted = Self::device_of(&pane)
            .and_then(|(device, _)| lock(&self.routes).unstarted.get(device).cloned());
        let message = match unstarted {
            Some(reason) => format!(
                "This device's helper is connected but carries no terminals ({reason}), so its terminal cannot attach; it attaches when the helper connects with them"
            ),
            None => "This device's helper is not connected, so its terminal cannot attach; it attaches when the helper connects".to_owned(),
        };
        self.reports.report(TerminalReport::State {
            pane,
            state: PaneTerminalState {
                state: "unavailable".to_owned(),
                mode: None,
                generation: 0,
                attempt: 0,
                message: Some(message),
                exit_category: Some("device_unavailable".to_owned()),
                retry_decision: "manual".to_owned(),
                last_attempt_at_unix_ms: None,
            },
        });
    }

    /// Sends each device the panes of its own on screen.
    fn shown_to(routes: &Routes, device: &str, node: &dyn TerminalNode) {
        let panes = routes
            .shown
            .iter()
            .filter_map(|pane| Self::device_of(pane))
            .filter(|(owner, _)| *owner == device)
            .map(|(_, source)| source.to_owned())
            .collect();
        node.control(TerminalControl::Shown { panes });
    }

    /// Tells a device whether the core's keyboard is in one of its panes.
    fn focus_to(routes: &Routes, device: &str, node: &dyn TerminalNode) {
        let pane = routes
            .focus
            .as_deref()
            .and_then(Self::device_of)
            .filter(|(owner, _)| *owner == device)
            .map(|(_, source)| source.to_owned());
        node.control(TerminalControl::Focus { pane });
    }

    /// Removes every device's terminals: each link's writer stops, and
    /// nothing of a device's holds this router any longer. A device's node
    /// reports to the router that holds it, so the two keep each other
    /// alive until this runs; its owner calls it once the core that
    /// installs devices is gone.
    pub fn remove_every_device(&self) {
        let devices = lock(&self.routes)
            .devices
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for device in devices {
            self.remove_device(&device);
        }
    }

    /// Tells every node whether a screen of this machine draws its panes
    /// (`TerminalControl::Mirror`): a node that draws them on its own
    /// screens sends their output up only while one does, and a node with
    /// no screen of its own sends it always.
    pub fn mirror(&self, on: bool) {
        let mut routes = lock(&self.routes);
        if routes.mirror == on {
            return;
        }
        routes.mirror = on;
        let nodes = routes.devices.values().cloned().collect::<Vec<_>>();
        drop(routes);
        for node in nodes {
            node.control(TerminalControl::Mirror { on });
        }
    }

    /// Tells a device which of its panes hold a sleeping agent or are
    /// closing.
    fn flags_to(routes: &Routes, device: &str, node: &dyn TerminalNode) {
        let own = |panes: &BTreeSet<String>| {
            panes
                .iter()
                .filter_map(|pane| Self::device_of(pane))
                .filter(|(owner, _)| *owner == device)
                .map(|(_, source)| source.to_owned())
                .collect::<Vec<_>>()
        };
        for pane in own(&routes.asleep) {
            node.control(TerminalControl::Asleep { pane, asleep: true });
        }
        for pane in own(&routes.closing) {
            node.control(TerminalControl::Closing {
                pane,
                closing: true,
            });
        }
    }
}

impl TerminalNode for Router {
    fn input_permit(
        &self,
        pane: &str,
        bytes: usize,
        reply: hide_node_link::terminal::InputPermitReply,
    ) {
        match self.route(pane) {
            Route::Local => self.local.input_permit(pane, bytes, reply),
            Route::Device { node, pane, .. } => node.input_permit(&pane, bytes, reply),
            Route::Unrouted => reply(Err(hide_node_link::terminal::InputPermitError::Unavailable)),
        }
    }

    fn control(&self, mut control: TerminalControl) {
        match &mut control {
            TerminalControl::Shown { panes } => {
                let mut routes = lock(&self.routes);
                routes.shown = panes.clone();
                let local = panes
                    .iter()
                    .filter(|pane| Self::device_of(pane).is_none())
                    .cloned()
                    .collect();
                for (device, node) in &routes.devices {
                    Self::shown_to(&routes, device, node.as_ref());
                }
                drop(routes);
                self.local.control(TerminalControl::Shown { panes: local });
                return;
            }
            TerminalControl::Focus { pane } => {
                let mut routes = lock(&self.routes);
                routes.focus = pane.clone();
                let local = pane
                    .as_ref()
                    .filter(|pane| Self::device_of(pane).is_none())
                    .cloned();
                for (device, node) in &routes.devices {
                    Self::focus_to(&routes, device, node.as_ref());
                }
                drop(routes);
                self.local.control(TerminalControl::Focus { pane: local });
                return;
            }
            // Kept for the device's next link, then routed like any control.
            TerminalControl::Asleep { pane, asleep } if Self::device_of(pane).is_some() => {
                let mut routes = lock(&self.routes);
                if *asleep {
                    routes.asleep.insert(pane.clone());
                } else {
                    routes.asleep.remove(pane.as_str());
                }
            }
            TerminalControl::Closing { pane, closing } if Self::device_of(pane).is_some() => {
                let mut routes = lock(&self.routes);
                if *closing {
                    routes.closing.insert(pane.clone());
                } else {
                    routes.closing.remove(pane.as_str());
                }
            }
            TerminalControl::RequestOpen { .. }
            | TerminalControl::RequestHold { .. }
            | TerminalControl::RequestResolve { .. }
            | TerminalControl::RequestDiscard { .. } => {
                // The core opens creation requests for this machine's panes.
                self.local.control(control);
                return;
            }
            TerminalControl::AttachmentRefuse { intent }
            | TerminalControl::AttachmentDeliver { intent, .. } => {
                let intent = intent.clone();
                let routes = lock(&self.routes);
                let node = match routes.intents.get(&intent).cloned().flatten() {
                    None => Some(Arc::clone(&self.local)),
                    Some(device) => routes.devices.get(&device).cloned(),
                };
                drop(routes);
                if let Some(node) = node {
                    node.control(control);
                }
                return;
            }
            // Sent to one node by `Router::mirror`, never routed by pane.
            TerminalControl::Mirror { .. } => return,
            TerminalControl::AttachmentRelease { intent } => {
                let mut routes = lock(&self.routes);
                let device = routes.intents.remove(intent.as_str());
                let node = match device.flatten() {
                    None => Some(Arc::clone(&self.local)),
                    Some(device) => routes.devices.get(&device).cloned(),
                };
                drop(routes);
                if let Some(node) = node {
                    node.control(control);
                }
                return;
            }
            _ => {}
        }
        let Some(scoped) = control.pane_mut().map(|pane| pane.clone()) else {
            return;
        };
        if let TerminalControl::AttachmentHold { intent, .. } = &control {
            let mut routes = lock(&self.routes);
            let device = Self::device_of(&scoped).map(|(device, _)| device.to_owned());
            routes.intents.insert(intent.clone(), device);
        }
        match self.route(&scoped) {
            Route::Local => self.local.control(control),
            Route::Device { device, node, pane } => {
                match control {
                    TerminalControl::Attach { .. } => {
                        lock(&self.routes)
                            .attached
                            .entry(device)
                            .or_default()
                            .insert(scoped);
                    }
                    TerminalControl::Release { .. } | TerminalControl::Forget { .. } => {
                        if let Some(panes) = lock(&self.routes).attached.get_mut(&device) {
                            panes.remove(&scoped);
                        }
                        self.hub.forget(&scoped);
                    }
                    _ => {}
                }
                if let Some(target) = control.pane_mut() {
                    *target = pane;
                }
                node.control(control);
            }
            Route::Unrouted => match control {
                TerminalControl::Attach { .. } => self.unrouted_attach(scoped),
                TerminalControl::Release { .. } | TerminalControl::Forget { .. } => {
                    self.hub.forget(&scoped)
                }
                _ => {}
            },
        }
    }

    fn key(&self, target: KeyTarget, bytes: Vec<u8>, typed_at_unix_ms: u64) {
        let KeyTarget::Pane(scoped) = target else {
            self.local.key(target, bytes, typed_at_unix_ms);
            return;
        };
        let route = {
            let mut routes = lock(&self.routes);
            let now = Instant::now();
            match &mut routes.keyed {
                Some((keyed, at)) if *keyed == scoped => *at = now,
                keyed => *keyed = Some((scoped.clone(), now)),
            }
            let route = Self::route_in(&routes, &scoped);
            // A key for a device with no link is refused, never kept for a
            // later link (D-19, B17), and a flood of them is reported once
            // a window and counted after that, like a node's own refusals.
            let counted = matches!(route, Route::Unrouted)
                && routes
                    .refusals
                    .refuse(&scoped, "terminal.device_disconnected", now)
                    != Refusal::Report;
            let due = routes.due();
            drop(routes);
            log_due(due);
            if counted {
                return;
            }
            route
        };
        match route {
            Route::Local => self
                .local
                .key(KeyTarget::Pane(scoped), bytes, typed_at_unix_ms),
            Route::Device { node, pane, .. } => {
                node.key(KeyTarget::Pane(pane), bytes, typed_at_unix_ms)
            }
            Route::Unrouted => self.reports.report(TerminalReport::Error {
                pane: scoped,
                kind: "terminal.device_disconnected".to_owned(),
                message: "This device's terminals are not connected, so the key was not sent"
                    .to_owned(),
            }),
        }
    }

    fn view(&self, pane: &str, size: GridSize, new_view: bool) {
        match self.route(pane) {
            Route::Local => self.local.view(pane, size, new_view),
            Route::Device { node, pane, .. } => node.view(&pane, size, new_view),
            Route::Unrouted => {}
        }
    }

    fn redraw(&self, pane: &str) {
        match self.route(pane) {
            Route::Local => self.local.redraw(pane),
            Route::Device { node, pane, .. } => node.redraw(&pane),
            Route::Unrouted => {}
        }
    }
}

impl TerminalRoutes for Router {
    fn install_device(&self, device: &str, node: Arc<dyn TerminalNode>) {
        let mut routes = lock(&self.routes);
        routes.unstarted.remove(device);
        // Known first, so the shown panes are read as this device's by the
        // same rule its lines are.
        let replaced = routes.devices.insert(device.to_owned(), Arc::clone(&node));
        Self::shown_to(&routes, device, node.as_ref());
        Self::focus_to(&routes, device, node.as_ref());
        Self::flags_to(&routes, device, node.as_ref());
        if routes.mirror {
            node.control(TerminalControl::Mirror { on: true });
        }
        drop(routes);
        // The replaced link's writer ends once it is dropped, outside the
        // routing lock.
        drop(replaced);
        crate::diagnostic!(json!({
            "component": "terminal_router",
            "kind": "terminal.device_installed",
            "device": device,
        }));
    }

    fn remove_device(&self, device: &str) {
        let mut routes = lock(&self.routes);
        routes.unstarted.remove(device);
        let removed = routes.devices.remove(device);
        let panes = routes.attached.remove(device).unwrap_or_default();
        routes.rates.remove(device);
        routes
            .intents
            .retain(|_, holder| holder.as_deref() != Some(device));
        drop(routes);
        for pane in panes {
            self.hub.forget(&pane);
        }
        if removed.is_some() {
            crate::diagnostic!(json!({
                "component": "terminal_router",
                "kind": "terminal.device_removed",
                "device": device,
            }));
        }
        drop(removed);
    }

    fn terminals_unstarted(&self, device: &str, reason: &str) {
        // Whatever an earlier link carried ended with it.
        self.remove_device(device);
        lock(&self.routes)
            .unstarted
            .insert(device.to_owned(), reason.to_owned());
    }
}

impl DeviceSink for Router {
    fn output(&self, device: &str, pane: &str, bytes: &[u8], full: bool) {
        let scoped = device_pane_id(device, pane);
        {
            let mut routes = lock(&self.routes);
            let due = routes.due();
            let attached = Router::device_of(&scoped).is_some_and(|(owner, _)| owner == device)
                && routes.attached(device, &scoped);
            let logged = !attached && routes.refused_line(device, LINE_UNADMITTED);
            drop(routes);
            log_due(due);
            if logged {
                log_refused_line(device, LINE_UNADMITTED, "output");
            }
            if !attached {
                return;
            }
        }
        self.hub.output(&scoped, bytes, full);
        // A release that ran while the output was on its way to the hub has
        // forgotten the pane there already; what this output put back goes
        // the same way.
        if !lock(&self.routes).attached(device, &scoped) {
            self.hub.forget(&scoped);
        }
    }

    fn report(&self, device: &str, mut report: TerminalReport) {
        if let Some(pane) = report.pane_mut() {
            *pane = device_pane_id(device, pane);
        }
        let mut routes = lock(&self.routes);
        let now = Instant::now();
        let due = routes.due();
        let admitted = match &mut report {
            // The core opens creation requests for this machine's panes only.
            TerminalReport::RequestDiscarded { .. } => false,
            TerminalReport::AttachmentInput { intent, .. }
            | TerminalReport::AttachmentDelivered { intent, .. } => routes
                .intents
                .get(intent.as_str())
                .is_some_and(|holder| holder.as_deref() == Some(device)),
            TerminalReport::Input { pane, focus, .. } => {
                *focus &= routes.keyed_recently(pane, now);
                routes.speaks_for(device, pane)
            }
            other => other
                .pane_mut()
                .is_some_and(|pane| routes.speaks_for(device, pane)),
        };
        let refused = if !admitted {
            Some(LINE_UNADMITTED)
        } else if !bound_device_report(&mut report) {
            Some(REPORT_MALFORMED)
        } else if !routes.within_rate(device, now) {
            Some(REPORTS_OVER_RATE)
        } else {
            None
        };
        let logged = refused.filter(|&kind| routes.refused_line(device, kind));
        drop(routes);
        log_due(due);
        if let Some(kind) = logged {
            log_refused_line(device, kind, "report");
        }
        if refused.is_none() {
            self.reports.report(report);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Recorder {
        permits: Mutex<Vec<(String, usize)>>,
        controls: Mutex<Vec<TerminalControl>>,
        keys: Mutex<Vec<(KeyTarget, Vec<u8>)>>,
    }

    impl TerminalNode for Recorder {
        fn input_permit(
            &self,
            pane: &str,
            bytes: usize,
            reply: hide_node_link::terminal::InputPermitReply,
        ) {
            self.permits.lock().unwrap().push((pane.to_owned(), bytes));
            reply(Ok(()));
        }
        fn control(&self, control: TerminalControl) {
            self.controls.lock().unwrap().push(control);
        }
        fn key(&self, target: KeyTarget, bytes: Vec<u8>, _typed_at_unix_ms: u64) {
            self.keys.lock().unwrap().push((target, bytes));
        }
        fn view(&self, _pane: &str, _size: GridSize, _new_view: bool) {}
        fn redraw(&self, _pane: &str) {}
    }

    #[derive(Default)]
    struct Hub {
        output: Mutex<Vec<(String, Vec<u8>)>>,
        forgotten: Mutex<Vec<String>>,
    }

    impl OutputSink for Hub {
        fn output(&self, pane: &str, bytes: &[u8], _full: bool) {
            self.output
                .lock()
                .unwrap()
                .push((pane.to_owned(), bytes.to_vec()));
        }
        fn forget(&self, pane: &str) {
            self.forgotten.lock().unwrap().push(pane.to_owned());
        }
    }

    #[derive(Default)]
    struct Reports(Mutex<Vec<TerminalReport>>);

    impl ReportSink for Reports {
        fn report(&self, report: TerminalReport) {
            self.0.lock().unwrap().push(report);
        }
    }

    fn attach(pane: &str) -> TerminalControl {
        TerminalControl::Attach {
            pane: pane.into(),
            size: None,
            manual: false,
        }
    }

    fn router() -> (Router, Arc<Recorder>, Arc<Hub>, Arc<Reports>) {
        let local = Arc::new(Recorder::default());
        let hub = Arc::new(Hub::default());
        let reports = Arc::new(Reports::default());
        let router = Router::new(
            Arc::clone(&local) as Arc<dyn TerminalNode>,
            Arc::clone(&hub) as Arc<dyn OutputSink>,
            Arc::clone(&reports) as Arc<dyn ReportSink>,
        );
        (router, local, hub, reports)
    }

    #[test]
    fn a_device_pane_reaches_its_device_by_the_devices_own_id() {
        let (router, local, hub, reports) = router();
        let mini = Arc::new(Recorder::default());
        router.control(TerminalControl::Shown {
            panes: vec!["w1:p1".into(), "remote:mini:pane:w2:p3".into()],
        });
        router.install_device("mini", Arc::clone(&mini) as Arc<dyn TerminalNode>);
        assert_eq!(
            mini.controls.lock().unwrap().as_slice(),
            [
                TerminalControl::Shown {
                    panes: vec!["w2:p3".into()]
                },
                TerminalControl::Focus { pane: None }
            ]
        );
        router.key(
            KeyTarget::Pane("remote:mini:pane:w2:p3".into()),
            b"a".to_vec(),
            1,
        );
        router.key(KeyTarget::Pane("w1:p1".into()), b"b".to_vec(), 1);
        router.control(attach("remote:mini:pane:w2:p3"));
        assert_eq!(
            mini.keys.lock().unwrap().as_slice(),
            [(KeyTarget::Pane("w2:p3".into()), b"a".to_vec())]
        );
        assert_eq!(
            local.keys.lock().unwrap().as_slice(),
            [(KeyTarget::Pane("w1:p1".into()), b"b".to_vec())]
        );
        // Output and reports from the device come back under the core's ids.
        router.output("mini", "w2:p3", b"frame", true);
        assert_eq!(
            hub.output.lock().unwrap().as_slice(),
            [("remote:mini:pane:w2:p3".to_owned(), b"frame".to_vec())]
        );
        DeviceSink::report(
            &router,
            "mini",
            TerminalReport::FirstFrame {
                pane: "w2:p3".into(),
                generation: 2,
            },
        );
        assert_eq!(
            reports.0.lock().unwrap().as_slice(),
            [TerminalReport::FirstFrame {
                pane: "remote:mini:pane:w2:p3".into(),
                generation: 2
            }]
        );
    }

    #[test]
    fn input_permissions_route_local_and_device_ids_and_refuse_unrouted_devices() {
        use hide_node_link::terminal::InputPermitError;
        let (router, local, _, _) = router();
        let mini = Arc::new(Recorder::default());
        router.install_device("mini", Arc::clone(&mini) as Arc<dyn TerminalNode>);
        for (pane, expected) in [
            ("w1:p1", Ok(())),
            ("remote:mini:pane:w2:p3", Ok(())),
            (
                "remote:missing:pane:w2:p3",
                Err(InputPermitError::Unavailable),
            ),
        ] {
            let (reply, result) = std::sync::mpsc::channel();
            router.input_permit(
                pane,
                7,
                Box::new(move |outcome| {
                    reply.send(outcome).unwrap();
                }),
            );
            assert_eq!(result.try_recv().unwrap(), expected);
        }
        assert_eq!(*local.permits.lock().unwrap(), [("w1:p1".into(), 7)]);
        assert_eq!(*mini.permits.lock().unwrap(), [("w2:p3".into(), 7)]);
    }

    /// A device linked again starts with a fresh node, which is told which
    /// of its panes hold a sleeping agent or are closing, whether the core
    /// said so while the last link was up or while there was none.
    #[test]
    fn a_device_linked_again_is_told_its_sleeping_and_closing_panes() {
        let (router, _local, _hub, _reports) = router();
        let asleep = |pane: &str, asleep| TerminalControl::Asleep {
            pane: pane.into(),
            asleep,
        };
        let closing = |pane: &str| TerminalControl::Closing {
            pane: pane.into(),
            closing: true,
        };
        router.install_device(
            "mini",
            Arc::new(Recorder::default()) as Arc<dyn TerminalNode>,
        );
        router.control(asleep("remote:mini:pane:w2:p3", true));
        router.remove_device("mini");
        router.control(closing("remote:mini:pane:w2:p4"));
        router.control(asleep("remote:mini:pane:w2:p5", true));
        router.control(asleep("remote:mini:pane:w2:p5", false));
        router.control(asleep("remote:studio:pane:w1:p1", true));
        router.control(asleep("w1:p1", true));
        let mini = Arc::new(Recorder::default());
        router.install_device("mini", Arc::clone(&mini) as Arc<dyn TerminalNode>);
        assert_eq!(
            mini.controls.lock().unwrap().as_slice(),
            [
                TerminalControl::Shown { panes: Vec::new() },
                TerminalControl::Focus { pane: None },
                asleep("w2:p3", true),
                closing("w2:p4"),
            ]
        );
    }

    /// A device whose link is up without terminals (its helper speaks an
    /// older protocol) says so on its panes, not that its helper is not
    /// connected, until a link with terminals is installed.
    #[test]
    fn a_device_linked_without_terminals_says_why_its_panes_cannot_attach() {
        let (router, _local, _hub, reports) = router();
        router.terminals_unstarted(
            "mini",
            "the device's helper speaks protocol 24, which carries no terminals",
        );
        router.control(attach("remote:mini:pane:w2:p3"));
        let message = |reports: &Reports| match reports.0.lock().unwrap().last() {
            Some(TerminalReport::State { state, .. }) => state.message.clone().unwrap(),
            other => panic!("no state: {other:?}"),
        };
        let unstarted = message(&reports);
        assert!(unstarted.contains("protocol 24"), "{unstarted}");
        assert!(!unstarted.contains("not connected"), "{unstarted}");
        router.install_device(
            "mini",
            Arc::new(Recorder::default()) as Arc<dyn TerminalNode>,
        );
        router.remove_device("mini");
        router.control(attach("remote:mini:pane:w2:p3"));
        assert!(message(&reports).contains("helper is not connected"));
    }

    #[test]
    fn a_device_without_a_link_refuses_keys_and_reads_unavailable() {
        let (router, _local, hub, reports) = router();
        let mini = Arc::new(Recorder::default());
        router.install_device("mini", Arc::clone(&mini) as Arc<dyn TerminalNode>);
        router.control(attach("remote:mini:pane:w2:p3"));
        router.output("mini", "w2:p3", b"frame", true);
        router.remove_device("mini");
        assert_eq!(
            hub.forgotten.lock().unwrap().as_slice(),
            ["remote:mini:pane:w2:p3"]
        );
        router.key(
            KeyTarget::Pane("remote:mini:pane:w2:p3".into()),
            b"a".to_vec(),
            1,
        );
        router.control(TerminalControl::Attach {
            pane: "remote:mini:pane:w2:p3".into(),
            size: None,
            manual: false,
        });
        let reports = reports.0.lock().unwrap();
        assert!(
            matches!(&reports[0], TerminalReport::Error { kind, .. } if kind == "terminal.device_disconnected")
        );
        assert!(
            matches!(&reports[1], TerminalReport::State { state, .. } if state.state == "unavailable")
        );
        // Output a removed device's late reader sends reaches no screen.
        drop(reports);
        router.output("mini", "w2:p3", b"late", false);
        assert_eq!(hub.output.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_pastes_later_controls_follow_its_pane_to_its_device() {
        let (router, local, _hub, _reports) = router();
        let mini = Arc::new(Recorder::default());
        router.install_device("mini", Arc::clone(&mini) as Arc<dyn TerminalNode>);
        router.control(TerminalControl::AttachmentHold {
            pane: "remote:mini:pane:w2:p3".into(),
            intent: "paste-1".into(),
        });
        router.control(TerminalControl::AttachmentDeliver {
            intent: "paste-1".into(),
            generation: 3,
            paste: "'/tmp/a.png' ".into(),
        });
        router.control(TerminalControl::AttachmentRelease {
            intent: "paste-1".into(),
        });
        // What it was told on connecting comes first.
        let controls = mini.controls.lock().unwrap();
        assert!(
            matches!(&controls[2], TerminalControl::AttachmentHold { pane, .. } if pane == "w2:p3")
        );
        assert!(matches!(
            &controls[3],
            TerminalControl::AttachmentDeliver { .. }
        ));
        assert!(matches!(
            &controls[4],
            TerminalControl::AttachmentRelease { .. }
        ));
        assert!(local.controls.lock().unwrap().is_empty());
    }

    /// The keyboard's pane reaches the node that holds it by that node's own
    /// id, every other node hears none, and a device that connects later
    /// hears it too.
    #[test]
    fn the_keyboard_pane_reaches_only_the_node_that_holds_it() {
        let (router, local, _hub, _reports) = router();
        let mini = Arc::new(Recorder::default());
        router.install_device("mini", Arc::clone(&mini) as Arc<dyn TerminalNode>);
        router.control(TerminalControl::Focus {
            pane: Some("remote:mini:pane:w2:p3".into()),
        });
        assert_eq!(
            local.controls.lock().unwrap().last(),
            Some(&TerminalControl::Focus { pane: None })
        );
        assert_eq!(
            mini.controls.lock().unwrap().last(),
            Some(&TerminalControl::Focus {
                pane: Some("w2:p3".into())
            })
        );
        let studio = Arc::new(Recorder::default());
        router.install_device("studio", Arc::clone(&studio) as Arc<dyn TerminalNode>);
        assert!(
            studio
                .controls
                .lock()
                .unwrap()
                .contains(&TerminalControl::Focus { pane: None })
        );
        router.control(TerminalControl::Focus {
            pane: Some("w1:p1".into()),
        });
        assert_eq!(
            local.controls.lock().unwrap().last(),
            Some(&TerminalControl::Focus {
                pane: Some("w1:p1".into())
            })
        );
    }

    /// Every paste the core holds is released when it ends, written or
    /// not, so a thousand pastes leave no route behind.
    #[test]
    fn released_pastes_leave_no_route_behind() {
        let (router, _local, _hub, _reports) = router();
        let mini = Arc::new(Recorder::default());
        router.install_device("mini", Arc::clone(&mini) as Arc<dyn TerminalNode>);
        for index in 0..1_000 {
            let intent = format!("paste-{index}");
            let pane = if index % 2 == 0 {
                "w1:p1"
            } else {
                "remote:mini:pane:w2:p3"
            };
            router.control(TerminalControl::AttachmentHold {
                pane: pane.into(),
                intent: intent.clone(),
            });
            router.control(TerminalControl::AttachmentRelease { intent });
        }
        assert!(lock(&router.routes).intents.is_empty());
    }

    /// A device's node is another machine's program: output for a pane the
    /// core never attached there, a report about a pane the core neither
    /// attached nor shows, a creation's discarded keys and a paste's outcome
    /// for a paste held elsewhere are not taken.
    #[test]
    fn a_devices_lines_the_core_never_asked_for_are_not_taken() {
        let (router, _local, hub, reports) = router();
        let mini = Arc::new(Recorder::default());
        let studio = Arc::new(Recorder::default());
        router.install_device("mini", Arc::clone(&mini) as Arc<dyn TerminalNode>);
        router.install_device("studio", Arc::clone(&studio) as Arc<dyn TerminalNode>);
        router.control(TerminalControl::Shown {
            panes: vec!["remote:mini:pane:w2:p4".into()],
        });
        router.control(attach("remote:mini:pane:w2:p3"));
        router.control(TerminalControl::AttachmentHold {
            pane: "remote:studio:pane:w1:p1".into(),
            intent: "paste-studio".into(),
        });
        router.control(TerminalControl::AttachmentHold {
            pane: "remote:mini:pane:w2:p3".into(),
            intent: "paste-mini".into(),
        });
        for pane in ["w2:p3", "w9:p9", "w2:p4"] {
            router.output("mini", pane, b"frame", true);
        }
        let error = |pane: &str| TerminalReport::Error {
            pane: pane.into(),
            kind: "terminal.write_failed".into(),
            message: "m".into(),
        };
        for report in [
            error("w2:p3"),
            error("w9:p9"),
            error("w2:p4"),
            TerminalReport::RequestDiscarded {
                request: "r1".into(),
                reason: "limit".into(),
            },
            TerminalReport::AttachmentDelivered {
                intent: "paste-studio".into(),
                written: true,
            },
            TerminalReport::AttachmentDelivered {
                intent: "paste-mini".into(),
                written: true,
            },
        ] {
            DeviceSink::report(&router, "mini", report);
        }
        assert_eq!(
            hub.output.lock().unwrap().as_slice(),
            [("remote:mini:pane:w2:p3".to_owned(), b"frame".to_vec())]
        );
        assert_eq!(
            reports.0.lock().unwrap().as_slice(),
            [
                error("remote:mini:pane:w2:p3"),
                error("remote:mini:pane:w2:p4"),
                TerminalReport::AttachmentDelivered {
                    intent: "paste-mini".into(),
                    written: true,
                },
            ]
        );
        // A forgotten pane's late output reaches no screen either.
        router.control(TerminalControl::Forget {
            pane: "remote:mini:pane:w2:p3".into(),
        });
        router.output("mini", "w2:p3", b"late", false);
        assert_eq!(hub.output.lock().unwrap().len(), 1);
        assert_eq!(
            hub.forgotten.lock().unwrap().as_slice(),
            ["remote:mini:pane:w2:p3"]
        );
    }

    /// A device's word that a key moved the keyboard counts only for the
    /// pane the last screen key went to; its input fact still counts.
    #[test]
    fn a_devices_focus_counts_only_for_the_pane_the_last_key_went_to() {
        let (router, _local, _hub, reports) = router();
        let mini = Arc::new(Recorder::default());
        router.install_device("mini", Arc::clone(&mini) as Arc<dyn TerminalNode>);
        router.control(attach("remote:mini:pane:w2:p3"));
        router.control(attach("remote:mini:pane:w2:p4"));
        let input = |pane: &str| TerminalReport::Input {
            pane: pane.into(),
            at_unix_ms: 7,
            submitted: false,
            focus: true,
        };
        router.key(
            KeyTarget::Pane("remote:mini:pane:w2:p3".into()),
            b"a".to_vec(),
            1,
        );
        DeviceSink::report(&router, "mini", input("w2:p3"));
        DeviceSink::report(&router, "mini", input("w2:p4"));
        // Long after the key, the device's word no longer moves the keyboard.
        lock(&router.routes).keyed.as_mut().expect("keyed").1 = Instant::now()
            .checked_sub(KEY_FOCUS_WINDOW)
            .expect("an instant that long ago");
        DeviceSink::report(&router, "mini", input("w2:p3"));
        router.key(
            KeyTarget::Pane("remote:mini:pane:w2:p3".into()),
            b"a".to_vec(),
            1,
        );
        router.key(KeyTarget::Pane("w1:p1".into()), b"b".to_vec(), 2);
        DeviceSink::report(&router, "mini", input("w2:p3"));
        let focus = reports
            .0
            .lock()
            .unwrap()
            .iter()
            .map(|report| match report {
                TerminalReport::Input { pane, focus, .. } => (pane.clone(), *focus),
                other => panic!("unexpected {other:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            focus,
            [
                ("remote:mini:pane:w2:p3".to_owned(), true),
                ("remote:mini:pane:w2:p4".to_owned(), false),
                ("remote:mini:pane:w2:p3".to_owned(), false),
                ("remote:mini:pane:w2:p3".to_owned(), false),
            ]
        );
    }

    /// What a device says is bounded before the core keeps it: a message
    /// is cut, a report naming something no node names is refused, and a
    /// flood of reports past the rate is refused.
    #[test]
    fn a_devices_reports_are_bounded_before_the_core_reads_them() {
        let (router, _local, _hub, reports) = router();
        let mini = Arc::new(Recorder::default());
        router.install_device("mini", Arc::clone(&mini) as Arc<dyn TerminalNode>);
        router.control(attach("remote:mini:pane:w2:p3"));
        let error = |kind: &str, message: String| TerminalReport::Error {
            pane: "w2:p3".into(),
            kind: kind.into(),
            message,
        };
        DeviceSink::report(&router, "mini", error("terminal.x", "é".repeat(8 << 20)));
        DeviceSink::report(&router, "mini", error("Not a name", "m".into()));
        let mut state = PaneTerminalState {
            state: "controlling".into(),
            mode: Some("control".into()),
            generation: 1,
            attempt: 1,
            message: Some("m".repeat(4_000)),
            exit_category: None,
            retry_decision: "none".into(),
            last_attempt_at_unix_ms: None,
        };
        let report_state = |state: &PaneTerminalState| TerminalReport::State {
            pane: "w2:p3".into(),
            state: state.clone(),
        };
        DeviceSink::report(&router, "mini", report_state(&state));
        state.state = "x".repeat(DEVICE_NAME_BYTES + 1);
        DeviceSink::report(&router, "mini", report_state(&state));
        {
            let reports = reports.0.lock().unwrap();
            let [
                TerminalReport::Error { message, .. },
                TerminalReport::State { state, .. },
            ] = reports.as_slice()
            else {
                panic!("one error and one state are taken: {reports:?}");
            };
            assert_eq!(message.chars().count(), DEVICE_MESSAGE_CHARS);
            assert_eq!(state.message.as_ref().unwrap().len(), DEVICE_MESSAGE_CHARS);
        }
        for _ in 0..DEVICE_REPORTS_PER_WINDOW * 2 {
            DeviceSink::report(&router, "mini", error("terminal.x", "m".into()));
        }
        let taken = reports.0.lock().unwrap().len() as u32;
        assert!(
            (DEVICE_REPORTS_PER_WINDOW..=DEVICE_REPORTS_PER_WINDOW * 2).contains(&taken),
            "{taken} reports taken"
        );
        assert!(
            taken < DEVICE_REPORTS_PER_WINDOW * 2 + 2,
            "the rate refused some"
        );
    }

    /// A release that runs while a device's output is on its way to the
    /// hub leaves no ring there for the released pane.
    #[test]
    fn a_release_racing_a_devices_output_leaves_nothing_in_the_hub() {
        #[derive(Default)]
        struct RacingHub {
            router: std::sync::OnceLock<std::sync::Weak<Router>>,
            events: Mutex<Vec<String>>,
        }
        impl OutputSink for RacingHub {
            fn output(&self, pane: &str, _bytes: &[u8], _full: bool) {
                // The release lands between the router's check and here.
                if let Some(router) = self.router.get().and_then(std::sync::Weak::upgrade) {
                    router.control(TerminalControl::Forget { pane: pane.into() });
                }
                self.events.lock().unwrap().push(format!("output {pane}"));
            }
            fn forget(&self, pane: &str) {
                self.events.lock().unwrap().push(format!("forget {pane}"));
            }
        }
        let hub = Arc::new(RacingHub::default());
        let router = Arc::new(Router::new(
            Arc::new(Recorder::default()) as Arc<dyn TerminalNode>,
            Arc::clone(&hub) as Arc<dyn OutputSink>,
            Arc::new(Reports::default()) as Arc<dyn ReportSink>,
        ));
        hub.router.set(Arc::downgrade(&router)).ok();
        router.install_device(
            "mini",
            Arc::new(Recorder::default()) as Arc<dyn TerminalNode>,
        );
        router.control(attach("remote:mini:pane:w2:p3"));
        router.output("mini", "w2:p3", b"frame", true);
        assert_eq!(
            hub.events.lock().unwrap().last().map(String::as_str),
            Some("forget remote:mini:pane:w2:p3")
        );
    }

    /// B5 on the failure path: a thousand keys into a device with no link
    /// are each refused, and the core hears it once a window.
    #[test]
    fn a_key_flood_into_a_device_without_a_link_is_reported_once_a_window() {
        let (router, _local, _hub, reports) = router();
        let started = Instant::now();
        for index in 0..1_000 {
            router.key(
                KeyTarget::Pane("remote:mini:pane:w2:p3".into()),
                b"k".to_vec(),
                index,
            );
        }
        let refused = reports.0.lock().unwrap().len() as u32;
        let windows = started.elapsed().as_secs() as u32 + 1;
        assert!(
            (1..=windows).contains(&refused),
            "{refused} reports in {windows} windows"
        );
    }
}
