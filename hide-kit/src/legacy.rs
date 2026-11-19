//! Takes the folders an older layout left off a machine once the kit pass
//! has put everything in `~/.hide` (PRD hide-home-layout D-13).
//!
//! Every path here is a constant the target was built with, never one a
//! request named. A path is removed only when `lstat` says it is a real
//! folder this account owns, so a link is never followed out of it; a shared
//! parent such as `~/.local/share` is never removed, and a parent this layout
//! emptied is removed only with `rmdir`, which refuses anything left in it.
//! What could not be removed is reported and tried again on the next pass;
//! the operator has nothing to do about it, so it goes to the log (design
//! principle 13).

use std::path::{Path, PathBuf};

use crate::{KitTarget, Retirement};

/// One legacy path and how it goes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Legacy {
    /// A folder only Hide wrote to, removed with everything in it.
    Tree(PathBuf),
    /// A device helper root of the old layout: its builds and `current` link
    /// go, then the root itself if nothing else is left in it. Kept while a
    /// hook entry or the `hide` link still names it, so no agent turn loses
    /// its hook in between (B22).
    HelperRoot(PathBuf),
    /// A parent folder of the old layout, removed only when it is empty.
    IfEmpty(PathBuf),
}

/// Runs every retirement of `target` in order.
pub(crate) fn retire(target: &KitTarget) -> Retirement {
    let mut outcome = Retirement::default();
    for legacy in &target.legacy {
        match legacy {
            Legacy::Tree(path) => tree(path, &mut outcome),
            Legacy::HelperRoot(path) => helper_root(target, path, &mut outcome),
            Legacy::IfEmpty(path) => if_empty(path, &mut outcome),
        }
    }
    outcome
}

/// Whether `path` is a real folder this account owns: `Ok(None)` when it is
/// not there, an error sentence when it is something else.
fn own_folder(path: &Path) -> Result<Option<()>, String> {
    let owned = || hide_platform::fs::private::owned_by_current_user(path).unwrap_or(false);
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && owned() => Ok(Some(())),
        Ok(_) => Err(format!(
            "{} is not a folder of this account, so Hide left it",
            path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "{} could not be read: {}",
            path.display(),
            error.kind()
        )),
    }
}

fn tree(path: &Path, outcome: &mut Retirement) {
    match own_folder(path) {
        Ok(None) => {}
        Ok(Some(())) => match std::fs::remove_dir_all(path) {
            Ok(()) => outcome.removed.push(path.display().to_string()),
            Err(error) => outcome.failures.push(format!(
                "{} could not be removed: {}",
                path.display(),
                error.kind()
            )),
        },
        Err(reason) => outcome.failures.push(reason),
    }
}

fn if_empty(path: &Path, outcome: &mut Retirement) {
    if !matches!(own_folder(path), Ok(Some(()))) {
        return;
    }
    if std::fs::remove_dir(path).is_ok() {
        outcome.removed.push(path.display().to_string());
    }
}

fn helper_root(target: &KitTarget, root: &Path, outcome: &mut Retirement) {
    match own_folder(root) {
        Ok(None) => return,
        Ok(Some(())) => {}
        Err(reason) => {
            outcome.failures.push(reason);
            return;
        }
    }
    if let Some(user) = still_named(target, root) {
        outcome.failures.push(format!(
            "{} is kept while {user} still names it",
            root.display()
        ));
        return;
    }
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) => {
            outcome.failures.push(format!(
                "{} could not be listed: {}",
                root.display(),
                error.kind()
            ));
            return;
        }
    };
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        // Only what the helper install wrote: build folders, the `current`
        // link and its staged replacements.
        let result = if is_build_name(name) && kind.is_dir() && !kind.is_symlink() {
            std::fs::remove_dir_all(&path)
        } else if kind.is_symlink()
            && (name == crate::CURRENT || name.starts_with(&format!(".{}-", crate::CURRENT)))
        {
            std::fs::remove_file(&path)
        } else {
            continue;
        };
        if let Err(error) = result {
            outcome.failures.push(format!(
                "{} could not be removed: {}",
                path.display(),
                error.kind()
            ));
        }
    }
    match std::fs::remove_dir(root) {
        Ok(()) => outcome.removed.push(root.display().to_string()),
        Err(_) => outcome.failures.push(format!(
            "{} holds files Hide did not put there, so it stays",
            root.display()
        )),
    }
}

/// What the kit wrote that still leads into `root`: the `hide` link or a
/// hook file of an agent runtime.
fn still_named(target: &KitTarget, root: &Path) -> Option<String> {
    let link = crate::cli::link_path(target);
    if std::fs::read_link(&link).is_ok_and(|destination| destination.starts_with(root)) {
        return Some(link.display().to_string());
    }
    let needle = root.display().to_string();
    [
        hide_agent_hooks::AgentRuntime::ClaudeCode,
        hide_agent_hooks::AgentRuntime::Codex,
    ]
    .into_iter()
    .map(|runtime| runtime.config_path(&target.home))
    .find(|config| std::fs::read_to_string(config).is_ok_and(|text| text.contains(needle.as_str())))
    .map(|config| config.display().to_string())
}

/// A build folder's name under a helper root: the first sixteen hex digits
/// of the build digest.
pub fn is_build_name(name: &str) -> bool {
    name.len() == 16 && name.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Where a build's `hided` sits: `<root>/<version>/hided` (`hided.exe` on
/// Windows), the layout the kit installs and a core's login item runs.
#[derive(Debug, Eq, PartialEq)]
pub struct BuildPlace {
    /// The install root, in the wire spelling (`/` between names).
    pub root: String,
    pub version: String,
}

/// `program`, an absolute path in the wire spelling on whichever machine
/// holds it, as a build's place; anything else is refused with why. The one
/// reading of a build's path, for a program on this machine and one on the
/// core's.
pub fn build_of(program: &str) -> Result<BuildPlace, String> {
    let not_a_build = || format!("{program} is not a build's hided");
    let (folder, name) = program.rsplit_once('/').ok_or_else(not_a_build)?;
    let (root, version) = folder.rsplit_once('/').ok_or_else(not_a_build)?;
    if !matches!(name, "hided" | "hided.exe")
        || !is_build_name(version)
        || !hide_platform::path::is_wire_absolute(root)
    {
        return Err(not_a_build());
    }
    Ok(BuildPlace {
        root: root.to_owned(),
        version: version.to_owned(),
    })
}

/// What this Mac's kit pass retires: the folders the labels plugin era left
/// (`hide-plugin-upgrade`, the plugin's log folder) and the share folder
/// they leave empty (B9).
pub fn local(home: &Path) -> Vec<Legacy> {
    vec![
        Legacy::Tree(home.join(".local/state/hide-plugin-upgrade")),
        Legacy::Tree(home.join(".local/share/hide/agent-context-labels")),
        Legacy::IfEmpty(home.join(".local/share/hide")),
    ]
}

/// What a device's kit pass retires once its helper runs from the default
/// root: the old helper root, the old bridge folder, and the folders they
/// leave empty (B19, B23). A helper under any other root retires nothing,
/// since its consent named that root and not these (D-12).
pub fn device(home: &Path, root: &Path) -> Vec<Legacy> {
    if root != crate::layout::helper_root(home) {
        return Vec::new();
    }
    let mut legacy = vec![
        Legacy::HelperRoot(crate::layout::legacy_helper_root(home)),
        Legacy::IfEmpty(home.join(".local/share/hide")),
    ];
    // A device whose XDG_STATE_HOME is ~/.local/state still uses that folder
    // (D-04), and its bridges with it.
    let legacy_state = crate::layout::legacy_state_dir(home);
    if crate::layout::state_dir_from_process(home) != legacy_state {
        legacy.push(Legacy::Tree(crate::layout::workspace_bridges(
            &legacy_state,
        )));
        legacy.push(Legacy::IfEmpty(legacy_state));
    }
    legacy
}
