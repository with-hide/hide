//! The version folders a holder runs from, outside the app (PRD
//! engine-bundle-2-pane-holder D-16, B20): a cleanup removes only the
//! folders nothing runs from, waits for a start that has reserved its
//! folder, and never touches the runtime copies beside them.

use std::path::Path;
use std::sync::Arc;
use std::thread;

use hide_holder_protocol::ClientMessage;
use hide_kit::holders::{Pruned, Source, program_name, prune, version_name};
use hide_kit::layout;

use crate::support::{Holder, Home, current_version, print_ok, shell, source, start_ready};

fn name_of(dir: &Path) -> String {
    dir.file_name().unwrap().to_string_lossy().into_owned()
}

fn answers(holder: &Holder) {
    let mut connection = holder.open();
    connection
        .send(ClientMessage::Input(print_ok().to_vec()))
        .unwrap();
    connection.wait_for_text("hide-ok");
}

#[test]
fn a_cleanup_removes_only_the_old_folders_nothing_runs_from() {
    let home = Home::new();
    let current = source();
    let current_name = version_name(current_version(), &current).unwrap();
    drop(home.prepare(current_version(), &current));

    // An older build's folder a pane still runs from: what runs there counts
    // as a user whether or not this build speaks its version.
    let older = home.prepare(0, &current);
    let older_name = name_of(&older.dir);
    let holder = start_ready(&older.program, &home.socket("p"), &shell(), &[]);
    drop(older);

    // An older build's folder nothing runs from.
    let unused_program = home.path().join("unused-holder");
    std::fs::write(&unused_program, b"an older holder").unwrap();
    let unused = Source {
        program: unused_program,
        conpty: current.conpty.clone(),
    };
    let unused_name = name_of(&home.prepare(0, &unused).dir);

    // A runtime copy beside them, under its own root.
    let runtime = layout::runtime_dir(home.path(), "build");
    std::fs::create_dir_all(&runtime).unwrap();
    std::fs::write(runtime.join("hided"), b"runtime").unwrap();

    let pruned = prune(home.path(), &[&current_name]);
    assert_eq!(
        pruned,
        Pruned {
            removed: vec![unused_name.clone()],
            in_use: vec![older_name.clone()],
            ..Pruned::default()
        }
    );
    assert!(!layout::holder_dir(home.path(), &unused_name).exists());
    for name in [&current_name, &older_name] {
        let dir = layout::holder_dir(home.path(), name);
        assert!(dir.join(program_name()).is_file(), "{name} lost its holder");
    }
    assert_eq!(std::fs::read(runtime.join("hided")).unwrap(), b"runtime");
    answers(&holder);

    // Once its pane has ended, the older folder goes too.
    let mut connection = holder.open();
    connection.send(ClientMessage::Terminate).unwrap();
    connection.wait_closed();
    holder.wait_gone();
    crate::support::wait_until("the ended pane's folder to be removed", || {
        prune(home.path(), &[&current_name]).removed == [older_name.clone()]
    });
    assert!(layout::holder_dir(home.path(), &current_name).is_dir());
    assert!(runtime.join("hided").is_file());
}

#[test]
fn a_cleanup_waits_for_a_start_that_reserved_its_folder() {
    let home = Arc::new(Home::new());
    let reservation = home.prepare(0, &source());
    let name = name_of(&reservation.dir);
    let pruning = {
        let home = Arc::clone(&home);
        thread::spawn(move || prune(home.path(), &[]))
    };
    // The cleanup cannot claim the folder until the reservation lets go,
    // and by then its holder runs from it.
    let holder = start_ready(&reservation.program, &home.socket("p"), &shell(), &[]);
    drop(reservation);
    let pruned = pruning.join().unwrap();
    assert_eq!(pruned.in_use, std::slice::from_ref(&name), "{pruned:?}");
    assert!(pruned.removed.is_empty(), "{pruned:?}");
    assert!(
        layout::holder_dir(home.path(), &name)
            .join(program_name())
            .is_file()
    );
    answers(&holder);
}

/// Starts and cleanups at once: every start finds its folder whole, and no
/// cleanup takes a folder a pane runs from.
#[test]
fn starts_and_cleanups_at_once_leave_every_pane_its_folder() {
    const STARTS: usize = 6;
    const CLEANUPS: usize = 3;
    let home = Arc::new(Home::new());
    let source = source();
    let starts: Vec<_> = (0..STARTS)
        .map(|index| {
            let home = Arc::clone(&home);
            let source = source.clone();
            thread::spawn(move || {
                let reservation = home.prepare((index % 2) as u16, &source);
                let name = name_of(&reservation.dir);
                let holder = start_ready(
                    &reservation.program,
                    &home.socket(&format!("p{index}")),
                    &shell(),
                    &[],
                );
                drop(reservation);
                (name, holder)
            })
        })
        .collect();
    let cleanups: Vec<_> = (0..CLEANUPS)
        .map(|_| {
            let home = Arc::clone(&home);
            thread::spawn(move || (0..4).map(|_| prune(home.path(), &[])).collect::<Vec<_>>())
        })
        .collect();
    let started: Vec<(String, Holder)> = starts
        .into_iter()
        .map(|start| start.join().unwrap())
        .collect();
    for cleanup in cleanups {
        for pruned in cleanup.join().unwrap() {
            assert!(pruned.failed.is_empty() && !pruned.busy, "{pruned:?}");
        }
    }
    let after = prune(home.path(), &[]);
    let mut names: Vec<String> = started.iter().map(|(name, _)| name.clone()).collect();
    names.sort();
    names.dedup();
    let mut in_use = after.in_use.clone();
    in_use.sort();
    assert_eq!(in_use, names, "{after:?}");
    for (name, holder) in &started {
        let dir = layout::holder_dir(home.path(), name);
        assert!(dir.join(program_name()).is_file(), "{name} lost its holder");
        answers(holder);
    }
}
