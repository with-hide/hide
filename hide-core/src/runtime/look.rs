//! What the operator looks at: the inputs of the read axis's look, owned
//! here so nothing else writes them, and the one boundary a selection
//! completes at (`Runtime::change_look`).
//!
//! A pane is read when it had the operator's look after its last state
//! change (`docs/status-model.md`, Hide owns the read axis). The look is made
//! of this machine's focus pane as the operator chose it, the device in front
//! (whose own focus counts while its window shows that pane's tab), the views
//! other machines show of this one, and the agents phones have open. The focus
//! and the device in front count only while the screen they were chosen on
//! shows: this machine's own windows always, another machine's view only
//! while that machine shows it. One selection can move several of them, such as
//! a device brought forward and then its tab chosen; it is read once, as it
//! completes, and never at a value it passed through. Two selections are two
//! reads, whatever runs between them and whether or not a window reads the
//! snapshot (with-hide/hide-private#685).

use super::*;

/// The paired machines a view can come from at once: a host grants 32
/// machines at most (`hide_peer::store`), and each holds one view.
const MAX_VIEWING_MACHINES: usize = 32;

/// The screen a window's or a view's event was sent from, as the operator's
/// look reads it (`Core::dispatch_bytes_from`); a tool's event names none.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum LookScreen {
    /// This machine's own windows and tools: a look chosen there counts as
    /// long as it stands.
    #[default]
    Own,
    /// Another machine's view of this one (`hided/src/remote_screen.rs`), by
    /// that machine's key: a look chosen there counts only while that
    /// machine shows the view, so leaving its tile, its socket closing or its
    /// window going ends it, and showing the tile again starts it again.
    View(String),
}

/// The inputs of the operator's look, and how many selections that move
/// them are under way. The fields are private to this module, so every write
/// is one of the setters below and a new path that moves the look goes
/// through the boundary that reads it.
pub(super) struct LookInputs {
    /// The pane the operator chose on this machine. The engine's own focus
    /// is not the operator's look: the engine keeps a focus for every tab,
    /// bringing a tab forward makes it report a focus nobody asked for, a
    /// spawned pane takes focus on its own, and a relaunch inherits whatever
    /// focus the arriving session names. Counting those cleared rows the
    /// operator never saw (one click on a Done row cleared two rows, a restart
    /// cleared the last unread one). Only a focus Hide dispatched on the
    /// operator's behalf sets this, and it is memory only: a launch starts
    /// with the operator having looked at nothing.
    focus: Option<String>,
    /// The device in front. `navigator.focused_device_id` is its projection
    /// on the wire, written with it and never read for a decision.
    front: Option<String>,
    /// The screen `focus` and `front` were last chosen on. A window's or a
    /// view's event that writes either sets it, to the same value too, since
    /// choosing the pane again on another screen moves where it is looked at;
    /// a tool's command (an agent's `hide` call), a daemon's event and a move
    /// Hide makes on its own (a start it follows, a pane that closed) keep
    /// the screen the look was chosen on, since none is the operator choosing
    /// somewhere: an agent's command after the operator left a view never
    /// makes the view's choice count again. Memory only.
    chosen: LookScreen,
    /// The machines whose view of this one shows now, by key: hided names
    /// every one whenever a view is seated or ends (`remote_views`). Memory
    /// only; a launch starts with none.
    views: BTreeSet<String>,
    /// The agents a phone has open now, by the core's pane id (a device's
    /// pane carries its device): hided names every one whenever a phone opens
    /// or closes an agent's detail or disconnects (`phone_viewing`). Memory
    /// only; a launch starts with no phone looking.
    phones: BTreeSet<String>,
    /// The screen of the client event being dispatched now, none outside
    /// one (`Runtime::dispatch_json_from`).
    sent_from: Option<LookScreen>,
    /// Selections under way (`Runtime::change_look`), nested.
    open: u32,
}

impl LookInputs {
    /// What a launch starts with: the device in front as the last session
    /// left it, and no look of the operator's or a phone's. Restoring is not
    /// looking, so nothing is read for it.
    pub(super) fn restored(front: Option<String>) -> Self {
        Self {
            focus: None,
            front,
            chosen: LookScreen::Own,
            views: BTreeSet::new(),
            phones: BTreeSet::new(),
            sent_from: None,
            open: 0,
        }
    }

    pub(super) fn focus(&self) -> Option<&str> {
        self.focus.as_deref()
    }

    /// Whether the screen the focus and the front were chosen on shows now:
    /// this machine's own always, another machine's view while it is seated.
    pub(super) fn chosen_shows(&self) -> bool {
        match &self.chosen {
            LookScreen::Own => true,
            LookScreen::View(machine) => self.views.contains(machine),
        }
    }

    /// The focus pane while it counts as the operator's look: while the
    /// screen it was chosen on shows.
    pub(super) fn looking_focus(&self) -> Option<&str> {
        self.focus().filter(|_| self.chosen_shows())
    }

    /// The screen a write of the focus or the front made now is chosen on:
    /// the client event's being dispatched, else the one the look was last
    /// chosen on.
    fn choosing(&self) -> LookScreen {
        self.sent_from
            .clone()
            .unwrap_or_else(|| self.chosen.clone())
    }

    pub(super) fn front(&self) -> Option<&str> {
        self.front.as_deref()
    }

    pub(super) fn phones(&self) -> &BTreeSet<String> {
        &self.phones
    }

    /// Whether a selection is under way: its values are not the look yet.
    pub(super) fn selecting(&self) -> bool {
        self.open > 0
    }
}

impl Runtime {
    /// Dispatches one client event sent from `screen`: whatever it chooses
    /// of the focus and the front is chosen on that screen (`LookInputs`).
    /// Otherwise as `dispatch_json`.
    pub fn dispatch_json_from(&mut self, bytes: &[u8], screen: LookScreen) -> bool {
        let before = self.look.sent_from.replace(screen);
        let answer =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.dispatch_json(bytes)));
        self.look.sent_from = before;
        match answer {
            Ok(changed) => changed,
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }

    /// Runs one selection. `change` may move any input of the look, and the
    /// rows are read once, as of the look it leaves, when the outermost
    /// selection completes (`follow_operator_look`); a selection inside
    /// another is part of it. Answers what `change` answered and whether that
    /// read moved a row, which the caller announces with its own change. A
    /// panic ends the selection unread and goes on unwinding.
    pub(super) fn change_look<R>(&mut self, change: impl FnOnce(&mut Self) -> R) -> (R, bool) {
        self.look.open += 1;
        let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| change(self)));
        self.look.open -= 1;
        match answer {
            Ok(answer) => {
                // Pair view goes where the look went (D-45), before the rows
                // are read.
                if !self.look.selecting() {
                    self.follow_pair_look();
                }
                (answer, self.follow_operator_look())
            }
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }

    /// Completes whatever moved the look outside a selection, such as a
    /// session that arrived: the rows are read if the look moved since they
    /// last were. Inside a selection it waits for that selection's end.
    /// Answers whether the read moved a row.
    pub(super) fn complete_look(&mut self) -> bool {
        self.change_look(|_| ()).1
    }

    /// The operator chose `pane` on this machine, or left none. Outside a
    /// selection this is one by itself and reads at once; answers whether
    /// that read moved a row.
    pub(super) fn set_operator_focus(&mut self, pane: Option<String>) -> bool {
        let chosen = self.look.choosing();
        if self.look.focus == pane && self.look.chosen == chosen {
            return false;
        }
        self.change_look(|runtime| {
            runtime.look.focus = pane;
            runtime.look.chosen = chosen;
        })
        .1
    }

    /// `device` is in front now. Outside a selection this is one by itself
    /// and reads at once; answers whether that read moved a row.
    pub(super) fn set_front_device(&mut self, device: Option<String>) -> bool {
        let chosen = self.look.choosing();
        if self.look.front == device && self.look.chosen == chosen {
            return false;
        }
        self.change_look(|runtime| {
            runtime.snapshot.navigator.focused_device_id = device.clone();
            runtime.look.front = device;
            runtime.look.chosen = chosen;
        })
        .1
    }

    /// Exactly `machines` show their view of this one now (`remote_views`,
    /// each by its key). Outside a selection this is one by itself and reads
    /// at once; answers whether that read moved a row.
    pub(super) fn set_remote_views(&mut self, machines: Vec<String>) -> bool {
        if machines.len() > MAX_VIEWING_MACHINES {
            self.push_diagnostic(
                "look.remote_views.refused",
                format!(
                    "A view list named {} machines; at most {MAX_VIEWING_MACHINES} view this one",
                    machines.len()
                ),
            );
            return false;
        }
        let views = machines.into_iter().collect::<BTreeSet<_>>();
        if views == self.look.views {
            return false;
        }
        crate::diagnostic!(serde_json::json!({
            "component": "runtime", "kind": "look.remote_views",
            "machines": views.iter().map(|key| key.get(..16).unwrap_or(key)).collect::<Vec<_>>(),
        }));
        self.change_look(|runtime| runtime.look.views = views).1
    }

    /// The phones have exactly `panes` open now. Outside a selection this is
    /// one by itself and reads at once; answers whether that read moved a
    /// row.
    pub(super) fn set_phone_look(&mut self, panes: BTreeSet<String>) -> bool {
        if self.look.phones == panes {
            return false;
        }
        self.change_look(|runtime| runtime.look.phones = panes).1
    }

    /// Reads every row again when the operator's look has moved from the
    /// look the rows were last read with, and does nothing otherwise or
    /// while a selection is under way; true when that changed a row.
    ///
    /// This is the one place a moved look is read, whatever moved it: a pane
    /// or tab chosen on this machine, a device brought forward or left, the
    /// tab a device's window shows, a device's own keyboard, a phone. A
    /// device's own keyboard that already rests on a pane sends nothing new,
    /// so the read cannot wait for the device's next session.
    fn follow_operator_look(&mut self) -> bool {
        if self.look.selecting() || self.operator_look() == self.read_look {
            return false;
        }
        self.refresh_read_state_everywhere()
    }
}
