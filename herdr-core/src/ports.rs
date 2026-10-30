//! Which TCP ports the machine is listening on, and where each listener was
//! started from.
//!
//! This reads the system, and nothing more: deciding which pane a listener
//! belongs to is [`attributed_ports`]'s job, and it is pure, so the rule can be
//! tested without a server to point it at.
//!
//! System reads happen on the reader's own worker thread, never on the
//! session-sync coordinator thread that applies Herdr's events (PRD
//! instant-pane-topology D-13), never while the runtime mutex is held and
//! never on a per-event path. Unix uses `lsof`; Windows uses the platform's
//! bounded native listener observation. [`PortsReader::read_if_due`] starts a
//! read only once a refresh window has lapsed and hands its answer back on a
//! later wake.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use hide_node_link::protocol::Call;

use crate::model::{ListeningPortSnapshot, ListeningPortsSnapshot, ServerEndpointSnapshot};
use crate::node_access::{NodeLink, call_as};

/// How stale the port list may be.
///
/// This is the window R12 measures a stopped listener's disappearance against.
/// Wide enough that two `lsof` calls every window are nowhere near a per-tick
/// fork, short enough that starting a dev server shows up while the operator
/// is still looking at the pane.
const REFRESH_INTERVAL: Duration = Duration::from_secs(5);

/// The node bounds each `lsof` at ten seconds, and there are two.
const READ_TIMEOUT: Duration = Duration::from_secs(30);

pub struct PortsReader {
    inner: crate::reader::BackgroundRead<(), ListeningPortsSnapshot>,
}

impl PortsReader {
    /// Reads the listeners of `node`'s machine.
    pub fn new(node: Arc<dyn NodeLink>) -> Self {
        // The refresh window starts when a sample is complete, so a slow
        // system read cannot make the next wake look due at once.
        Self {
            inner: crate::reader::BackgroundRead::new(
                REFRESH_INTERVAL,
                Duration::ZERO,
                move |_: &()| read_now(node.as_ref()),
            ),
        }
    }

    pub fn read_if_due(&mut self) -> Option<ListeningPortsSnapshot> {
        self.inner.poll(())
    }
}

/// The listeners of `node`'s machine right now, for a decision that cannot
/// use the last sample. Waits on the node, so only a worker thread may call
/// it. A node that cannot be asked is an unavailable read.
pub(crate) fn read_now(node: &dyn NodeLink) -> ListeningPortsSnapshot {
    match call_as::<hide_node_link::ports::ListeningPorts>(node, Call::ListeningPorts, READ_TIMEOUT)
    {
        Ok(read) => ListeningPortsSnapshot {
            entries: read
                .entries
                .into_iter()
                .map(|entry| ListeningPortSnapshot {
                    host: entry.host,
                    port: entry.port,
                    cwd: entry.cwd,
                })
                .collect(),
            unavailable_reason: read.unavailable_reason,
        },
        Err(error) => ListeningPortsSnapshot {
            entries: Vec::new(),
            unavailable_reason: Some(error.to_string()),
        },
    }
}

/// The ports a pane is answerable for: those whose listener was started at or
/// below the pane's own working directory.
///
/// Comparing whole path components is what keeps `/srv/app` from claiming a
/// listener started in `/srv/app-staging`.
pub fn attributed_ports(pane_cwd: &str, listeners: &[ListeningPortSnapshot]) -> Vec<u16> {
    let pane_cwd = pane_cwd.trim();
    if pane_cwd.is_empty() {
        return Vec::new();
    }
    let pane_path = Path::new(pane_cwd);
    let mut ports: Vec<u16> = listeners
        .iter()
        .filter(|listener| Path::new(listener.cwd.as_str()).starts_with(pane_path))
        .map(|listener| listener.port)
        .collect();
    ports.sort_unstable();
    ports.dedup();
    ports
}

/// Reuses the same cwd attribution, retaining each observed address.
pub fn attributed_servers(
    pane_cwd: &str,
    listeners: &[ListeningPortSnapshot],
) -> Vec<ServerEndpointSnapshot> {
    if pane_cwd.trim().is_empty() {
        return Vec::new();
    }
    let mut servers: Vec<_> = listeners
        .iter()
        .filter(|listener| Path::new(&listener.cwd).starts_with(Path::new(pane_cwd)))
        .map(|listener| ServerEndpointSnapshot {
            host: listener.host.clone(),
            port: listener.port,
        })
        .collect();
    servers.sort();
    servers.dedup();
    servers
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listener(port: u16, cwd: &str) -> ListeningPortSnapshot {
        ListeningPortSnapshot {
            host: "127.0.0.1".into(),
            port,
            cwd: cwd.to_owned(),
        }
    }

    #[test]
    fn a_pane_claims_listeners_started_at_or_below_its_own_directory() {
        let listeners = [
            listener(5173, "/srv/app"),
            listener(8080, "/srv/app/api"),
            listener(9000, "/srv/other"),
        ];
        assert_eq!(attributed_ports("/srv/app", &listeners), vec![5173, 8080]);
        assert_eq!(attributed_ports("/srv/app/api", &listeners), vec![8080]);
    }

    #[test]
    fn a_sibling_directory_with_a_shared_prefix_is_not_below_the_pane() {
        // `/srv/app-staging` starts with the text `/srv/app` and is not inside
        // it, which is the whole reason attribution compares path components.
        let listeners = [listener(5173, "/srv/app-staging")];
        assert!(attributed_ports("/srv/app", &listeners).is_empty());
    }

    #[test]
    fn a_pane_with_no_working_directory_claims_nothing() {
        assert!(attributed_ports("   ", &[listener(5173, "/srv/app")]).is_empty());
    }

    /// A real native listener checks the platform boundary, not invented field
    /// output that could agree with a wrong parser or foreign-memory layout.
    #[test]
    fn a_real_listener_is_found_and_attributed_to_the_directory_it_runs_in() {
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("a loopback port is bindable");
        let port = listener
            .local_addr()
            .expect("the bound address is readable")
            .port();
        let cwd = std::env::current_dir().expect("the test process has a working directory");

        let read = settled(&mut PortsReader::new(Arc::new(
            hide_node::Local::of_process(),
        )));

        assert_eq!(
            read.unavailable_reason, None,
            "listener observation should be readable here"
        );
        let found = read
            .entries
            .iter()
            .find(|entry| entry.port == port)
            .expect("the listener this test just opened should be reported");
        assert_eq!(Path::new(found.cwd.as_str()), cwd.as_path());
        assert!(
            attributed_ports(&cwd.to_string_lossy(), &read.entries).contains(&port),
            "a listener in the pane's own directory is attributed to it"
        );

        drop(listener);
    }

    #[cfg(windows)]
    #[test]
    fn cleanup_listener_child() {
        use std::io::{Read, Write};
        if std::env::var_os("HIDE_TEST_CLEANUP_LISTENER").is_none() {
            return;
        }
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        println!(
            "READY {}\t{}",
            listener.local_addr().unwrap(),
            std::env::current_dir().unwrap().to_str().unwrap()
        );
        std::io::stdout().flush().unwrap();
        let _ = std::io::stdin().read_exact(&mut [0]);
        drop(listener);
    }

    #[cfg(windows)]
    struct CleanupListener {
        child: Option<hide_platform::process::OwnedChild>,
        output: Option<std::thread::JoinHandle<()>>,
    }

    #[cfg(windows)]
    impl Drop for CleanupListener {
        fn drop(&mut self) {
            drop(self.child.take());
            if let Some(output) = self.output.take() {
                let joined = output.join();
                if !std::thread::panicking() {
                    joined.unwrap();
                }
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn cleanup_keeps_a_checkout_in_use_when_a_listener_starts_through_another_spelling() {
        use crate::worktree_cleanup::{CheckoutFacts, read_in_use};
        use hide_platform::fs::{identity, link};
        use std::io::{BufRead, BufReader};
        use std::path::PathBuf;
        use std::process::{Command, Stdio};

        let root = tempfile::tempdir().unwrap();
        let checkout = root.path().join("Checkout");
        let cwd = checkout.join("native listener 한글");
        std::fs::create_dir_all(&cwd).unwrap();
        let canonical_checkout = identity::canonical(&checkout).unwrap();
        let canonical_cwd = identity::canonical(&cwd).unwrap();
        let alias = root.path().join("ListenerAlias");
        link::create_link(&checkout, &alias).unwrap();
        let mut spellings = vec![
            alias.join("native listener 한글"),
            PathBuf::from(format!(r"\\?\{}", alias.display())).join("native listener 한글"),
        ];
        // Case sensitivity belongs to this folder, not to the OS. Junction
        // and verbatim cases always run; alternate case applies where it is
        // another spelling of the same existing directory.
        if !identity::case_sensitive(root.path()).unwrap() {
            spellings.push(root.path().join("listeneralias/native listener 한글"));
        }
        let facts = [CheckoutFacts {
            path: checkout.clone(),
            ..Default::default()
        }];
        for spelling in spellings {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "ports::tests::cleanup_listener_child",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env("HIDE_TEST_CLEANUP_LISTENER", "1")
                .current_dir(&spelling)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit());
            let mut child = hide_platform::process::OwnedChild::spawn(&mut command).unwrap();
            let stdout = child.take_stdout().unwrap();
            let (ready, heard) = std::sync::mpsc::channel();
            let output = std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let line = line.unwrap();
                    if let Some((_, value)) = line.split_once("READY ") {
                        let (endpoint, cwd) = value.split_once('\t').unwrap();
                        let endpoint: std::net::SocketAddr = endpoint.parse().unwrap();
                        let _ = ready.send((endpoint, PathBuf::from(cwd)));
                    }
                }
            });
            let fixture = CleanupListener {
                child: Some(child),
                output: Some(output),
            };
            let (endpoint, observed_by_child) = heard
                .recv_timeout(Duration::from_secs(10))
                .expect("the owned listener announced its socket and actual cwd");
            assert!(identity::same_file(&observed_by_child, &canonical_cwd).unwrap());
            assert!(
                !observed_by_child.starts_with(&canonical_checkout),
                "the fixture must exercise the spelling that previously escaped cleanup"
            );
            drop(std::net::TcpStream::connect_timeout(&endpoint, Duration::from_secs(2)).unwrap());
            // Review and the fresh destructive-action recheck both consume
            // the real reader, not a handcrafted listening-port snapshot.
            for _ in 0..2 {
                let node = hide_node::Local::of_process();
                let sample = read_now(&node);
                assert_eq!(sample.unavailable_reason, None);
                let found = sample
                    .entries
                    .iter()
                    .find(|entry| entry.port == endpoint.port())
                    .unwrap();
                assert_eq!(Path::new(&found.cwd), canonical_cwd);
                let in_use = read_in_use(&node, &facts, Ok(sample.entries), |_| {
                    panic!("a checkout with no terminal panes has no process query")
                })
                .unwrap();
                let exclusion = in_use
                    .get(&checkout)
                    .expect("cleanup must exclude the live listener's checkout");
                assert_eq!(exclusion.code, "port");
                assert_eq!(exclusion.port, Some(endpoint.port()));
            }
            drop(fixture);
        }
        link::remove_link(&alias).unwrap();
    }

    /// The first answer: the first call starts the worker, which is joined
    /// before the answer is collected.
    fn settled(reader: &mut PortsReader) -> ListeningPortsSnapshot {
        assert!(reader.read_if_due().is_none(), "a read is never inline");
        reader.inner.join_pending();
        reader
            .read_if_due()
            .expect("the finished read is handed back")
    }

    #[test]
    fn the_first_read_is_due_and_the_next_one_inside_the_window_is_not() {
        let mut reader = PortsReader::new(Arc::new(hide_node::Local::of_process()));
        settled(&mut reader);
        assert!(reader.read_if_due().is_none());
    }
}
