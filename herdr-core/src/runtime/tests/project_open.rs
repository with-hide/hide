//! `hide <path>` through the core: a folder a registered checkout holds comes
//! forward with nothing registered, any other folder goes through the same
//! `create_workspace` the Add a project dialog sends, and the request reads
//! its own receipt of how that ended.

use super::*;
use crate::project_open::ProjectOpenReceipt;

fn registered(runtime: &mut Runtime, path: &str, label: &str) -> WorkspaceRegistration {
    let registration = workspace::registration(path, label, crate::node::TEST_NODE).unwrap();
    runtime
        .snapshot
        .ui_state
        .workspace_registrations
        .push(registration.clone());
    registration
}

/// The workspace, checkout and path of the checkout listed for `path`,
/// which the catalog spells as the system does (`/private/tmp` on macOS).
fn checkout_at(runtime: &Runtime, path: &str) -> (String, String, String) {
    runtime
        .snapshot
        .navigator
        .workspaces
        .iter()
        .flat_map(|workspace| {
            workspace
                .checkouts
                .iter()
                .map(move |checkout| (workspace, checkout))
        })
        .find(|(_, checkout)| checkout.path.ends_with(path))
        .map(|(workspace, checkout)| {
            (
                workspace.id.clone(),
                checkout.id.clone(),
                checkout.path.clone(),
            )
        })
        .expect("the checkout is listed")
}

#[test]
fn a_folder_a_registered_checkout_holds_comes_forward_and_registers_nothing() {
    let mut runtime = live_runtime();
    registered(&mut runtime, "/tmp/hide-context-alpha", "Alpha");
    registered(&mut runtime, "/tmp/hide-context-zeta", "Zeta");
    runtime.ingest_session(Ok(context_payload()));
    let (zeta_workspace, zeta, _) = checkout_at(&runtime, "/tmp/hide-context-zeta");
    runtime.focus_checkout(&zeta_workspace, &zeta);
    let (_, alpha, alpha_path) = checkout_at(&runtime, "/tmp/hide-context-alpha");

    let node = runtime.node.as_str().to_owned();
    assert!(
        runtime
            .project_open(&node, &format!("{alpha_path}/src"), "1-open")
            .unwrap()
    );

    assert_eq!(
        runtime.project_open_receipt("1-open"),
        Some(ProjectOpenReceipt::Opened {
            label: "Alpha".to_owned(),
            path: alpha_path,
            added: false,
        })
    );
    assert_eq!(
        runtime.snapshot.navigator.focused_checkout_id.as_deref(),
        Some(alpha.as_str())
    );
    assert_eq!(runtime.snapshot.ui_state.workspace_registrations.len(), 2);
    assert!(runtime.workspace_creations_in_flight.is_empty());
}

#[test]
fn another_folder_is_added_by_create_workspace_and_its_receipt_follows_the_creation() {
    let mut runtime = live_runtime();
    let node = runtime.node.as_str().to_owned();
    let path = "/tmp/hide-open-new";

    assert!(runtime.project_open(&node, path, "1-new").unwrap());
    assert_eq!(
        runtime.project_open_receipt("1-new"),
        Some(ProjectOpenReceipt::Pending)
    );
    assert!(
        runtime.workspace_creations_in_flight.contains(path),
        "the dialog's own creation runs"
    );
    // The same request asked again joins it rather than adding twice.
    assert!(!runtime.project_open(&node, path, "1-new").unwrap());

    let registration = workspace::registration(path, "hide-open-new", &node).unwrap();
    assert!(runtime.ingest_workspace_creation(
        path,
        Ok(live::WorkspaceCreationOutcome {
            registration: registration.clone(),
            base_registrations: Vec::new(),
            registrations: vec![registration],
            workspaces: Vec::new(),
            session: context_payload(),
            created_pane_id: None,
            git_init_error: None,
            paths: Default::default(),
        }),
        3,
    ));
    assert!(matches!(
        runtime.project_open_receipt("1-new"),
        Some(ProjectOpenReceipt::Opened { added: true, .. })
    ));
}

#[test]
fn a_registration_that_fails_is_the_receipt_s_refusal() {
    let mut runtime = live_runtime();
    let node = runtime.node.as_str().to_owned();
    let path = "/tmp/hide-open-missing";
    runtime.project_open(&node, path, "1-missing").unwrap();

    runtime.ingest_workspace_creation(
        path,
        Err(live::WorkspaceCreationFailure::failed(
            "Workspace path does not exist",
        )),
        2,
    );
    assert_eq!(
        runtime.project_open_receipt("1-missing"),
        Some(ProjectOpenReceipt::Refused {
            reason: "workspace_create_failed".to_owned()
        })
    );

    // With no Herdr to open the project's first tab in, the add is refused at once.
    let mut offline = super::runtime();
    let node = offline.node.as_str().to_owned();
    offline.project_open(&node, path, "2-offline").unwrap();
    assert_eq!(
        offline.project_open_receipt("2-offline"),
        Some(ProjectOpenReceipt::Refused {
            reason: "workspace_control_unavailable".to_owned()
        })
    );
}

#[test]
fn every_receipt_still_waiting_refuses_one_more_request() {
    let mut runtime = live_runtime();
    let node = runtime.node.as_str().to_owned();
    for index in 0..32 {
        runtime
            .project_open(&node, &format!("/tmp/hide-open-{index}"), &format!("{index}-wait"))
            .unwrap();
    }
    assert_eq!(
        runtime.project_open(&node, "/tmp/hide-open-more", "99-more"),
        Err("project_open_busy".to_owned())
    );
}
