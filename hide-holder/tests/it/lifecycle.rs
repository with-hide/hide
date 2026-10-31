//! A holder's life as a node observes it: started from a version folder as a
//! resident child, its shell's input and output, a release and a second
//! connection, and the end of its pane, asked for or on its own, which leaves
//! nothing of the pane running (PRD engine-bundle-2-pane-holder B11, B12,
//! B18, B19).

use std::ffi::OsString;

use crate::role::EXIT_EARLY;
use crate::support::{
    Connection, Home, PIDS_FILE, ROLE, Running, Start, current_version, pids_in, print_ok,
    role_program, shell, source, start_ready, start_with,
};
use hide_holder_protocol::{ClientMessage, CloseReason, Size};

#[test]
fn a_holder_runs_its_shell_and_carries_input_and_output_across_connections() {
    let home = Home::new();
    let folder = home.prepare(current_version(), &source());
    let socket = home.socket("p");
    let holder = start_ready(&folder.program, &socket, &shell(), &[]);
    drop(folder);

    // The job that holds the holder, said for its starter to record: none
    // on Unix; on Windows a job above the test's may hold it.
    if cfg!(unix) {
        assert_eq!(holder.job, "none");
    } else {
        assert!(
            holder.job == "none" || holder.job.contains("kill_on_close="),
            "{}",
            holder.job
        );
    }

    let mut first = holder.open();
    assert_eq!(first.welcome.generation, 1);
    assert_eq!(first.welcome.program, holder.program.pid);
    assert_eq!(first.welcome.size, Size { rows: 24, cols: 80 });
    first
        .send(ClientMessage::Input(print_ok().to_vec()))
        .unwrap();
    first.wait_for_text("hide-ok");

    #[cfg(unix)]
    {
        first
            .send(ClientMessage::Resize(Size {
                rows: 40,
                cols: 100,
            }))
            .unwrap();
        first
            .send(ClientMessage::Input(b"stty size\r".to_vec()))
            .unwrap();
        first.wait_for_text("40 100");
    }

    // Letting go leaves the pane running for the next connection.
    first.send(ClientMessage::Release).unwrap();
    assert_eq!(first.wait_closed().reason, CloseReason::Released);
    assert!(holder.process.is_running() && holder.program.is_running());

    let mut second = holder.open();
    assert_eq!(second.welcome.generation, 2);
    second
        .send(ClientMessage::Input(print_ok().to_vec()))
        .unwrap();
    second.wait_for_text("hide-ok");

    second.send(ClientMessage::Terminate).unwrap();
    let closed = second.wait_closed();
    assert_eq!(closed.reason, CloseReason::Terminated, "{closed:?}");
    holder.wait_gone();
    assert!(!holder.program.is_running());
    assert!(
        !hide_platform::ipc::is_endpoint(&socket).unwrap_or(false),
        "the holder left its socket"
    );
}

#[test]
fn a_connection_that_drops_without_a_word_leaves_the_pane_running() {
    let home = Home::new();
    let folder = home.prepare(current_version(), &source());
    let holder = start_ready(&folder.program, &home.socket("p"), &shell(), &[]);
    drop(folder);
    let first = holder.open();
    drop(first);
    let mut second = holder.open();
    second
        .send(ClientMessage::Input(print_ok().to_vec()))
        .unwrap();
    second.wait_for_text("hide-ok");
    assert!(holder.process.is_running());
}

/// Each of `pids`, named while it runs.
fn running(pids: Vec<u32>) -> Vec<Running> {
    pids.into_iter()
        .map(|pid| Running::of(pid).unwrap_or_else(|| panic!("process {pid} ended early")))
        .collect()
}

fn assert_ended(processes: &[Running], owner: &str) {
    for process in processes {
        assert!(
            !process.is_running(),
            "process {} outlived {owner}",
            process.pid
        );
    }
}

fn stubborn_pane(
    home: &Home,
    environment: &[(&str, &OsString)],
) -> (crate::support::Holder, std::path::PathBuf) {
    let folder = home.prepare(current_version(), &source());
    let pids = home.path().join("pids");
    let role: OsString = "stubborn_tree".into();
    let pids_value: OsString = pids.clone().into();
    let mut variables: Vec<(&str, &OsString)> = vec![(ROLE, &role), (PIDS_FILE, &pids_value)];
    variables.extend_from_slice(environment);
    let holder = start_ready(
        &folder.program,
        &home.socket("p"),
        &role_program("stubborn_tree"),
        &variables,
    );
    drop(folder);
    (holder, pids)
}

#[test]
fn ending_a_pane_ends_what_ignores_its_signals_and_what_it_starts_meanwhile() {
    let home = Home::new();
    let (holder, pids) = stubborn_pane(&home, &[]);
    let mut connection = holder.open();
    connection.wait_for_text("TREE READY");
    // A Windows process is told nothing when its job is ended, so there the
    // child starts a chain of grandchildren at once, each starting the next;
    // the end is sent while the chain is a few processes long and still
    // growing, so processes start while the pane ends, as on Unix the
    // signals make them.
    #[cfg(windows)]
    pids_in(&pids, 8);
    let started = running(pids_in(&pids, 2));

    connection.send(ClientMessage::Terminate).unwrap();
    let closed = connection.wait_closed();
    assert_eq!(closed.reason, CloseReason::Terminated, "{closed:?}");
    holder.wait_gone();
    assert_ended(&started, "its pane");
    let all = pids_in(&pids, started.len());
    #[cfg(unix)]
    assert!(
        all.len() > started.len(),
        "the child started nothing as it was asked to end; the holder's log:\n{}",
        holder.log_text()
    );
    // Counted: every process the tree recorded, the ones named before the
    // end and every one started after them, ended with the pane.
    eprintln!(
        "the tree recorded {} processes, {} of them named before the end",
        all.len(),
        started.len()
    );
    for pid in all {
        assert!(
            Running::of(pid).is_none(),
            "process {pid} outlived its pane; the holder's log:\n{}",
            holder.log_text()
        );
    }
}

#[test]
fn a_program_that_exits_on_its_own_takes_what_it_left_with_it() {
    let home = Home::new();
    let early: OsString = "1".into();
    let (holder, pids) = stubborn_pane(&home, &[(EXIT_EARLY, &early)]);
    // The program may exit before a connection is made, or while one opens,
    // which is then told why; what is left of its tree is ended either way.
    let left: Vec<Running> = pids_in(&pids, 2)
        .into_iter()
        .filter_map(Running::of)
        .collect();
    match holder.try_open() {
        Ok((stream, welcome)) => {
            let closed = Connection::new(stream, welcome, &holder.socket).wait_closed();
            assert_eq!(closed.reason, CloseReason::ProgramExited, "{closed:?}");
            assert_eq!(closed.exit_code, Some(0), "{closed:?}");
        }
        Err(hide_holder_protocol::OpenError::Refused(closed)) => {
            assert_eq!(closed.reason, CloseReason::ProgramExited, "{closed:?}");
            assert_eq!(closed.exit_code, Some(0), "{closed:?}");
        }
        // Past the end the socket is gone.
        Err(error) => assert!(
            matches!(error, hide_holder_protocol::OpenError::Io(_)),
            "{error}"
        ),
    }
    holder.wait_gone();
    assert_ended(&left, "its program");
}

/// A connection that never reads while the program floods it gets no queue,
/// only the screen when it reads again, and an end asked for in the middle
/// of the flood finishes.
#[test]
fn a_pane_that_floods_its_output_still_ends() {
    let home = Home::new();
    let folder = home.prepare(current_version(), &source());
    let pids = home.path().join("pids");
    let role: OsString = "flood".into();
    let pids_value: OsString = pids.clone().into();
    let holder = start_ready(
        &folder.program,
        &home.socket("p"),
        &role_program("flood"),
        &[(ROLE, &role), (PIDS_FILE, &pids_value)],
    );
    drop(folder);
    let flooding = running(pids_in(&pids, 1));
    let mut connection = holder.open();
    connection.wait_for_text("xxxx");
    connection.send(ClientMessage::Terminate).unwrap();
    let closed = connection.wait_closed();
    assert_eq!(closed.reason, CloseReason::Terminated, "{closed:?}");
    holder.wait_gone();
    assert_ended(&flooding, "its pane");
}

/// The holder is killed outright: on Windows the job only it held ends the
/// pane's whole tree with it.
#[cfg(windows)]
#[test]
fn a_killed_holder_takes_its_pane_with_it() {
    let home = Home::new();
    let (holder, pids) = stubborn_pane(&home, &[]);
    let mut connection = holder.open();
    connection.wait_for_text("TREE READY");
    let started = running(pids_in(&pids, 3));
    hide_platform::process::terminate(holder.process.pid).unwrap();
    holder.wait_gone();
    for process in started {
        process.wait_gone();
    }
}

/// On Unix the kernel hangs up the pane's session when its holder dies; an
/// ordinary shell ends with it, and what ignores the hangup is the next
/// node's to end (holder-record).
#[cfg(unix)]
#[test]
fn a_killed_holder_hangs_up_its_shell() {
    let home = Home::new();
    let folder = home.prepare(current_version(), &source());
    let holder = start_ready(&folder.program, &home.socket("p"), &shell(), &[]);
    drop(folder);
    // SAFETY: a plain call naming the holder this test started.
    unsafe { libc::kill(holder.process.pid as libc::pid_t, libc::SIGKILL) };
    holder.wait_gone();
    holder.program.wait_gone();
}

/// A panic on any of the holder's threads ends the holder at once, after a
/// log line, and the pane goes with it as with a killed holder: no holder
/// runs on with part of it gone.
#[test]
fn a_holder_that_panics_ends_and_takes_its_pane_with_it() {
    let home = Home::new();
    let folder = home.prepare(current_version(), &source());
    let panic: OsString = "1".into();
    let holder = start_ready(
        &folder.program,
        &home.socket("p"),
        &shell(),
        &[("HIDE_HOLDER_TEST_PANIC_ON_INPUT", &panic)],
    );
    drop(folder);
    let mut connection = holder.open();
    connection
        .send(ClientMessage::Input(print_ok().to_vec()))
        .unwrap();
    holder.wait_gone();
    holder.program.wait_gone();
    let log = holder.log_text();
    assert!(log.contains("\"holder.panicked\""), "{log}");
    assert!(log.contains("\"pty-writer\""), "{log}");
    // It aborted: no orderly end ran, which would have logged one (a
    // holder that only lost its writer ends when its shell reads the EOF
    // the writer leaves behind).
    assert!(!log.contains("\"holder.ended\""), "{log}");
}

/// The holder works in its version folder, so it keeps no hold on the
/// folder it was started from (on Windows a working folder cannot be
/// removed), and runs its program in the folder the start names.
#[test]
fn a_holder_leaves_the_folder_it_started_from_and_runs_its_program_where_asked() {
    let home = Home::new();
    let folder = home.prepare(current_version(), &source());
    let start_folder = home.path().join("start");
    let work = home.path().join("pane-work-here");
    std::fs::create_dir(&start_folder).unwrap();
    std::fs::create_dir(&work).unwrap();
    let Start::Ready(holder) = start_with(
        &folder.program,
        &home.socket("p"),
        &shell(),
        &[],
        &start_folder,
        &work,
    ) else {
        panic!("the holder did not start");
    };
    std::fs::remove_dir(&start_folder).unwrap();
    #[cfg(unix)]
    assert_eq!(
        hide_platform::process::cwd_of(holder.process.pid)
            .unwrap()
            .canonicalize()
            .unwrap(),
        folder.program.parent().unwrap().canonicalize().unwrap()
    );
    drop(folder);
    let mut connection = holder.open();
    let ask: &[u8] = if cfg!(windows) { b"cd\r" } else { b"pwd\r" };
    connection.send(ClientMessage::Input(ask.to_vec())).unwrap();
    connection.wait_for_text("pane-work-here");
}

/// Copies `from` into `to`, folders and files, as an app's install lays them.
fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// The app's folder, the one a holder is prepared from, can be replaced by
/// an update and then removed while the holder it gave runs on: the holder
/// runs from its version folder (on Windows on that folder's ConPTY
/// runtime), so neither move touches it and its pane carries input and
/// output, to the connection it had and to a new one (B19).
#[test]
fn replacing_and_removing_the_app_folder_leaves_a_running_holder_alone() {
    let home = Home::new();
    let built = source();
    let app = home.path().join("app");
    std::fs::create_dir(&app).unwrap();
    let program = app.join(built.program.file_name().unwrap());
    std::fs::copy(&built.program, &program).unwrap();
    let conpty = built.conpty.as_ref().map(|runtime| {
        let copy = app.join("conpty");
        copy_tree(runtime, &copy);
        copy
    });
    let installed = hide_kit::holders::Source { program, conpty };
    let folder = home.prepare(current_version(), &installed);
    let holder = start_ready(&folder.program, &home.socket("p"), &shell(), &[]);
    drop(folder);
    let mut connection = holder.open();
    connection
        .send(ClientMessage::Input(print_ok().to_vec()))
        .unwrap();
    connection.wait_for_text("hide-ok");

    // An update: the new app is laid beside the old, the old moved aside and
    // the new put in its place, then the old removed. On Windows a rename
    // or a removal of a folder any process holds a file of fails.
    let next = home.path().join("app-next");
    copy_tree(&app, &next);
    let previous = home.path().join("app-previous");
    std::fs::rename(&app, &previous).unwrap();
    std::fs::rename(&next, &app).unwrap();
    std::fs::remove_dir_all(&previous).unwrap();
    std::fs::remove_dir_all(&app).unwrap();

    let ask = if cfg!(windows) {
        b"set H=still& call echo %H%-here\r".as_slice()
    } else {
        b"H=still; echo \"$H-here\"\r".as_slice()
    };
    connection.send(ClientMessage::Input(ask.to_vec())).unwrap();
    connection.wait_for_text("still-here");
    drop(connection);
    let mut again = holder.open();
    again.send(ClientMessage::Redraw).unwrap();
    again.wait_for_text("still-here");
    assert!(holder.process.is_running());
}
