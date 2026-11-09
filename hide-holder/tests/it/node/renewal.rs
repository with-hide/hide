//! A pane an earlier build's holder runs, brought to this build's holder
//! under its own id when its agent is asleep and nothing but its shell runs
//! in it (with-hide/hide-private#646), through the node calls the core's own
//! engine makes over its link (`LinkPanes`, the same calls a device's node
//! answers): the shell is started again from this build's version folder,
//! the earlier start's incarnation replaced, its screen attached again and
//! its facts followed, and the pane neither closes nor runs twice; a pane
//! already on this build is left alone; a pane whose agent is awake, whose
//! shell runs anything (in its terminal or behind it), or whose writes to
//! its holder do not settle is kept on its holder untouched; a renewal that
//! ended the earlier holder and could not start the next one says no holder
//! of it runs the pane; and a close asked during a renewal ends the holder
//! the renewal leaves.

use super::ownership::Connection;
use super::*;
use hide_kit::panes::holder_pane;
#[cfg(unix)]
use hide_node_link::holders::RENEWAL_PANE_CLOSED;
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

/// Tells the connection's node that `PANE`'s agent is asleep or awake, as
/// the core does from its sleep record: asleep, the pane takes no input but
/// the sleep's own.
fn asleep(connection: &Connection, asleep: bool) {
    connection.service.control(TerminalControl::Asleep {
        pane: PANE.to_owned(),
        asleep,
    });
}

/// What the renewal judges by: the pane's holder and shell as its record
/// names them.
fn alone(folder: &Folder) -> bool {
    let record = pane_record(folder);
    let recorded = |identity: Identity| hide_platform::process::Recorded {
        pid: identity.pid,
        started: identity.started,
    };
    hide_platform::process::runs_alone(
        recorded(record.holder.unwrap()),
        recorded(record.program.unwrap()),
    )
    .unwrap()
}

/// Uses the same connection request sequence as pane creation, as a wake
/// does; an unsettled renewal keeps its request until the connection ends.
fn renew_pane(
    holders: &dyn PaneHolders,
    start: &HolderStart,
) -> Result<HolderRenewal, HolderError> {
    let request = holders.request()?;
    let result = holders.renew(start, request);
    if !matches!(&result, Err(HolderError::Unreachable { .. }))
        && !matches!(&result, Err(HolderError::Refused { code, .. }) if code == RENEWAL_UNSETTLED)
    {
        holders.retire(request)?;
    }
    result
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

    // A pane whose agent is awake takes input, so it is never renewed.
    let awake = renew_pane(&next.linked, &shell_start(&folder));
    assert!(
        matches!(&awake, Ok(HolderRenewal::Kept { code, .. }) if code == "pane_awake"),
        "{awake:?}"
    );
    assert_eq!(pane_record(&folder), before);

    // Asleep, the operator's keys typed all through the renewal are the
    // sleeping pane's, dropped as at any sleeping pane, and none is refused
    // as at a closing one.
    asleep(&next, true);
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
        let renewed = renew_pane(&next.linked, &shell_start(&folder));
        done.send(()).unwrap();
        renewed
    })
    .unwrap();
    let errors: Vec<String> = next
        .reports
        .told
        .lock()
        .unwrap()
        .iter()
        .filter(|report| matches!(report, TerminalReport::Error { pane, .. } if pane == PANE))
        .map(|report| format!("{report:?}"))
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    asleep(&next, false);
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
    // An elevated cmd puts "Administrator: " before the title it is given.
    told.wait(PANE, "the renewed holder's title", |facts| {
        facts
            .title
            .as_deref()
            .is_some_and(|title| title.ends_with("renewed"))
    });

    // A wake that needs a new tab after renewing another sleeping pane
    // can still start it: renewal must not consume a node-only number that
    // this connection's next creation would reuse.
    let mut new_tab = shell_start(&folder);
    new_tab.pane = "w2:p1".to_owned();
    start_pane(&next.linked, &new_tab).expect("a new tab starts after renewal");
    next.linked
        .input(&new_tab.pane, PaneInput::Bytes(print_ok().to_vec()))
        .unwrap();
    wait_until("the tab created after renewal answers", || {
        next.linked
            .read(&new_tab.pane, 200)
            .is_ok_and(|(text, _)| text.contains("hide-ok"))
    });

    // Asked again, a pane on this build is left as it runs.
    assert_eq!(
        renew_pane(&next.linked, &shell_start(&folder)).unwrap(),
        HolderRenewal::Current
    );
    assert_eq!(pane_record(&folder), after);
    assert!(holder.runs());

    // The next node finds both panes running, never closed or to be
    // started again.
    drop(next);
    let last = Connection::open(folder.home.path(), &folder.state);
    let found = last.linked.recovered(HANG_LIMIT).unwrap();
    let mut present = found.present.clone();
    present.sort();
    assert_eq!(present, vec![PANE.to_owned(), new_tab.pane], "{found:?}");
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
        let kept = renew_pane(&broken.linked, &shell_start(&folder));
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
    asleep(&next, true);
    let kept = renew_pane(&next.linked, &shell_start(&folder));
    asleep(&next, false);
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
    assert!(alone(&folder));
    let before_job = hide_platform::process::descendants(shell).unwrap();

    let job: &[u8] = if cfg!(windows) {
        b"start /b ping -n 120 127.0.0.1 >nul\r"
    } else {
        b"sleep 120 &\r"
    };
    next.linked
        .input(PANE, PaneInput::Bytes(job.to_vec()))
        .unwrap();
    wait_until("the job runs behind the prompt", || !alone(&folder));
    let jobs: Vec<u32> = hide_platform::process::descendants(shell)
        .unwrap()
        .into_iter()
        .filter(|pid| !before_job.contains(pid))
        .collect();
    assert!(!jobs.is_empty());

    asleep(&next, true);
    let kept = renew_pane(&next.linked, &shell_start(&folder));
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
    wait_until("the shell runs alone again", || alone(&folder));
    let renewed = renew_pane(&next.linked, &shell_start(&folder));
    asleep(&next, false);
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

    asleep(&broken, true);
    let lost = renew_pane(&broken.linked, &shell_start(&folder));
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

/// A close asked while the pane's holder renews waits for the renewal and
/// ends the holder it left, so no holder runs a closed pane, whether or not
/// the renewal's answer reached anyone; a renewal asked after the close
/// finds the pane closed and says so, never that a holder runs it.
#[cfg(unix)]
#[test]
fn a_close_during_a_renewal_ends_the_holder_the_renewal_leaves() {
    let folder = Folder::new();
    let before = started_by_earlier_build(&folder);
    let earlier_holder = before.holder.unwrap();
    let next = Connection::open(folder.home.path(), &folder.state);
    next.linked.recovered(HANG_LIMIT).unwrap();
    answers(&next, "the earlier holder's shell answers");
    // A shell that ignores the hangup and the terminate makes its holder's
    // end take until the kill, so the close comes while the renewal has
    // the pane's turn.
    next.linked
        .input(PANE, PaneInput::Bytes(b"trap '' HUP TERM\r".to_vec()))
        .unwrap();
    answers(&next, "the shell set its traps");
    asleep(&next, true);

    let renewed = thread::scope(|scope| {
        let renewal = scope.spawn(|| renew_pane(&next.linked, &shell_start(&folder)));
        wait_until("the renewal has the pane's turn", || {
            next.holders.changing(PANE) || renewal.is_finished()
        });
        next.linked.terminate(PANE).unwrap();
        renewal.join().unwrap()
    });
    assert!(
        matches!(&renewed, Ok(HolderRenewal::Renewed { .. })),
        "{renewed:?}"
    );
    wait_until("the earlier holder ends", || !earlier_holder.runs());
    let closed = record::read(&layout::pane_record(&folder.state, &holder_pane(PANE))).unwrap();
    assert!(
        closed
            .as_ref()
            .is_none_or(|record| record.holder.is_none_or(|holder| !holder.runs())),
        "no holder runs the closed pane: {closed:?}"
    );
    let after = renew_pane(&next.linked, &shell_start(&folder));
    assert!(
        matches!(&after, Err(HolderError::Refused { code, .. }) if code == RENEWAL_PANE_CLOSED),
        "{after:?}"
    );
}

/// The writes handed to the pane's holder before its agent slept are
/// waited for before anything is judged, a released session's included,
/// whose writer goes on writing what it was handed; the core's input that
/// comes beside them never reaches the asleep pane, and writes that do not
/// settle leave the earlier holder untouched. The holder is stopped, so its
/// connection stops taking what the node writes.
#[cfg(unix)]
fn keeps_a_pane_whose_writes_do_not_settle(released: bool) {
    let folder = Folder::new();
    let before = started_by_earlier_build(&folder);
    let earlier_holder = before.holder.unwrap();
    let next = Connection::open(folder.home.path(), &folder.state);
    next.linked.recovered(HANG_LIMIT).unwrap();
    answers(&next, "the earlier holder's shell answers");
    let pid = i32::try_from(earlier_holder.pid).unwrap();
    // SAFETY: a signal to the test's own holder process.
    assert_eq!(unsafe { libc::kill(pid, libc::SIGSTOP) }, 0);
    struct Resume(i32);
    impl Drop for Resume {
        fn drop(&mut self) {
            // SAFETY: as above.
            unsafe { libc::kill(self.0, libc::SIGCONT) };
        }
    }
    let _resume = Resume(pid);
    // The node's whole unwritten input cap, none of it refused: more than
    // the holder's connection takes on every system (about 200 KiB on Linux,
    // 8 KiB on macOS), so the writer stops on it with bytes left, and the
    // renewal's barrier, which a backlog past the cap would refuse at once,
    // waits behind them.
    let piece = hide_node_link::terminal::MAX_UNSENT_KEY_BYTES / 8;
    for _ in 0..8 {
        next.service
            .key(KeyTarget::Pane(PANE.to_owned()), vec![b' '; piece], 0);
    }
    thread::scope(|scope| {
        // The holder is stopped, so the input can never be written: it is
        // either cancelled where it waits or refused at the asleep pane,
        // and both are the caller's error.
        let typed = scope.spawn(|| next.linked.input(PANE, PaneInput::Bytes(b"ls\r".to_vec())));
        asleep(&next, true);
        let cancelled = typed.join().unwrap();
        assert!(cancelled.is_err(), "{cancelled:?}");
    });
    if released {
        // The pane's tab left the last shown: its session goes, and its
        // writer still holds the keys.
        next.service.control(TerminalControl::Release {
            pane: PANE.to_owned(),
            message: "released".to_owned(),
        });
    }
    let started = Instant::now();
    let kept = renew_pane(&next.linked, &shell_start(&folder));
    assert!(
        matches!(&kept, Ok(HolderRenewal::Kept { code, .. }) if code == "pane_input_unsettled"),
        "{kept:?}"
    );
    if !released {
        assert!(started.elapsed() >= Duration::from_secs(4), "it waited");
    }
    // A released session's writer is let go a second after its release, its
    // keys not written: the renewal is told that, or waits it out.
    assert_eq!(pane_record(&folder), before);
    assert!(earlier_holder.runs());
}

#[cfg(unix)]
#[test]
fn a_renewal_waits_for_the_writes_before_it_and_keeps_a_pane_whose_writes_do_not_settle() {
    keeps_a_pane_whose_writes_do_not_settle(false);
}

#[cfg(unix)]
#[test]
fn a_released_sessions_unwritten_keys_keep_the_pane_on_its_holder() {
    keeps_a_pane_whose_writes_do_not_settle(true);
}
