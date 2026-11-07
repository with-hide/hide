//! with-hide/hide-private#646: a node replaces a shell pane's holder that
//! runs another build's version folder with its own, under the same pane id,
//! and keeps a holder that is its own build's or is not the start it was
//! asked about.
use hide_kit::panes::CloseError;
use hide_node::terminal::holder::ReplaceRefusal;
use hide_node_link::holders::HolderIntent;

use super::*;

/// Names `version_folder` in the pane's record, which is all a node reads
/// of the build its holder runs, and answers the one it named before.
fn run_from(folder: &Folder, pane: &str, version_folder: &str) -> String {
    let _lock = record::lock(&layout::pane_records_lock(&folder.state), record::LOCK_WAIT)
        .unwrap()
        .unwrap();
    let mut record = folder.record(pane);
    let before = std::mem::replace(&mut record.version_folder, version_folder.to_owned());
    record::write(&layout::pane_record(&folder.state, pane), &record).unwrap();
    before
}

const EARLIER: &str = "v1-an-earlier-build";

/// Settles a request as the engine settles each one it made.
fn retire(node: &Node, request: hide_kit::panes::Request<'_>) {
    let number = request.number();
    drop(request);
    assert!(
        node.owner()
            .retire(node.owner().incarnation(), number)
            .unwrap(),
        "request {number} still has workers"
    );
}

#[test]
fn a_shell_on_another_builds_holder_is_replaced_under_its_id_and_a_kept_one_runs_on() {
    let folder = Folder::new();
    let mut node = Node::start(folder.home.path(), &folder.state);
    let pane = "p1";
    let intent = HolderIntent {
        pane: pane.to_owned(),
        digest: "a".repeat(64),
        cwd: hide_platform::path::to_wire(folder.home.path()).unwrap(),
    };
    let first = node.owner().issue(pane, &intent.digest).unwrap();
    let reserved = node.holders().reserve(&first, &intent, None).unwrap();
    node.holders()
        .start_reserved(&first, &node.service, &intent, reserved, || {
            Ok(folder.shell(pane))
        })
        .unwrap();
    node.screens.first_full(pane);
    // While the pane's own start is unresolved, a replacement would be
    // refused only after the holder ended: it keeps the holder instead.
    let this_build = run_from(&folder, pane, EARLIER);
    let early = node.owner().issue(pane, &intent.digest).unwrap();
    let refused = node
        .holders()
        .replace(
            node.owner(),
            &early,
            &node.service,
            &intent,
            folder.record(pane).incarnation,
        )
        .unwrap_err();
    assert!(
        matches!(refused, ReplaceRefusal::Kept(CloseError::Kept(_))),
        "{refused:?}"
    );
    retire(&node, early);
    retire(&node, first);
    run_from(&folder, pane, &this_build);
    node.type_into(pane, &print_node("before"));
    wait_until("the first shell's answer", || {
        node.text(pane).contains("before-node")
    });
    let started = folder.record(pane);
    let old = started.holder.unwrap();

    // Its own build's holder is not outdated, and a replacement keeps it.
    assert_eq!(node.holders().outdated(pane).unwrap(), None);
    let kept = node.owner().issue(pane, &intent.digest).unwrap();
    let refused = node
        .holders()
        .replace(node.owner(), &kept, &node.service, &intent, started.incarnation)
        .unwrap_err();
    assert!(
        matches!(refused, ReplaceRefusal::Kept(CloseError::Kept(_))),
        "{refused:?}"
    );
    retire(&node, kept);
    assert!(old.runs(), "a holder of this build was ended");
    assert_eq!(folder.record(pane), started, "a kept holder's record moved");

    run_from(&folder, pane, EARLIER);
    assert_eq!(
        node.holders().outdated(pane).unwrap(),
        Some(started.incarnation)
    );
    // A replacement that names another start than the one running keeps it.
    let stale = node.owner().issue(pane, &intent.digest).unwrap();
    let refused = node
        .holders()
        .replace(
            node.owner(),
            &stale,
            &node.service,
            &intent,
            started.incarnation + 1,
        )
        .unwrap_err();
    assert!(matches!(refused, ReplaceRefusal::Kept(_)), "{refused:?}");
    retire(&node, stale);
    assert!(old.runs(), "a holder was ended for another start's replacement");
    node.type_into(pane, &print_node("still"));
    wait_until("the kept shell's answer", || {
        node.text(pane).contains("still-node")
    });

    let frames = node.screens.count(pane);
    let replacing = node.owner().issue(pane, &intent.digest).unwrap();
    let reserved = node
        .holders()
        .replace(
            node.owner(),
            &replacing,
            &node.service,
            &intent,
            started.incarnation,
        )
        .unwrap();
    assert!(!old.runs(), "the earlier build's holder runs on");
    assert_ne!(reserved.incarnation, started.incarnation);
    node.holders()
        .start_reserved(&replacing, &node.service, &intent, reserved, || {
            Ok(folder.shell(pane))
        })
        .unwrap();
    retire(&node, replacing);
    let now = folder.record(pane);
    assert_eq!(now.pane, started.pane);
    assert_eq!(now.phase, Phase::Running);
    assert_eq!(now.incarnation, reserved.incarnation);
    assert_eq!(now.version_folder, started.version_folder, "not this build's");
    assert_eq!(node.holders().outdated(pane).unwrap(), None);
    let new = now.holder.unwrap();
    assert!(new.runs() && new != old);
    // The new holder's screen reaches the same pane, and so does input.
    node.type_into(pane, &print_node("after"));
    node.screens.wait_drawn_after(pane, frames, "after-node");
    assert!(
        !node.text(pane).contains("before-node"),
        "the replacement kept the old holder's screen"
    );

    // The next owner finds one running pane, never one that closed or ended.
    drop(node.owner.take());
    node.owner = Some(node.holders().acquire(Duration::from_secs(5)).unwrap());
    node.holders().recover(node.owner(), &node.service);
    let recovered = node.holders().recovered(Duration::from_secs(5)).unwrap();
    assert_eq!(recovered.present, vec![pane.to_owned()]);
    assert!(recovered.closed.is_empty(), "{recovered:?}");
    assert!(recovered.ended.is_empty(), "{recovered:?}");
    node.service.close();
}
