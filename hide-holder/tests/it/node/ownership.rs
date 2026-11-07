//! Real local connection workers retain the state-folder OS lock through
//! cancellation and close; another process obeys the same boundary.

use super::*;
use hide_node_link::holders::{PaneCall, PaneInput};
use hide_node_link::link_panes::LinkPanes;
use hide_node_link::protocol::Call;
use hide_node_link::{LinkError, NodeLink};

pub(super) struct Connection {
    pub(super) local: Arc<hide_node::Local>,
    pub(super) holders: Arc<Holders>,
    pub(super) linked: LinkPanes,
    pub(super) service: Arc<Service>,
    pub(super) reports: Arc<Reports>,
    pub(super) screens: Arc<Screens>,
}

impl Connection {
    pub(super) fn open(home: &Path, state: &Path) -> Self {
        Self::with_attacher(home, state, |attacher| attacher)
    }

    pub(super) fn with_attacher(
        home: &Path,
        state: &Path,
        wrap: impl FnOnce(Box<dyn Attacher>) -> Box<dyn Attacher>,
    ) -> Self {
        Self::built(home, state, source(), wrap)
    }

    /// A connection whose node starts its holders from `holder`, as the
    /// daemon of the build that carries it does.
    pub(super) fn of_build(home: &Path, state: &Path, holder: hide_kit::holders::Source) -> Self {
        Self::built(home, state, holder, |attacher| attacher)
    }

    fn built(
        home: &Path,
        state: &Path,
        holder: hide_kit::holders::Source,
        wrap: impl FnOnce(Box<dyn Attacher>) -> Box<dyn Attacher>,
    ) -> Self {
        let screens = Arc::new(Screens::default());
        let reports = Arc::new(Reports::default());
        let holders = Arc::new(
            Holders::open(
                HolderFolders {
                    home,
                    state_dir: state,
                },
                holder,
                reports.clone(),
            )
            .unwrap(),
        );
        let service = Arc::new(
            Service::start(
                wrap(holders.attacher()),
                screens.clone(),
                reports.clone(),
                RetryPolicy::Automatic,
            )
            .unwrap(),
        );
        let local = Arc::new(
            hide_node::Local::new(Some(home.to_owned())).with_panes(Arc::new(LocalPanes::new(
                Arc::clone(&holders),
                Arc::clone(&service),
                PaneShell::Fixed(shell()),
            ))),
        );
        let linked = LinkPanes::new(local.clone(), Arc::new(|_| {}));
        Self {
            local,
            holders,
            linked,
            service,
            screens,
            reports,
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.local.close("test connection ended");
        self.service.close();
    }
}

/// On assertion failure, close the caller, release the filesystem barrier
/// and wait for the real worker before Folder can remove its private home.
struct DelayedStart {
    lock: Option<hide_platform::fs::lock::Lock>,
    local: Arc<hide_node::Local>,
    caller: Option<thread::JoinHandle<()>>,
    state: PathBuf,
}

impl Drop for DelayedStart {
    fn drop(&mut self) {
        self.local.close("the delayed start finished");
        drop(self.lock.take());
        if let Some(caller) = self.caller.take() {
            let _ = caller.join();
        }
        let drained = record::lock(&layout::pane_owner_lock(&self.state), HANG_LIMIT);
        if !matches!(drained, Ok(Some(_))) {
            if thread::panicking() {
                eprintln!("the delayed node worker did not drain: {drained:?}");
            } else {
                panic!("the delayed node worker did not drain: {drained:?}");
            }
        }
    }
}

/// Uses the production Local/LocalPanes path in an independent process,
/// not a second descriptor in the parent or a stand-in PaneHolders.
pub fn owner_role() {
    let home = PathBuf::from(std::env::var_os(NODE_HOME).unwrap());
    let state = PathBuf::from(std::env::var_os(NODE_STATE).unwrap());
    let answer = PathBuf::from(std::env::var_os(NODE_READY).unwrap());
    let connection = Connection::open(&home, &state);
    let result = connection.linked.recovered(Duration::ZERO);
    std::fs::write(answer, serde_json::to_vec(&result).unwrap()).unwrap();
}

fn other_process(
    folder: &Folder,
    name: &str,
) -> Result<hide_node_link::holders::HolderRecovery, HolderError> {
    let answer = folder.home.path().join(name);
    let mut child = role(
        "node_owner",
        &[
            (NODE_HOME, &folder.home.path().into()),
            (NODE_STATE, &folder.state.clone().into()),
            (NODE_READY, &answer.clone().into()),
        ],
    );
    wait_until("the independent node owner exits", || {
        child.0.try_wait().unwrap().is_some()
    });
    assert!(child.0.wait().unwrap().success());
    serde_json::from_slice(&std::fs::read(answer).unwrap()).unwrap()
}

/// The persisted preparing state proves the actual worker has entered.
/// A version-folder lock delays it without replacing any production call.
#[test]
fn a_closed_connection_keeps_ownership_until_its_cancelled_worker_finishes() {
    use hide_platform::fs::lock::{Mode, lock_file_blocking};

    let folder = Folder::new();
    let first = Connection::open(folder.home.path(), &folder.state);
    let original = first.linked.recovered(HANG_LIMIT).unwrap();
    let pane = "w1:p1";
    let start = HolderStart {
        pane: pane.to_owned(),
        program: None,
        cwd: hide_platform::path::to_wire(folder.home.path()).unwrap(),
        env: Vec::new(),
        rows: SIZE.rows,
        cols: SIZE.cols,
        notice: None,
    };
    let request = first.linked.request().unwrap();
    let reserved = first.linked.reserve(&start, None, request).unwrap();
    let versions = layout::holders_root(folder.home.path());
    std::fs::create_dir_all(&versions).unwrap();
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(versions.join(".lock"))
        .unwrap();
    let mut blocked = DelayedStart {
        lock: Some(lock_file_blocking(file, Mode::Exclusive).unwrap()),
        local: Arc::clone(&first.local),
        caller: None,
        state: folder.state.clone(),
    };
    let worker = Arc::clone(&first.local);
    let start_call = PaneCall::StartReserved {
        start: start.clone(),
        reserved,
        request,
    };
    let (answered, answer) = channel();
    blocked.caller = Some(thread::spawn(move || {
        let result = worker.call(Call::Pane { call: start_call }, HANG_LIMIT);
        let _ = answered.send(result);
    }));
    let record_path = layout::pane_record(&folder.state, &hide_kit::panes::holder_pane(pane));
    wait_until("the admitted worker durably claims its reservation", || {
        record::read(&record_path)
            .unwrap()
            .is_some_and(|r| r.phase == Phase::Preparing)
    });
    assert!(
        answer.try_recv().is_err(),
        "the real worker is still blocked"
    );
    first
        .linked
        .cancel(&start.intent().unwrap(), request)
        .unwrap();
    let cancelled = record::read(&record_path).unwrap().unwrap();
    assert!(cancelled.cancelled());
    assert_eq!(cancelled.phase, Phase::Ended);
    assert!(matches!(
        first.linked.retire(request),
        Err(HolderError::Unreachable { message })
            if message == "the pane request still has workers draining"
    ));
    assert!(matches!(
        first.linked.retire(request),
        Err(HolderError::Unreachable { message })
            if message == "the pane request still has workers draining"
    ));
    assert!(matches!(
        first.linked.reserve(&start, None, request),
        Err(HolderError::Refused { .. })
    ));

    first.local.close("the original connection disappeared");
    assert!(matches!(
        answer.recv_timeout(HANG_LIMIT).unwrap(),
        Err(LinkError::Unknown(_))
    ));
    blocked.caller.take().unwrap().join().unwrap();
    assert!(matches!(
        first.local.call(Call::Hello, HANG_LIMIT),
        Err(LinkError::NotConnected(_))
    ));
    let next = Connection::open(folder.home.path(), &folder.state);
    assert!(matches!(
        next.linked.recovered(Duration::ZERO),
        Err(HolderError::Unreachable { message })
            if message == "the previous pane owner still has work in flight"
    ));
    assert!(matches!(
        other_process(&folder, "blocked-owner.json"),
        Err(HolderError::Unreachable { message })
            if message == "the previous pane owner still has work in flight"
    ));
    assert_eq!(record::read(&record_path).unwrap().unwrap(), cancelled);

    drop(blocked);
    let recovered = next.linked.recovered(HANG_LIMIT).unwrap();
    assert_ne!(recovered.owner, original.owner);
    assert!(recovered.closed.contains(&pane.to_owned()));
    assert!(!recovered.recoverable.iter().any(|p| p.pane == pane));
    let settled = record::read(&record_path).unwrap().unwrap();
    // Reconcile rechecks cancellation cleanup and may refresh its diagnostic
    // text and time. Every identity, intent and terminal outcome stays fixed.
    let mut expected = cancelled.clone();
    let end = expected.end.as_mut().unwrap();
    let settled_end = settled.end.as_ref().unwrap();
    end.detail.clone_from(&settled_end.detail);
    end.at_unix_ms = settled_end.at_unix_ms;
    assert_eq!(settled, expected);
    assert!(matches!(
        next.linked.reserve(&start, None, request),
        Err(HolderError::Refused { .. })
    ));
    assert!(matches!(
        next.linked.retire(request),
        Err(HolderError::Refused { .. })
    ));

    // A live holder does not inherit the node's owner lock. Retirement of
    // its creation intent also does not remove its independent close right.
    let live = HolderStart {
        pane: "w1:p2".to_owned(),
        ..start
    };
    let live_request = next.linked.request().unwrap();
    let live_reserved = next.linked.reserve(&live, None, live_request).unwrap();
    next.linked
        .start_reserved(&live, live_reserved, live_request)
        .unwrap();
    next.linked.retire(live_request).unwrap();
    next.linked.retire(live_request).unwrap();
    next.linked
        .input(&live.pane, PaneInput::Bytes(print_ok().to_vec()))
        .unwrap();
    wait_until("the retired creation still has a working pane", || {
        next.linked
            .read(&live.pane, 200)
            .unwrap()
            .0
            .contains("hide-ok")
    });
    let live_path = layout::pane_record(&folder.state, &hide_kit::panes::holder_pane(&live.pane));
    let running = record::read(&live_path).unwrap().unwrap();
    next.local.close("handoff with a live holder");
    let independent = other_process(&folder, "acquired-owner.json").unwrap();
    assert_ne!(independent.owner, recovered.owner);
    assert!(independent.present.contains(&live.pane));
    assert_eq!(record::read(&live_path).unwrap().unwrap(), running);
    assert!(running.holder.unwrap().runs());
    let last = Connection::open(folder.home.path(), &folder.state);
    last.linked.recovered(HANG_LIMIT).unwrap();
    last.linked.terminate(&live.pane).unwrap();
    wait_until("the adopted holder ends", || {
        !running.holder.unwrap().runs()
    });
}
