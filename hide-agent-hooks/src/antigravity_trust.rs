//! Hide AI's tool-free folder for agy (PRD antigravity-first-class D-09):
//! agy loads a folder's own hook file only for a folder it trusts, so the
//! folder Hide AI runs agy in is made with its refuse-every-tool hook file and
//! recorded in agy's trust list through [`crate::folder_trust`], beside the
//! worktrees Hide makes. Switching agy off takes both out.

use std::io;
use std::path::Path;

use crate::folder_trust::{self, Agent, Tools};
use crate::install::InstallFailure;

fn settings_path(home: &Path) -> std::path::PathBuf {
    Agent::Antigravity.settings_path(home)
}

/// Makes Hide AI's tool-free folder for agy (`hide_ai::read_only_workspace`)
/// with its refuse-every-tool hook file, and records agy's trust of it, which
/// is what makes agy load that hook (PRD antigravity-first-class D-09). Done
/// when Hide installs agy's hook, once agy has made its own settings folder;
/// switching agy off takes both out ([`remove_read_only`]).
pub fn prepare_read_only(home: &Path) -> Result<(), InstallFailure> {
    if !settings_path(home).parent().is_some_and(Path::is_dir) {
        return Ok(());
    }
    let folder = hide_ai::read_only_workspace(home);
    let hooks = hide_ai::read_only_hooks_file(&folder);
    let failure = |detail: String| InstallFailure::NotWritable {
        path: hooks.display().to_string(),
        detail,
    };
    if let Some(parent) = hooks.parent() {
        hide_platform::fs::private::create_dir_all(parent)
            .map_err(|error| failure(error.to_string()))?;
    }
    if std::fs::read(&hooks).ok().as_deref() != Some(hide_ai::READ_ONLY_HOOKS.as_bytes()) {
        hide_platform::fs::atomic::write_file(
            &hooks,
            hide_ai::READ_ONLY_HOOKS.as_bytes(),
            hide_platform::fs::Access::Private,
        )
        .map_err(|error| failure(error.to_string()))?;
    }
    let canonical = hide_platform::fs::identity::canonical(&folder)
        .map_err(|error| failure(error.to_string()))?;
    folder_trust::trust(Agent::Antigravity, home, &canonical, Tools::none())
}

/// Whether Hide AI's agy folder is as [`prepare_read_only`] leaves it.
pub fn read_only_ready(home: &Path) -> bool {
    hide_ai::read_only_prepared(home).is_ok()
}

/// Whether [`prepare_read_only`] has something to do: agy has made its
/// settings folder and Hide AI's folder is not ready.
pub fn read_only_due(home: &Path) -> bool {
    settings_path(home).parent().is_some_and(Path::is_dir) && !read_only_ready(home)
}

/// Takes Hide AI's agy folder away; its trust goes with
/// [`folder_trust::forget_all`]. Answers whether there was one.
pub fn remove_read_only(home: &Path) -> Result<bool, InstallFailure> {
    let folder = hide_ai::read_only_workspace(home);
    match std::fs::remove_dir_all(&folder) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(InstallFailure::NotWritable {
            path: folder.display().to_string(),
            detail: error.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hide_ai_s_folder_is_made_trusted_and_taken_away_with_agy() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".gemini/antigravity-cli")).unwrap();
        prepare_read_only(home.path()).unwrap();
        prepare_read_only(home.path()).unwrap();
        assert!(read_only_ready(home.path()));
        assert_eq!(
            folder_trust::forget_all(Agent::Antigravity, home.path(), Tools::none()).unwrap(),
            1
        );
        assert!(remove_read_only(home.path()).unwrap());
        assert!(!read_only_ready(home.path()));
        assert!(!remove_read_only(home.path()).unwrap());
        // Before agy has made its settings folder there is nothing to trust in.
        let fresh = tempfile::tempdir().unwrap();
        prepare_read_only(fresh.path()).unwrap();
        assert!(!fresh.path().join(".hide").exists());
    }
}
