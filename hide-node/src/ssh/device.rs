//! The SSH transport behind `hide_node_link::device`: the node that holds
//! the account's SSH configuration and keys reaches each registered device
//! with it, and the core sees only the traits.

use super::host::{self, HelperPackages, PaneEventsSlot, PaneHook, TerminalHook};
use super::hosts::{Resolve, ssh_g};
use super::*;
use hide_node_link::attachments::AttachmentFile;
use hide_node_link::device::{
    DeviceConnector, DeviceTransport, DialedTransport, EstablishError, Established, HostConsent,
    OnClose, SshHostListing,
};

/// Opens the SSH transport to a device by alias. It carries this build's
/// device packages, which a transport installs on a device that lacks them.
pub struct Connector {
    packages: HelperPackages,
    /// What resolves an alias for Add device's list: `ssh -G`, or a test's
    /// answers.
    resolve: Resolve,
    /// Where each device's pane events go; none for a connector whose
    /// devices' panes do not reach this process.
    panes: Option<PaneEventsSlot>,
    /// Where each device's terminals' output and reports go; none for a
    /// connector whose devices' terminals do not reach this process.
    terminals: Option<Arc<dyn crate::terminal::device::DeviceSink>>,
    /// The helper root the core installs under on each device, whose pinned
    /// Herdr the status read asks first.
    helper_root: Option<String>,
    herdr_program: PathBuf,
}

impl Connector {
    /// `helper_dir` is the folder holding the builds devices run; `None`
    /// leaves every device's node `unsupported` with that reason.
    pub fn new(helper_dir: Option<PathBuf>) -> Self {
        Self {
            packages: HelperPackages::new(helper_dir),
            resolve: ssh_g(PathBuf::from("ssh")),
            panes: None,
            terminals: None,
            helper_root: None,
            herdr_program: PathBuf::from("herdr"),
        }
    }

    /// The helper root the core installs under on each device.
    pub fn with_helper_root(mut self, root: String) -> Self {
        self.helper_root = Some(root);
        self
    }

    pub fn with_herdr_program(mut self, program: PathBuf) -> Self {
        self.herdr_program = program;
        self
    }

    /// Each device's panes' terminals flow inside its node link, and their
    /// output and reports go to `sink`.
    pub fn with_terminals(mut self, sink: Arc<dyn crate::terminal::device::DeviceSink>) -> Self {
        self.terminals = Some(sink);
        self
    }

    /// Each device's node serves its panes' `hide` over its link, and what
    /// they send goes to `events`.
    pub fn with_pane_events(mut self, events: PaneEventsSlot) -> Self {
        self.panes = Some(events);
        self
    }

    /// Resolves aliases for Add device's list with `ssh` instead of the
    /// `ssh` on `PATH`.
    pub fn with_ssh_program(self, ssh: PathBuf) -> Self {
        self.with_resolve(ssh_g(ssh))
    }

    /// Resolves aliases for Add device's list with `resolve`, so a test
    /// decides each answer and no child races the `ssh -G` deadline.
    pub fn with_resolve(mut self, resolve: Resolve) -> Self {
        self.resolve = resolve;
        self
    }
}

impl DeviceConnector for Connector {
    fn transport(
        &self,
        home: &Path,
        node: &str,
        alias: &str,
        herdr_socket: Option<String>,
    ) -> Result<Arc<dyn DialedTransport>, String> {
        self.transport_session(home, node, alias, herdr_socket, None)
    }

    fn transport_session(
        &self,
        home: &Path,
        node: &str,
        alias: &str,
        herdr_socket: Option<String>,
        session: Option<String>,
    ) -> Result<Arc<dyn DialedTransport>, String> {
        if let Some(session) = &session {
            super::profiles::validate_session(session)?;
            if herdr_socket.is_some() {
                return Err(
                    "A saved session cannot also select a different Herdr socket".to_owned(),
                );
            }
        }
        super::profiles::Target::parse(alias)?;
        Ok(Arc::new(SshDevice {
            client: Arc::new(LazyClient {
                home: home.to_owned(),
                target: alias.to_owned(),
                herdr_socket,
                session,
                helper_root: self.helper_root.clone(),
                initialized: std::sync::OnceLock::new(),
            }),
            packages: self.packages.clone(),
            panes: self.panes.clone().map(|events| PaneHook {
                node: node.to_owned(),
                events,
            }),
            terminals: self.terminals.clone().map(|sink| TerminalHook {
                node: node.to_owned(),
                sink,
            }),
        }))
    }

    fn ssh_hosts(&self, home: &Path, registered: &[String], stop: &AtomicBool) -> SshHostListing {
        super::hosts::list(&self.resolve, home, registered, stop)
    }

    fn herdr_profiles(
        &self,
        home: &Path,
        registered: &[String],
        stop: &AtomicBool,
    ) -> Result<hide_node_link::device::HerdrProfileListing, String> {
        super::profiles::list(&self.herdr_program, home, registered, stop)
    }
}

/// One device reached over SSH.
pub struct SshDevice {
    client: Arc<LazyClient>,
    packages: HelperPackages,
    panes: Option<PaneHook>,
    terminals: Option<TerminalHook>,
}

impl SshDevice {
    /// The already initialized SSH client, for a ready route's forwards.
    /// A snapshot or route read never resolves configuration or starts I/O.
    pub fn client(&self) -> Option<&Arc<RusshRemoteClient>> {
        self.client
            .initialized
            .get()
            .and_then(|result| result.as_ref().ok())
    }
}

/// Transport construction happens under Runtime; file reads and the SSH
/// runtime are deferred until a session or helper worker uses the transport.
struct LazyClient {
    home: PathBuf,
    target: String,
    herdr_socket: Option<String>,
    session: Option<String>,
    helper_root: Option<String>,
    initialized: std::sync::OnceLock<Result<Arc<RusshRemoteClient>, String>>,
}

impl LazyClient {
    fn resolve(&self) -> Result<&Arc<RusshRemoteClient>, String> {
        self.initialized
            .get_or_init(|| {
                let resolved =
                    SshAlias::from_target_file(&self.home.join(".ssh/config"), &self.target)
                        .map_err(|error| {
                            format!(
                                "SSH target {} could not be resolved: {}",
                                self.target,
                                error.diagnostic().reason
                            )
                        })?;
                RusshRemoteClient::new(resolved)
                    .map(|client| {
                        Arc::new(
                            client
                                .with_herdr_socket(self.herdr_socket.clone())
                                .with_herdr_session(self.session.clone())
                                .with_helper_root(self.helper_root.clone()),
                        )
                    })
                    .map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(Clone::clone)
    }
}

struct LazyApiConnector(Arc<LazyClient>);

impl ApiConnector for LazyApiConnector {
    fn connect_until(
        &self,
        deadline: std::time::Instant,
    ) -> Result<Box<dyn hide_herdr_client::ApiStream>, ApiError> {
        // A proof cannot spend its short budget resolving account files or
        // starting a runtime. The session owner initializes this transport.
        self.0
            .initialized
            .get()
            .and_then(|result| result.as_ref().ok())
            .ok_or_else(|| {
                ApiError::Transport(
                    "SSH transport is not initialized for a deadline-bound request".to_owned(),
                )
            })?
            .herdr_api_connector()
            .connect_until(deadline)
    }

    fn connect(&self) -> Result<Box<dyn hide_herdr_client::ApiStream>, ApiError> {
        self.0
            .resolve()
            .map_err(ApiError::Transport)?
            .herdr_api_connector()
            .connect()
    }
}

impl DeviceTransport for SshDevice {
    fn herdr_api_connector(&self) -> Arc<dyn ApiConnector> {
        Arc::new(LazyApiConnector(Arc::clone(&self.client)))
    }

    fn cached_herdr_socket(&self) -> Option<String> {
        self.client()
            .and_then(|client| client.cached_herdr_socket())
    }

    fn cached_herdr_version(&self) -> Option<String> {
        self.client()
            .and_then(|client| client.cached_herdr_version())
    }

    fn into_any(self: Arc<Self>) -> Arc<dyn std::any::Any + Send + Sync> {
        self
    }
}

impl DialedTransport for SshDevice {
    fn capability_test(&self, operation_id: &str, check: SnapshotCheck<'_>) -> CapabilityReport {
        match self.client.resolve() {
            Ok(client) => client.staged_capability_test(operation_id, check),
            Err(error) => {
                let mut report = CapabilityReport::new(
                    operation_id,
                    RemoteHostIdentity {
                        host_id: format!("ssh:{}", self.client.target),
                        alias: self.client.target.clone(),
                        hostname: self.client.target.clone(),
                        port: 22,
                    },
                );
                report.fail(RemoteStage::Alias, error, false, true);
                report
            }
        }
    }

    fn establish(
        &self,
        consent: &HostConsent,
        retirement_projects: &[String],
        on_close: OnClose,
    ) -> Result<Established, EstablishError> {
        host::establish(
            self.client.resolve().map_err(EstablishError::Helper)?,
            &self.packages,
            consent,
            retirement_projects,
            self.panes.clone(),
            self.terminals.clone(),
            on_close,
        )
    }

    fn stage_attachments(
        &self,
        request_id: &str,
        files: &[AttachmentFile],
        cancelled: &AtomicBool,
    ) -> Result<Vec<String>, String> {
        self.client
            .resolve()?
            .stage_attachments(request_id, files, cancelled)
    }

    fn remove_attachments(&self, request_id: &str, files: &[AttachmentFile]) {
        if let Some(client) = self.client() {
            client.remove_attachments(request_id, files);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registering_a_transport_defers_configuration_failure_until_worker_connect() {
        let home = tempfile::tempdir().unwrap();
        // A directory in place of the file fails a real configuration read
        // on every platform, before any SSH connection can be attempted.
        std::fs::create_dir_all(home.path().join(".ssh/config")).unwrap();
        let device = Connector::new(None)
            .transport_session(
                home.path(),
                "fixture",
                "fixture.invalid",
                None,
                Some("review".to_owned()),
            )
            .unwrap();
        assert!(device.cached_herdr_version().is_none());
        assert!(device.cached_herdr_socket().is_none());
        let connector = device.herdr_api_connector();
        let proof = connector.connect_until(std::time::Instant::now() + Duration::from_secs(1));
        assert!(
            matches!(proof, Err(ApiError::Transport(reason)) if reason.contains("not initialized"))
        );
        let error = match connector.connect() {
            Err(error) => error,
            Ok(_) => panic!("unreadable configuration must refuse before SSH"),
        };
        assert!(error.to_string().contains("could not be resolved"));
        assert!(device.cached_herdr_socket().is_none());
    }
}
