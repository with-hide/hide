//! A checkout's disk usage as its node measured it: the total, the biggest
//! top-level folder, and the share by layer (source, the build caches and
//! dependencies a tool makes again, and the rest).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The `unavailable_code` of the account's home folder, which is never
/// measured: it holds the whole account, other projects and app data
/// included, so its sum is neither a project's size nor worth the walk (PRD
/// home-folder-project D-06). The row has no total and no reason, so the
/// Projects tab draws no size and no warning for it.
pub const HOME_FOLDER_CODE: &str = "home_folder";

/// A layer Hide will empty.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layer {
    BuildCache,
    Dependencies,
}

impl Layer {
    pub fn code(self) -> &'static str {
        match self {
            Layer::BuildCache => "build_cache",
            Layer::Dependencies => "dependencies",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "build_cache" => Some(Layer::BuildCache),
            "dependencies" => Some(Layer::Dependencies),
            _ => None,
        }
    }
}

/// Why a folder is no longer safe to move, decided right before the move.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FolderRefusal {
    NotFound,
    Symlink,
    /// No longer ignored, or no rule or tag vouches for it as this layer.
    Changed,
}

impl FolderRefusal {
    pub fn code(self) -> &'static str {
        match self {
            FolderRefusal::NotFound => "not_found",
            FolderRefusal::Symlink => "symlink",
            FolderRefusal::Changed => "changed",
        }
    }
}

/// One cell of the table: what a checkout holds in one layer.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct LayerCell {
    pub bytes: u64,
    pub folders: usize,
    /// The biggest folder of the cell, relative to the checkout.
    pub largest_name: Option<String>,
}

/// A checkout's allocated bytes by layer.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DiskLayers {
    pub build_cache: LayerCell,
    pub dependencies: LayerCell,
    /// Ignored folders no rule vouches for. Measured, never removed.
    pub other: LayerCell,
    /// Everything the ignore rules do not name.
    pub source_bytes: u64,
}

impl DiskLayers {
    pub fn cell(&self, layer: Layer) -> &LayerCell {
        match layer {
            Layer::BuildCache => &self.build_cache,
            Layer::Dependencies => &self.dependencies,
        }
    }
}

/// A folder of a layer, kept by the core so the cleanup can move it. The
/// wire carries the cell totals, never these paths.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LayerFolder {
    pub path: PathBuf,
    pub layer: Layer,
    pub bytes: u64,
}

/// One measured root. `folders` names the layer folders behind the cells,
/// which the core keeps so a cleanup can move them; the snapshot wire never
/// carries them.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DiskUsage {
    pub measured_at_unix_ms: Option<u64>,
    pub path: Option<String>,
    pub total_bytes: Option<u64>,
    pub largest_child_name: Option<String>,
    pub largest_child_bytes: Option<u64>,
    pub unavailable_reason: Option<String>,
    /// Why there is no total, as a code for the log.
    pub unavailable_code: Option<String>,
    pub layers: Option<DiskLayers>,
    pub volume_free_bytes: Option<u64>,
    /// The size of that volume, for the Projects tab's `여유 X / 전체 Y`.
    #[serde(default)]
    pub volume_total_bytes: Option<u64>,
    pub folders: Vec<LayerFolder>,
}
