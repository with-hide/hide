//! A state folder's panes as a node finds them through their records: starts
//! that converge and stop at the cap, a pane that ends with no node there,
//! and the reconcile that adopts what its records name and leaves what it
//! cannot judge (PRD engine-bundle-2-pane-holder D-13, D-14, D-17, B2, B7,
//! B12, B13, B14).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use hide_holder_protocol::record::{self, EndReason, Identity, PaneRecord, Phase, RECORD_FORMAT};
use hide_holder_protocol::{PROTOCOL_VERSION, Size};
use hide_kit::layout;
use hide_kit::panes::{
    ENDED_KEPT, HOLDER_CAP, Judgment, Panes, RECONCILE_BUDGET, START_LIMIT, StartRefusal,
    StartRequest, Started,
};

use crate::role::GO;
use crate::support::{
    Holder, Home, PIDS_FILE, ROLE, Running, current_version, node_theme, pids_in, role_program,
    shell, source, start_with, wait_until,
};

static EVENTS: Mutex<Vec<serde_json::Value>> = Mutex::new(Vec::new());

fn sink(event: serde_json::Value) {
    EVENTS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(event);
}

/// A private HOME with one state folder. Dropping it ends every holder its
/// records name, and on Unix what each pane's program left, by pid and start
/// time.
struct Folder {
    home: Home,
    state: PathBuf,
    panes: Panes,
}

impl Folder {
    fn new() -> Self {
        let home = Home::new();
        let state = home.path().join("s");
        let panes = Panes::open(home.path(), &state, sink).unwrap();
        Self { home, state, panes }
    }

    fn request(
        &self,
        pane: &str,
        program: Vec<OsString>,
        env: Vec<(OsString, OsString)>,
    ) -> StartRequest {
        StartRequest {
            pane: pane.to_owned(),
            program,
            cwd: self.home.path().to_owned(),
            env,
            size: Size { rows: 24, cols: 80 },
            theme: node_theme(),
        }
    }

    /// A pane whose program plays `role`, writing its pids to `pids` and
    /// waiting for `go` when the role does.
    fn role_request(&self, pane: &str, role: &str) -> StartRequest {
        self.request(
            pane,
            role_program(role),
            vec![
                (ROLE.into(), role.into()),
                (PIDS_FILE.into(), self.pids().into()),
                (GO.into(), self.go().into()),
            ],
        )
    }

    fn pids(&self) -> PathBuf {
        self.home.path().join("pids")
    }

    fn go(&self) -> PathBuf {
        self.home.path().join("go")
    }

    fn start(&self, request: &StartRequest) -> PaneRecord {
        match self.panes.start(request, &source()) {
            Ok(Started::Started(record)) => record,
            other => panic!("the pane did not start: {other:?}"),
        }
    }

    fn record(&self, pane: &str) -> PaneRecord {
        self.panes
            .record(pane)
            .unwrap()
            .expect("the pane has a record")
    }

    /// A holder started the way a node starts one, at this state folder's
    /// socket for `pane`, with no record of its own: a test writes the record
    /// that names it.
    fn bare_holder(
        &self,
        pane: &str,
        program: &[OsString],
        environment: &[(&str, &OsString)],
    ) -> Holder {
        let folder = self.home.prepare(current_version(), &source());
        let socket = layout::holder_socket(&self.state, pane);
        let holder = match start_with(
            &folder.program,
            &socket,
            program,
            environment,
            &folder.dir,
            self.home.path(),
        ) {
            crate::support::Start::Ready(holder) => holder,
            crate::support::Start::Failed(reason) => panic!("the holder did not start: {reason}"),
        };
        drop(folder);
        holder
    }

    fn write(&self, record: &PaneRecord) {
        record::write(&layout::pane_record(&self.state, &record.pane), record).unwrap();
    }

    /// A record as a start reserves one.
    fn reserved(&self, pane: &str, phase: Phase) -> PaneRecord {
        PaneRecord {
            format: RECORD_FORMAT,
            pane: pane.to_owned(),
            owner: self.panes.owner().to_owned(),
            incarnation: 1,
            phase,
            protocol: PROTOCOL_VERSION,
            version_folder: String::new(),
            spec: String::new(),
            reserved_at_unix_ms: record::now_unix_ms(),
            boot: hide_platform::process::boot_id().ok(),
            holder: None,
            program: None,
            end: None,
        }
    }

    fn events(&self, name: &str) -> Vec<serde_json::Value> {
        EVENTS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|event| {
                event["state_dir"] == serde_json::json!(self.state) && event["event"] == name
            })
            .cloned()
            .collect()
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let Ok(entries) = std::fs::read_dir(layout::pane_records(&self.state)) else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(Some(record)) = record::read(&entry.path()) else {
                continue;
            };
            if let Some(holder) = record.holder
                && holder.runs()
            {
                let _ = hide_platform::process::kill_tree(holder.pid);
            }
            #[cfg(unix)]
            if let Some(program) = record.program
                && let Ok(mut session) =
                    hide_platform::process::Session::left_by(program.pid, program.started)
            {
                let _ = session.end(Duration::from_secs(5));
            }
        }
    }
}

fn identity(process: &Running) -> Identity {
    Identity::of(process.pid).expect("the process runs")
}

fn running(pids: Vec<u32>) -> Vec<Running> {
    pids.into_iter()
        .map(|pid| Running::of(pid).unwrap_or_else(|| panic!("process {pid} ended early")))
        .collect()
}

fn end_reason(record: &PaneRecord) -> EndReason {
    record
        .end
        .as_ref()
        .expect("the record says why it ended")
        .reason
}

/// The processes running a version folder's holder under `home`, a test's
/// own.
fn holders_in(home: &Path) -> Vec<u32> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(layout::holders_root(home))
        .unwrap()
        .flatten()
    {
        let program = entry.path().join(hide_kit::holders::program_name());
        let Ok(file) = hide_platform::fs::identity::file_id(&program) else {
            continue;
        };
        found.extend(hide_platform::process::running(&file).unwrap());
    }
    found
}

#[test]
fn starts_of_one_pane_at_once_converge_on_one_holder_which_ends_when_closed() {
    let folder = Folder::new();
    let request = folder.request("p1", shell(), Vec::new());
    let source = source();
    let answers: Vec<Started> = thread::scope(|scope| {
        let starts: Vec<_> = (0..4)
            .map(|_| scope.spawn(|| folder.panes.start(&request, &source).unwrap()))
            .collect();
        starts
            .into_iter()
            .map(|start| start.join().unwrap())
            .collect()
    });
    let started: Vec<&PaneRecord> = answers
        .iter()
        .filter_map(|answer| match answer {
            Started::Started(record) => Some(record),
            Started::Existing(_) => None,
        })
        .collect();
    assert_eq!(started.len(), 1, "{answers:?}");
    for answer in &answers {
        let (Started::Started(record) | Started::Existing(record)) = answer;
        assert_eq!(record.incarnation, started[0].incarnation, "{answers:?}");
    }
    let socket = layout::holder_socket(&folder.state, "p1");
    let holder = folder
        .record("p1")
        .holder
        .expect("the holder wrote itself in");
    assert_eq!(holders_in(folder.home.path()), vec![holder.pid]);
    // The same pane id with another command is another intent, refused.
    let other = folder.request("p1", role_program("flood"), Vec::new());
    assert!(
        matches!(
            folder.panes.start(&other, &source),
            Err(StartRefusal::Conflict(_))
        ),
        "a second command took pane p1"
    );

    let ended = folder.panes.close("p1").unwrap();
    assert_eq!(ended.phase, Phase::Ended, "{ended:?}");
    assert_eq!(end_reason(&ended), EndReason::Terminated, "{ended:?}");
    assert!(!holder.runs());

    // The same start asked again once the pane ended starts it again.
    let again = folder.start(&request);
    assert!(again.incarnation > ended.incarnation, "{again:?}");
    let second = again.holder.expect("the holder wrote itself in");
    assert_ne!(second, holder);
    assert_eq!(holders_in(folder.home.path()), vec![second.pid]);
    folder.panes.close("p1").unwrap();
    folder.panes.forget("p1").unwrap();
    assert!(folder.panes.record("p1").unwrap().is_none());
    assert!(!socket.exists());
}

#[test]
fn a_program_that_cannot_start_leaves_why_in_its_record() {
    let folder = Folder::new();
    let missing = folder.home.path().join("no-such-program");
    let request = folder.request("p1", vec![missing.into()], Vec::new());
    let refused = folder.panes.start(&request, &source());
    assert!(
        matches!(refused, Err(StartRefusal::Failed(_))),
        "{refused:?}"
    );
    let ended = folder.record("p1");
    assert_eq!(end_reason(&ended), EndReason::StartFailed, "{ended:?}");
}

#[test]
fn starts_at_once_stop_at_the_cap_and_say_so() {
    let folder = Folder::new();
    for number in 0..HOLDER_CAP - 2 {
        folder.write(&folder.reserved(&format!("f{number}"), Phase::Running));
    }
    // An unreadable record may still name a live holder and spends a slot.
    std::fs::write(layout::pane_record(&folder.state, "f0"), b"{").unwrap();
    let source = source();
    let requests: Vec<StartRequest> = (0..4)
        .map(|number| folder.request(&format!("a{number}"), shell(), Vec::new()))
        .collect();
    let answers: Vec<Result<Started, StartRefusal>> = thread::scope(|scope| {
        let starts: Vec<_> = requests
            .iter()
            .map(|request| scope.spawn(|| folder.panes.start(request, &source)))
            .collect();
        starts
            .into_iter()
            .map(|start| start.join().unwrap())
            .collect()
    });
    let started = answers
        .iter()
        .filter(|answer| matches!(answer, Ok(Started::Started(_))))
        .count();
    let refused = answers
        .iter()
        .filter(|answer| matches!(answer, Err(StartRefusal::Cap { count }) if *count == HOLDER_CAP))
        .count();
    assert_eq!((started, refused), (2, 2), "{answers:?}");
    let logged = folder.events("pane_holder.start_refused");
    assert_eq!(logged.len(), 2, "{logged:?}");
    assert!(
        logged.iter().all(|event| event["reason"] == "holder_cap"),
        "{logged:?}"
    );
}

/// A directory at the file-count boundary still has room when its records
/// are ended. Admission counts their states, retains only the newest ended
/// reasons, and treats an unreadable record as a possibly live holder.
#[test]
fn ended_records_at_the_capacity_boundary_leave_room_and_keep_their_retention_bound() {
    let folder = Folder::new();
    for number in 0..=ENDED_KEPT {
        let mut ended = folder.reserved(&format!("e{number}"), Phase::Ended);
        let mut end = record::End::new(EndReason::ProgramExited, "ended");
        end.at_unix_ms = number as u64 + 1;
        ended.end = Some(end);
        folder.write(&ended);
    }
    std::fs::write(layout::pane_record(&folder.state, "unreadable"), b"{").unwrap();
    let running = folder.start(&folder.request("fresh", shell(), Vec::new()));
    assert_eq!(running.phase, Phase::Running);
    assert_eq!(folder.panes.live().unwrap(), 2);
    assert!(folder.panes.record("e0").unwrap().is_none());
    for number in 1..=ENDED_KEPT {
        assert_eq!(folder.record(&format!("e{number}")).phase, Phase::Ended);
    }
    assert!(folder.panes.record("unreadable").is_err());
    folder.panes.close("fresh").unwrap();
}

#[test]
fn a_program_that_ends_with_no_node_there_leaves_why_in_its_record() {
    let folder = Folder::new();
    let started = folder.start(&folder.role_request("p1", "exit_when_told"));
    let holder = started.holder.expect("the holder wrote itself in");
    std::fs::write(folder.go(), b"").unwrap();
    wait_until("the pane's end in its record", || {
        folder.record("p1").phase == Phase::Ended
    });
    let ended = folder.record("p1");
    let end = ended.end.as_ref().unwrap();
    assert_eq!(end.reason, EndReason::ProgramExited, "{ended:?}");
    assert_eq!(end.exit_code, Some(3), "{ended:?}");
    wait_until("the holder's end", || !holder.runs());
}

/// The holder waits a bounded time for the records' lock and exits without
/// the end it could not write; the next reconcile settles the pane as one
/// whose holder was lost.
#[test]
fn an_end_the_holder_could_not_record_is_settled_as_a_lost_holder() {
    let folder = Folder::new();
    let started = folder.start(&folder.role_request("p1", "exit_when_told"));
    let holder = started.holder.expect("the holder wrote itself in");
    let lock = record::lock(
        &layout::pane_records_lock(&folder.state),
        Duration::from_secs(5),
    )
    .unwrap()
    .expect("the records' lock");
    std::fs::write(folder.go(), b"").unwrap();
    wait_until("the holder's end", || !holder.runs());
    drop(lock);
    assert_eq!(folder.record("p1").phase, Phase::Running);

    let found = folder.panes.reconcile(RECONCILE_BUDGET);
    let Some(Judgment::Ended(ended)) = found.panes.get("p1") else {
        panic!("{found:?}");
    };
    let end = ended.end.as_ref().unwrap();
    assert_eq!(end.reason, EndReason::HolderLost, "{ended:?}");
}

#[test]
fn holders_whose_records_cannot_be_judged_are_left_running() {
    let folder = Folder::new();
    let corrupt = folder.bare_holder("c1", &shell(), &[]);
    std::fs::write(layout::pane_record(&folder.state, "c1"), b"{ not a record").unwrap();
    let unknown = folder.bare_holder("u1", &shell(), &[]);
    let mut future = folder.reserved("u1", Phase::Running);
    future.format = RECORD_FORMAT + 1;
    future.holder = Some(identity(&unknown.process));
    std::fs::write(
        layout::pane_record(&folder.state, "u1"),
        serde_json::to_vec(&future).unwrap(),
    )
    .unwrap();
    let elsewhere = folder.bare_holder("x1", &shell(), &[]);
    let mut theirs = folder.reserved("x1", Phase::Running);
    theirs.owner = "another-state-folder".to_owned();
    theirs.holder = Some(identity(&elsewhere.process));
    folder.write(&theirs);

    let found = folder.panes.reconcile(RECONCILE_BUDGET);
    assert!(
        matches!(found.panes.get("c1"), Some(Judgment::Unreadable(_))),
        "{found:?}"
    );
    assert!(
        matches!(found.panes.get("u1"), Some(Judgment::Unreadable(_))),
        "{found:?}"
    );
    assert!(
        matches!(found.panes.get("x1"), Some(Judgment::OtherOwner(_))),
        "{found:?}"
    );
    for holder in [&corrupt, &unknown, &elsewhere] {
        assert!(holder.process.is_running(), "{found:?}");
        assert!(holder.program.is_running(), "{found:?}");
    }
}

#[test]
fn a_reservation_takes_the_holder_it_names_and_one_left_past_its_bound_ends() {
    let folder = Folder::new();
    let named = folder.bare_holder("r1", &shell(), &[]);
    let mut taken = folder.reserved("r1", Phase::Preparing);
    taken.holder = Some(identity(&named.process));
    folder.write(&taken);
    folder.write(&folder.reserved("r2", Phase::Preparing));
    let mut stale = folder.reserved("r3", Phase::Preparing);
    stale.reserved_at_unix_ms -= (START_LIMIT + Duration::from_secs(1)).as_millis() as u64;
    folder.write(&stale);

    let found = folder.panes.reconcile(RECONCILE_BUDGET);
    assert!(
        matches!(found.panes.get("r1"), Some(Judgment::Alive(_))),
        "{found:?}"
    );
    assert!(
        matches!(found.panes.get("r2"), Some(Judgment::Starting(_))),
        "{found:?}"
    );
    let Some(Judgment::Ended(ended)) = found.panes.get("r3") else {
        panic!("{found:?}");
    };
    assert_eq!(end_reason(ended), EndReason::StartInterrupted);
    assert!(named.process.is_running());
}

/// A holder killed outright: on Unix what ignores the hangup is ended by
/// the next reconcile from the program's recorded start, on Windows the
/// holder's job ended it; either way the record says the holder was lost
/// and nothing of the pane is left, its socket included.
#[test]
fn a_killed_holders_pane_is_settled_by_the_next_reconcile() {
    let folder = Folder::new();
    let started = folder.start(&folder.role_request("p1", "stubborn_tree"));
    let holder = started.holder.expect("the holder wrote itself in");
    let tree = running(pids_in(&folder.pids(), 2));
    #[cfg(unix)]
    // SAFETY: a plain call naming the holder this test started, still
    // running as it was recorded.
    unsafe {
        assert!(holder.runs());
        libc::kill(holder.pid as libc::pid_t, libc::SIGKILL);
    }
    #[cfg(windows)]
    hide_platform::process::terminate(holder.pid).unwrap();
    wait_until("the holder's end", || !holder.runs());
    #[cfg(unix)]
    assert!(
        tree.iter().all(Running::is_running),
        "what ignores the hangup should outlive the holder"
    );

    let found = folder.panes.reconcile(RECONCILE_BUDGET);
    let Some(Judgment::Ended(ended)) = found.panes.get("p1") else {
        panic!("{found:?}");
    };
    assert_eq!(end_reason(ended), EndReason::HolderLost, "{ended:?}");
    for process in &tree {
        process.wait_gone();
    }
    for pid in pids_in(&folder.pids(), tree.len()) {
        assert!(
            Running::of(pid).is_none(),
            "process {pid} outlived its pane"
        );
    }
    let socket = layout::holder_socket(&folder.state, "p1");
    assert!(!socket.exists());
    let mut lock = socket.into_os_string();
    lock.push(".lock");
    assert!(!Path::new(&lock).exists());
}

/// Pids and start times of another boot may name processes of this one, so
/// such a record is ended without touching anything it names.
#[cfg(unix)]
#[test]
fn a_record_of_an_earlier_boot_ends_nothing_it_names() {
    let folder = Folder::new();
    let named = folder.bare_holder("b1", &shell(), &[]);
    let mut earlier = folder.reserved("b1", Phase::Running);
    earlier.boot = Some("an-earlier-boot".to_owned());
    earlier.holder = Some(identity(&named.process));
    earlier.program = Some(identity(&named.program));
    folder.write(&earlier);

    let found = folder.panes.reconcile(RECONCILE_BUDGET);
    let Some(Judgment::Ended(ended)) = found.panes.get("b1") else {
        panic!("{found:?}");
    };
    assert_eq!(end_reason(ended), EndReason::MachineRestarted);
    assert!(named.process.is_running());
    assert!(named.program.is_running());
}

/// After a power loss the device may keep a pane record empty, cut off or
/// older than the last write, and the incarnation counter the same way,
/// since a record is flushed to the device but not past its cache (a pane
/// record need not outlive the machine: its holder dies with it). None of
/// that may stop the panes starting again: a node that opens the folder
/// removes the unparseable records written before this boot, ends the
/// older readable one as of an earlier boot, moves the counter past every
/// incarnation a record holds, and every pane then starts as new. A record
/// damaged in this boot is no crash's and is left as it is, refusing its
/// pane as before, since its holder may run.
#[test]
fn records_a_power_loss_left_empty_cut_off_or_old_do_not_stop_the_panes_starting() {
    let folder = Folder::new();
    let records = layout::pane_records(&folder.state);
    let before_boot = std::time::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    let age = |path: &Path| {
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(before_boot)
            .unwrap();
    };

    let mut old = folder.reserved("old", Phase::Running);
    old.boot = Some("an-earlier-boot".to_owned());
    old.incarnation = 500;
    old.reserved_at_unix_ms = 1_000_000_000_000;
    folder.write(&old);
    let whole = serde_json::to_vec(&old).unwrap();
    std::fs::write(records.join("empty.json"), b"").unwrap();
    std::fs::write(records.join("cut.json"), &whole[..whole.len() / 2]).unwrap();
    std::fs::write(layout::pane_incarnations(&folder.state), b"").unwrap();
    for name in ["old.json", "empty.json", "cut.json", "next"] {
        age(&records.join(name));
    }
    // Damaged in this boot: not a crash's.
    std::fs::write(records.join("fresh.json"), b"").unwrap();

    let reopened = Panes::open(folder.home.path(), &folder.state, sink).unwrap();
    assert!(!records.join("empty.json").exists());
    assert!(!records.join("cut.json").exists());
    assert!(
        records.join("fresh.json").exists(),
        "a record of this boot is left"
    );
    let removed: Vec<_> = folder
        .events("pane_holder.record_removed")
        .into_iter()
        .map(|event| event["pane"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(removed.len(), 2, "{removed:?}");
    assert!(removed.contains(&"empty".to_owned()) && removed.contains(&"cut".to_owned()));
    let next: u64 = std::fs::read_to_string(layout::pane_incarnations(&folder.state))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(
        next > 500,
        "the counter is past every incarnation a record holds: {next}"
    );

    let found = reopened.reconcile(RECONCILE_BUDGET);
    let Some(Judgment::Ended(ended)) = found.panes.get("old") else {
        panic!("{found:?}");
    };
    // Windows names no boot; the old reservation has no holder and is
    // past START_LIMIT, so its interrupted start is ended.
    let expected = if cfg!(unix) {
        EndReason::MachineRestarted
    } else {
        EndReason::StartInterrupted
    };
    assert_eq!(end_reason(ended), expected);

    for pane in ["empty", "cut", "old"] {
        let started = match reopened.start(&folder.request(pane, shell(), Vec::new()), &source()) {
            Ok(Started::Started(record)) => record,
            other => panic!("{pane} did not start: {other:?}"),
        };
        assert!(started.incarnation > 500, "{pane}: {}", started.incarnation);
        assert_eq!(started.phase, Phase::Running);
    }
    match reopened.start(&folder.request("fresh", shell(), Vec::new()), &source()) {
        Err(StartRefusal::RecordUnreadable(_)) => {}
        other => panic!("a record damaged in this boot refuses its pane: {other:?}"),
    }
    std::fs::remove_file(records.join("fresh.json")).unwrap();
}

/// A reconcile takes up no pane past its budget and leaves the rest to the
/// next one; panes are judged several at once, so many lost holders do not
/// add up one after another.
#[test]
fn a_reconcile_past_its_budget_leaves_the_rest_to_the_next() {
    const PANES: usize = 8;
    let folder = Folder::new();
    let mut holders = Vec::new();
    for number in 0..PANES {
        let started = folder.start(&folder.role_request(&format!("p{number}"), "stubborn_tree"));
        holders.push(started.holder.expect("the holder wrote itself in"));
    }
    let tree = running(pids_in(&folder.pids(), 2 * PANES));
    for holder in &holders {
        #[cfg(unix)]
        // SAFETY: a plain call naming a holder this test started, still
        // running as it was recorded.
        unsafe {
            assert!(holder.runs());
            libc::kill(holder.pid as libc::pid_t, libc::SIGKILL);
        }
        #[cfg(windows)]
        hide_platform::process::terminate(holder.pid).unwrap();
    }
    for holder in &holders {
        wait_until("the holder's end", || !holder.runs());
    }

    let none = folder.panes.reconcile(Duration::ZERO);
    assert_eq!(none.pending(), PANES, "{none:?}");
    assert!(!none.is_complete());
    #[cfg(unix)]
    assert!(
        tree.iter().all(Running::is_running),
        "nothing is ended past the budget"
    );

    let started = Instant::now();
    let found = folder.panes.reconcile(RECONCILE_BUDGET);
    let took = started.elapsed();
    eprintln!("reconcile of {PANES} lost panes with hangup-ignoring trees took {took:?}");
    assert!(found.is_complete(), "{found:?}");
    for number in 0..PANES {
        let Some(Judgment::Ended(ended)) = found.panes.get(&format!("p{number}")) else {
            panic!("{found:?}");
        };
        assert_eq!(end_reason(ended), EndReason::HolderLost, "{ended:?}");
    }
    for process in &tree {
        process.wait_gone();
    }
}

/// A closing pane whose holder nothing answers for at its socket is ended
/// by its pids, as an orphan is, and settled as a lost holder.
#[cfg(unix)]
#[test]
fn a_closing_pane_whose_holder_cannot_be_asked_is_ended_by_its_pids() {
    let folder = Folder::new();
    let named = folder.bare_holder("c1", &shell(), &[]);
    let mut closing = folder.reserved("c1", Phase::Closing);
    closing.holder = Some(identity(&named.process));
    closing.program = Some(identity(&named.program));
    folder.write(&closing);
    std::fs::remove_file(&named.socket).unwrap();

    let found = folder.panes.reconcile(RECONCILE_BUDGET);
    let Some(Judgment::Ended(ended)) = found.panes.get("c1") else {
        panic!("{found:?}");
    };
    assert_eq!(end_reason(ended), EndReason::HolderLost, "{ended:?}");
    named.process.wait_gone();
    named.program.wait_gone();
}

/// A pane closing when it is started again is ended first, then started as
/// a new incarnation.
#[test]
fn a_closing_pane_started_again_ends_first() {
    let folder = Folder::new();
    let request = folder.request("p1", shell(), Vec::new());
    let first = folder.start(&request);
    let holder = first.holder.expect("the holder wrote itself in");
    let mut closing = folder.record("p1");
    closing.phase = Phase::Closing;
    folder.write(&closing);

    let again = folder.start(&request);
    assert!(again.incarnation > first.incarnation, "{again:?}");
    assert!(!holder.runs());
    assert_ne!(again.holder, Some(holder));
}

/// An earlier start that ended but left processes running holds the pane's
/// id, naming them, until they are gone.
#[test]
fn an_earlier_start_that_left_processes_holds_its_id_until_they_end() {
    let folder = Folder::new();
    let named = folder.bare_holder("e1", &shell(), &[]);
    let program = identity(&named.program);
    let mut ended = folder.reserved("e1", Phase::Ended);
    ended.program = Some(program);
    let mut end = record::End::new(EndReason::TreeEndUnconfirmed, "left running");
    end.left = vec![program.pid];
    ended.end = Some(end);
    folder.write(&ended);
    let request = folder.request("e1", shell(), Vec::new());

    match folder.panes.start(&request, &source()) {
        Err(StartRefusal::EarlierRuns(left)) => assert!(left.contains(&program.pid), "{left:?}"),
        other => panic!("the pane's id was taken while its program ran: {other:?}"),
    }
    drop(named);
    let started = folder.start(&request);
    assert_eq!(started.phase, Phase::Running, "{started:?}");
    assert_ne!(started.program, Some(program), "{started:?}");
}

#[test]
fn closing_a_pane_still_starting_says_so() {
    let folder = Folder::new();
    folder.write(&folder.reserved("s1", Phase::Preparing));
    let answer = folder.panes.close("s1");
    assert!(
        matches!(answer, Err(hide_kit::panes::CloseError::Starting)),
        "{answer:?}"
    );
    // The start finds the record no longer reserves it.
    assert_eq!(folder.record("s1").phase, Phase::Closing);
}
