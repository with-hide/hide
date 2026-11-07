//! A sleeping agent's pane moved onto the holder of the build its node runs
//! now, as the wake begins (with-hide/hide-private#646).
//!
//! An update leaves every open pane on the holder of the build that started
//! it. An agent that sleeps in its pane leaves only its shell there, so
//! ending that holder loses nothing: the wake asks the pane's node whether
//! the holder runs another build's version folder and, when it does, has the
//! node end it and reserve its replacement under the same pane id, naming
//! the ended start as the one it consumes, then starts it as a restore
//! starts a pane whose holder ended. The pane keeps its id, place, tab and
//! everything named by it; its scrollback goes with the old holder.

use std::sync::Arc;

use hide_node_link::holders::HOLDER_KEPT;

use super::{
    EngineError, OnFailure, OwnEngine, creation, end_holders, settle_start, start_request, starts,
};

impl OwnEngine<'_> {
    /// [`crate::engine::Engine::replace_outdated_holder`].
    pub(super) fn replace_outdated(&self, pane: &str) -> Result<bool, EngineError> {
        self.check()?;
        let (cwd, env) = {
            let topology = self.topology();
            let held = topology.pane(pane)?;
            // Only a shell with nothing in it: an agent, a pane started with
            // a program, one closing, failed or still starting keeps its
            // holder.
            let shell = held.command.is_none()
                && held.agent.is_none()
                && !held.closing
                && held.failed.is_none()
                && held.starting.as_ref().is_none_or(|start| start.started());
            if !shell {
                return Ok(false);
            }
            // The variables the pane was started with, which its shell
            // and the agent it resumes see again; a pane a restore started
            // has none, as its restart had none.
            (held.cwd.clone(), held.env.clone())
        };
        let previous = match self.shared.holders.outdated(pane) {
            Ok(None) => return Ok(false),
            Ok(Some(previous)) => previous,
            Err(error) => {
                kept(pane, None, &error.to_string());
                return Ok(false);
            }
        };
        let ticket = match self.admit_start(start_request(pane, None, &cwd, env, None), false, None)
        {
            Ok(ticket) => ticket,
            Err(error) => {
                kept(pane, Some(previous), &error.to_string());
                return Ok(false);
            }
        };
        // Readers of the pane wait for the replacement as for any start, and
        // a close meanwhile leaves the holder to this call
        // (`settle_start`).
        let starting: Arc<starts::Starting> = Arc::default();
        let before = {
            let mut topology = self.topology();
            match topology.pane_mut(pane) {
                Ok(held) if held.agent.is_none() && !held.closing => {
                    held.starting.replace(Arc::clone(&starting))
                }
                _ => {
                    drop(topology);
                    creation::retire(
                        &*self.shared.holders,
                        &ticket,
                        &Arc::downgrade(&self.shared),
                    )?;
                    return Ok(false);
                }
            }
        };
        let reserved = self.reserve_admitted_by(ticket, |holders, start, request| {
            holders.replace(start, previous, request)
        });
        let ticket = match reserved {
            Ok(ticket) => ticket,
            Err(EngineError::Refused { code, message }) if code == HOLDER_KEPT => {
                self.keep(pane, &starting, before);
                kept(pane, Some(previous), &message);
                return Ok(false);
            }
            Err(error) => {
                // The old holder may be gone: the pane says its terminal
                // could not start, as a restore's that did not start does.
                settle_start(
                    &self.shared.topology,
                    &*self.shared.holders,
                    pane,
                    &starting,
                    &Err(error.clone()),
                    OnFailure::Mark,
                );
                self.changed();
                return Err(error);
            }
        };
        // The new holder's facts start afresh: its prompt count, folder and
        // foreground, and its first rule decision.
        if let Ok(held) = self.topology().pane_mut(pane) {
            held.live = Default::default();
        }
        let started = match ticket.publish(true, || Ok(())) {
            Ok(()) => creation::run(ticket, &*self.shared.holders, &Arc::downgrade(&self.shared)),
            Err(error) => {
                creation::run(ticket, &*self.shared.holders, &Arc::downgrade(&self.shared))
                    .and(Err(error))
            }
        };
        settle_start(
            &self.shared.topology,
            &*self.shared.holders,
            pane,
            &starting,
            &started,
            OnFailure::Mark,
        );
        self.changed();
        if started.is_ok() {
            crate::diagnostic!(serde_json::json!({
                "component": "own_engine",
                "kind": "engine.holder_replaced",
                "pane_id": pane,
                "incarnation": previous,
            }));
        }
        started.map(|()| true)
    }

    /// Puts the pane back as it was when its node kept its holder; a pane
    /// closed meanwhile left that holder to this call, which ends it.
    fn keep(&self, pane: &str, starting: &starts::Starting, before: Option<Arc<starts::Starting>>) {
        let present = {
            let mut topology = self.topology();
            starting.finish(Ok(()));
            match topology.pane_mut(pane) {
                Ok(held) => {
                    held.starting = before;
                    true
                }
                Err(_) => false,
            }
        };
        if !present {
            let _ = end_holders(&*self.shared.holders, &[pane.to_owned()]);
        }
    }
}

/// The node kept the pane's holder, or could not be asked: the wake goes on
/// in it as before.
fn kept(pane: &str, incarnation: Option<u64>, reason: &str) {
    crate::diagnostic!(serde_json::json!({
        "component": "own_engine",
        "kind": "engine.holder_kept",
        "pane_id": pane,
        "incarnation": incarnation,
        "message": reason,
    }));
}
