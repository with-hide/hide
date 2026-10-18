//! A saved start creates one explicitly claimed terminal, then joins that
//! terminal into the requested tab without replacing any existing PTY.
use super::*;
use crate::recent_closed::ClosedLayout;
use crate::start_claim::{BoundPane, Effect};
use hide_node_link::panes::PaneIdentity;
use hide_node_link::protocol::ShellLaunch;

#[cfg(test)]
#[path = "checkout_start_tests.rs"]
mod tests;

/// All blocking work remains on the existing task worker. Saving crosses
/// the same coalesced settings writer as the original intent.
pub(super) struct Control<'a> {
    pub connector: &'a dyn ApiConnector,
    pub node: &'a dyn crate::node_access::NodeLink,
    pub save: &'a dyn Fn(&Effect) -> Result<(), String>,
    pub check: &'a dyn Fn() -> Result<(), String>,
}

fn uncertain(message: &str) -> ControlFailure {
    ControlFailure::Ambiguous(message.to_owned())
}

fn inspect(control: &Control<'_>, pane: &str) -> Result<PaneIdentity, ControlFailure> {
    (control.check)().map_err(ControlFailure::Ambiguous)?;
    crate::node_access::call_as(
        control.node,
        Call::PaneInspect {
            pane_id: pane.to_owned(),
        },
        Duration::from_secs(5),
    )
    .map_err(|_| {
        uncertain(
            "The terminal identity could not be checked. Reconnect and check this start's status.",
        )
    })
}

fn save(control: &Control<'_>, effect: &Effect) -> Result<(), ControlFailure> {
    (control.check)().map_err(ControlFailure::Ambiguous)?;
    (control.save)(effect).map_err(ControlFailure::Ambiguous)
}

fn export(control: &Control<'_>, marker: &str, tab: &str) -> Result<ClosedLayout, ControlFailure> {
    (control.check)().map_err(ControlFailure::Ambiguous)?;
    export_reopen_layout(control.connector, marker, tab).map_err(ControlFailure::Ambiguous)
}

fn leaves<'a>(node: &'a ClosedLayoutNode, output: &mut Vec<(&'a str, &'a ClosedLayoutNode)>) {
    match node {
        ClosedLayoutNode::Pane {
            pane_id: Some(pane),
            ..
        } => output.push((pane, node)),
        ClosedLayoutNode::Pane { .. } => {}
        ClosedLayoutNode::Split { first, second, .. } => {
            leaves(first, output);
            leaves(second, output);
        }
    }
}

fn verify_bound(control: &Control<'_>, bound: &BoundPane) -> Result<(), ControlFailure> {
    if inspect(control, &bound.pane_id)? != bound.identity {
        return Err(uncertain(
            "The original terminal was replaced. Inspect its tab before continuing.",
        ));
    }
    Ok(())
}

/// Discovery never uses a cwd, human title, pane count or new ID as a claim.
/// The physical bound includes staging alongside 64 admitted logical tabs.
fn discover(
    control: &Control<'_>,
    marker: &str,
    effect: &Effect,
) -> Result<(ClosedLayout, String, Option<Vec<String>>), ControlFailure> {
    (control.check)().map_err(ControlFailure::Ambiguous)?;
    let snapshot = fetch_session_with_connector(control.connector).map_err(|_| {
        uncertain("The start's topology could not be checked. Reconnect and check its status.")
    })?;
    let tabs: Vec<_> = snapshot
        .tabs
        .iter()
        .filter(|tab| tab.workspace_id == effect.workspace_id)
        .collect();
    if tabs.len() > 128 {
        return Err(uncertain(
            "Too many tabs for bounded start confirmation. Inspect the checkout before continuing.",
        ));
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    let claim = crate::start_claim::label(marker);
    let mut found = None;
    for tab in tabs {
        if Instant::now() > deadline {
            return Err(uncertain(
                "Start confirmation timed out. Check the same start's status again.",
            ));
        }
        let layout = export(control, marker, &tab.tab_id)?;
        if layout.workspace_id != effect.workspace_id || layout.tab_id != tab.tab_id {
            return Err(uncertain(
                "The start's owner changed. Inspect its original checkout.",
            ));
        }
        let mut nodes = Vec::new();
        leaves(&layout.root, &mut nodes);
        let matches: Vec<_> = nodes
            .into_iter()
            .filter_map(|(pane, node)| match node {
                ClosedLayoutNode::Pane { label, command, .. }
                    if label.as_deref() == Some(claim.as_str()) =>
                {
                    Some((pane.to_owned(), command.clone()))
                }
                _ => None,
            })
            .collect();
        if matches.len() > 1 || (found.is_some() && !matches.is_empty()) {
            return Err(uncertain(
                "More than one terminal carries this start. Inspect its checkout before continuing.",
            ));
        }
        if let Some((pane, command)) = matches.into_iter().next() {
            if snapshot
                .panes
                .iter()
                .any(|item| item.pane_id == pane && item.failed_terminal_id.is_some())
            {
                return Err(uncertain(
                    "The original terminal could not be restored. Inspect its tab before continuing.",
                ));
            }
            found = Some((layout, pane, command));
        }
    }
    found.ok_or_else(|| uncertain("The original start is not yet confirmed. Retry confirmation; no second process was started."))
}

fn validate_target(
    control: &Control<'_>,
    marker: &str,
    effect: &Effect,
) -> Result<Option<ClosedLayout>, ControlFailure> {
    let Some(target) = &effect.target else {
        return Ok(None);
    };
    let layout = export(control, marker, &target.tab_id)?;
    let mut nodes = Vec::new();
    leaves(&layout.root, &mut nodes);
    if layout.workspace_id != effect.workspace_id
        || layout.tab_id != target.tab_id
        || !nodes.iter().any(|(id, _)| *id == target.pane_id)
    {
        return Err(uncertain(
            "The selected tab changed. Inspect the original start before continuing.",
        ));
    }
    verify_bound(control, target)?;
    Ok(Some(layout))
}

pub(super) fn run(
    control: &Control<'_>,
    request: &CheckoutTabRequest,
    launch: Option<&ShellLaunch>,
) -> Result<WorktreeTaskOutcome, ControlFailure> {
    let start = request
        .start
        .as_ref()
        .ok_or_else(|| ControlFailure::Definite("The start has no original intent.".into()))?;
    let claim = crate::start_claim::label(&start.marker);
    if !crate::start_claim::is_reserved(&claim) {
        return Err(uncertain(
            "The saved start has no valid creation claim. Inspect its original tab.",
        ));
    }
    (control.check)().map_err(ControlFailure::Definite)?;
    let mut effect = match &start.effect {
        Some(effect) => effect.clone(),
        None if start.recovery_only => {
            return Err(uncertain(
                "The original start has no saved terminal proof. Inspect its tab; no second process was started.",
            ));
        }
        None => {
            let launch = launch.filter(|launch| !launch.command.is_empty()).ok_or_else(|| {
                ControlFailure::Definite("The device's normal shell could not be confirmed. Check its shell configuration and reconnect before retrying; no terminal was started.".into())
            })?;
            let (workspace_id, seed) = match &request.host {
                crate::checkout_owner::TabHost::Workspace(id) => (id.clone(), None),
                crate::checkout_owner::TabHost::Open(owner) => {
                    let (workspace, first) =
                        ensure_owner(control.connector, owner, false, Default::default())?;
                    (workspace, first.map(|(tab, _, _)| tab))
                }
            };
            let target = if let Some(tab) = &start.tab_id {
                let layout = export(control, &start.marker, tab)?;
                if layout.workspace_id != workspace_id || layout.tab_id != *tab {
                    return Err(ControlFailure::Definite(
                        "The selected tab no longer belongs to this checkout. Refresh and retry."
                            .into(),
                    ));
                }
                Some(BoundPane {
                    tab_id: tab.clone(),
                    pane_id: layout.focused_pane_id.clone(),
                    identity: inspect(control, &layout.focused_pane_id)?,
                })
            } else {
                None
            };
            let effect = Effect {
                workspace_id,
                seed_tab_id: seed,
                target,
                expected_command: Some(launch.command.clone()),
                shell_env: launch.env.clone(),
                shell_readiness: launch.readiness,
                binding: None,
            };
            save(control, &effect)?;
            effect
        }
    };
    if let crate::checkout_owner::TabHost::Workspace(owner) = &request.host
        && *owner != effect.workspace_id
    {
        return Err(uncertain(
            "The checkout's owner changed. Inspect the original start before continuing.",
        ));
    }
    validate_target(control, &start.marker, &effect)?;
    let mut acknowledged = None;
    if !start.recovery_only && effect.binding.is_none() {
        (control.check)().map_err(ControlFailure::Ambiguous)?;
        let params = wire::checkout_start_tab_params(
            &effect.workspace_id,
            effect.seed_tab_id.as_deref(),
            &request.cwd,
            &request.label,
            effect.shell_env.clone(),
            effect.expected_command.as_deref(),
            &claim,
        )?;
        match mutation_request(control.connector, "layout.apply", params) {
            Ok(value) => acknowledged = wire::launched_tab(value).ok(),
            Err(failure) if !failure.is_ambiguous() => return Err(failure),
            Err(_) => {}
        }
    }
    let (mut layout, pane, _) = discover(control, &start.marker, &effect)?;
    let identity = inspect(control, &pane)?;
    if let Some(bound) = &effect.binding {
        if bound.pane_id != pane
            || bound.identity != identity
            || (layout.tab_id != bound.tab_id
                && effect
                    .target
                    .as_ref()
                    .is_none_or(|target| layout.tab_id != target.tab_id))
        {
            return Err(uncertain(
                "The original terminal or its placement changed. Inspect its tab before continuing.",
            ));
        }
    } else {
        if acknowledged
            .as_ref()
            .is_some_and(|(tab, expected_pane)| *tab != layout.tab_id || *expected_pane != pane)
        {
            return Err(uncertain(
                "The creation answer and terminal claim disagree. Inspect its tab before continuing.",
            ));
        }
        // Bracket a fresh authoritative claim/command read with incarnation
        // reads. The ACK identifies a pane, not the shell still occupying it.
        let (confirmed_layout, confirmed_pane, command) =
            discover(control, &start.marker, &effect)?;
        let confirmed_identity = inspect(control, &confirmed_pane)?;
        if confirmed_pane != pane
            || confirmed_layout.tab_id != layout.tab_id
            || confirmed_identity != identity
            || effect.expected_command.as_ref().is_none_or(Vec::is_empty)
            || command != effect.expected_command
        {
            return Err(uncertain(
                "The original shell's continuity is not confirmed. Inspect its tab; no second process was started.",
            ));
        }
        layout = confirmed_layout;
        effect.binding = Some(BoundPane {
            tab_id: layout.tab_id.clone(),
            pane_id: pane.clone(),
            identity,
        });
        save(control, &effect)?;
    }
    let bound = effect
        .binding
        .as_ref()
        .expect("the observed incarnation was saved");
    if let Some(target) = &effect.target {
        if layout.tab_id != target.tab_id {
            let before = validate_target(control, &start.marker, &effect)?.expect("target exists");
            let mut source_nodes = Vec::new();
            leaves(&layout.root, &mut source_nodes);
            if source_nodes.len() != 1 || layout.zoomed {
                return Err(uncertain(
                    "The staged terminal changed. Inspect it before continuing this start.",
                ));
            }
            if before.zoomed {
                (control.check)().map_err(ControlFailure::Ambiguous)?;
                mutation_request(
                    control.connector,
                    "pane.zoom",
                    wire::pane_unzoom_params(&target.pane_id)?,
                )
                .map_err(|_| {
                    uncertain(
                        "The terminal was created, but its tab could not be prepared. Check this start's status before continuing.",
                    )
                })?;
                let unzoomed =
                    validate_target(control, &start.marker, &effect)?.expect("target exists");
                if unzoomed.zoomed || unzoomed.root != before.root {
                    return Err(uncertain(
                        "The selected layout changed while starting. Inspect it before continuing.",
                    ));
                }
            }
            verify_bound(control, bound)?;
            (control.check)().map_err(ControlFailure::Ambiguous)?;
            let moved = mutation_request(
                control.connector,
                "pane.move",
                wire::pane_join_params(&pane, &target.tab_id, &target.pane_id)?,
            );
            if let Ok(value) = moved {
                let answer = wire::joined_pane(value).map_err(ControlFailure::Ambiguous)?;
                if !answer.changed && answer.reason.as_deref() != Some("same_tab") {
                    return Err(uncertain(
                        "The terminal could not join the selected tab. Check its status after the layout settles.",
                    ));
                }
            } else if let Err(failure) = moved
                && !failure.is_ambiguous()
            {
                return Err(ControlFailure::Ambiguous(failure.message().to_owned()));
            }
            let (joined, found_pane, _) = discover(control, &start.marker, &effect)?;
            if found_pane != pane || joined.tab_id != target.tab_id {
                return Err(uncertain(
                    "The original terminal is still waiting to join. Check this start's status again.",
                ));
            }
            let mut before_nodes = Vec::new();
            leaves(&before.root, &mut before_nodes);
            let mut after_nodes = Vec::new();
            leaves(&joined.root, &mut after_nodes);
            if before_nodes
                .iter()
                .any(|(id, _)| !after_nodes.iter().any(|(after, _)| after == id))
            {
                return Err(uncertain(
                    "The selected tab changed while joining. Inspect its existing terminals before continuing.",
                ));
            }
            layout = joined;
        }
        validate_target(control, &start.marker, &effect)?;
    }
    verify_bound(control, bound)?;
    let saved_launch = effect.expected_command.map(|command| ShellLaunch {
        command,
        readiness: effect.shell_readiness,
        env: effect.shell_env,
    });
    Ok(WorktreeTaskOutcome {
        path: request.checkout_path.clone(),
        pane_id: pane,
        created_tab_id: Some(layout.tab_id),
        purpose_error: None,
        unconfirmed_purpose_token: None,
        issue_error: None,
        shell: crate::agent_start::ShellStart::of_tab(true, saved_launch.as_ref()),
    })
}
