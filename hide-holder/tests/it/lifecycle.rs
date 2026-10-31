//! A holder's life as a node observes it: started from a version folder as a
//! resident child, its shell's input and output, a release and a second
//! connection, and the end of its pane, asked for or on its own, which leaves
//! nothing of the pane running (PRD engine-bundle-2-pane-holder B11, B12,
//! B18, B19).

use std::ffi::OsString;

use crate::role::EXIT_EARLY;
use crate::support::{
    Connection, Home, PIDS_FILE, ROLE, Running, current_version, pids_in, print_ok, role_program,
    shell, source, start_ready,
};
use hide_holder_protocol::{ClientMessage, CloseReason, Size};

#[test]
fn a_holder_runs_its_shell_and_carries_input_and_output_across_connections() {
    let home = Home::new();
    let folder = home.prepare(current_version(), &source());
    let socket = home.socket("p");
    let holder = start_ready(&folder.program, &socket, &shell(), &[]);
    drop(folder);

    let mut first = holder.open();
    assert_eq!(first.welcome.generation, 1);
    assert_eq!(first.welcome.program, holder.program.pid);
    assert_eq!(first.welcome.size, Size { rows: 24, cols: 80 });
    first
        .send(ClientMessage::Input(print_ok().to_vec()))
        .unwrap();
    first.wait_for_output("hide-ok");

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
        first.wait_for_output("40 100");
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
    second.wait_for_output("hide-ok");

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
    second.wait_for_output("hide-ok");
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
    connection.wait_for_output("TREE READY");
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
    // The program may exit before a connection is made; what is left of
    // its tree is ended either way.
    let left: Vec<Running> = pids_in(&pids, 2)
        .into_iter()
        .filter_map(Running::of)
        .collect();
    match holder.try_open() {
        Ok((stream, welcome)) => {
            let closed = Connection::new(stream, welcome).wait_closed();
            assert_eq!(closed.reason, CloseReason::ProgramExited, "{closed:?}");
            assert_eq!(closed.exit_code, Some(0), "{closed:?}");
        }
        Err(error) => assert!(
            !holder.process.is_running() || matches!(error, hide_holder_protocol::OpenError::Io(_)),
            "{error}"
        ),
    }
    holder.wait_gone();
    assert_ended(&left, "its program");
}

/// A connection that never reads while the program floods it is closed for
/// it, the program runs on, and an end asked for in the middle of the flood
/// finishes.
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
    connection.wait_for_output("xxxx");
    connection.send(ClientMessage::Terminate).unwrap();
    let closed = connection.wait_closed();
    assert!(
        matches!(
            closed.reason,
            CloseReason::Terminated | CloseReason::OutputQueueFull
        ),
        "{closed:?}"
    );
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
    connection.wait_for_output("TREE READY");
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
