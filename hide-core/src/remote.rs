//! What the core names of a device: reaching it is the node's
//! (`hide_node::ssh`, behind `hide_node_link::device`), and the core reads
//! the device's session through its engine like this machine's.

pub use hide_node_link::device::{
    Arrived, Attempt, Cancel, CapabilityReport, CapabilityResult, CapabilityState, DEFAULT_CLI_DIR,
    DEFAULT_HELPER_ROOT, DeviceConnector, DeviceTransport, DialedTransport, EstablishError,
    Established, HOST_CONSENT_CARRIED_FROM, HOST_CONSENT_CONTRACT, NodeReady, OnClose,
    RemoteConnectionState, RemoteDiagnostic, RemoteError, RemoteHostIdentity, RemoteResult,
    RemoteStage, SshFailure, SshHostListing, SshProblem, Wake, WslListing,
};
