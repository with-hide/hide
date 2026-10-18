//! One account's hook settings have one live daemon owner.

use std::io;
use std::path::Path;
use std::time::Duration;

use hide_platform::fs::lock::{Lock, Mode, Waited, lock_file};

use crate::{KitRefusal, KitTarget, RefusalCause, layout, record::Record};

pub(crate) fn claim(target: &KitTarget, record: &Record) -> Result<Option<Lock>, KitRefusal> {
    let state = layout::state_dir_from_process(&target.home);
    let helper = target.kit_dir.join(hide_agent_hooks::HELPER_BINARY_NAME);
    claim_route(&helper, &state, record.state_route())
}

fn claim_route(
    helper: &Path,
    state: &Path,
    previous: Option<&Path>,
) -> Result<Option<Lock>, KitRefusal> {
    hide_agent_hooks::route::HookTarget::new(helper, state)
        .map_err(|reason| KitRefusal::unsafe_path(state, reason))?;
    let Some(previous) = previous.filter(|previous| *previous != state) else {
        return Ok(None);
    };
    hide_agent_hooks::route::HookTarget::new(helper, previous)
        .map_err(|reason| KitRefusal::record(previous, reason))?;
    // A spelling change is still this daemon's folder: taking its lifetime
    // lock again would refuse our own kit pass on aliases or case changes.
    match hide_platform::fs::identity::same_file(previous, state) {
        Ok(true) => return Ok(None),
        Ok(false) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(KitRefusal::unsafe_path(previous, error.to_string())),
    }
    let path = layout::daemon_lock(previous);
    let file = match hide_platform::fs::private::open_own_file(&path, false) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(KitRefusal::unsafe_path(&path, error.to_string())),
    };
    match lock_file(file, Mode::Exclusive, Duration::ZERO, &|| false) {
        Ok(Waited::Locked(lock)) => Ok(Some(lock)),
        Ok(Waited::TimedOut | Waited::Cancelled) => Err(KitRefusal::new(
            RefusalCause::Busy,
            Some(previous),
            "another running daemon owns this account's hooks; use a separate HOME or stop it first",
        )),
        Err(error) => Err(KitRefusal::unsafe_path(&path, error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ownership_waits_for_the_previous_daemon_to_end_on_every_system() {
        let home = tempfile::tempdir().unwrap();
        let previous = home.path().join("previous");
        let wanted = home.path().join("wanted");
        let helper = home.path().join("helper");
        std::fs::create_dir(&previous).unwrap();
        let path = layout::daemon_lock(&previous);
        assert!(
            claim_route(&helper, &wanted, Some(&previous))
                .unwrap()
                .is_none()
        );
        assert!(!path.exists(), "observing an absent owner creates no state");
        let file = hide_platform::fs::private::open_own_file(&path, true).unwrap();
        let Waited::Locked(owner) =
            lock_file(file, Mode::Exclusive, Duration::ZERO, &|| false).unwrap()
        else {
            panic!("fixture owns its lifetime lock")
        };
        assert_eq!(
            claim_route(&helper, &wanted, Some(&previous))
                .unwrap_err()
                .cause,
            RefusalCause::Busy
        );
        let child = previous.join("child");
        std::fs::create_dir(&child).unwrap();
        let alias = child.join("..");
        assert!(
            claim_route(&helper, &alias, Some(&previous))
                .unwrap()
                .is_none(),
            "another spelling of the current owner's folder is not another daemon"
        );
        drop(owner);
        assert!(
            claim_route(&helper, &wanted, Some(&previous))
                .unwrap()
                .is_some()
        );
        assert!(claim_route(&helper, Path::new("relative"), None).is_err());
    }
}
