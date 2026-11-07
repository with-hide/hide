//! A pane an earlier build's holder runs, brought to this build's holder
//! under its own id when only its shell is left in it (with-hide/hide-private#646),
//! through the node calls the core's own engine makes over its link
//! (`LinkPanes`, the same calls a device's node answers): the shell is
//! started again from this build's version folder, the earlier start's
//! incarnation replaced, its screen attached again, and the pane neither
//! closes nor runs twice; a pane already on this build is left alone; and a
//! renewal that cannot happen leaves the earlier holder running as it was.

use super::ownership::Connection;
use super::*;
use hide_kit::panes::holder_pane;
use hide_node_link::holders::{HolderRenewal, PaneInput};

const PANE: &str = "w1:p1";

/// A copy of this build's holder that hashes to a version folder of its
/// own, as an earlier build's holder does: the same program with bytes
/// after its end, which it never reads.
fn earlier_build(folder: &Folder) -> hide_kit::holders::Source {
    let built = source();
    let dir = folder.home.path().join("earlier");
    std::fs::create_dir_all(&dir).unwrap();
    let program = dir.join(hide_kit::holders::program_name());
    std::fs::copy(&built.program, &program).unwrap();
    let mut bytes = std::fs::read(&program).unwrap();
    bytes.extend_from_slice(b"an earlier build");
    std::fs::write(&program, bytes).unwrap();
    hide_kit::holders::Source {
        program,
        conpty: built.conpty,
    }
}

fn shell_start(folder: &Folder) -> HolderStart {
    HolderStart {
        pane: PANE.to_owned(),
        program: None,
        cwd: hide_platform::path::to_wire(folder.home.path()).unwrap(),
        env: Vec::new(),
        rows: SIZE.rows,
        cols: SIZE.cols,
        notice: None,
    }
}

fn pane_record(folder: &Folder) -> PaneRecord {
    folder.record(&holder_pane(PANE))
}

/// `PANE` started by a node of the earlier build, which then ends and
/// leaves it running, as an app update leaves a pane: its record.
fn started_by_earlier_build(folder: &Folder) -> PaneRecord {
    let earlier = Connection::of_build(folder.home.path(), &folder.state, earlier_build(folder));
    earlier.linked.recovered(HANG_LIMIT).unwrap();
    start_pane(&earlier.linked, &shell_start(folder)).unwrap();
    drop(earlier);
    pane_record(folder)
}

/// Waits until the pane's shell answers a line typed into it.
fn answers(connection: &Connection, what: &str) {
    connection
        .linked
        .input(PANE, PaneInput::Bytes(print_ok().to_vec()))
        .unwrap();
    wait_until(what, || {
        connection
            .linked
            .read(PANE, 200)
            .is_ok_and(|(text, _)| text.contains("hide-ok"))
    });
}

#[test]
fn a_shell_on_an_earlier_builds_holder_is_renewed_under_its_id_and_a_current_one_is_left() {
    let folder = Folder::new();
    let before = started_by_earlier_build(&folder);
    let earlier_holder = before.holder.unwrap();

    let next = Connection::open(folder.home.path(), &folder.state);
    let found = next.linked.recovered(HANG_LIMIT).unwrap();
    assert!(found.present.contains(&PANE.to_owned()), "{found:?}");
    answers(&next, "the earlier holder's shell answers");

    let renewed = next.linked.renew(&shell_start(&folder)).unwrap();
    let after = pane_record(&folder);
    let current =
        hide_kit::holders::version_name(hide_holder_protocol::PROTOCOL_VERSION, &source()).unwrap();
    assert_eq!(
        renewed,
        HolderRenewal::Renewed {
            from: before.version_folder.clone(),
            to: current.clone(),
        }
    );
    assert_ne!(before.version_folder, current);
    assert_eq!(after.version_folder, current);
    assert_eq!(after.phase, Phase::Running);
    assert!(after.incarnation > before.incarnation);
    let holder = after.holder.unwrap();
    assert_ne!(holder, earlier_holder);
    assert!(holder.runs());
    wait_until("the earlier holder ends", || !earlier_holder.runs());
    // The same pane, on the new holder, takes what is typed into it.
    answers(&next, "the renewed pane's shell answers");

    // Asked again, a pane on this build is left as it runs.
    assert_eq!(
        next.linked.renew(&shell_start(&folder)).unwrap(),
        HolderRenewal::Current
    );
    assert_eq!(pane_record(&folder), after);
    assert!(holder.runs());

    // The next node finds the one pane running, never closed or to be
    // started again.
    drop(next);
    let last = Connection::open(folder.home.path(), &folder.state);
    let found = last.linked.recovered(HANG_LIMIT).unwrap();
    assert_eq!(found.present, vec![PANE.to_owned()], "{found:?}");
    assert!(
        found.closed.is_empty() && found.ended.is_empty(),
        "{found:?}"
    );
    assert_eq!(pane_record(&folder).holder, Some(holder));
}

#[test]
fn a_renewal_that_cannot_happen_leaves_the_earlier_holder_running() {
    let folder = Folder::new();
    let before = started_by_earlier_build(&folder);
    let earlier_holder = before.holder.unwrap();
    let unchanged = |connection: &Connection, why: &str| {
        assert_eq!(pane_record(&folder), before, "{why}");
        assert!(earlier_holder.runs(), "{why}");
        answers(connection, why);
    };

    // This build's holder cannot be found (an update left it half there):
    // nothing is ended.
    let missing = hide_kit::holders::Source {
        program: folder.home.path().join("gone").join("hide-holder"),
        conpty: source().conpty,
    };
    {
        let broken = Connection::of_build(folder.home.path(), &folder.state, missing);
        broken.linked.recovered(HANG_LIMIT).unwrap();
        let refused = broken.linked.renew(&shell_start(&folder));
        assert!(
            matches!(&refused, Err(HolderError::Refused { code, .. }) if code == "holder_folder_unread"),
            "{refused:?}"
        );
        unchanged(&broken, "a build with no holder renews nothing");
    }

    // A program other than the shell holds the terminal: it is not ended.
    let next = Connection::open(folder.home.path(), &folder.state);
    next.linked.recovered(HANG_LIMIT).unwrap();
    let waits: &[u8] = if cfg!(windows) {
        b"ping -n 120 127.0.0.1 >nul\r"
    } else {
        b"sleep 120\r"
    };
    next.linked
        .input(PANE, PaneInput::Bytes(waits.to_vec()))
        .unwrap();
    wait_until("the waiting program holds the terminal", || {
        next.linked
            .processes(PANE, HANG_LIMIT)
            .is_ok_and(|processes| {
                processes
                    .foreground
                    .is_some_and(|foreground| foreground.group != processes.shell)
            })
    });
    let refused = next.linked.renew(&shell_start(&folder));
    assert!(
        matches!(&refused, Err(HolderError::Refused { code, .. }) if code == "pane_busy"),
        "{refused:?}"
    );
    assert_eq!(pane_record(&folder), before);
    assert!(earlier_holder.runs());
    // The program is still the pane's: interrupted, its shell answers.
    next.linked
        .input(PANE, PaneInput::Bytes(vec![0x03]))
        .unwrap();
    unchanged(&next, "a busy pane renews nothing");
}
