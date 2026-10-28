//! Agent labels, made by the core (PRD labels-in-hided).
//!
//! What each agent pane is doing (the session's goal, one line for its turn,
//! how the turn ended) used to come from a separate Herdr plugin through
//! pane tokens. The core now makes them itself: each Herdr
//! server's session-sync coordinator owns a [`worker::LabelWorker`], the
//! core owns one [`analyzer::LabelAnalyzer`] for all of them, and
//! [`store::LabelStore`] keeps `labels.json` beside the daemon's state.
//! `docs/status-model.md` owns how labels reach the screen.

pub(crate) mod analysis;
pub(crate) mod analyzer;
mod context_label;
pub(crate) mod facts;
pub(crate) mod generator;
mod import;
pub(crate) mod input;
pub(crate) mod overlay;
pub(crate) mod record;
pub(crate) mod store;
pub(crate) mod worker;

#[cfg(test)]
mod tests;

/// The store key of a device's or linked node's records, which stay in
/// memory (`store`).
pub(crate) fn device_target(device: &str) -> String {
    format!("device:{device}")
}

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use hide_node_link::protocol::Call;
use hide_session::label_transcript::{LabelTranscript, LabelTranscriptRequest};

use crate::node_access::{LinkError, NodeLink, call_as};
use crate::runtime::Runtime;
use analyzer::LabelAnalyzer;
use generator::LockPlace;
use store::LabelStore;
use worker::{LabelWorker, ReadFailure, TranscriptSource, Wake, WorkerConfig};

/// A device read waits at most this long for its helper.
const DEVICE_READ_TIMEOUT: Duration = Duration::from_secs(15);

/// What every coordinator's worker shares: the store and the analyzer.
pub(crate) struct LabelServices {
    pub(crate) ledger: Arc<Mutex<Option<crate::links::worker::LinkClient>>>,
    pub(crate) suggestions: worker::SuggestionsHandle,
    pub(crate) store: Arc<LabelStore>,
    pub(crate) analyzer: Arc<LabelAnalyzer>,
    /// How the agent Hide AI runs on stood at the analyzer's last job.
    pub(crate) standing: Arc<crate::ai::AiStanding>,
    /// The operator's submits the runtime records for every worker.
    pub(crate) input: Arc<input::OperatorInput>,
    /// Wakes this Mac's worker, for news that reaches the runtime rather
    /// than its coordinator (new pull request creation times).
    local_wake: Mutex<Option<Wake>>,
    home: Option<PathBuf>,
}

impl LabelServices {
    /// Opens the store beside `state_dir` and starts the analyzer, which
    /// reads the operator's provider choice from the runtime before each
    /// analysis, or from the saved file while the runtime has not read it
    /// yet, so no conversation goes to a provider the operator did not
    /// choose.
    pub(crate) fn start(
        state_dir: Option<&Path>,
        home: Option<PathBuf>,
        runtime: Weak<Mutex<Runtime>>,
        node: &crate::node::NodeId,
        own_node: Arc<dyn NodeLink>,
        gate: Arc<hide_ai::UsageLimits>,
        usage: Arc<dyn hide_ai::AiUsageSink>,
    ) -> Result<Self, String> {
        let store = Arc::new(LabelStore::open(state_dir, home.as_deref(), node.as_str()));
        let settings_home = home.clone();
        let standing = Arc::new(crate::ai::AiStanding::default());
        // The labels run on the logins of the machine each agent is placed
        // on: the core's own unless the operator chose a device's.
        let machines = crate::ai::AiMachines {
            own: own_node,
            devices: crate::runtime::device_links(runtime.clone()),
        };
        let analyzer = LabelAnalyzer::spawn(
            Box::new(move || analysis_settings(&runtime, settings_home.as_deref())),
            Arc::clone(&standing),
            machines,
            gate,
            usage,
        )?;
        Ok(Self {
            ledger: Arc::default(),
            suggestions: Arc::default(),
            store,
            analyzer: Arc::new(analyzer),
            standing,
            input: Arc::new(input::OperatorInput::new(node.as_str())),
            local_wake: Mutex::new(None),
            home,
        })
    }

    /// The worker for this machine's Herdr server at `socket_path`, reading
    /// conversations through the core's own node and asking in `language`.
    pub(crate) fn local_worker(
        &self,
        socket_path: &Path,
        node: Arc<dyn NodeLink>,
        wake: Wake,
        language: hide_ai::OutputLanguage,
        connector: Arc<dyn hide_herdr_client::ApiConnector>,
    ) -> Result<Option<LabelWorker>, String> {
        if self.home.is_none() {
            return Ok(None);
        }
        *self
            .local_wake
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(Arc::clone(&wake));
        LabelWorker::spawn(
            WorkerConfig {
                target: self.store.node().to_owned(),
                lock: LockPlace {
                    node: own_channel(&node),
                    herdr_socket: Some(socket_path.display().to_string()),
                },
                input: Arc::clone(&self.input),
                ledger: Arc::clone(&self.ledger),
                suggestions: Arc::clone(&self.suggestions),
                language,
                start_delay: worker::START_DELAY,
            },
            Arc::clone(&self.store),
            Arc::clone(&self.analyzer),
            Arc::new(NodeTranscripts::new(own_channel(&node)).with_connector(connector)),
            wake,
        )
        .map(Some)
    }

    /// Wakes this Mac's worker, which takes the runtime's news on its wake.
    pub(crate) fn wake_local(&self) {
        let wake = self
            .local_wake
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        if let Some(wake) = wake {
            wake();
        }
    }

    /// The worker for a device's Herdr server, reading through its helper
    /// and asking in the core's `language`, not the device's.
    pub(crate) fn device_worker(
        &self,
        device_id: &str,
        runtime: Weak<Mutex<Runtime>>,
        wake: Wake,
        language: hide_ai::OutputLanguage,
        connector: Arc<dyn hide_herdr_client::ApiConnector>,
    ) -> Result<LabelWorker, String> {
        LabelWorker::spawn(
            WorkerConfig {
                target: device_target(device_id),
                // The device's node locks the server its pane service serves.
                lock: LockPlace {
                    node: device_channel(device_id, runtime.clone()),
                    herdr_socket: None,
                },
                input: Arc::clone(&self.input),
                ledger: Arc::clone(&self.ledger),
                suggestions: Arc::clone(&self.suggestions),
                language,
                start_delay: worker::START_DELAY,
            },
            Arc::clone(&self.store),
            Arc::clone(&self.analyzer),
            Arc::new(
                NodeTranscripts::new(device_channel(device_id, runtime)).with_connector(connector),
            ),
            wake,
        )
    }
}

/// The provider choice an analysis runs with: the runtime's once it has read
/// the settings, the saved file before then, and the defaults only when no
/// choice was saved or the file cannot be read, which is logged.
pub(crate) fn analysis_settings(
    runtime: &Weak<Mutex<Runtime>>,
    home: Option<&Path>,
) -> hide_ai::AiSettings {
    let loaded = runtime.upgrade().and_then(|runtime| {
        runtime
            .lock()
            .ok()
            .and_then(|guard| guard.label_ai_settings())
    });
    if let Some(settings) = loaded {
        return settings;
    }
    let Some(home) = home else {
        return hide_ai::AiSettings::default();
    };
    hide_ai::settings::load(home).unwrap_or_else(|error| {
        crate::diagnostic!(serde_json::json!({
            "component": "labels",
            "kind": "settings.unreadable",
            "message": error.to_string(),
        }));
        hide_ai::AiSettings::default()
    })
}

/// The device helper's channel when it has one, or the stable reason it does
/// not (no connection yet, a helper still starting).
pub(crate) type ChannelSource =
    Box<dyn Fn() -> Result<Arc<dyn NodeLink>, &'static str> + Send + Sync>;

/// A node's conversations, read on its machine and brought here in memory
/// only (PRD D-03); nothing of them is stored. The core's own node answers in
/// process, a device's through its helper.
pub(crate) struct NodeTranscripts {
    channel: ChannelSource,
    connector: Option<Arc<dyn hide_herdr_client::ApiConnector>>,
}

impl NodeTranscripts {
    pub(crate) fn new(channel: ChannelSource) -> Self {
        Self {
            channel,
            connector: None,
        }
    }

    fn with_connector(mut self, connector: Arc<dyn hide_herdr_client::ApiConnector>) -> Self {
        self.connector = Some(connector);
        self
    }
}

/// The core's own node, which is always there.
fn own_channel(node: &Arc<dyn NodeLink>) -> ChannelSource {
    let node = Arc::clone(node);
    Box::new(move || Ok(Arc::clone(&node)))
}

/// The registered device's helper, taken under a brief runtime lock; the
/// call itself runs outside it.
fn device_channel(device_id: &str, runtime: Weak<Mutex<Runtime>>) -> ChannelSource {
    let device_id = device_id.to_owned();
    Box::new(move || {
        let runtime = runtime.upgrade().ok_or("runtime_gone")?;
        let mut guard = runtime.lock().map_err(|_| "runtime_poisoned")?;
        guard
            .node_link(&device_id)
            .map_err(|_| "device_helper_not_ready")
    })
}

impl TranscriptSource for NodeTranscripts {
    fn resolve(
        &self,
        pane: &str,
        agent: hide_session::Agent,
    ) -> Result<Option<(String, String)>, ReadFailure> {
        let connector = self
            .connector
            .as_ref()
            .ok_or_else(|| ReadFailure::Unavailable("session_process_unavailable".into()))?;
        let params = crate::wire::pane_process_info_params(pane).map_err(ReadFailure::Refused)?;
        let value = hide_herdr_client::request_small_response(
            connector.as_ref(),
            "pane.process_info",
            params,
            Duration::from_secs(1),
        )
        .map_err(|_| ReadFailure::Unavailable("session_process_unavailable".into()))?;
        crate::wire::resumed_session_reference(value, pane, agent).map_err(ReadFailure::Refused)
    }

    fn read(&self, request: &LabelTranscriptRequest) -> Result<LabelTranscript, ReadFailure> {
        let channel =
            (self.channel)().map_err(|reason| ReadFailure::Unavailable(reason.to_owned()))?;
        call_as::<LabelTranscript>(
            channel.as_ref(),
            Call::LabelTranscript {
                request: request.clone(),
            },
            DEVICE_READ_TIMEOUT,
        )
        .map_err(|error| match error {
            LinkError::NotConnected(_) => {
                ReadFailure::Unavailable("device_helper_not_connected".to_owned())
            }
            LinkError::Busy => ReadFailure::Unavailable("device_helper_busy".to_owned()),
            LinkError::Unknown(_) => ReadFailure::Unavailable("device_helper_unknown".to_owned()),
            // A helper older than protocol 12 does not know the call; the
            // device's kit status already offers the reinstall (B15).
            LinkError::Refused(error)
                if error.code == hide_node_link::ErrorCode::InvalidRequest =>
            {
                ReadFailure::Refused("device_helper_unsupported".to_owned())
            }
            LinkError::Refused(error) if error.code == hide_node_link::ErrorCode::Busy => {
                ReadFailure::Pending(error.message)
            }
            LinkError::Refused(error) => ReadFailure::Refused(error.message),
        })
    }
}
