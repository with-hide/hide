//! Which screen a pane's terminal size follows (PRD core-host-node-remote-core
//! B12, D-11). Every screen of the core, the core machine's own windows and
//! the screen machine's through their relays, says the grid it draws each
//! pane at; the pane runs at the grid of the screen that last sent it input,
//! and at the grid a screen last said while none has. One screen sizes its
//! panes exactly as before: each of its grids goes on at once.
//!
//! Input is bytes a screen routed to the pane: keys, a mouse report, an
//! attachment's paste; a view, a scroll or a focus is not. Only the core's
//! `terminal_resize` sizes a pane, so this sits where screens' events reach
//! the core and decides which of them go on. The pane's node expects frames
//! at the grid its views name, so a screen that does not size the pane
//! views it at the grid it runs at, and a pane that changes screens is
//! viewed and sized at the new grid together.
//!
//! A grid comes with what the screen measured to fit it (`floor`, which the
//! core's pane floor judges by and this table does not read), and the
//! measure goes on with the grid it came with: the core only ever holds the
//! measure of the screen whose grid the pane runs at.

use std::collections::HashMap;
use std::sync::Mutex;

use hide_node_link::terminal::GridSize;
use serde_json::{Value, json};

/// The panes whose screens' grids are kept. A pane leaves when the core
/// forgets it; past the cap a screen's grid goes on at once, as with one
/// screen, and the crossing is logged once.
const MAX_PANES: usize = 1024;

#[derive(Default)]
struct Pane {
    /// Each screen's grid for the pane, the latest said last.
    grids: Vec<(u64, GridSize)>,
    /// What each screen measured with its grid, when it did.
    floors: Vec<(u64, Value)>,
    /// The screen that last sent the pane input, while it is connected.
    sizing: Option<u64>,
    /// The grid last sent on to the core.
    applied: Option<GridSize>,
    /// The measure sent on with it.
    applied_floor: Option<Value>,
}

impl Pane {
    fn floor_of(&self, screen: u64) -> Option<Value> {
        self.floors
            .iter()
            .find(|(held, _)| *held == screen)
            .map(|(_, floor)| floor.clone())
    }
}

#[derive(Default)]
pub struct PaneSizes {
    panes: Mutex<HashMap<String, Pane>>,
    /// Set while the table is full and the crossing has been logged.
    full: std::sync::atomic::AtomicBool,
}

impl PaneSizes {
    /// `screen` draws `pane` at `size`: the grid to send on now, or `None`
    /// while another screen sizes the pane.
    pub fn resized(&self, screen: u64, pane: &str, size: GridSize) -> Option<GridSize> {
        self.resized_measured(screen, pane, size, None)
    }

    /// [`Self::resized`], with what the screen measured to fit `size`.
    pub fn resized_measured(
        &self,
        screen: u64,
        pane: &str,
        size: GridSize,
        floor: Option<Value>,
    ) -> Option<GridSize> {
        let mut panes = self.lock();
        if !panes.contains_key(pane) && panes.len() >= MAX_PANES {
            if !self.full.swap(true, std::sync::atomic::Ordering::Relaxed) {
                hide_core::diagnostic!(json!({
                    "component": "pane_sizes",
                    "kind": "pane_sizes.full",
                    "pane_id": pane,
                    "cap": MAX_PANES,
                }));
            }
            return Some(size);
        }
        let entry = panes.entry(pane.to_owned()).or_default();
        entry.grids.retain(|(held, _)| *held != screen);
        entry.grids.push((screen, size));
        entry.floors.retain(|(held, _)| *held != screen);
        if let Some(floor) = &floor {
            entry.floors.push((screen, floor.clone()));
        }
        if entry.sizing.is_some_and(|sizing| sizing != screen) {
            return None;
        }
        entry.applied = Some(size);
        entry.applied_floor = floor;
        Some(size)
    }

    /// The measure the pane's grid went on with, for a grid this table
    /// sends on itself (`input`, `left`).
    pub fn applied_floor(&self, pane: &str) -> Option<Value> {
        self.lock()
            .get(pane)
            .and_then(|entry| entry.applied_floor.clone())
    }

    /// The grid `screen`'s view of `pane` at `size` is drawn at: its own,
    /// unless another screen sizes the pane.
    pub fn view(&self, screen: u64, pane: &str, size: GridSize) -> GridSize {
        let panes = self.lock();
        match panes.get(pane) {
            Some(entry) if entry.sizing.is_some_and(|sizing| sizing != screen) => {
                entry.applied.unwrap_or(size)
            }
            _ => size,
        }
    }

    /// `screen` sent `pane` input: the screen's grid to send on now when
    /// the pane now follows it at another grid than it runs at. A screen
    /// that said no grid for the pane does not size it.
    pub fn input(&self, screen: u64, pane: &str) -> Option<GridSize> {
        let mut panes = self.lock();
        let entry = panes.get_mut(pane)?;
        if entry.sizing == Some(screen) {
            return None;
        }
        let (_, size) = *entry.grids.iter().find(|(held, _)| *held == screen)?;
        entry.sizing = Some(screen);
        let floor = entry.floor_of(screen);
        if entry.applied == Some(size) && entry.applied_floor == floor {
            return None;
        }
        entry.applied = Some(size);
        entry.applied_floor = floor;
        Some(size)
    }

    /// `screen` closed: for each pane it sized, the grid the next screen
    /// said last, when it differs from the one the pane runs at.
    pub fn left(&self, screen: u64) -> Vec<(String, GridSize)> {
        let mut panes = self.lock();
        let mut resized = Vec::new();
        panes.retain(|pane, entry| {
            entry.grids.retain(|(held, _)| *held != screen);
            entry.floors.retain(|(held, _)| *held != screen);
            if entry.sizing == Some(screen) {
                entry.sizing = None;
                if let Some(&(next, size)) = entry.grids.last() {
                    let floor = entry.floor_of(next);
                    if entry.applied != Some(size) || entry.applied_floor != floor {
                        entry.applied = Some(size);
                        entry.applied_floor = floor;
                        resized.push((pane.clone(), size));
                    }
                }
            }
            !entry.grids.is_empty()
        });
        resized
    }

    /// The pane is gone: no screen sizes it any more.
    pub fn forget(&self, pane: &str) {
        let mut panes = self.lock();
        if panes.remove(pane).is_some() && panes.len() < MAX_PANES {
            self.full.store(false, std::sync::atomic::Ordering::Relaxed);
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Pane>> {
        self.panes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDE: GridSize = GridSize {
        rows: 40,
        cols: 160,
    };
    const NARROW: GridSize = GridSize { rows: 30, cols: 80 };
    const SMALL: GridSize = GridSize { rows: 20, cols: 60 };

    #[test]
    fn one_screen_sizes_its_panes_as_it_always_did() {
        let sizes = PaneSizes::default();
        assert_eq!(sizes.resized(1, "p", WIDE), Some(WIDE));
        assert_eq!(sizes.input(1, "p"), None);
        assert_eq!(sizes.resized(1, "p", NARROW), Some(NARROW));
        assert_eq!(sizes.resized(1, "p", NARROW), Some(NARROW));
    }

    #[test]
    fn the_pane_follows_the_screen_that_last_sent_it_input() {
        let sizes = PaneSizes::default();
        assert_eq!(sizes.resized(1, "p", WIDE), Some(WIDE));
        // With no input yet, the last grid said goes on.
        assert_eq!(sizes.resized(2, "p", NARROW), Some(NARROW));
        // The wide screen types: the pane takes its grid again.
        assert_eq!(sizes.input(1, "p"), Some(WIDE));
        // The narrow screen's grid waits while the wide one sizes the pane.
        assert_eq!(sizes.resized(2, "p", SMALL), None);
        assert_eq!(sizes.input(1, "p"), None, "already following");
        assert_eq!(sizes.input(2, "p"), Some(SMALL));
        assert_eq!(sizes.resized(1, "p", NARROW), None);
        assert_eq!(sizes.resized(2, "p", WIDE), Some(WIDE));
    }

    #[test]
    fn a_screen_that_does_not_size_the_pane_views_it_at_the_grid_it_runs_at() {
        let sizes = PaneSizes::default();
        assert_eq!(sizes.view(1, "p", WIDE), WIDE, "a pane no screen sized");
        sizes.resized(1, "p", WIDE);
        sizes.input(1, "p");
        assert_eq!(sizes.view(2, "p", NARROW), WIDE);
        assert_eq!(sizes.view(1, "p", SMALL), SMALL, "the sizing screen's own");
    }

    #[test]
    fn a_closed_screen_hands_the_pane_to_the_grid_said_last() {
        let sizes = PaneSizes::default();
        sizes.resized(1, "p", WIDE);
        sizes.resized(2, "p", NARROW);
        sizes.resized(3, "p", SMALL);
        assert_eq!(sizes.input(1, "p"), Some(WIDE));
        assert_eq!(sizes.left(2), Vec::new(), "it sized nothing");
        assert_eq!(sizes.left(1), vec![("p".to_owned(), SMALL)]);
        assert_eq!(sizes.left(3), Vec::new());
        // Nothing is kept for a pane no screen draws.
        assert_eq!(sizes.input(3, "p"), None);
        assert!(sizes.lock().is_empty());
    }

    #[test]
    fn input_to_a_pane_no_screen_drew_changes_nothing() {
        let sizes = PaneSizes::default();
        assert_eq!(sizes.input(1, "p"), None);
        sizes.resized(2, "p", WIDE);
        assert_eq!(sizes.input(1, "p"), None, "the typing screen said no grid");
        assert_eq!(sizes.resized(2, "p", NARROW), Some(NARROW));
    }

    /// The core judges the pane floor by the measure of the screen whose
    /// grid the pane runs at, so a switch of screens sends that screen's
    /// measure even when the two grids match.
    #[test]
    fn a_grid_goes_on_with_the_measure_its_screen_took() {
        let sizes = PaneSizes::default();
        let wide = Some(json!({"canvas_px": 1200.0}));
        let narrow = Some(json!({"canvas_px": 600.0}));
        assert_eq!(
            sizes.resized_measured(1, "p", WIDE, wide.clone()),
            Some(WIDE)
        );
        assert_eq!(sizes.applied_floor("p"), wide);
        assert_eq!(sizes.input(1, "p"), None);
        // Another screen's measure waits with its grid.
        assert_eq!(sizes.resized_measured(2, "p", WIDE, narrow.clone()), None);
        assert_eq!(sizes.applied_floor("p"), wide);
        // That screen types: the grid is the same, its measure is not.
        assert_eq!(sizes.input(2, "p"), Some(WIDE));
        assert_eq!(sizes.applied_floor("p"), narrow);
        // It closes: the first screen's grid and measure come back.
        assert_eq!(sizes.left(2), vec![("p".to_owned(), WIDE)]);
        assert_eq!(sizes.applied_floor("p"), wide);
    }

    #[test]
    fn past_the_pane_cap_a_grid_goes_on_at_once() {
        let sizes = PaneSizes::default();
        for pane in 0..MAX_PANES {
            sizes.resized(1, &pane.to_string(), WIDE);
        }
        assert_eq!(sizes.resized(2, "over", NARROW), Some(NARROW));
        assert_eq!(sizes.lock().len(), MAX_PANES);
    }

    /// A pane the core forgets leaves the table, so a screen open for weeks
    /// keeps only live panes and the size rule stays on for new ones (R7).
    #[test]
    fn a_forgotten_pane_leaves_room_for_the_next() {
        let sizes = PaneSizes::default();
        for pane in 0..MAX_PANES {
            sizes.resized(1, &pane.to_string(), WIDE);
        }
        sizes.forget("0");
        assert_eq!(sizes.resized(1, "new", WIDE), Some(WIDE));
        sizes.input(1, "new");
        assert_eq!(sizes.resized(2, "new", NARROW), None, "the size rule holds");
    }
}
