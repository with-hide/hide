//! Bounded process-name reads for panes in the attach window, on their host.
use super::*;
use crate::reader::BackgroundRead;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Debug, PartialEq)]
struct Request {
    generation: u64,
    panes: Vec<(String, String)>,
}

type Answer = (Request, Vec<(String, Option<String>)>);

pub(super) struct ProcessReader {
    reader: BackgroundRead<Request, Answer>,
    desired: Request,
    connection: u64,
    cancelled: Arc<AtomicBool>,
}

impl Drop for ProcessReader {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        self.reader.join_pending();
    }
}

impl ProcessReader {
    pub(super) fn new(context: &SessionSyncContext) -> Self {
        let engine = Arc::clone(&context.engine);
        let owner = context.runtime.clone();
        let target = context.log_target().to_owned();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        Self {
            cancelled,
            reader: BackgroundRead::new(
                Duration::from_secs(30),
                Duration::ZERO,
                move |request: &Request| {
                    let mut names = Vec::with_capacity(request.panes.len());
                    for (pane_id, _) in &request.panes {
                        if worker_cancelled.load(Ordering::Acquire) || owner.upgrade().is_none() {
                            break;
                        }
                        let result = engine
                            .foreground_program(Wait::Small(SYNC_REQUEST_TIMEOUT), pane_id)
                            .map_err(String::from);
                        let name = match result {
                            Ok(name) => name,
                            Err(message) => {
                                crate::diagnostic!(
                                    json!({"component":"tab_label", "kind":"process.read_failed", "target":target, "pane_id":pane_id, "message":message})
                                );
                                None
                            }
                        };
                        names.push((pane_id.clone(), name));
                    }
                    (request.clone(), names)
                },
            ),
            desired: Request {
                generation: 0,
                panes: Vec::new(),
            },
            connection: 0,
        }
    }

    pub(super) fn poll(
        &mut self,
        context: &SessionSyncContext,
        replica: &mut SessionReplica,
        connection: u64,
    ) -> bool {
        let Some(runtime) = context.runtime.upgrade() else {
            return false;
        };
        let target = match &context.target {
            SessionSyncTarget::Local { .. } => None,
            SessionSyncTarget::Remote { target_id, .. } => Some(target_id.as_str()),
        };
        let (tabs, focused) = match runtime.lock() {
            Ok(guard) => (
                guard.process_info_attached_tabs(target),
                guard.process_info_focused_pane(target),
            ),
            Err(_) => return false,
        };
        drop(runtime);
        let panes = replica
            .state
            .layouts
            .iter()
            .filter(|layout| {
                let id = target
                    .map(|target| replica::remote_tab_id(target, &layout.tab_id))
                    .unwrap_or_else(|| layout.tab_id.clone());
                tabs.contains(&id)
            })
            .take(crate::runtime::ATTACHED_TAB_LIMIT)
            .filter_map(|layout| {
                let focused = focused
                    .as_deref()
                    .filter(|id| layout.panes.iter().any(|pane| pane.pane_id == *id))
                    .unwrap_or(&layout.focused_pane_id);
                replica
                    .state
                    .panes
                    .iter()
                    .find(|pane| pane.pane_id == focused)
                    .map(|pane| (pane.pane_id.clone(), pane.agent_status.clone()))
            })
            .collect::<Vec<_>>();
        if panes != self.desired.panes || self.connection != connection {
            self.desired = Request {
                generation: self.desired.generation.wrapping_add(1),
                panes,
            };
            self.connection = connection;
        }
        let answer = self.reader.poll(self.desired.clone());
        let mut changed = false;
        for pane in &mut replica.state.panes {
            if !self.desired.panes.iter().any(|(id, _)| *id == pane.pane_id) {
                changed |= pane.foreground_process.take().is_some();
            }
        }
        if let Some((request, names)) = answer.filter(|(request, _)| *request == self.desired) {
            let _ = request;
            for (id, name) in names {
                if let Some(pane) = replica
                    .state
                    .panes
                    .iter_mut()
                    .find(|pane| pane.pane_id == id)
                    && pane.foreground_process != name
                {
                    pane.foreground_process = name;
                    changed = true;
                }
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_reader_shutdown_joins_current_request_and_cancels_remaining_panes() {
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let herdr = crate::fake_herdr::FakeHerdr::start("process-owner", move |method, params| {
            assert_eq!(method, "pane.process_info");
            started_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            json!({"type":"pane_process_info", "process_info":{"pane_id":params["pane_id"], "foreground_processes":[]}})
        });
        let runtime = Arc::new(std::sync::Mutex::new(Runtime::new(serde_json::from_value(json!({"schema_version":crate::model::SCHEMA_VERSION, "node_id":"test-node", "herdr_socket_path":null, "app_state_path":"/tmp/hide-process-owner-unused.json"})).unwrap(), crate::environment::EnvironmentReport { statuses: Vec::new(), home_path: None, codex_home: None }, std::sync::Arc::new(hide_node::Local::of_process()), crate::node::test_devices())));
        let context = SessionSyncContext::remote(
            "fixture",
            "Fixture",
            herdr.engine(),
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
        );
        let mut reader = ProcessReader::new(&context);
        let cancelled = Arc::clone(&reader.cancelled);
        let request = Request {
            generation: 1,
            panes: (1..=5)
                .map(|i| (format!("w1:p{i}"), "idle".into()))
                .collect(),
        };
        assert!(reader.reader.poll(request).is_none());
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let dropped = std::thread::spawn(move || drop(reader));
        while !cancelled.load(Ordering::Acquire) {
            std::thread::yield_now();
        }
        release_tx.send(()).unwrap();
        dropped.join().unwrap();
        assert_eq!(herdr.methods(), ["pane.process_info"]);
        assert!(
            Arc::strong_count(&runtime) > 0,
            "the runtime is still alive when its reader ends"
        );
    }
}
