//! What hided gives Desk's process (PRD Desk 1차): a Workspace capability
//! that acts as Hide's own `hide:desk` and holds only while that process
//! runs, and the places it runs from. The core's Desk worker starts and owns
//! the process (`herdr-core/src/runtime/desk.rs`); this side only issues,
//! binds and ends its identity.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use herdr_core::desk::{DeskCredential, DeskPlaces, DeskPlatform};

use crate::pane_auth::Registry;

pub struct DeskHost {
    registry: Arc<Registry>,
    port: u16,
    node: String,
    places: DeskPlaces,
}

impl DeskHost {
    /// `hide_dir` is the folder of the `hide` program beside this hided, which
    /// the Desk agent runs first on its `PATH`.
    pub fn new(
        registry: Arc<Registry>,
        port: u16,
        node: String,
        home: &Path,
        state_dir: &Path,
        hide_dir: PathBuf,
    ) -> Self {
        Self {
            registry,
            port,
            node,
            places: DeskPlaces {
                adapters: hide_kit::layout::desk_adapters(home),
                desk: hide_kit::layout::desk_dir(state_dir),
                codex_home: hide_kit::layout::desk_codex_home(state_dir),
                hide_dir,
            },
        }
    }
}

impl DeskPlatform for DeskHost {
    fn issue(&self) -> Result<DeskCredential, String> {
        let mut bytes = [0u8; 16];
        getrandom::getrandom(&mut bytes).map_err(|error| error.to_string())?;
        let nonce = hex::encode(bytes);
        let reference = self
            .registry
            .issue_desk(
                &nonce,
                self.port,
                herdr_core::desk::desk_context(&self.node),
            )
            .map_err(str::to_owned)?;
        Ok(DeskCredential { reference, nonce })
    }

    fn bind(&self, credential: &DeskCredential, pid: u32, started: u64) -> Result<(), String> {
        let pid = i32::try_from(pid).map_err(|_| "pid out of range".to_owned())?;
        self.registry
            .bind_desk(&credential.reference, pid, started)
            .map_err(str::to_owned)
    }

    fn revoke(&self, credential: &DeskCredential) {
        self.registry.revoke_desk(&credential.reference);
    }

    fn places(&self) -> DeskPlaces {
        self.places.clone()
    }
}
