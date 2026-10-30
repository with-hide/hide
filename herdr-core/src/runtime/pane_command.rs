//! What `hide pane`, `hide agent read` and `hide workspace purpose` take
//! from the runtime under its lock: the caller's checkout, the pane or agent
//! named, and the engine of the machine it is on (`crate::pane_command`).

use crate::pane_command::{PaneCommand, Prepared, Step};
use crate::workspace::authoritative_session_space;
use crate::workspace_control::{Caller, Context};

use super::Runtime;

/// A pane id as the core spells it: a device's as
/// `remote:<device>:pane:<id>`, this machine's bare.
fn split_scoped<'a>(node: &'a str, scoped: &'a str) -> (&'a str, &'a str) {
    scoped
        .strip_prefix("remote:")
        .and_then(|rest| rest.split_once(":pane:"))
        .unwrap_or((node, scoped))
}

impl Runtime {
    /// Admits `command` for the pane-bound `caller` on `device` whose
    /// checkout is `expected`, and resolves what it acts on. A bare pane id
    /// is a pane of the caller's own machine, as Herdr numbered it there.
    pub(crate) fn prepare_pane_command(
        &self,
        device: &str,
        caller: &str,
        expected: &Context,
        command: PaneCommand,
    ) -> Result<Prepared, String> {
        command.check()?;
        let Caller::Pane(caller_pane) = Caller::parse(caller) else {
            return Err("pane_required".into());
        };
        let context = self
            .resolve_caller(device, Caller::Pane(caller_pane))
            .map_err(|refusal| refusal.reason.to_owned())?;
        if context != *expected {
            return Err("caller_context_changed".into());
        }
        let node = self.node.as_str();
        // A pane named in full stays on the machine it names; a bare one is
        // the caller's.
        let target = |pane: &str| -> (String, String) {
            if pane.starts_with("remote:") {
                let (machine, raw) = split_scoped(node, pane);
                (machine.to_owned(), raw.to_owned())
            } else {
                (device.to_owned(), pane.to_owned())
            }
        };
        let engine_of = |machine: &str| self.delivery_engine(machine).ok_or("engine_unavailable");
        let (machine, step) = match command {
            PaneCommand::Read { pane, lines } => {
                let (machine, raw) = target(&pane);
                (machine, Step::Read { pane, raw, lines })
            }
            PaneCommand::Send { pane, text, enter } => {
                let (machine, raw) = target(&pane);
                (
                    machine,
                    Step::Send {
                        pane,
                        raw,
                        text,
                        enter,
                    },
                )
            }
            PaneCommand::AgentRead { agent, lines } => {
                let observation = self.agent_named(device, &agent)?;
                (
                    observation.actor.device_id.clone(),
                    Step::Read {
                        pane: agent,
                        raw: observation.raw_pane_id.clone(),
                        lines,
                    },
                )
            }
            PaneCommand::Open { command, focus } => {
                let (_, raw) = split_scoped(node, caller_pane);
                (
                    device.to_owned(),
                    Step::Open {
                        caller: raw.to_owned(),
                        cwd: context.checkout_path.clone(),
                        command,
                        focus,
                        // The caller's machine numbers its own panes, so the
                        // new one is told bare, as its own pane is.
                        remote_prefix: None,
                    },
                )
            }
            PaneCommand::Purpose { text } => (
                device.to_owned(),
                Step::Purpose {
                    workspace: self.purpose_workspace(device, &context.checkout_id)?,
                    text: text.trim().to_owned(),
                },
            ),
        };
        Ok(Prepared {
            engine: engine_of(&machine)?,
            step,
        })
    }

    /// The live agent `key` names for a caller on `device`, as a letter's
    /// target is named: its registered name or id, the name Herdr gave it,
    /// or its pane.
    fn agent_named(
        &self,
        device: &str,
        key: &str,
    ) -> Result<&super::delivery::Observation, String> {
        let pane = if self.node == device || key.starts_with("remote:") {
            key.to_owned()
        } else {
            format!("remote:{device}:pane:{key}")
        };
        let registered = self
            .delivery_ledger
            .as_ref()
            .ok()
            .and_then(|ledger| crate::coordination::resolve_registered(ledger, key));
        let mut matches = self.delivery_observations.values().filter(|observation| {
            observation.actor.pane_id == pane
                || observation.given_name() == Some(key)
                || registered.is_some_and(|registered| registered.same_identity(&observation.actor))
        });
        let first = matches.next().ok_or("agent_unavailable")?;
        if matches.next().is_some() {
            return Err("target_ambiguous".into());
        }
        Ok(first)
    }

    /// The engine workspace that holds the purpose of the caller's checkout:
    /// a device checkout's owner workspace, this machine's authoritative
    /// session space, as the checkout row's Set purpose writes it.
    fn purpose_workspace(&self, device: &str, checkout_id: &str) -> Result<String, String> {
        let find = |workspaces: &[crate::model::WorkspaceSnapshot]| {
            workspaces.iter().find_map(|workspace| {
                workspace
                    .checkouts
                    .iter()
                    .find(|checkout| checkout.id == checkout_id)
                    .map(|checkout| (workspace.clone(), checkout.clone()))
            })
        };
        let found = if self.node == device {
            find(&self.snapshot.navigator.workspaces).and_then(|(workspace, checkout)| {
                authoritative_session_space(&self.last_session_spaces, &workspace, &checkout.path)
                    .map(|space| space.id.clone())
            })
        } else {
            self.snapshot
                .status
                .remote
                .iter()
                .filter(|remote| remote.target_id == device)
                .find_map(|remote| find(&remote.session.as_ref()?.workspaces))
                .and_then(|(_, checkout)| checkout.owner_workspace_id)
        };
        found.ok_or_else(|| "purpose_no_workspace".to_owned())
    }
}
