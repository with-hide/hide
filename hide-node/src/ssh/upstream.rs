//! The one SSH connection a node opens to its core on another machine (PRD
//! core-host-node-remote-core D-04, D-07): an exec channel that runs the
//! core machine's attach role carries the node's link, and the screen relay
//! reaches the core's loopback port through forwards on the same connection.
//!
//! The screen machine needs no SSH server of its own, only the alias. The
//! connection checks the core machine's key against `known_hosts` exactly as
//! a device the core dials is checked, and ends when its owner drops it.

use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use hide_node_link::device::{RemoteResult, RemoteStage};
use hide_platform::ipc::LocalStream;
use russh::ChannelMsg;
use tokio::io::AsyncWriteExt;
use tokio::sync::oneshot;

use super::{RemoteLocalForward, RusshRemoteClient, SshAlias, remote_error, shell_quote};

/// Bytes carried per read in either direction of the attach channel.
const CHUNK: usize = 64 * 1024;

/// The node's connection to its core's machine.
pub struct Upstream {
    client: RusshRemoteClient,
    /// Set while a resolution of the core machine's name runs: one at a
    /// time, however long the system's resolver takes.
    resolving: Arc<AtomicBool>,
}

/// A probe of the SSH greeting, bounded as a whole by `within`, each
/// resolved address tried in turn with an even share of the time left. The
/// name is resolved on a thread of its own the probe waits for only until its
/// deadline: the system's resolver has no bound, and hangs exactly when the
/// network is down, so it never holds the caller (the node's watch, which
/// the role's end joins). That thread ends when the resolver answers; while
/// one runs, a probe answers not reachable rather than start another.
fn probe<R>(within: Duration, resolving: &Arc<AtomicBool>, resolve: R) -> bool
where
    R: FnOnce() -> io::Result<Vec<SocketAddr>> + Send + 'static,
{
    let deadline = Instant::now() + within;
    if resolving.swap(true, Ordering::SeqCst) {
        return false;
    }
    let (found, answer) = mpsc::channel();
    let running = Arc::clone(resolving);
    let started = std::thread::Builder::new()
        .name("core-resolve".to_owned())
        .spawn(move || {
            let addresses = resolve();
            running.store(false, Ordering::SeqCst);
            let _ = found.send(addresses);
        });
    if started.is_err() {
        resolving.store(false, Ordering::SeqCst);
        return false;
    }
    let Ok(Ok(addresses)) = answer.recv_timeout(within) else {
        return false;
    };
    // Each address has an even share of what is left, so one that never
    // answers (a dead IPv6 address listed first) leaves the next its turn.
    let count = addresses.len();
    addresses.into_iter().enumerate().any(|(index, address)| {
        let left = deadline.saturating_duration_since(Instant::now());
        let share = left / (count - index) as u32;
        if share.is_zero() {
            return false;
        }
        let until = Instant::now() + share;
        let Ok(mut socket) = TcpStream::connect_timeout(&address, share) else {
            return false;
        };
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() || socket.set_read_timeout(Some(left)).is_err() {
            return false;
        }
        let mut greeting = [0_u8; 4];
        socket.read_exact(&mut greeting).is_ok() && &greeting == b"SSH-"
    })
}

/// The attach role's channel, as a local stream: what the node writes to it
/// reaches the attach role's standard input, and the attach role's standard
/// output is what the node reads. The channel ends when the stream ends, and
/// the stream ends when the channel does.
pub struct AttachChannel {
    pub stream: LocalStream,
    /// What the attach role wrote to its standard error, for the diagnostic
    /// when the channel ends before a handshake.
    pub stderr: Arc<std::sync::Mutex<Vec<u8>>>,
}

/// The most of the attach role's standard error kept for a diagnostic.
const STDERR_CAP: usize = 4096;

impl Upstream {
    pub fn new(alias: SshAlias) -> RemoteResult<Self> {
        Ok(Self {
            client: RusshRemoteClient::new(alias)?,
            resolving: Arc::default(),
        })
    }

    /// The alias's host id, for diagnostics.
    pub fn host_id(&self) -> &str {
        &self.client.host.host_id
    }

    /// The alias this connection dials, as it was read.
    pub fn alias(&self) -> &SshAlias {
        &self.client.host
    }

    /// Dials the core's machine when the connection is not up, and runs
    /// `program attach [--state-dir <dir>]` there on a new exec channel.
    pub fn attach(&self, program: &str, state_dir: Option<&str>) -> RemoteResult<AttachChannel> {
        let target = self.client.host.host_id.clone();
        let mut command = format!("{} attach", shell_quote(program));
        if let Some(state_dir) = state_dir {
            command.push_str(" --state-dir ");
            command.push_str(&shell_quote(state_dir));
        }
        let client = &self.client;
        let channel = client.runtime.block_on(async {
            let permit = client
                .session_channel("node-attach", RemoteStage::Ssh)
                .await?;
            let session = client.shared_session().await?;
            let opened = super::bounded_ssh_operation(session.channel_open_session()).await;
            let channel = match opened {
                Ok(channel) => channel,
                Err(error) => {
                    client.forget_session(&session).await;
                    return Err(remote_error(
                        "node-attach",
                        &target,
                        RemoteStage::Ssh,
                        error,
                        true,
                        false,
                    ));
                }
            };
            super::bounded_ssh_operation(channel.exec(true, command))
                .await
                .map_err(|error| {
                    remote_error("node-attach", &target, RemoteStage::Ssh, error, true, false)
                })?;
            Ok::<_, hide_node_link::device::RemoteError>((channel, permit))
        })?;
        let (ours, theirs) = LocalStream::pair().map_err(|error| {
            remote_error(
                "node-attach",
                &target,
                RemoteStage::Ssh,
                error,
                false,
                false,
            )
        })?;
        let stderr = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (channel, permit) = channel;
        let (mut read_half, write_half) = channel.split();
        let to_node = theirs.duplicate();
        let shutdown = theirs.shutdown_handle();
        // The channel's output, to the node: data to the stream, standard
        // error to the diagnostic, and the stream ended with the channel.
        {
            let stderr = Arc::clone(&stderr);
            client.runtime.spawn(async move {
                let _permit = permit;
                let mut to_node = to_node;
                while let Some(message) = read_half.wait().await {
                    match message {
                        ChannelMsg::Data { data } => {
                            let mut to_node_now = to_node;
                            let written = tokio::task::spawn_blocking(move || {
                                let result = to_node_now.write_all(&data);
                                (to_node_now, result)
                            })
                            .await;
                            match written {
                                Ok((back, Ok(()))) => to_node = back,
                                _ => break,
                            }
                        }
                        ChannelMsg::ExtendedData { data, .. } => {
                            let mut kept = stderr.lock().unwrap_or_else(|p| p.into_inner());
                            let room = STDERR_CAP.saturating_sub(kept.len());
                            kept.extend_from_slice(&data[..data.len().min(room)]);
                        }
                        ChannelMsg::Eof | ChannelMsg::Close => break,
                        _ => {}
                    }
                }
                shutdown.shutdown();
            });
        }
        // The node's bytes, to the channel, until the node's end of the
        // stream closes; then the attach role reads the end of its input.
        let handle = client.runtime.handle().clone();
        std::thread::Builder::new()
            .name("node-attach-up".to_owned())
            .spawn(move || {
                let mut from_node = theirs;
                let mut writer = write_half.make_writer();
                let mut buffer = vec![0_u8; CHUNK];
                loop {
                    match from_node.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(read) => {
                            let sent = handle.block_on(async {
                                writer.write_all(&buffer[..read]).await?;
                                writer.flush().await
                            });
                            if sent.is_err() {
                                break;
                            }
                        }
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                        Err(_) => break,
                    }
                }
                let _ = handle.block_on(async {
                    let _ = writer.shutdown().await;
                    write_half.close().await
                });
                from_node.shutdown_handle().shutdown();
            })
            .map_err(|error| {
                remote_error(
                    "node-attach",
                    &target,
                    RemoteStage::Ssh,
                    error,
                    false,
                    false,
                )
            })?;
        Ok(AttachChannel {
            stream: ours,
            stderr,
        })
    }

    /// Runs `command` on the machine on a new exec channel of the same
    /// connection and answers its exit status and output. Output past `cap`
    /// bytes on either stream, or a command still running at `timeout`, is a
    /// failure: the channel is closed and the command is not waited for.
    pub fn exec(
        &self,
        operation: &'static str,
        command: &str,
        cap: usize,
        timeout: Duration,
    ) -> RemoteResult<super::RemoteCommandOutput> {
        let client = &self.client;
        let target = client.host.host_id.clone();
        client.runtime.block_on(async {
            let _permit = client.session_channel(operation, RemoteStage::Ssh).await?;
            let session = client.shared_session().await?;
            let failed = |error: String| {
                remote_error(operation, &target, RemoteStage::Ssh, error, true, false)
            };
            let run = async {
                let mut channel = super::bounded_ssh_operation(session.channel_open_session())
                    .await
                    .map_err(|error| failed(error.to_string()))?;
                super::bounded_ssh_operation(channel.exec(true, command))
                    .await
                    .map_err(|error| failed(error.to_string()))?;
                let (mut stdout, mut stderr, mut exit_status) = (Vec::new(), Vec::new(), None);
                while let Some(message) = channel.wait().await {
                    let (kept, data) = match message {
                        ChannelMsg::Data { data } => (&mut stdout, data),
                        ChannelMsg::ExtendedData { data, .. } => (&mut stderr, data),
                        ChannelMsg::ExitStatus {
                            exit_status: status,
                        } => {
                            exit_status = Some(status);
                            continue;
                        }
                        ChannelMsg::Close => break,
                        _ => continue,
                    };
                    if kept.len() + data.len() > cap {
                        let _ = channel.close().await;
                        return Err(failed(format!("the command wrote more than {cap} bytes")));
                    }
                    kept.extend_from_slice(&data);
                }
                Ok(super::RemoteCommandOutput {
                    stdout: String::from_utf8_lossy(&stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&stderr).into_owned(),
                    exit_status: exit_status.ok_or_else(|| {
                        failed("the command closed without an exit status".to_owned())
                    })?,
                })
            };
            tokio::time::timeout(timeout, run)
                .await
                .map_err(|_| failed(format!("the command did not finish in {timeout:?}")))?
        })
    }

    /// Copies `files` to the machine over one SFTP channel of the same
    /// connection, one after another, each streamed and renamed into place
    /// whole (`transfer::upload_file`); `sent` hears the bytes as each
    /// chunk is answered. The first failure stops the copy; the files
    /// already placed stay.
    pub fn upload(
        &self,
        files: &[super::transfer::FileCopy],
        sent: &(dyn Fn(u64) + Sync),
    ) -> Result<(), super::transfer::TransferError> {
        self.sftp("core-move-upload", |raw| async move {
            for file in files {
                super::transfer::upload_file(&raw, file, sent).await?;
            }
            Ok(())
        })
    }

    /// Copies `files` from the machine over one SFTP channel of the same
    /// connection, one after another, each streamed and renamed into place
    /// here whole (`transfer::download_file`), and each below the folder
    /// `into`; `received` hears the bytes as each chunk is written. The
    /// first failure stops the copy; the files already placed stay.
    pub fn download(
        &self,
        into: &std::path::Path,
        files: &[super::transfer::FileCopy],
        received: &(dyn Fn(u64) + Sync),
    ) -> Result<(), super::transfer::TransferError> {
        self.sftp("core-move-download", |raw| async move {
            for file in files {
                super::transfer::download_file(&raw, into, file, received).await?;
            }
            Ok(())
        })
    }

    /// Puts this build's programs in their version folder under
    /// `helper_root` on the machine, over one SFTP channel of the same
    /// connection, reusing the files a stopped upload already placed: a
    /// core update (PRD core-host-node-move B10). Answers the path of the
    /// build's `hided` there.
    pub fn install_build(
        &self,
        packages: &super::host::HelperPackages,
        helper_root: &str,
    ) -> Result<String, String> {
        let uname = self
            .exec(
                "core-update-platform",
                "uname -s -m",
                4096,
                Duration::from_secs(15),
            )
            .map_err(|error| error.to_string())?;
        if uname.exit_status != 0 {
            return Err(format!(
                "the machine could not report its platform: {}",
                uname.stderr.trim()
            ));
        }
        let (os, arch) = super::host::platform_of(uname.stdout.trim())?;
        self.sftp("core-update-install", |raw| async move {
            super::host::install_build(&raw, packages, &os, &arch, helper_root)
                .await
                .map_err(|error| super::transfer::TransferError::Remote(error.to_string()))
        })
        .map_err(|error| error.to_string())
    }

    /// Runs `copy` on one SFTP channel of the same connection.
    fn sftp<'a, T, F, Fut>(
        &self,
        operation: &'static str,
        copy: F,
    ) -> Result<T, super::transfer::TransferError>
    where
        F: FnOnce(std::sync::Arc<russh_sftp::client::RawSftpSession>) -> Fut,
        Fut: std::future::Future<Output = Result<T, super::transfer::TransferError>> + 'a,
    {
        use super::transfer::TransferError;
        let client = &self.client;
        client.runtime.block_on(async {
            let remote = |error: String| TransferError::Remote(error);
            let _permit = client
                .session_channel(operation, RemoteStage::Sftp)
                .await
                .map_err(|error| remote(error.to_string()))?;
            let (session, mut ended) = client
                .shared_session_heard()
                .await
                .map_err(|error| remote(error.to_string()))?;
            let channel = super::bounded_ssh_operation(session.channel_open_session())
                .await
                .map_err(|error| {
                    remote(format!("the SFTP channel could not be opened: {error}"))
                })?;
            channel
                .request_subsystem(true, "sftp")
                .await
                .map_err(|error| remote(format!("SFTP is not available: {error}")))?;
            let raw = std::sync::Arc::new(russh_sftp::client::RawSftpSession::new(
                channel.into_stream(),
            ));
            // The limit for a request on a connection that looks up but no
            // longer answers; a connection that ends ends the copy at once,
            // since the SFTP session keeps waiting on its requests after its
            // stream closed.
            raw.set_timeout(30);
            let work = async {
                match raw.init().await {
                    Ok(_) => copy(std::sync::Arc::clone(&raw)).await,
                    Err(error) => Err(remote(format!("SFTP did not start: {error}"))),
                }
            };
            let copied = tokio::select! {
                copied = work => copied,
                _ = ended.wait_for(|ended| *ended) => {
                    Err(remote("the connection closed during the copy".to_owned()))
                }
            };
            let _ = raw.close_session();
            copied
        })
    }

    /// A loopback port on this machine whose connections reach `port` on
    /// the core machine's loopback, over the same connection.
    pub fn forward(&self, port: u16) -> RemoteResult<RemoteLocalForward> {
        let (_cancel, canceled) = oneshot::channel();
        // The sender is kept alive for the call: a dropped sender reads as a
        // cancel.
        let forward = self.page_forward(
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
            None,
            true,
            canceled,
        );
        drop(_cancel);
        forward
    }

    /// A page's way to `remote` on the core machine's loopback, over the
    /// same connection, as a device's page route reaches its device
    /// (`start_local_workspace_forward`): `alternate` is the other address
    /// family a `localhost` page may resolve to, and a numeric HTTPS host
    /// keeps its address for the certificate.
    pub fn page_forward(
        &self,
        remote: SocketAddr,
        alternate: Option<SocketAddr>,
        preserve_numeric_host: bool,
        canceled: oneshot::Receiver<()>,
    ) -> RemoteResult<RemoteLocalForward> {
        self.client.start_local_workspace_forward(
            remote,
            alternate,
            preserve_numeric_host,
            canceled,
        )
    }

    /// Whether the connection still answers: one SSH ping, answered
    /// within `within`. A connection that is not up is not alive. Asked after
    /// the machine woke or its network changed, when a connection that looks
    /// up may lead nowhere.
    pub fn alive(&self, within: Duration) -> bool {
        let connection = &self.client.connection;
        self.client.runtime.block_on(async {
            let session = connection.session.lock().await.clone();
            let Some(session) = session.filter(|session| !session.is_closed()) else {
                return false;
            };
            let answered = tokio::time::timeout(within, session.send_ping())
                .await
                .is_ok_and(|sent| sent.is_ok());
            answered && !session.is_closed()
        })
    }

    /// Whether the core machine's SSH server answers now: its name resolved,
    /// a connection to its port, and the server's `SSH-` greeting read, all
    /// within `within`. A server that takes the connection and closes it, a
    /// port nothing answers, or a name the resolver has not answered in time
    /// is not reachable.
    pub fn reachable(&self, within: Duration) -> bool {
        let host = &self.client.host;
        let (name, port) = (host.hostname.clone(), host.port);
        probe(within, &self.resolving, move || {
            (name.as_str(), port)
                .to_socket_addrs()
                .map(|found| found.collect())
        })
    }

    /// Ends the connection and every channel on it.
    pub fn close(&self) {
        self.client.disconnect();
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;

    use super::*;

    /// An address that takes the connection and never greets (a dead
    /// address listed first) leaves the next address its own share of the
    /// deadline, so a live one behind it still answers.
    #[test]
    fn a_silent_first_address_leaves_the_next_its_share_of_the_probe() {
        let silent = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let live = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addresses = vec![silent.local_addr().unwrap(), live.local_addr().unwrap()];
        let greeting = std::thread::spawn(move || {
            let (mut stream, _) = live.accept().unwrap();
            stream.write_all(b"SSH-2.0-fixture\r\n").unwrap();
        });
        let resolving = Arc::new(AtomicBool::new(false));
        let started = Instant::now();
        assert!(probe(Duration::from_millis(600), &resolving, move || Ok(
            addresses
        )));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
        greeting.join().unwrap();
        drop(silent);
    }

    /// A resolver that does not answer holds the probe only until its
    /// deadline, and no second resolution starts while it runs; once it has
    /// answered, a probe reads the server's greeting again.
    #[test]
    fn a_resolver_that_hangs_holds_the_probe_only_until_its_deadline() {
        let resolving = Arc::new(AtomicBool::new(false));
        let (release, released) = mpsc::channel::<()>();
        let within = Duration::from_millis(200);
        let started = Instant::now();
        let reached = probe(within, &resolving, move || {
            let _ = released.recv_timeout(Duration::from_secs(30));
            Ok(Vec::new())
        });
        assert!(!reached);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );

        let asked = Arc::new(AtomicBool::new(false));
        let second = Arc::clone(&asked);
        assert!(!probe(within, &resolving, move || {
            second.store(true, Ordering::SeqCst);
            Ok(Vec::new())
        }));
        assert!(!asked.load(Ordering::SeqCst), "a second resolution started");

        drop(release);
        let deadline = Instant::now() + Duration::from_secs(10);
        while resolving.load(Ordering::SeqCst) {
            assert!(Instant::now() < deadline, "the resolution never ended");
            std::thread::yield_now();
        }
        let server = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = server.local_addr().unwrap();
        let greeting = std::thread::spawn(move || {
            let (mut stream, _) = server.accept().unwrap();
            stream.write_all(b"SSH-2.0-fixture\r\n").unwrap();
        });
        assert!(probe(Duration::from_secs(5), &resolving, move || Ok(vec![
            address
        ])));
        greeting.join().unwrap();
    }
}
