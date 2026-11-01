//! Which ignored folders of a checkout a build tool makes again.
//!
//! The disk walk sorts everything a checkout holds into source, two layers a
//! tool rebuilds (build cache, dependencies) and the rest. Only a folder the
//! ignore rules name is ever a candidate, and it becomes a layer only when
//! something proves what it is: a signed `CACHEDIR.TAG`, or a marker file of
//! a known ecosystem in the same parent. Everything else stays where it is
//! and is counted as `other`, size only. This module owns those rules and the
//! recheck the cleanup runs again on each folder right before it moves it.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

pub use hide_node_link::disk::FolderRefusal;
use hide_node_link::disk::{DiskLayers, Layer, LayerFolder};

use crate::index::IgnoreRules;

/// What the walk holds of one top ignored folder while it is being counted.
pub(crate) struct FolderTally {
    pub path: PathBuf,
    /// `None` is a folder or file no rule vouches for.
    pub layer: Option<Layer>,
    pub bytes: u64,
    /// A `.git` was seen inside: another repository lives here.
    pub repository: bool,
}

/// The walk's per-checkout tally, turned into cells once the walk is done.
#[derive(Default)]
pub(crate) struct Tally {
    pub source_bytes: u64,
    pub folders: Vec<FolderTally>,
}

impl Tally {
    pub fn finish(self, root: &Path) -> (DiskLayers, Vec<LayerFolder>) {
        let mut layers = DiskLayers {
            source_bytes: self.source_bytes,
            ..Default::default()
        };
        // The biggest folder of each cell names it: build cache, dependencies, other.
        let mut largest: [Option<u64>; 3] = [None; 3];
        let mut kept = Vec::new();
        for folder in self.folders {
            let layer = folder.layer.filter(|_| !folder.repository);
            let (index, cell) = match layer {
                Some(Layer::BuildCache) => (0, &mut layers.build_cache),
                Some(Layer::Dependencies) => (1, &mut layers.dependencies),
                None => (2, &mut layers.other),
            };
            cell.folders += 1;
            cell.bytes = cell.bytes.saturating_add(folder.bytes);
            if largest[index].is_none_or(|bytes| folder.bytes > bytes) {
                largest[index] = Some(folder.bytes);
                cell.largest_name = Some(relative(root, &folder.path));
            }
            if let Some(layer) = layer {
                kept.push(LayerFolder {
                    path: folder.path,
                    layer,
                    bytes: folder.bytes,
                });
            }
        }
        (layers, kept)
    }
}

/// A folder's name below the walked root for the screen, in the wire's
/// spelling so it reads the same whatever system measured it.
fn relative(root: &Path, path: &Path) -> String {
    let below = path.strip_prefix(root).unwrap_or(path);
    hide_platform::path::RelPath::from_native(below)
        .map(hide_platform::path::RelPath::into_string)
        .unwrap_or_else(|_| below.to_string_lossy().into_owned())
}

/// The first 43 bytes of a cache directory tag (bford.info/cachedir).
const CACHEDIR_SIGNATURE: &[u8] = b"Signature: 8a477f597d28d172789f06886806bc55";

struct Rule {
    /// File names in the same parent that mark the ecosystem.
    markers: &'static [&'static str],
    /// File extensions of such a marker (`csproj`).
    extensions: &'static [&'static str],
    build_cache: &'static [&'static str],
    dependencies: &'static [&'static str],
}

/// Hide's own table (derived from kondo-lib, MIT, and split by whether a
/// tool makes the folder from sources alone or downloads it). A rule applies
/// only to a folder the ignore rules name and only beside its marker.
const RULES: &[Rule] = &[
    Rule {
        markers: &["Cargo.toml"],
        extensions: &[],
        build_cache: &["target"],
        dependencies: &[],
    },
    Rule {
        markers: &["package.json"],
        extensions: &[],
        build_cache: &["dist", "out", ".next", ".vite", ".turbo"],
        dependencies: &["node_modules"],
    },
    Rule {
        markers: &["pyproject.toml", "setup.py", "requirements.txt"],
        extensions: &["py"],
        build_cache: &["__pycache__", ".pytest_cache", ".mypy_cache", ".ruff_cache"],
        dependencies: &[".venv"],
    },
    Rule {
        markers: &[
            "build.gradle",
            "build.gradle.kts",
            "settings.gradle",
            "settings.gradle.kts",
            "pom.xml",
        ],
        extensions: &[],
        build_cache: &["build", ".gradle", "target"],
        dependencies: &[],
    },
    Rule {
        markers: &["Package.swift"],
        extensions: &[],
        build_cache: &[".build"],
        dependencies: &[],
    },
    Rule {
        markers: &["pubspec.yaml"],
        extensions: &[],
        build_cache: &[".dart_tool", "build"],
        dependencies: &[],
    },
    Rule {
        markers: &["mix.exs"],
        extensions: &[],
        build_cache: &["_build"],
        dependencies: &["deps"],
    },
    Rule {
        markers: &[],
        extensions: &["csproj", "fsproj", "vbproj"],
        build_cache: &["bin", "obj"],
        dependencies: &[],
    },
    Rule {
        markers: &["composer.json"],
        extensions: &[],
        build_cache: &[],
        dependencies: &["vendor"],
    },
];

/// The names a folder's parent holds, read once per directory.
pub(crate) struct Siblings {
    names: HashSet<String>,
}

impl Siblings {
    pub fn new(names: impl IntoIterator<Item = String>) -> Self {
        Self {
            names: names.into_iter().collect(),
        }
    }

    fn holds(&self, rule: &Rule) -> bool {
        rule.markers
            .iter()
            .any(|marker| self.names.contains(*marker))
            || self.names.iter().any(|name| {
                Path::new(name)
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| rule.extensions.contains(&extension))
            })
    }
}

/// The layer an ignored folder belongs to, if anything vouches for it: a
/// signed `CACHEDIR.TAG` inside it, else a rule of the table whose marker is
/// beside it. `None` means `other`.
pub(crate) fn layer_of(folder: &Path, siblings: &Siblings) -> Option<Layer> {
    if has_cache_tag(folder) {
        return Some(Layer::BuildCache);
    }
    let name = folder.file_name()?.to_str()?;
    RULES
        .iter()
        .filter(|rule| siblings.holds(rule))
        .find_map(|rule| {
            if rule.build_cache.contains(&name) {
                Some(Layer::BuildCache)
            } else if rule.dependencies.contains(&name) {
                Some(Layer::Dependencies)
            } else {
                None
            }
        })
}

fn has_cache_tag(folder: &Path) -> bool {
    use std::io::Read;
    let tag = folder.join("CACHEDIR.TAG");
    // Not through a link: a tag that points elsewhere vouches for nothing.
    let Ok(metadata) = std::fs::symlink_metadata(&tag) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    let mut head = [0u8; CACHEDIR_SIGNATURE.len()];
    std::fs::File::open(&tag)
        .and_then(|mut file| file.read_exact(&mut head))
        .is_ok()
        && head == CACHEDIR_SIGNATURE
}

/// Reads the folder's classification again from the files: it is inside the
/// checkout, reached through no link, ignored by the rules of every folder
/// above it, and still vouched for as `layer`. `exclude_dir` is the shared
/// Git directory's `info` folder.
pub fn verify_folder(
    root: &Path,
    folder: &Path,
    layer: Layer,
    exclude_dir: Option<&Path>,
) -> Result<(), FolderRefusal> {
    let relative = folder
        .strip_prefix(root)
        .map_err(|_| FolderRefusal::Changed)?;
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(FolderRefusal::Changed);
    }
    let mut rules = base_rules(root, exclude_dir);
    let mut here = root.to_path_buf();
    let steps: Vec<_> = relative.components().collect();
    for (index, component) in steps.iter().enumerate() {
        // `here` is an ancestor directory: the checkout root, then each folder below it.
        let metadata = std::fs::symlink_metadata(&here).map_err(|_| FolderRefusal::NotFound)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(FolderRefusal::Symlink);
        }
        rules = rules_in(&rules, &here);
        here.push(component);
        // An ignored ancestor would have been the candidate itself.
        if index + 1 < steps.len() && rules.ignores(&here, true) {
            return Err(FolderRefusal::Changed);
        }
    }
    let metadata = std::fs::symlink_metadata(folder).map_err(|_| FolderRefusal::NotFound)?;
    if metadata.file_type().is_symlink() {
        return Err(FolderRefusal::Symlink);
    }
    if !metadata.is_dir() || !rules.ignores(folder, true) {
        return Err(FolderRefusal::Changed);
    }
    let parent = folder.parent().ok_or(FolderRefusal::Changed)?;
    let siblings = Siblings::new(
        std::fs::read_dir(parent)
            .map_err(|_| FolderRefusal::NotFound)?
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned()),
    );
    match layer_of(folder, &siblings) {
        Some(found) if found == layer => Ok(()),
        _ => Err(FolderRefusal::Changed),
    }
}

/// The ignore rules at a checkout's root: the global excludes file and the
/// shared Git directory's `info/exclude`.
pub(crate) fn base_rules(root: &Path, exclude_dir: Option<&Path>) -> IgnoreRules {
    let rules = IgnoreRules::new(root);
    match exclude_dir
        .and_then(|dir| cap_std::fs::Dir::open_ambient_dir(dir, cap_std::ambient_authority()).ok())
    {
        Some(dir) => rules.with_file(&dir, root, "exclude"),
        None => rules,
    }
}

/// `rules` plus the `.gitignore` of `dir`.
pub(crate) fn rules_in(rules: &IgnoreRules, dir: &Path) -> IgnoreRules {
    match cap_std::fs::Dir::open_ambient_dir(dir, cap_std::ambient_authority()) {
        Ok(handle) => rules.with_file(&handle, dir, ".gitignore"),
        Err(_) => rules.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn siblings(names: &[&str]) -> Siblings {
        Siblings::new(names.iter().map(|name| (*name).to_owned()))
    }

    #[test]
    fn a_marker_beside_the_folder_decides_its_layer() {
        let root = tempfile::tempdir().unwrap();
        let cases = [
            ("target", &["Cargo.toml"][..], Some(Layer::BuildCache)),
            ("node_modules", &["package.json"], Some(Layer::Dependencies)),
            ("dist", &["package.json"], Some(Layer::BuildCache)),
            ("__pycache__", &["main.py"], Some(Layer::BuildCache)),
            (".venv", &["pyproject.toml"], Some(Layer::Dependencies)),
            ("obj", &["App.csproj"], Some(Layer::BuildCache)),
            ("vendor", &["composer.json"], Some(Layer::Dependencies)),
            // The same name without its marker vouches for nothing.
            ("dist", &["index.html"], None),
            ("target", &["package.json"], None),
            ("node_modules", &["Cargo.toml"], None),
        ];
        for (name, beside, expected) in cases {
            let folder = root.path().join(name);
            std::fs::create_dir_all(&folder).unwrap();
            assert_eq!(
                layer_of(&folder, &siblings(beside)),
                expected,
                "{name} {beside:?}"
            );
        }
    }

    #[test]
    fn only_a_signed_cache_tag_makes_a_build_cache() {
        let root = tempfile::tempdir().unwrap();
        let signed = root.path().join("signed");
        let forged = root.path().join("forged");
        std::fs::create_dir_all(&signed).unwrap();
        std::fs::create_dir_all(&forged).unwrap();
        std::fs::write(
            signed.join("CACHEDIR.TAG"),
            "Signature: 8a477f597d28d172789f06886806bc55\n# made by a tool\n",
        )
        .unwrap();
        std::fs::write(forged.join("CACHEDIR.TAG"), "keep me\n").unwrap();
        assert_eq!(layer_of(&signed, &siblings(&[])), Some(Layer::BuildCache));
        assert_eq!(layer_of(&forged, &siblings(&[])), None);
    }

    #[test]
    fn verify_folder_refuses_what_changed_since_the_measurement() {
        let root = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(root.path()).unwrap();
        std::fs::write(root.join("Cargo.toml"), "").unwrap();
        std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
        std::fs::create_dir_all(root.join("target")).unwrap();
        let target = root.join("target");
        assert_eq!(
            verify_folder(&root, &target, Layer::BuildCache, None),
            Ok(())
        );
        // The rule that names it is gone.
        std::fs::write(root.join(".gitignore"), "").unwrap();
        assert_eq!(
            verify_folder(&root, &target, Layer::BuildCache, None),
            Err(FolderRefusal::Changed)
        );
        // Swapped for a link.
        std::fs::write(root.join(".gitignore"), "target/\ntarget\n").unwrap();
        std::fs::remove_dir(&target).unwrap();
        hide_platform::fs::link::create_link(&std::env::temp_dir(), &target).unwrap();
        assert_eq!(
            verify_folder(&root, &target, Layer::BuildCache, None),
            Err(FolderRefusal::Symlink)
        );
        hide_platform::fs::link::remove_link(&target).unwrap();
        assert_eq!(
            verify_folder(&root, &target, Layer::BuildCache, None),
            Err(FolderRefusal::NotFound)
        );
    }
}
