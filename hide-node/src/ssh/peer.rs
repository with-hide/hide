//! Another machine that runs Hide, as a device of this core (PRD
//! device-connect-iroh D-61): the core's node runs there as a process of
//! this core's own build, started and supervised by that machine's Hide,
//! and its link travels on the connection the two machines keep.
//!
//! This crate does not link the connection. Whoever does hands in a
//! [`PeerDial`], which has the other machine run this build's node and
//! answers the node link's two byte streams; from there the link is any
//! device's: the same reader, calls, pane holders and terminals.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use hide_node_link::call_as;
use hide_node_link::device::{
    Attempt, Cancel, CapabilityReport, DeviceTransport, DialedTransport, EstablishError,
    Established, HostConsent, HostIdentity, NodeReady, RemoteHostIdentity, RemoteStage, Upload,
    Wake,
};
use hide_node_link::protocol::{Call, Hello, PROTOCOL_VERSION};
use serde_json::json;

use super::{HelperPackages, PaneHook, RemoteHost, TerminalHook, lock_recover};

/// Opens this core's node on another Hide's machine.
pub trait PeerDial: Send + Sync {
    /// Has the machine paired under the connection key `key` run this
    /// core's node, from the build `packages` holds for that machine's
    /// platform, and opens the node's link. `device` is the id the core
    /// knows that machine's panes under, which what the node reports of
    /// them beside its link is applied to. `wake` says whether the attempt
    /// may start a machine that is off, which only a WSL distribution of
    /// this PC can be (PRD device-connect-iroh B43). Blocking and bounded;
    /// a raised `cancel` ends it at its next wait.
    fn open(
        &self,
        key: &str,
        device: &str,
        packages: &HelperPackages,
        wake: Wake,
        cancel: &Cancel,
    ) -> Result<Opened, EstablishError>;
}

/// A node link another machine carries, as its two byte streams.
pub struct Opened {
    pub reading: Box<dyn Read + Send>,
    pub writing: Box<dyn Write + Send>,
    /// Ends the link's stream from outside, which ends a read that waits.
    pub close: Box<dyn FnOnce() + Send>,
    /// The machine's name, as it names itself on the connection.
    pub name: String,
    /// This build was placed on the machine on this connection, so its
    /// node's first start may be a new program's first start.
    pub placed: bool,
}

pub struct PeerDevice {
    node: String,
    key: String,
    dial: Arc<dyn PeerDial>,
    packages: HelperPackages,
    panes: Option<PaneHook>,
    terminals: Option<TerminalHook>,
    link: Mutex<Option<RemoteHost>>,
}

impl PeerDevice {
    pub(in crate::ssh) fn new(
        node: &str,
        key: &str,
        dial: Arc<dyn PeerDial>,
        packages: HelperPackages,
        panes: Option<PaneHook>,
        terminals: Option<TerminalHook>,
    ) -> Self {
        Self {
            node: node.to_owned(),
            key: key.to_owned(),
            dial,
            packages,
            panes,
            terminals,
            link: Mutex::new(None),
        }
    }

    fn target(&self) -> String {
        format!("peer:{}", self.node)
    }

    /// The identity a connection to this machine has: its key, which the
    /// connection itself proves.
    fn identity(&self, name: &str) -> HostIdentity {
        HostIdentity {
            user: String::new(),
            hostname: name.to_owned(),
            port: 0,
            machine_id: Some(self.node.clone()),
            host_key_sha256: self.key.clone(),
        }
    }
}

impl DeviceTransport for PeerDevice {}

impl DialedTransport for PeerDevice {
    fn capability_test(&self, operation_id: &str) -> CapabilityReport {
        let mut report = CapabilityReport::new(
            operation_id,
            RemoteHostIdentity {
                host_id: self.target(),
                alias: self.node.clone(),
                hostname: self.node.clone(),
                port: 0,
            },
        );
        // The test reads the link the device has; it dials nothing.
        let link = lock_recover(&self.link).clone();
        let Some(link) = link.filter(|link| link.closed_reason().is_none()) else {
            report.fail(
                RemoteStage::Ssh,
                "This machine is not connected",
                true,
                false,
            );
            return report;
        };
        report.pass(RemoteStage::Ssh, "connected to its Hide");
        report.pass(RemoteStage::Auth, "paired");
        match link.protocol() {
            Some(protocol) => report.pass(RemoteStage::Protocol, format!("protocol {protocol}")),
            None => {
                report.fail(
                    RemoteStage::Protocol,
                    "This Hide's node there has not greeted the core",
                    true,
                    false,
                );
                return report;
            }
        }
        report.pass(RemoteStage::Pty, "through the node link");
        if !super::check_git(&link, &mut report) {
            return report;
        }
        report.connected();
        report
    }

    fn establish(
        &self,
        _consent: &HostConsent,
        _retirement_projects: &[String],
        attempt: Attempt,
    ) -> Result<Established, EstablishError> {
        let Attempt {
            wake,
            cancel,
            on_close,
            ..
        } = attempt;
        let since = Instant::now();
        let target = self.target();
        let stage = |name: &str| super::establish_stage(&target, name, since);
        let ended = || EstablishError::Helper(super::ATTEMPT_ENDED.to_owned());
        stage("node");
        let opened = self
            .dial
            .open(&self.key, &self.node, &self.packages, wake, &cancel)?;
        if cancel.is_cancelled() {
            (opened.close)();
            return Err(ended());
        }
        let identity = self.identity(&opened.name);
        let host = super::over_stream(
            &target,
            opened.reading,
            opened.writing,
            opened.close,
            self.panes.clone(),
            on_close,
        )
        .map_err(|error| {
            EstablishError::Helper(format!("This machine's link could not start: {error}"))
        })?;
        stage("hello");
        let hello: Hello = call_as(&host, Call::Hello, super::hello_timeout(opened.placed))
            .map_err(|error| {
                host.close("the node did not answer Hello");
                EstablishError::Helper(format!("This Hide's node there did not start: {error}"))
            })?;
        // The node there is this build's own program: nothing but this
        // build's protocol is its.
        if hello.protocol != PROTOCOL_VERSION {
            host.close("node protocol mismatch");
            return Err(EstablishError::Helper(format!(
                "The node on this machine speaks protocol {}, this Hide speaks {PROTOCOL_VERSION}; it is not this build's node",
                hello.protocol
            )));
        }
        let readers = hello.readers();
        for reason in readers.diagnostics() {
            crate::diagnostic!(json!({
                "component": "remote_host",
                "kind": "host.reader_advertisement_refused",
                "target": target,
                "reason": reason,
            }));
        }
        host.take_readers(hello.protocol, readers)
            .map_err(EstablishError::Helper)?;
        *lock_recover(&self.link) = Some(host.clone());
        stage("holders");
        let terminals = super::start_holders_at(&host, self.terminals.clone());
        if cancel.is_cancelled() {
            host.close(super::ATTEMPT_ENDED);
            return Err(ended());
        }
        stage("ready");
        Ok(Established {
            node: NodeReady {
                host: Arc::new(host),
                hello,
                terminals,
            },
            identity,
            installed: opened.placed,
            helper_path: String::new(),
            upload: Upload::default(),
            kit: None,
        })
    }
}
