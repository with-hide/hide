//! Real loopback SSH boundary, with fixture-only keys and owned channel jobs.
//! The account is a private HOME; streamlocal opens only its exact Herdr socket,
//! and direct-tcpip only a loopback port.
//! Shared by the core's own SSH link's tests (`remote_core`, `core_move`),
//! each of which uses part of it; a device is reached with the system `ssh`,
//! whose tests use the system's `sshd` (`sshd.rs`).
#![allow(dead_code)]

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use hide_platform::process::OwnedChild;
use russh::server::{self, ChannelOpenHandle, Msg, Session};
use russh::{Channel, ChannelId};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UnixStream};
use tokio::sync::watch;
use tokio::task::JoinSet;

/// The private account the server's sessions run as: every process it
/// starts is built here, with that account's home and environment.
pub trait Account: Send + Sync + 'static {
    fn command(&self, program: &OsStr) -> Command;
    fn home(&self) -> &Path;
}

const CONNECTION_CAP: usize = 8;
const CHANNEL_CAP: usize = 32;
const JOB_CAP: usize = 128;
const STOP_BOUND: Duration = Duration::from_secs(5);

struct Shared {
    jobs: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    failures: Mutex<Vec<String>>,
    connections: Mutex<Vec<(server::Handle, watch::Sender<bool>)>>,
    environment: Box<dyn Account>,
    socket: PathBuf,
    /// Connections accepted since the server started.
    accepted: AtomicUsize,
    client_key: russh::keys::PublicKey,
    /// The file opens SFTP sessions may still receive before every
    /// connection is cut, as a network that drops mid-upload would.
    sftp_opens_left: Mutex<Option<usize>>,
}

impl Shared {
    fn job(&self, work: impl Future<Output = ()> + Send + 'static) -> Result<()> {
        let mut jobs = self.jobs.lock().expect("SSH jobs");
        // A channel that ended frees its place, so the cap bounds the jobs
        // running, not every channel a long run ever opened; one that
        // panicked is a failure the stop reports.
        let mut running = Vec::with_capacity(jobs.len());
        for mut job in jobs.drain(..) {
            if !job.is_finished() {
                running.push(job);
            } else if let Some(Err(error)) = futures_util::FutureExt::now_or_never(&mut job) {
                let mut failures = self.failures.lock().expect("SSH failures");
                if failures.len() < 16 {
                    failures.push(format!("private SSH channel panicked: {error}"));
                }
            }
        }
        *jobs = running;
        if jobs.len() >= JOB_CAP {
            bail!("private SSH channel job cap reached");
        }
        jobs.push(tokio::spawn(work));
        Ok(())
    }
}

enum Control {
    Online(bool, mpsc::SyncSender<()>),
    Stop,
}

pub struct Ssh {
    pub port: u16,
    shared: Arc<Shared>,
    controls: tokio::sync::mpsc::Sender<Control>,
    thread: Option<JoinHandle<()>>,
    finished: mpsc::Receiver<Result<()>>,
}

impl Ssh {
    pub fn start(
        environment: impl Account,
        socket: PathBuf,
        host_key: &Path,
        client_key: &Path,
    ) -> Result<Self> {
        let host_key = russh::keys::load_secret_key(host_key, None)?;
        let client_key = russh::keys::load_secret_key(client_key, None)?
            .public_key()
            .clone();
        let config = Arc::new(server::Config {
            keys: vec![host_key],
            auth_rejection_time: Duration::from_millis(5),
            inactivity_timeout: Some(Duration::from_secs(60)),
            channel_buffer_size: 8,
            event_buffer_size: 32,
            ..server::Config::default()
        });
        let shared = Arc::new(Shared {
            jobs: Mutex::new(Vec::new()),
            failures: Mutex::new(Vec::new()),
            connections: Mutex::new(Vec::new()),
            environment: Box::new(environment),
            socket,
            accepted: AtomicUsize::new(0),
            client_key,
            sftp_opens_left: Mutex::new(None),
        });
        let kept = Arc::clone(&shared);
        let (controls, receiver) = tokio::sync::mpsc::channel(4);
        let (ready, readiness) = mpsc::sync_channel(1);
        let (ended, finished) = mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("private-delivery-ssh".into())
            .spawn(move || {
                let result = (|| {
                    let runtime = tokio::runtime::Builder::new_multi_thread()
                        .worker_threads(2)
                        .enable_all()
                        .build()?;
                    runtime.block_on(serve(config, shared, receiver, ready))
                })();
                let _ = ended.send(result);
            })?;
        // Own the thread before readiness can fail, so a failed start also
        // reaches the same stop-and-join boundary.
        let mut ssh = Self {
            port: 0,
            shared: kept,
            controls,
            thread: Some(thread),
            finished,
        };
        ssh.port = readiness
            .recv_timeout(STOP_BOUND)
            .context("private SSH readiness")??;
        Ok(ssh)
    }

    /// How many connections clients have opened to this server.
    pub fn accepted(&self) -> usize {
        self.shared.accepted.load(Ordering::SeqCst)
    }

    /// How many client connections are open now.
    pub fn open(&self) -> usize {
        self.shared
            .connections
            .lock()
            .expect("SSH connections")
            .len()
    }

    /// Cuts every connection as the `nth` file open from now reaches an
    /// SFTP session, before the server sees it; the server stays online, so
    /// a client can dial again.
    pub fn cut_sftp_at_open(&self, nth: usize) {
        *self.shared.sftp_opens_left.lock().expect("SFTP cut") = Some(nth);
    }

    pub fn online(&self, value: bool) -> Result<()> {
        let (ack, done) = mpsc::sync_channel(1);
        self.controls.blocking_send(Control::Online(value, ack))?;
        done.recv_timeout(STOP_BOUND)
            .context("private SSH disconnect barrier")?;
        Ok(())
    }

    pub fn stop(&mut self) -> Result<()> {
        if self.thread.is_none() {
            return Ok(());
        }
        self.controls.blocking_send(Control::Stop)?;
        self.finished
            .recv_timeout(STOP_BOUND)
            .context("private SSH cleanup unconfirmed")??;
        self.thread
            .take()
            .expect("SSH thread")
            .join()
            .map_err(|_| anyhow::anyhow!("private SSH thread panicked"))?;
        Ok(())
    }
}

impl Drop for Ssh {
    fn drop(&mut self) {
        if let Err(error) = self.stop() {
            eprintln!("private SSH fixture cleanup failed: {error}");
        }
    }
}

async fn disconnect(shared: &Shared) {
    let connections = shared.connections.lock().expect("SSH connections").clone();
    for (handle, stop) in connections {
        let _ = stop.send(true);
        let _ = tokio::time::timeout(
            Duration::from_secs(1),
            handle.disconnect(
                russh::Disconnect::ByApplication,
                "fixture boundary closed".into(),
                "".into(),
            ),
        )
        .await;
    }
    shared.connections.lock().expect("SSH connections").clear();
}

async fn serve(
    config: Arc<server::Config>,
    shared: Arc<Shared>,
    mut controls: tokio::sync::mpsc::Receiver<Control>,
    ready: mpsc::SyncSender<Result<u16>>,
) -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    ready.send(Ok(listener.local_addr()?.port()))?;
    let mut online = true;
    let mut sessions = JoinSet::new();
    loop {
        tokio::select! {
            control = controls.recv() => match control {
                Some(Control::Online(value, ack)) => {
                    online = value;
                    if !online { disconnect(&shared).await; }
                    let _ = ack.send(());
                }
                Some(Control::Stop) | None => break,
            },
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                if !online || sessions.len() >= CONNECTION_CAP { continue; }
                shared.accepted.fetch_add(1, Ordering::SeqCst);
                // OpenSSH sets TCP_NODELAY on its sockets. Without it a reply
                // split over two writes waits for the client's delayed ACK
                // (about 40 ms on Linux), and the install's serial 32 KB SFTP
                // reads then took 46 ms each instead of 2. A peer that already
                // reset the socket fails the call; its session ends on its own.
                let _ = stream.set_nodelay(true);
                let config = Arc::clone(&config);
                let shared = Arc::clone(&shared);
                sessions.spawn(async move {
                    let (stop, cancelled) = watch::channel(false);
                    let handler = Handler {
                        shared: Arc::clone(&shared), channels: HashMap::new(),
                        stop: stop.clone(), cancelled,
                    };
                    if let Ok(running) = server::run_stream(config, stream, handler).await {
                        let handle = running.handle();
                        shared.connections.lock().expect("SSH connections").push((handle, stop.clone()));
                        let _ = running.await;
                    }
                    let _ = stop.send(true);
                    shared.connections.lock().expect("SSH connections").retain(|(_, old)| !old.same_channel(&stop));
                });
            },
            Some(_) = sessions.join_next(), if !sessions.is_empty() => {}
        }
    }
    disconnect(&shared).await;
    tokio::time::timeout(Duration::from_secs(2), async {
        while sessions.join_next().await.is_some() {}
    })
    .await
    .context("private SSH sessions did not end")?;
    let jobs = std::mem::take(&mut *shared.jobs.lock().expect("SSH jobs"));
    tokio::time::timeout(Duration::from_secs(2), async {
        for job in jobs {
            job.await.context("private SSH channel panicked")?;
        }
        Ok::<(), anyhow::Error>(())
    })
    .await
    .context("private SSH channel cleanup unconfirmed")??;
    let failures = shared.failures.lock().expect("SSH failures");
    if !failures.is_empty() {
        bail!("private SSH channel failed: {}", failures.join("; "));
    }
    Ok(())
}

struct Handler {
    shared: Arc<Shared>,
    channels: HashMap<ChannelId, Channel<Msg>>,
    stop: watch::Sender<bool>,
    cancelled: watch::Receiver<bool>,
}

impl Drop for Handler {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
    }
}

impl server::Handler for Handler {
    type Error = anyhow::Error;

    async fn auth_publickey(
        &mut self,
        user: &str,
        key: &russh::keys::PublicKey,
    ) -> Result<server::Auth> {
        Ok(
            if user == "fixture" && key.key_data() == self.shared.client_key.key_data() {
                server::Auth::Accept
            } else {
                server::Auth::reject()
            },
        )
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<()> {
        if self.channels.len() < CHANNEL_CAP {
            reply.accept().await;
            self.channels.insert(channel.id(), channel);
        }
        Ok(())
    }

    async fn channel_open_direct_streamlocal(
        &mut self,
        channel: Channel<Msg>,
        path: &str,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<()> {
        if Path::new(path) != self.shared.socket {
            return Ok(());
        }
        let mut socket = UnixStream::connect(&self.shared.socket).await?;
        reply.accept().await;
        let mut stream = channel.into_stream();
        let mut stop = self.cancelled.clone();
        self.shared.job(async move {
            tokio::select! {
                _ = tokio::io::copy_bidirectional(&mut socket, &mut stream) => {}
                _ = stop.changed() => {}
            }
        })
    }

    /// A forward to a loopback port of this host, as OpenSSH's sshd makes
    /// one for `direct-tcpip`; any other host is refused.
    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        host_to_connect: &str,
        port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<()> {
        let Ok(port) = u16::try_from(port_to_connect) else {
            return Ok(());
        };
        if !matches!(host_to_connect, "127.0.0.1" | "localhost") {
            return Ok(());
        }
        let Ok(mut socket) = tokio::net::TcpStream::connect(("127.0.0.1", port)).await else {
            return Ok(());
        };
        reply.accept().await;
        let mut stream = channel.into_stream();
        let mut stop = self.cancelled.clone();
        self.shared.job(async move {
            tokio::select! {
                _ = tokio::io::copy_bidirectional(&mut socket, &mut stream) => {}
                _ = stop.changed() => {}
            }
        })
    }

    async fn exec_request(
        &mut self,
        id: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<()> {
        let text = std::str::from_utf8(data)?;
        if text.len() > 16 * 1024 {
            session.channel_failure(id)?;
            return Ok(());
        }
        let mut command = self.shared.environment.command(OsStr::new("/bin/sh"));
        command.args(["-c", text]);
        self.start_process(id, command, session)
    }

    async fn subsystem_request(
        &mut self,
        id: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<()> {
        if name != "sftp" {
            session.channel_failure(id)?;
            return Ok(());
        }
        let program = ["/usr/libexec/sftp-server", "/usr/lib/openssh/sftp-server"]
            .into_iter()
            .find(|path| Path::new(path).is_file())
            .context("system SFTP server unavailable")?;
        let mut command = self.shared.environment.command(OsStr::new(program));
        command.args(["-d"]).arg(self.shared.environment.home());
        let shared = Arc::clone(&self.shared);
        self.start_process_within(id, command, session, Some(shared))
    }
}

impl Handler {
    fn start_process(
        &mut self,
        id: ChannelId,
        command: Command,
        session: &mut Session,
    ) -> Result<()> {
        self.start_process_within(id, command, session, None)
    }

    /// Starts `command` on the channel; with `cut`, the file opens the
    /// channel carries to it count toward the SFTP cut.
    fn start_process_within(
        &mut self,
        id: ChannelId,
        mut command: Command,
        session: &mut Session,
        cut: Option<Arc<Shared>>,
    ) -> Result<()> {
        let channel = self
            .channels
            .remove(&id)
            .context("SSH exec without session channel")?;
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = OwnedChild::spawn(&mut command)?;
        session.channel_success(id)?;
        let handle = session.handle();
        let cancelled = self.cancelled.clone();
        let shared = Arc::clone(&self.shared);
        self.shared.job(async move {
            if let Err(error) = process(child, channel, handle, cancelled, cut).await {
                let mut failures = shared.failures.lock().expect("SSH failures");
                if failures.len() < 16 {
                    failures.push(error.to_string());
                }
            }
        })
    }
}

fn pipe(pipe: impl Into<OwnedFd>) -> Result<tokio::io::unix::AsyncFd<File>> {
    let file = File::from(pipe.into());
    // These pipes belong solely to the fixture's child. AsyncFd requires
    // nonblocking descriptors, and the kernel preserves the existing flags.
    let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
    if flags < 0
        || unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(tokio::io::unix::AsyncFd::new(file)?)
}

async fn read_pipe(
    pipe: &tokio::io::unix::AsyncFd<File>,
    buffer: &mut [u8],
) -> std::io::Result<usize> {
    loop {
        let mut ready = pipe.readable().await?;
        if let Ok(result) = ready.try_io(|pipe| {
            let mut file = pipe.get_ref();
            file.read(buffer)
        }) {
            return result;
        }
    }
}

async fn write_pipe(
    pipe: &tokio::io::unix::AsyncFd<File>,
    mut buffer: &[u8],
) -> std::io::Result<()> {
    while !buffer.is_empty() {
        let mut ready = pipe.writable().await?;
        if let Ok(result) = ready.try_io(|pipe| {
            let mut file = pipe.get_ref();
            file.write(buffer)
        }) {
            let size = result?;
            if size == 0 {
                return Err(std::io::ErrorKind::WriteZero.into());
            }
            buffer = &buffer[size..];
        }
    }
    Ok(())
}

async fn process(
    mut child: OwnedChild,
    channel: Channel<Msg>,
    handle: server::Handle,
    mut stop: watch::Receiver<bool>,
    cut: Option<Arc<Shared>>,
) -> Result<()> {
    let stdin = pipe(child.take_stdin().context("SSH child stdin")?)?;
    let stdout = pipe(child.take_stdout().context("SSH child stdout")?)?;
    let stderr = pipe(child.take_stderr().context("SSH child stderr")?)?;
    let id = channel.id();
    let (mut reader, mut writer) = tokio::io::split(channel.into_stream());
    let mut pumps = Pumps(Vec::with_capacity(3));
    pumps.0.push(tokio::spawn(async move {
        let mut buffer = [0; 16 * 1024];
        let mut opens = SftpOpens::default();
        while let Ok(size) = reader.read(&mut buffer).await {
            if let Some(shared) = &cut
                && opens.reach_cut(&buffer[..size], shared)
            {
                disconnect(shared).await;
                break;
            }
            if size == 0 || write_pipe(&stdin, &buffer[..size]).await.is_err() {
                break;
            }
        }
    }));
    pumps.0.push(tokio::spawn(async move {
        let mut buffer = [0; 16 * 1024];
        while let Ok(size) = read_pipe(&stdout, &mut buffer).await {
            if size == 0 || writer.write_all(&buffer[..size]).await.is_err() {
                break;
            }
        }
    }));
    let errors_handle = handle.clone();
    pumps.0.push(tokio::spawn(async move {
        let mut buffer = [0; 16 * 1024];
        while let Ok(size) = read_pipe(&stderr, &mut buffer).await {
            if size == 0
                || errors_handle
                    .extended_data(id, 1, buffer[..size].to_vec())
                    .await
                    .is_err()
            {
                break;
            }
        }
    }));
    let mut poll = tokio::time::interval(Duration::from_millis(20));
    let status = loop {
        tokio::select! {
            _ = stop.changed() => {
                child.kill_tree()?;
                let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
                let status = loop {
                    if let Some(status) = child.try_wait()? { break status; }
                    ensure_before(deadline)?;
                    poll.tick().await;
                };
                break status;
            },
            _ = poll.tick() => if let Some(status) = child.try_wait()? { break status; }
        }
    };
    // ChannelStream closes the channel when its final half drops. Queue the
    // child's status while the input half is still owned, before aborting it.
    let _ = handle
        .exit_status_request(id, status.code().unwrap_or(1) as u32)
        .await;
    let input = pumps.0.remove(0);
    input.abort();
    let _ = input.await;
    for mut job in pumps.0.drain(..) {
        if tokio::time::timeout(Duration::from_secs(1), &mut job)
            .await
            .is_err()
        {
            job.abort();
            let _ = job.await;
        }
    }
    let _ = handle.eof(id).await;
    let _ = handle.close(id).await;
    Ok(())
}

/// The SFTP requests a client sends, framed as the protocol frames them
/// (a 4-byte length, then the type), counted for the fixture's cut.
#[derive(Default)]
struct SftpOpens {
    pending: Vec<u8>,
}

impl SftpOpens {
    const OPEN: u8 = 3;

    /// Whether `bytes` carry the file open the cut waits for; the count is
    /// shared by every session.
    fn reach_cut(&mut self, bytes: &[u8], shared: &Shared) -> bool {
        self.pending.extend_from_slice(bytes);
        let mut opens = 0;
        while self.pending.len() >= 5 {
            let length = u32::from_be_bytes([
                self.pending[0],
                self.pending[1],
                self.pending[2],
                self.pending[3],
            ]) as usize;
            if self.pending.len() < 4 + length {
                break;
            }
            if self.pending[4] == Self::OPEN {
                opens += 1;
            }
            self.pending.drain(..4 + length);
        }
        let mut left = shared.sftp_opens_left.lock().expect("SFTP cut");
        match left.as_mut() {
            Some(count) if opens >= *count => {
                *left = None;
                true
            }
            Some(count) => {
                *count -= opens;
                false
            }
            None => false,
        }
    }
}

/// Errors also cancel every pipe pump; the normal path awaits all three.
struct Pumps(Vec<tokio::task::JoinHandle<()>>);

impl Drop for Pumps {
    fn drop(&mut self) {
        for job in &self.0 {
            job.abort();
        }
    }
}

fn ensure_before(deadline: tokio::time::Instant) -> Result<()> {
    if tokio::time::Instant::now() >= deadline {
        bail!("private SSH child cleanup unconfirmed");
    }
    Ok(())
}
