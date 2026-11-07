//! A pane an earlier build's holder runs, brought to this build's holder
//! under its own id when nothing but its shell runs in it
//! (with-hide/hide-private#646), through the node calls the core's own engine
//! makes over its link (`LinkPanes`, the same calls a device's node
//! answers): the shell is started again from this build's version folder,
//! the earlier start's incarnation replaced, its screen attached again and
//! its facts followed, the operator's keys typed meanwhile refused and said
//! to be, and the pane neither closes nor runs twice; a pane already on this build is
//! left alone; a pane whose shell runs anything, in its terminal or behind
//! it, is kept on its holder until a later renewal finds it alone; and a
//! renewal that ended the earlier holder and could not start the next one
//! says no holder of it runs the pane.

use super::ownership::Connection;
use super::*;
use hide_kit::panes::holder_pane;
use hide_node_link::holders::{HolderRenewal, PaneInput, RENEWAL_LOST, RENEWAL_UNSETTLED};

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
    let told = Arc::new(Told::default());
    next.linked.watch_facts(told.sink());
    answers(&next, "the earlier holder's shell answers");

    // The operator types into the pane all through its renewal: a key the
    // pane cannot take while its holder is judged and replaced is refused,
    // and the refusal is said, never dropped unsaid.
    let (done, typing) = channel::<()>();
    let service = &next.service;
    let renewed = thread::scope(|scope| {
        scope.spawn(move || {
            // A key every few milliseconds until the renewal answers.
            while let Err(std::sync::mpsc::RecvTimeoutError::Timeout) =
                typing.recv_timeout(Duration::from_millis(2))
            {
                service.key(KeyTarget::Pane(PANE.to_owned()), b"x".to_vec(), 0);
            }
        });
        let renewed = next.linked.renew(&shell_start(&folder));
        done.send(()).unwrap();
        renewed
    })
    .unwrap();
    let errors: Vec<(String, String)> = next
        .reports
        .told
        .lock()
        .unwrap()
        .iter()
        .filter_map(|report| match report {
            TerminalReport::Error { pane, kind, .. } if pane == PANE => {
                Some((kind.clone(), format!("{report:?}")))
            }
            _ => None,
        })
        .collect();
    assert!(
        errors
            .iter()
            .any(|(kind, _)| kind == "terminal.close_pending"),
        "keys typed during the renewal were refused unsaid: {errors:?}"
    );
    assert!(
        errors
            .iter()
            .all(|(kind, _)| kind == "terminal.close_pending"),
        "{errors:?}"
    );
    let clear_line: &[u8] = if cfg!(windows) { b"\x1b" } else { b"\x15" };
    next.linked
        .input(PANE, PaneInput::Bytes(clear_line.to_vec()))
        .unwrap();
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
    // The same pane, on the new holder, takes what is typed into it, and
    // its facts are followed from the new holder.
    answers(&next, "the renewed pane's shell answers");
    let titled: &[u8] = if cfg!(windows) {
        b"title renewed\r"
    } else {
        b"printf '\\033]0;renewed\\007'\r"
    };
    next.linked
        .input(PANE, PaneInput::Bytes(titled.to_vec()))
        .unwrap();
    told.wait(PANE, "the renewed holder's title", |facts| {
        facts.title.as_deref() == Some("renewed")
    });

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
        let kept = broken.linked.renew(&shell_start(&folder));
        assert!(
            matches!(&kept, Ok(HolderRenewal::Kept { code, .. }) if code == "holder_folder_unread"),
            "{kept:?}"
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
    let kept = next.linked.renew(&shell_start(&folder));
    assert!(
        matches!(&kept, Ok(HolderRenewal::Kept { code, .. }) if code == "pane_busy"),
        "{kept:?}"
    );
    assert_eq!(pane_record(&folder), before);
    assert!(earlier_holder.runs());
    // The program is still the pane's: interrupted, its shell answers.
    next.linked
        .input(PANE, PaneInput::Bytes(vec![0x03]))
        .unwrap();
    unchanged(&next, "a busy pane renews nothing");
}

/// A job the shell runs behind its prompt would end with its holder, so the
/// pane is kept on it; once the job is over, the next renewal, as the next
/// wake asks it, finds the shell alone and renews it.
#[test]
fn a_shell_with_a_background_job_is_kept_until_a_later_renewal_finds_it_alone() {
    let folder = Folder::new();
    let before = started_by_earlier_build(&folder);
    let earlier_holder = before.holder.unwrap();
    let next = Connection::open(folder.home.path(), &folder.state);
    next.linked.recovered(HANG_LIMIT).unwrap();
    answers(&next, "the earlier holder's shell answers");
    let shell = next.linked.processes(PANE, HANG_LIMIT).unwrap().shell;
    assert!(hide_platform::process::runs_alone(shell).unwrap());
    let alone = hide_platform::process::descendants(shell).unwrap();

    let job: &[u8] = if cfg!(windows) {
        b"start /b ping -n 120 127.0.0.1 >nul\r"
    } else {
        b"sleep 120 &\r"
    };
    next.linked
        .input(PANE, PaneInput::Bytes(job.to_vec()))
        .unwrap();
    wait_until("the job runs behind the prompt", || {
        !hide_platform::process::runs_alone(shell).unwrap()
    });
    let jobs: Vec<u32> = hide_platform::process::descendants(shell)
        .unwrap()
        .into_iter()
        .filter(|pid| !alone.contains(pid))
        .collect();
    assert!(!jobs.is_empty());

    let kept = next.linked.renew(&shell_start(&folder));
    assert!(
        matches!(&kept, Ok(HolderRenewal::Kept { code, .. }) if code == "pane_busy"),
        "{kept:?}"
    );
    assert_eq!(pane_record(&folder), before);
    assert!(earlier_holder.runs());
    assert!(
        jobs.iter()
            .all(|pid| hide_platform::process::descendants(shell)
                .unwrap()
                .contains(pid)),
        "the job runs on"
    );

    for pid in jobs {
        hide_platform::process::kill_tree(pid).unwrap();
    }
    wait_until("the shell runs alone again", || {
        hide_platform::process::runs_alone(shell).unwrap()
    });
    let renewed = next.linked.renew(&shell_start(&folder));
    assert!(
        matches!(&renewed, Ok(HolderRenewal::Renewed { from, .. }) if *from == before.version_folder),
        "{renewed:?}"
    );
    wait_until("the earlier holder ends", || !earlier_holder.runs());
    answers(&next, "the renewed pane's shell answers");
}

/// This build's holder program is there but does not run: the earlier
/// holder was ended before the new one failed, so the renewal says no
/// holder of it runs the pane, never that the earlier one was kept. The
/// pane's store says whether the failed start may have left anything
/// (unsettled, as a holder that ends before it says why does) or not
/// (lost); the core uses either pane as one a restore could not start.
#[test]
fn a_renewal_whose_new_holder_does_not_start_says_no_holder_runs_the_pane() {
    let folder = Folder::new();
    let before = started_by_earlier_build(&folder);
    let earlier_holder = before.holder.unwrap();
    let failing = failing_build(&folder);
    let broken = Connection::of_build(folder.home.path(), &folder.state, failing);
    broken.linked.recovered(HANG_LIMIT).unwrap();
    answers(&broken, "the earlier holder's shell answers");

    let lost = broken.linked.renew(&shell_start(&folder));
    assert!(
        matches!(&lost, Err(HolderError::Refused { code, .. })
            if code == RENEWAL_LOST || code == RENEWAL_UNSETTLED),
        "{lost:?}"
    );
    wait_until("the earlier holder ends", || !earlier_holder.runs());
    let after = pane_record(&folder);
    assert!(
        after.holder.is_none_or(|holder| !holder.runs()),
        "no holder runs the pane: {after:?}"
    );
}

/// A holder program of its own version folder that exits at once.
fn failing_build(folder: &Folder) -> hide_kit::holders::Source {
    let dir = folder.home.path().join("failing");
    std::fs::create_dir_all(&dir).unwrap();
    let program = dir.join(hide_kit::holders::program_name());
    #[cfg(windows)]
    {
        let system = std::env::var_os("SystemRoot").expect("SystemRoot names Windows");
        std::fs::copy(
            Path::new(&system).join("System32").join("whoami.exe"),
            &program,
        )
        .unwrap();
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(&program, "#!/bin/sh\nexit 3\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    hide_kit::holders::Source {
        program,
        conpty: source().conpty,
    }
}

/// A pane closed while its holder renews, and its end asked again once the
/// renewal answered, as the core asks it of a pane closed meanwhile: however
/// the close and the renewal met, no holder runs the pane afterwards.
#[test]
fn a_pane_closed_while_its_holder_renews_is_left_with_no_holder() {
    let folder = Folder::new();
    let before = started_by_earlier_build(&folder);
    let earlier_holder = before.holder.unwrap();
    let next = Connection::open(folder.home.path(), &folder.state);
    next.linked.recovered(HANG_LIMIT).unwrap();
    answers(&next, "the earlier holder's shell answers");

    thread::scope(|scope| {
        let renewal = scope.spawn(|| next.linked.renew(&shell_start(&folder)));
        let _ = next.linked.terminate(PANE);
        let _ = renewal.join().unwrap();
    });
    let _ = next.linked.terminate(PANE);
    wait_until("the earlier holder ends", || !earlier_holder.runs());
    wait_until("no holder runs the pane", || {
        record::read(&layout::pane_record(&folder.state, &holder_pane(PANE)))
            .unwrap()
            .is_none_or(|record| record.holder.is_none_or(|holder| !holder.runs()))
    });
}
