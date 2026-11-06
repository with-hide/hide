//! The catalog's bounded node read; topology never waits for its answer.
use super::*;
use crate::reader::BackgroundRead;

#[derive(Clone, PartialEq)]
pub(super) struct Request {
    // Facts depend on paths and the connection, not mutable agent/PR display
    // metadata. Publication separately fences the current catalog inputs.
    pub connection: u64,
    pub paths: BTreeSet<String>,
}

type Answer = (Request, Result<workspace::PathIndex, String>);

pub(super) struct PathReader {
    reader: BackgroundRead<Request, Answer>,
    cancelled: Arc<AtomicBool>,
}

impl PathReader {
    pub(super) fn new(context: &SessionSyncContext, wake: Option<RepublishWaker>) -> Self {
        let node = context.node().cloned();
        let owner = context.runtime.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        // This reader asks its node's paths on every refresh, so it owns
        // what was logged of the Projects that node could not read.
        let unread = std::sync::Mutex::new(workspace::UnreadProjects::default());
        let mut reader = BackgroundRead::new(
            CATALOG_REFRESH_INTERVAL,
            Duration::ZERO,
            move |request: &Request| {
                let result = node
                    .as_ref()
                    .ok_or_else(|| "local catalog has no node".to_owned())
                    .and_then(|node| {
                        workspace::PathIndex::ask_while(
                            node.as_ref(),
                            request.paths.clone(),
                            || {
                                !worker_cancelled.load(Ordering::Acquire)
                                    && owner.upgrade().is_some()
                            },
                        )
                    });
                if let (Ok(index), Ok(mut unread)) = (&result, unread.lock()) {
                    unread.record(index);
                }
                (request.clone(), result)
            },
        );
        if let Some(wake) = wake {
            reader.on_ready(move || wake.wake());
        }
        Self { reader, cancelled }
    }

    /// The caller supplies its latest inputs each time. BackgroundRead holds
    /// one active request and starts only that latest request when it ends.
    pub(super) fn poll(
        &mut self,
        desired: Request,
    ) -> Option<Result<workspace::PathIndex, String>> {
        self.reader
            .poll(desired.clone())
            .filter(|(request, _)| *request == desired)
            .map(|(_, result)| result)
    }
}

impl Drop for PathReader {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        self.reader.join_pending();
    }
}
