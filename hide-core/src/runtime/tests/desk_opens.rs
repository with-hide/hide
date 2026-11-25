use super::workspace_view::{second_checkout, with_new_views};
use super::*;
use crate::model::DeskOpenSnapshot;
use crate::workspace_control::{Action, ActionPreparation, ActionResult, DESK_CALLER, Refusal};

// `hide file open` from Desk's agent (docs/delivery.md, Desk; docs/ARCHITECTURE.md,
// Desk): Desk has no checkout and no View, so the file is checked inside the
// project Hide shows that holds it, read on that checkout's host as an agent's
// file open is, and kept in the `desk` section for a window showing Desk to
// open in its tab panel; it brings no checkout forward and shows in no View.
// `hide diff open` and every other Workspace action stay refused.

/// A running Desk beside two registered checkouts, the second not in front.
fn desk_runtime(name: &str) -> (Runtime, PathBuf) {
    let (mut runtime, _, directory) = strip_checkout(name);
    let (other, _) = second_checkout(&mut runtime, &directory);
    let mut runtime = with_new_views(runtime, name);
    // This machine's engine is connected, as the caller check of an agent's
    // file open requires.
    runtime.snapshot.status.engine.state = "connected".into();
    runtime.open_test_desk(4);
    (runtime, other)
}

fn desk_context() -> crate::workspace_control::Context {
    crate::desk::desk_context(crate::node::TEST_NODE)
}

/// Desk's command as hided runs it: prepare under the lock, the host read
/// off it, then the commit.
fn desk_action(runtime: &mut Runtime, id: &str, action: Action) -> Result<ActionResult, Refusal> {
    let expected = desk_context();
    let material = match runtime.workspace_control_prepare_action(
        crate::node::TEST_NODE,
        DESK_CALLER,
        &expected,
        id,
        &action,
    )? {
        ActionPreparation::Cached(cached) => return cached,
        ActionPreparation::Ready => Ok(None),
        ActionPreparation::Read(source) => source.read().map(Some),
    };
    runtime.workspace_control_action(
        crate::node::TEST_NODE,
        DESK_CALLER,
        &expected,
        id,
        action,
        material,
    )
}

fn open_file(runtime: &mut Runtime, id: &str, path: &Path) -> Result<ActionResult, Refusal> {
    desk_action(
        runtime,
        id,
        Action::OpenFile {
            path: path.to_string_lossy().into_owned(),
        },
    )
}

fn opens(runtime: &Runtime) -> Vec<DeskOpenSnapshot> {
    runtime
        .snapshot
        .desk
        .as_ref()
        .expect("desk section")
        .opens
        .clone()
}

fn front(runtime: &Runtime) -> (Option<String>, Option<String>) {
    let navigator = &runtime.snapshot.navigator;
    (
        navigator.focused_workspace_id.clone(),
        navigator.focused_checkout_id.clone(),
    )
}

#[test]
fn a_file_desk_opens_waits_in_the_desk_section_until_a_window_takes_it() {
    let (mut runtime, other) = desk_runtime("desk-open-waits");
    let file = other.join("plan.md");
    std::fs::write(&file, "plan\n").expect("fixture");
    let before = front(&runtime);

    let opened = open_file(&mut runtime, "r-1", &file).expect("Desk opens a file of a project");

    assert_eq!(opened.context, desk_context(), "it answers as Desk");
    let waiting = opens(&runtime);
    assert_eq!(
        waiting,
        vec![DeskOpenSnapshot {
            id: waiting[0].id,
            device_id: crate::node::TEST_NODE.to_owned(),
            root: other.to_string_lossy().into_owned(),
            path: file.to_string_lossy().into_owned(),
        }],
        "the file waits with its machine and its checkout's folder"
    );
    assert_eq!(front(&runtime), before, "no checkout comes forward");
    assert!(
        runtime
            .snapshot
            .editor
            .tabs
            .iter()
            .all(|tab| tab.path != file.to_string_lossy()),
        "nothing opens until a window holds the file"
    );

    // The same command again, or the same file again, is the one open.
    let again = open_file(&mut runtime, "r-1", &file).unwrap();
    assert_eq!(again.view_id, opened.view_id);
    let same_file = open_file(&mut runtime, "r-2", &file).unwrap();
    assert_eq!(same_file.view_id, opened.view_id);
    assert_eq!(opens(&runtime).len(), 1);

    let id = waiting[0].id;
    runtime.dispatch_json(&explorer_event(
        "desk_open_taken",
        serde_json::json!({ "id": id }),
    ));
    assert_eq!(runtime.snapshot.status.last_error, None);
    assert!(opens(&runtime).is_empty(), "a window took it");
}

#[test]
fn desk_is_refused_a_diff_another_action_a_path_outside_every_project_and_a_missing_file() {
    let (mut runtime, other) = desk_runtime("desk-open-refused");
    let file = other.join("a.rs");
    std::fs::write(&file, "a\n").expect("fixture");
    let reason = |result: Result<ActionResult, Refusal>| result.unwrap_err().reason;

    assert_eq!(
        reason(desk_action(
            &mut runtime,
            "diff",
            Action::OpenDiff {
                path: file.to_string_lossy().into_owned()
            }
        )),
        "desk_diff_unsupported"
    );
    assert_eq!(
        reason(desk_action(
            &mut runtime,
            "select",
            Action::Select {
                view_id: "v".into(),
                expected_browser_area: None
            }
        )),
        "desk_has_no_checkout"
    );
    let outside = other.with_file_name("desk-open-refused-outside");
    std::fs::create_dir_all(&outside).unwrap();
    let stray = outside.join("notes.md");
    std::fs::write(&stray, "x\n").unwrap();
    assert_eq!(
        reason(open_file(&mut runtime, "outside", &stray)),
        "desk_file_outside_project"
    );
    // A sibling folder whose name only starts with the checkout's is outside it.
    let sibling = PathBuf::from(format!("{}x/a.rs", other.to_string_lossy()));
    assert_eq!(
        reason(open_file(&mut runtime, "sibling", &sibling)),
        "desk_file_outside_project"
    );
    assert_eq!(
        reason(open_file(&mut runtime, "missing", &other.join("gone.md"))),
        "file_missing"
    );
    assert!(opens(&runtime).is_empty(), "nothing refused waits");
}

#[test]
fn desk_opens_nothing_while_its_process_is_not_running() {
    let (mut runtime, other) = desk_runtime("desk-open-stopped");
    let file = other.join("a.md");
    std::fs::write(&file, "a\n").expect("fixture");
    runtime.close_test_desk();

    assert_eq!(
        open_file(&mut runtime, "stopped", &file)
            .unwrap_err()
            .reason,
        "desk_not_running"
    );
}

#[test]
fn past_sixteen_waiting_opens_the_oldest_is_dropped_and_turning_desk_off_drops_the_rest() {
    let (mut runtime, other) = desk_runtime("desk-open-cap");
    let files: Vec<PathBuf> = (0..17)
        .map(|n| {
            let file = other.join(format!("f{n}.md"));
            std::fs::write(&file, "x\n").expect("fixture");
            file
        })
        .collect();
    for (n, file) in files.iter().enumerate() {
        open_file(&mut runtime, &format!("cap-{n}"), file).unwrap();
    }

    let waiting: Vec<String> = opens(&runtime).into_iter().map(|open| open.path).collect();
    assert_eq!(waiting.len(), 16);
    assert_eq!(waiting[0], files[1].to_string_lossy(), "the oldest went");
    assert_eq!(waiting[15], files[16].to_string_lossy());

    runtime.dispatch_json(&explorer_event(
        "desk_settings_set",
        serde_json::json!({ "enabled": false }),
    ));
    assert!(
        opens(&runtime).is_empty(),
        "Desk switched off keeps no open"
    );
}
