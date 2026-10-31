//! A Windows pane runs on the pinned ConPTY runtime in its holder's version
//! folder and on nothing else (PRD engine-bundle-2-pane-holder D-19, B12):
//! the holder starts with no console of its own, the runtime's OpenConsole
//! from that folder carries the shell, and a folder whose runtime is missing
//! or changed refuses the start instead of falling back to the system's.

use std::path::Path;

use hide_holder_protocol::ClientMessage;
use hide_kit::holders::Source;

use crate::support::{Home, Start, current_version, print_ok, shell, source, start, start_ready};

/// Whether a process runs `file`: Windows refuses to open a running
/// program's file for writing, though it lets anyone read it
/// (`hide_platform::fs::in_use`). Nothing is written.
fn held(file: &Path) -> bool {
    match std::fs::OpenOptions::new().write(true).open(file) {
        Ok(_) => false,
        Err(error) if hide_platform::fs::in_use::held_by_another(&error) => true,
        Err(error) => panic!("{} could not be asked: {error}", file.display()),
    }
}

#[test]
fn a_pane_runs_on_the_pinned_conpty_in_its_version_folder() {
    let home = Home::new();
    let folder = home.prepare(current_version(), &source());
    let open_console = folder
        .dir
        .join("conpty")
        .join("x64")
        .join("OpenConsole.exe");
    let holder = start_ready(&folder.program, &home.socket("p"), &shell(), &[]);
    drop(folder);
    let mut connection = holder.open();
    connection
        .send(ClientMessage::Input(print_ok().to_vec()))
        .unwrap();
    connection.wait_for_output("hide-ok");
    assert!(
        held(&open_console),
        "the shell runs on a console other than the version folder's OpenConsole"
    );
}

fn refused(home: &Home, source: &Source) -> String {
    let folder = home.prepare(current_version(), source);
    match start(&folder.program, &home.socket("p"), &shell(), &[]) {
        Start::Failed(reason) => reason,
        Start::Ready(_) => panic!("the holder started without its pinned ConPTY runtime"),
    }
}

#[test]
fn a_folder_without_the_pinned_conpty_refuses_the_start() {
    let home = Home::new();
    let mut without = source();
    without.conpty = None;
    let reason = refused(&home, &without);
    assert!(reason.contains("ConPTY runtime is missing"), "{reason}");
}

fn copy_folder(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_folder(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

#[test]
fn a_changed_conpty_runtime_refuses_the_start() {
    let home = Home::new();
    let pinned = source();
    let changed = home.path().join("changed-conpty");
    copy_folder(pinned.conpty.as_ref().unwrap(), &changed);
    let library = changed.join("conpty.dll");
    let mut permissions = std::fs::metadata(&library).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)] // Windows' read-only attribute only
    permissions.set_readonly(false);
    std::fs::set_permissions(&library, permissions).unwrap();
    let mut bytes = std::fs::read(&library).unwrap();
    bytes.push(0);
    std::fs::write(&library, bytes).unwrap();
    let reason = refused(
        &home,
        &Source {
            program: pinned.program,
            conpty: Some(changed),
        },
    );
    assert!(reason.contains("conpty.dll has hash"), "{reason}");
}
