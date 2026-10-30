//! The contract of `hide_platform::ipc`, stated as what a caller observes.
//! The same file runs on macOS, Linux and Windows.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use hide_platform::ipc::{LocalListener, LocalStream, is_endpoint};
use tempfile::TempDir;

/// A folder and an endpoint path inside it. Kept short: a Unix socket path
/// is limited to about a hundred bytes.
fn endpoint() -> (TempDir, PathBuf) {
    let folder = tempfile::Builder::new().prefix("hp").tempdir().unwrap();
    let path = folder.path().join("s.sock");
    (folder, path)
}

#[test]
fn endpoint_kind_is_read_only_and_missing_entries_stay_missing() {
    let (folder, path) = endpoint();
    assert!(
        matches!(is_endpoint(&path), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
    );
    assert!(!path.exists());
    let listener = LocalListener::bind(&path).unwrap();
    let before = std::fs::read_dir(folder.path()).unwrap().count();
    assert!(is_endpoint(&path).unwrap());
    assert!(is_endpoint(&path).unwrap());
    assert_eq!(std::fs::read_dir(folder.path()).unwrap().count(), before);
    assert!(path.exists());
    drop(listener);
}

#[test]
fn endpoint_kind_keeps_folders_and_checks_the_native_marker_kind() {
    let (_folder, path) = endpoint();
    std::fs::create_dir(&path).unwrap();
    assert!(!is_endpoint(&path).unwrap());
    assert!(path.is_dir());
    std::fs::remove_dir(&path).unwrap();
    std::fs::write(&path, b"preserved marker bytes").unwrap();
    assert_eq!(is_endpoint(&path).unwrap(), cfg!(windows));
    assert_eq!(std::fs::read(path).unwrap(), b"preserved marker bytes");
}

#[cfg(unix)]
#[test]
fn endpoint_kind_refuses_a_link_to_an_actual_socket_without_following_it() {
    let (folder, path) = endpoint();
    let _listener = LocalListener::bind(&path).unwrap();
    let alias = folder.path().join("alias");
    std::os::unix::fs::symlink(&path, &alias).unwrap();
    assert!(!is_endpoint(&alias).unwrap());
    assert_eq!(std::fs::read_link(&alias).unwrap(), path);
    assert!(is_endpoint(&path).unwrap());
}

fn read_line(stream: &mut LocalStream) -> String {
    let mut line = Vec::new();
    let mut byte = [0_u8; 1];
    while stream.read(&mut byte).unwrap() == 1 {
        line.push(byte[0]);
        if byte[0] == b'\n' {
            break;
        }
    }
    String::from_utf8(line).unwrap()
}

// A second process is how a test kills a peer without killing itself. The
// test binary runs itself with `HIDE_PLATFORM_ROLE` set, and the one test
// below that reads the variable plays the role; without the variable it does
// nothing.
const ROLE: &str = "HIDE_PLATFORM_ROLE";
const ENDPOINT: &str = "HIDE_PLATFORM_ENDPOINT";

#[test]
#[allow(clippy::disallowed_methods)] // a child process the test kills later: it sleeps to stay alive
fn child_role() {
    let Ok(role) = std::env::var(ROLE) else {
        return;
    };
    let path = PathBuf::from(std::env::var(ENDPOINT).unwrap());
    match role.as_str() {
        "listen" => {
            let _listener = LocalListener::bind(&path).unwrap();
            println!("READY");
            thread::sleep(Duration::from_secs(60));
        }
        "connect" => {
            let _stream = LocalStream::connect(&path).unwrap();
            println!("READY");
            thread::sleep(Duration::from_secs(60));
        }
        other => panic!("unknown role {other}"),
    }
}

struct Spawned {
    child: Child,
}

impl Drop for Spawned {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn_role(role: &str, path: &Path) -> Spawned {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "ipc::child_role",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(ROLE, role)
        .env(ENDPOINT, path)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (ready, heard) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            // libtest prints "test ipc::child_role ... " before the line.
            if line.contains("READY") {
                let _ = ready.send(());
            }
        }
    });
    let spawned = Spawned { child };
    heard
        .recv_timeout(Duration::from_secs(30))
        .expect("the child role did not become ready");
    spawned
}

#[test]
fn bytes_travel_both_ways() {
    let (_folder, path) = endpoint();
    let listener = LocalListener::bind(&path).unwrap();
    let server = thread::spawn(move || {
        let mut stream = listener.accept().unwrap();
        let line = read_line(&mut stream);
        stream.write_all(line.to_uppercase().as_bytes()).unwrap();
    });
    let mut client = LocalStream::connect(&path).unwrap();
    client.write_all(b"ping\n").unwrap();
    assert_eq!(read_line(&mut client), "PING\n");
    server.join().unwrap();
}

#[test]
fn a_pair_is_two_connected_ends() {
    let (mut left, mut right) = LocalStream::pair().unwrap();
    left.write_all(b"to the right\n").unwrap();
    right.write_all(b"to the left\n").unwrap();
    assert_eq!(read_line(&mut right), "to the right\n");
    assert_eq!(read_line(&mut left), "to the left\n");
}

/// A relay reads one direction while it writes the other: the reader blocks
/// on a duplicate while the original writes, and each direction's bytes
/// arrive whole.
#[test]
fn a_duplicate_reads_while_the_original_writes() {
    let (_folder, path) = endpoint();
    let listener = LocalListener::bind(&path).unwrap();
    let payload: Vec<u8> = (0..256 * 1024).map(|index| (index % 251) as u8).collect();
    let expected = payload.clone();
    let server = thread::spawn(move || {
        let mut stream = listener.accept().unwrap();
        let mut echoed = vec![0_u8; expected.len()];
        stream.read_exact(&mut echoed).unwrap();
        stream.write_all(&echoed).unwrap();
    });
    let client = LocalStream::connect(&path).unwrap();
    let mut reader = client.duplicate();
    let reading = thread::spawn(move || {
        let mut back = Vec::new();
        reader.read_to_end(&mut back).unwrap();
        back
    });
    let mut writer = client;
    writer.write_all(&payload).unwrap();
    server.join().unwrap();
    assert_eq!(reading.join().unwrap(), payload);
}

#[test]
fn a_closing_peer_ends_the_read_with_zero() {
    let (_folder, path) = endpoint();
    let listener = LocalListener::bind(&path).unwrap();
    let server = thread::spawn(move || {
        let mut stream = listener.accept().unwrap();
        stream.write_all(b"last\n").unwrap();
    });
    let mut client = LocalStream::connect(&path).unwrap();
    assert_eq!(read_line(&mut client), "last\n");
    server.join().unwrap();
    let mut byte = [0_u8; 1];
    assert_eq!(client.read(&mut byte).unwrap(), 0);
}

#[test]
fn connecting_to_nothing_finds_nothing() {
    let (_folder, path) = endpoint();
    let error = LocalStream::connect(&path).unwrap_err();
    assert!(
        matches!(
            error.kind(),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
        ),
        "{error:?}"
    );
}

#[test]
fn a_live_listener_refuses_a_second_bind() {
    let (_folder, path) = endpoint();
    let _first = LocalListener::bind(&path).unwrap();
    let error = LocalListener::bind(&path).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::AddrInUse, "{error:?}");
}

#[test]
fn what_a_killed_listener_left_behind_is_replaced() {
    let (_folder, path) = endpoint();
    let mut holder = spawn_role("listen", &path);
    assert!(path.exists(), "a bound listener has something at its path");
    holder.child.kill().unwrap();
    holder.child.wait().unwrap();

    let listener = LocalListener::bind(&path).unwrap();
    let server = thread::spawn(move || {
        let mut stream = listener.accept().unwrap();
        stream.write_all(b"new\n").unwrap();
    });
    let mut client = LocalStream::connect(&path).unwrap();
    assert_eq!(read_line(&mut client), "new\n");
    server.join().unwrap();
}

#[test]
fn dropping_the_listener_removes_its_path() {
    let (_folder, path) = endpoint();
    let listener = LocalListener::bind(&path).unwrap();
    assert!(path.exists());
    drop(listener);
    assert!(!path.exists());
}

#[test]
fn bind_refuses_to_delete_what_is_not_an_endpoint() {
    let (_folder, path) = endpoint();
    std::fs::create_dir(&path).unwrap();
    let error = LocalListener::bind(&path).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    assert!(path.is_dir(), "the folder must still be there");
}

#[cfg(unix)]
#[test]
fn bind_keeps_a_regular_file_at_the_path() {
    let (_folder, path) = endpoint();
    std::fs::write(&path, b"mine").unwrap();
    let error = LocalListener::bind(&path).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read(&path).unwrap(), b"mine");
}

#[test]
fn a_listener_with_a_full_backlog_is_still_live_to_a_second_bind() {
    // macOS refuses a connect the same way for a dead socket and for a live
    // listener whose queue is full (#403), so the refusal cannot tell them
    // apart. Nobody accepts and the clients stay, so the queue fills where
    // the system's queue is small; Linux queues every one and Windows waits,
    // and the answer is the same.
    const PAST_EVERY_SMALL_BACKLOG: usize = 300;
    let (_folder, path) = endpoint();
    let first = LocalListener::bind(&path).unwrap();
    let (done, finished) = mpsc::channel();
    let flood = {
        let path = path.clone();
        thread::spawn(move || {
            let mut queued = Vec::new();
            while queued.len() < PAST_EVERY_SMALL_BACKLOG {
                match connect_and_ask(&path, "queued\n") {
                    Ok(client) => queued.push(client),
                    Err(error) => {
                        let _ = done.send(());
                        return (queued, Some(error.kind()));
                    }
                }
            }
            let _ = done.send(());
            (queued, None)
        })
    };
    // Each client asks as it connects, because `serve_lines` answers one
    // stream at a time in the order the system hands them out, which on
    // Windows is not the order they connected: a client that waited to be
    // served before asking would deadlock the server on another's silence.
    // Only macOS needs the queue full first, and its refusal comes at once and
    // ends the flood. Elsewhere the second bind's answer does not depend on
    // the queue, and a connect that waits for room never ends the flood, so
    // the bind goes ahead while the flood is still queuing.
    if std::env::consts::OS == "macos" {
        finished.recv().expect("the flood ended without an answer");
    }
    let error = LocalListener::bind(&path).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::AddrInUse, "{error:?}");
    assert!(
        is_endpoint(&path).unwrap(),
        "the live listener's socket was removed"
    );

    // The first listener still serves: it is not a replaced one.
    let closer = first.closer();
    let server = serve_lines(first);
    let (queued, refusal) = flood.join().unwrap();
    if std::env::consts::OS == "macos" {
        // The case this test exists for: the system refused while the
        // listener lived.
        assert_eq!(
            refusal,
            Some(std::io::ErrorKind::ConnectionRefused),
            "{} connects queued without the refusal #403 needs",
            queued.len()
        );
        assert!(queued.len() >= IN_FLIGHT, "refused after {}", queued.len());
    }
    for (index, mut client) in queued.into_iter().enumerate() {
        assert_eq!(read_line(&mut client), "QUEUED\n", "queued connect {index}");
    }
    closer.close();
    server.join().unwrap();
}

#[test]
fn a_dropped_listener_leaves_nothing_behind() {
    // Whatever bind uses to know a listener is alive goes with the listener.
    let (folder, path) = endpoint();
    let listener = LocalListener::bind(&path).unwrap();
    drop(listener);
    let left: Vec<_> = std::fs::read_dir(folder.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert!(left.is_empty(), "left behind: {left:?}");
    drop(LocalListener::bind(&path).unwrap());
}

/// The lock file a listener at `path` holds.
fn lock_file_of(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".lock");
    PathBuf::from(name)
}

#[test]
fn a_lock_file_left_with_no_holder_does_not_block_the_next_bind() {
    // What a killed listener leaves: its lock file, held by no one.
    let (_folder, path) = endpoint();
    std::fs::write(lock_file_of(&path), b"").unwrap();
    let listener = LocalListener::bind(&path).unwrap();
    drop(listener);
    assert!(
        !lock_file_of(&path).exists(),
        "the claimed file went with it"
    );
}

#[cfg(unix)]
#[test]
fn a_linked_lock_file_is_refused_and_its_target_left_alone() {
    let (folder, path) = endpoint();
    let target = folder.path().join("target");
    std::fs::write(&target, b"mine").unwrap();
    std::os::unix::fs::symlink(&target, lock_file_of(&path)).unwrap();
    let error = LocalListener::bind(&path).unwrap_err();
    assert!(
        error.to_string().contains(".lock"),
        "the error names the file: {error}"
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"mine");
    assert!(
        std::fs::symlink_metadata(lock_file_of(&path))
            .unwrap()
            .is_symlink(),
        "the planted link is the caller's to remove"
    );
    assert!(!path.exists(), "nothing was bound");
}

#[test]
fn a_lock_file_with_a_second_name_is_refused() {
    let (folder, path) = endpoint();
    let other = folder.path().join("other");
    std::fs::write(&other, b"mine").unwrap();
    std::fs::hard_link(&other, lock_file_of(&path)).unwrap();
    let error = LocalListener::bind(&path).unwrap_err();
    assert_eq!(
        error.kind(),
        std::io::ErrorKind::PermissionDenied,
        "{error:?}"
    );
    assert!(error.to_string().contains(".lock"), "{error}");
    assert_eq!(std::fs::read(&other).unwrap(), b"mine");
    assert!(!path.exists(), "nothing was bound");
}

#[cfg(unix)]
#[test]
fn a_dropped_listener_keeps_a_lock_file_that_is_no_longer_its_own() {
    // A cleaner replaced the lock file under a live listener. The file at the
    // path is now another's to keep: removing it would leave whoever holds it
    // unprotected, which is #403 again.
    let (folder, path) = endpoint();
    let listener = LocalListener::bind(&path).unwrap();
    let replacement = folder.path().join("replacement");
    std::fs::write(&replacement, b"").unwrap();
    std::fs::rename(&replacement, lock_file_of(&path)).unwrap();
    drop(listener);
    assert!(
        lock_file_of(&path).exists(),
        "another's lock file was removed"
    );
}

/// Serves `listener` until it is closed: answers each line in capitals, and
/// passes over a stream whose client already left.
fn serve_lines(listener: LocalListener) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        loop {
            let mut stream = match listener.accept() {
                Ok(stream) => stream,
                Err(error) => {
                    // A later refused connect must retain the accept loop's
                    // reason for ending, rather than silently dropping it.
                    eprintln!(
                        "ipc.test.accept_failed kind={:?} raw_os_error={:?}",
                        error.kind(),
                        error.raw_os_error()
                    );
                    break;
                }
            };
            let line = read_line(&mut stream);
            if !line.is_empty() {
                stream.write_all(line.to_uppercase().as_bytes()).unwrap();
            }
        }
    })
}

/// A connect that answers within the bound, or names the attempt that did
/// not. Windows waits two seconds for a busy pipe before `TimedOut`.
fn connect_promptly(path: &Path, attempt: usize) -> LocalStream {
    let started = Instant::now();
    let stream = LocalStream::connect(path)
        .unwrap_or_else(|error| panic!("connect {attempt} failed: {error:?}"));
    let waited = started.elapsed();
    assert!(
        waited < Duration::from_secs(1),
        "connect {attempt} waited {waited:?}"
    );
    stream
}

#[test]
fn clients_that_leave_before_accept_never_hold_up_the_next_connect() {
    // #315: on Windows a client that connected and left kept the listener's
    // only pipe instance, and every later connect timed out until an accept
    // cleared it. Nobody accepts here until the leavers are gone.
    let (_folder, path) = endpoint();
    let listener = LocalListener::bind(&path).unwrap();
    let closer = listener.closer();
    for attempt in 0..40 {
        drop(connect_promptly(&path, attempt));
    }
    let server = serve_lines(listener);
    let mut client = connect_promptly(&path, 40);
    client.write_all(b"stayed\n").unwrap();
    assert_eq!(read_line(&mut client), "STAYED\n");
    closer.close();
    server.join().unwrap();
}

/// How far a burst runs ahead of the accept loop: every this many connects,
/// one client stays for its answer. Half the smallest backlog the contract
/// promises (64 on Windows, 128 on macOS).
const IN_FLIGHT: usize = 32;

/// A client that sends `line` and is kept: its connection stays until it is
/// accepted, on every system, and its answer proves the accept.
fn connect_and_ask(path: &Path, line: &str) -> std::io::Result<LocalStream> {
    let mut client = LocalStream::connect(path)?;
    client.set_read_timeout(Some(Duration::from_secs(10)))?;
    client.write_all(line.as_bytes())?;
    Ok(client)
}

#[test]
fn a_burst_of_clients_that_connect_and_leave_loses_no_connect() {
    // #315 as it was seen: an accept loop running, and clients connecting and
    // leaving as fast as they can (the 33rd of 50 timed out on Windows). Each
    // is also shut down before its first read, the race the blocked-read test
    // cannot reach. The burst stays inside the backlog the contract names:
    // every `IN_FLIGHT`th client waits for its answer, because a starved
    // accept thread let 128 pile up and macOS refused the next (#396).
    let (_folder, path) = endpoint();
    let listener = LocalListener::bind(&path).unwrap();
    let closer = listener.closer();
    let server = serve_lines(listener);
    for attempt in 0..200 {
        if attempt % IN_FLIGHT == IN_FLIGHT - 1 {
            let mut client = connect_and_ask(&path, "stayed\n")
                .unwrap_or_else(|error| panic!("connect {attempt} failed: {error:?}"));
            assert_eq!(read_line(&mut client), "STAYED\n", "connect {attempt}");
            continue;
        }
        let mut client = connect_promptly(&path, attempt);
        let handle = client.shutdown_handle();
        handle.shutdown();
        handle.shutdown();
        assert_eq!(client.read(&mut [0_u8; 1]).unwrap(), 0);
        assert_eq!(
            client.write(b"x").unwrap_err().kind(),
            std::io::ErrorKind::BrokenPipe
        );
    }
    let mut client = connect_promptly(&path, 200);
    client.write_all(b"after\n").unwrap();
    assert_eq!(read_line(&mut client), "AFTER\n");
    closer.close();
    server.join().unwrap();
}

#[test]
fn a_connect_past_the_backlog_is_refused_and_the_listener_serves_on() {
    // Nobody accepts while clients pile up, so the backlog fills: macOS
    // refuses the next connect at once, Windows after its wait, and Linux,
    // whose backlog is far larger, may queue every one. Once the accept loop
    // runs, every queued client is answered and the listener serves as before.
    const PAST_EVERY_SMALL_BACKLOG: usize = 300;
    let (_folder, path) = endpoint();
    let listener = LocalListener::bind(&path).unwrap();
    let closer = listener.closer();
    let (done, finished) = mpsc::channel();
    let flood = {
        let path = path.clone();
        thread::spawn(move || {
            let mut queued = Vec::new();
            while queued.len() < PAST_EVERY_SMALL_BACKLOG {
                let started = Instant::now();
                match connect_and_ask(&path, "queued\n") {
                    Ok(client) => queued.push(client),
                    Err(error) => {
                        let _ = done.send(());
                        return (queued, Some((error.kind(), started.elapsed())));
                    }
                }
            }
            let _ = done.send(());
            (queued, None)
        })
    };
    // The accept loop starts once the flood has its answer. A system whose
    // connect waits for room instead of refusing never gives one, so this is
    // also how long such a connect is left waiting before the loop frees it.
    let _ = finished.recv_timeout(Duration::from_secs(10));
    let server = serve_lines(listener);
    let (queued, refusal) = flood.join().unwrap();
    match refusal {
        Some((kind, waited)) => {
            let at = queued.len();
            assert!(at >= IN_FLIGHT, "refused after only {at} connects");
            assert!(
                matches!(
                    kind,
                    std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::TimedOut
                ),
                "connect {at} failed with {kind:?}"
            );
            assert!(
                waited < Duration::from_secs(5),
                "connect {at} took {waited:?} to be refused"
            );
        }
        None => assert_eq!(
            std::env::consts::OS,
            "linux",
            "{PAST_EVERY_SMALL_BACKLOG} connects queued without a refusal"
        ),
    }
    for (index, mut client) in queued.into_iter().enumerate() {
        assert_eq!(read_line(&mut client), "QUEUED\n", "queued connect {index}");
    }
    let mut client = connect_promptly(&path, PAST_EVERY_SMALL_BACKLOG);
    client.write_all(b"after\n").unwrap();
    assert_eq!(read_line(&mut client), "AFTER\n");
    closer.close();
    server.join().unwrap();
}

/// A connect is one connection, never two: nothing the stream does on
/// either end opens another one behind it.
#[test]
fn one_connect_is_one_accepted_connection() {
    let (_folder, path) = endpoint();
    let listener = LocalListener::bind(&path).unwrap();
    let closer = listener.closer();
    let (accepted, arrivals) = mpsc::channel();
    let accepting = thread::spawn(move || {
        while let Ok(mut stream) = listener.accept() {
            let line = read_line(&mut stream);
            accepted.send(line).unwrap();
        }
    });
    let mut client = LocalStream::connect(&path).unwrap();
    client.write_all(b"one\n").unwrap();
    assert_eq!(
        arrivals.recv_timeout(Duration::from_secs(5)).unwrap(),
        "one\n"
    );
    assert_eq!(
        arrivals.recv_timeout(Duration::from_millis(500)),
        Err(mpsc::RecvTimeoutError::Timeout),
        "a second connection arrived for one connect"
    );
    closer.close();
    drop(client);
    accepting.join().unwrap();
}

/// What the listener's end writes just before it is dropped still arrives
/// whole, however slowly the client reads: the pane bootstrap answers that
/// way, and a Windows pipe closed with unread bytes throws them away.
///
/// The client leaves the last bytes unread until the listener's end is gone.
/// They are fewer than either system buffers (a Windows pipe here is asked
/// for 512), so the write can finish without them being read.
#[test]
fn an_answer_written_just_before_the_end_is_dropped_arrives_whole() {
    const ANSWER: usize = 256 * 1024;
    const UNREAD: usize = 256;
    let (_folder, path) = endpoint();
    let listener = LocalListener::bind(&path).unwrap();
    let (dropped, gone) = mpsc::channel();
    let answering = thread::spawn(move || {
        let mut stream = listener.accept().unwrap();
        stream.write_all(&vec![7_u8; ANSWER]).unwrap();
        drop(stream);
        dropped.send(()).unwrap();
    });
    let mut client = LocalStream::connect(&path).unwrap();
    let mut answer = vec![0_u8; ANSWER - UNREAD];
    client.read_exact(&mut answer).unwrap();
    gone.recv_timeout(Duration::from_secs(10))
        .expect("the listener's end never finished writing");
    client.read_to_end(&mut answer).unwrap();
    assert_eq!(answer.len(), ANSWER);
    answering.join().unwrap();
}

/// The answer is the same when the close lands before the accept blocks, so
/// a slow thread can make this pass without blocking but never fail.
#[test]
#[allow(clippy::disallowed_methods)] // time for the other thread to block in the call it frees: no portable state says a thread is inside a system call
fn close_from_another_thread_frees_a_blocked_accept() {
    let (_folder, path) = endpoint();
    let listener = LocalListener::bind(&path).unwrap();
    let closer = listener.closer();
    let (done, finished) = mpsc::channel();
    let (accepting, about_to_accept) = mpsc::channel();
    thread::spawn(move || {
        accepting.send(()).unwrap();
        let first = listener.accept().map(drop).map_err(|error| error.kind());
        let later = listener.accept().map(drop).map_err(|error| error.kind());
        let _ = done.send((first, later));
    });
    about_to_accept
        .recv_timeout(Duration::from_secs(10))
        .expect("the accepting thread never started");
    // Only the last step, from the signal into the accept, is left to time.
    thread::sleep(Duration::from_millis(300));
    let started = Instant::now();
    closer.close();
    closer.close();
    let (first, later) = finished
        .recv_timeout(Duration::from_secs(5))
        .expect("the waiting accept was not freed");
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(first, Err(std::io::ErrorKind::ConnectionAborted));
    assert_eq!(later, Err(std::io::ErrorKind::ConnectionAborted));
}

/// The peer stays open and silent until the read has answered, so only the
/// timeout can end the read.
#[test]
fn a_read_timeout_fires_after_the_time_and_not_much_later() {
    let (_folder, path) = endpoint();
    let listener = LocalListener::bind(&path).unwrap();
    let (release, released) = mpsc::channel::<()>();
    let silent = thread::spawn(move || {
        let stream = listener.accept().unwrap();
        // Bounded only so a read that never times out ends in a failure.
        let _ = released.recv_timeout(Duration::from_secs(10));
        drop(stream);
    });
    let mut client = LocalStream::connect(&path).unwrap();
    client
        .set_read_timeout(Some(Duration::from_millis(300)))
        .unwrap();
    let started = Instant::now();
    let error = client.read(&mut [0_u8; 1]).unwrap_err();
    let waited = started.elapsed();
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut, "{error:?}");
    assert!(
        waited >= Duration::from_millis(300),
        "returned early: {waited:?}"
    );
    assert!(waited < Duration::from_secs(2), "returned late: {waited:?}");
    drop(release);
    drop(client);
    silent.join().unwrap();
}

/// The peer writes only once the client's read has timed out.
#[test]
fn a_timeout_does_not_lose_bytes_that_arrive_later() {
    let (_folder, path) = endpoint();
    let listener = LocalListener::bind(&path).unwrap();
    let (timed_out, heard) = mpsc::channel();
    let server = thread::spawn(move || {
        let mut stream = listener.accept().unwrap();
        heard
            .recv_timeout(Duration::from_secs(10))
            .expect("the client's read never timed out");
        stream.write_all(b"late\n").unwrap();
    });
    let mut client = LocalStream::connect(&path).unwrap();
    client
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    assert_eq!(
        client.read(&mut [0_u8; 1]).unwrap_err().kind(),
        std::io::ErrorKind::TimedOut
    );
    timed_out.send(()).unwrap();
    client.set_read_timeout(None).unwrap();
    assert_eq!(read_line(&mut client), "late\n");
    server.join().unwrap();
}

/// The peer's end is closed once its thread has joined: a Unix stream closes
/// its descriptor on drop.
#[test]
fn a_timeout_can_be_set_after_the_peer_has_closed_and_its_bytes_still_read() {
    let (_folder, path) = endpoint();
    let listener = LocalListener::bind(&path).unwrap();
    let server = thread::spawn(move || {
        let mut stream = listener.accept().unwrap();
        stream.write_all(b"ack\nevent\n").unwrap();
    });
    let mut client = LocalStream::connect(&path).unwrap();
    server.join().unwrap();
    // macOS refuses `SO_RCVTIMEO` on a socket whose peer has gone.
    client
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    assert_eq!(read_line(&mut client), "ack\n");
    assert_eq!(read_line(&mut client), "event\n");
    assert_eq!(client.read(&mut [0_u8; 1]).unwrap(), 0);
}

#[test]
fn a_zero_timeout_is_refused() {
    let (_folder, path) = endpoint();
    let _listener = LocalListener::bind(&path).unwrap();
    let client = LocalStream::connect(&path).unwrap();
    assert_eq!(
        client
            .set_read_timeout(Some(Duration::ZERO))
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::InvalidInput
    );
}

/// The peer stays open and silent, so only the shutdown can end the read. The
/// answer is the same when the shutdown lands before the read blocks, so a
/// slow thread can make this pass without blocking but never fail.
#[test]
#[allow(clippy::disallowed_methods)] // time for the other thread to block in the call it frees: no portable state says a thread is inside a system call
fn shutdown_from_another_thread_frees_a_blocked_read() {
    let (_folder, path) = endpoint();
    let listener = LocalListener::bind(&path).unwrap();
    let (release, released) = mpsc::channel::<()>();
    let held = thread::spawn(move || {
        let stream = listener.accept().unwrap();
        let _ = released.recv_timeout(Duration::from_secs(10));
        drop(stream);
    });
    let mut client = LocalStream::connect(&path).unwrap();
    let handle = client.shutdown_handle();
    let (done, finished) = mpsc::channel();
    let (reading, about_to_read) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut byte = [0_u8; 1];
        reading.send(()).unwrap();
        let result = client.read(&mut byte);
        let after = client.write(b"x");
        let _ = done.send((result.map_err(|error| error.kind()), after.is_err()));
    });
    about_to_read
        .recv_timeout(Duration::from_secs(10))
        .expect("the reading thread never started");
    // Only the last step, from the signal into the read, is left to time.
    thread::sleep(Duration::from_millis(300));
    let started = Instant::now();
    handle.shutdown();
    let (read, write_failed) = finished
        .recv_timeout(Duration::from_secs(5))
        .expect("the blocked read was not freed");
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(read, Ok(0));
    assert!(write_failed, "a write after shutdown must fail");
    reader.join().unwrap();
    drop(release);
    held.join().unwrap();
}

/// A shutdown of one end ends the read the other end is blocked in, after
/// the bytes already sent, round after round. macOS sometimes leaves a
/// `recv` blocked on a local socket asleep when its peer shuts down (on
/// 2026-10-30, about one round in five thousand at load 30), and a `recv`
/// that missed it then misses its own end's shutdown too, while a wait in
/// `poll` hears both, so a read waits for its socket in `poll` first. The
/// rounds are what this needs: a lost end shows only now and then. Unix
/// only: a Windows shutdown cancels this end's own reads and writes, and its
/// peer hears the end when the handle closes.
///
/// The rounds run in an exact-test subprocess the test owns and bounds: a
/// read that missed its end cannot be woken from inside its process, so a
/// failed round ends with the subprocess, killed and reaped, not with a
/// thread left in the test binary.
#[test]
#[cfg(unix)]
fn a_shutdown_ends_the_peers_blocked_read_every_time() {
    const TEST: &str = "ipc::a_shutdown_ends_the_peers_blocked_read_every_time";
    const ROUNDS: usize = 5000;
    if std::env::var(ROLE).as_deref() != Ok(TEST) {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", TEST, "--nocapture"])
            .env(ROLE, TEST)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut rounds =
            hide_platform::process::OwnedChild::spawn(&mut command).expect("the rounds start");
        // A hang guard over all the rounds, which take well under a second.
        let output = match rounds.capture_until(Instant::now() + Duration::from_secs(60), 64 * 1024)
        {
            Ok(output) => output,
            Err(error) => {
                // A capture whose cleanup was not confirmed leaves the child
                // ours: end it and reap it before the failure is reported.
                if error.cleanup.is_some() {
                    let ended = rounds.kill_tree().and_then(|_| rounds.wait());
                    panic!("the rounds did not end: {error}; their end: {ended:?}");
                }
                panic!("the rounds did not end: {error}");
            }
        };
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("test result: ok. 1 passed;"),
            "the rounds must run, not be filtered out: {output:?}"
        );
        return;
    }
    for round in 0..ROUNDS {
        let (mut left, right) = LocalStream::pair().unwrap();
        let (done, finished) = mpsc::channel();
        let wake = right.shutdown_handle();
        let reader = thread::spawn(move || {
            let mut right = right;
            let mut buffer = [0_u8; 4096];
            let mut read = 0;
            let end = loop {
                match right.read(&mut buffer) {
                    Ok(0) => break Ok(read),
                    Ok(count) => read += count,
                    Err(error) => break Err(error.kind()),
                }
            };
            let _ = done.send(end);
        });
        let sent = (round % 8) * 1024;
        left.write_all(&vec![7_u8; sent]).unwrap();
        left.shutdown_handle().shutdown();
        let heard = finished
            .recv_timeout(Duration::from_secs(10))
            .unwrap_or_else(|_| panic!("round {round}: the peer's read never heard the shutdown"));
        wake.shutdown();
        reader.join().unwrap();
        assert_eq!(heard, Ok(sent), "round {round}");
    }
}

/// A peer that stops reading holds a writer once the stream's buffer is
/// full; a shutdown from another thread ends that write with an error.
#[test]
fn shutdown_from_another_thread_frees_a_write_a_silent_peer_holds() {
    let (_folder, path) = endpoint();
    let listener = LocalListener::bind(&path).unwrap();
    let (release, released) = mpsc::channel::<()>();
    let held = thread::spawn(move || {
        let stream = listener.accept().unwrap();
        let _ = released.recv_timeout(Duration::from_secs(20));
        drop(stream);
    });
    let client = LocalStream::connect(&path).unwrap();
    let handle = client.shutdown_handle();
    let (done, finished) = mpsc::channel();
    let writer = thread::spawn(move || {
        let mut client = client;
        let chunk = vec![7_u8; 64 * 1024];
        // Writes until the peer's silence blocks one: no buffer on any
        // system holds a gigabyte.
        let mut written = 0_usize;
        let result = loop {
            if let Err(error) = client.write_all(&chunk) {
                break Err(error.kind());
            }
            written += chunk.len();
            if written >= 1 << 30 {
                break Ok(written);
            }
        };
        let _ = done.send(result);
    });
    // The writer is blocked once it stops making progress; wait for that
    // rather than for a time.
    assert!(
        finished.recv_timeout(Duration::from_secs(2)).is_err(),
        "the write never blocked on the silent peer"
    );
    let started = Instant::now();
    handle.shutdown();
    let result = finished
        .recv_timeout(Duration::from_secs(5))
        .expect("the blocked write was not freed");
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(result.is_err(), "a freed write fails: {result:?}");
    writer.join().unwrap();
    drop(release);
    held.join().unwrap();
}

/// As above, with a read timeout far longer than the test.
#[test]
#[allow(clippy::disallowed_methods)] // time for the other thread to block in the call it frees: no portable state says a thread is inside a system call
fn shutdown_frees_a_read_that_has_a_timeout_too() {
    let (_folder, path) = endpoint();
    let listener = LocalListener::bind(&path).unwrap();
    let (release, released) = mpsc::channel::<()>();
    let held = thread::spawn(move || {
        let stream = listener.accept().unwrap();
        let _ = released.recv_timeout(Duration::from_secs(10));
        drop(stream);
    });
    let mut client = LocalStream::connect(&path).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let handle = client.shutdown_handle();
    let (done, finished) = mpsc::channel();
    let (reading, about_to_read) = mpsc::channel();
    thread::spawn(move || {
        reading.send(()).unwrap();
        let _ = done.send(client.read(&mut [0_u8; 1]).map_err(|error| error.kind()));
    });
    about_to_read
        .recv_timeout(Duration::from_secs(10))
        .expect("the reading thread never started");
    // Only the last step, from the signal into the read, is left to time.
    thread::sleep(Duration::from_millis(300));
    handle.shutdown();
    assert_eq!(
        finished.recv_timeout(Duration::from_secs(5)).unwrap(),
        Ok(0)
    );
    drop(release);
    held.join().unwrap();
}

/// The three tests above leave the last step into the blocking call to the
/// scheduler; a close or shutdown that lands before the call must give the
/// same answer, or a slow runner would fail them.
#[test]
fn a_close_before_the_accept_gives_the_answer_a_blocked_accept_gets() {
    let (_folder, path) = endpoint();
    let listener = LocalListener::bind(&path).unwrap();
    let closer = listener.closer();
    closer.close();
    closer.close();
    let first = listener.accept().map(drop).map_err(|error| error.kind());
    let later = listener.accept().map(drop).map_err(|error| error.kind());
    assert_eq!(first, Err(std::io::ErrorKind::ConnectionAborted));
    assert_eq!(later, Err(std::io::ErrorKind::ConnectionAborted));
}

#[test]
fn a_shutdown_before_the_read_gives_the_answer_a_blocked_read_gets() {
    for timeout in [None, Some(Duration::from_secs(30))] {
        let (_folder, path) = endpoint();
        let listener = LocalListener::bind(&path).unwrap();
        let (release, released) = mpsc::channel::<()>();
        let held = thread::spawn(move || {
            let stream = listener.accept().unwrap();
            let _ = released.recv_timeout(Duration::from_secs(10));
            drop(stream);
        });
        let mut client = LocalStream::connect(&path).unwrap();
        client.set_read_timeout(timeout).unwrap();
        client.shutdown_handle().shutdown();
        let read = client.read(&mut [0_u8; 1]).map_err(|error| error.kind());
        assert_eq!(read, Ok(0), "timeout {timeout:?}");
        assert!(
            client.write(b"x").is_err(),
            "a write after shutdown must fail"
        );
        drop(release);
        held.join().unwrap();
    }
}

#[test]
fn the_accepted_stream_names_the_connecting_process() {
    let (_folder, path) = endpoint();
    let listener = LocalListener::bind(&path).unwrap();
    // This process owns the listener; the peer is a different one.
    let (served, accepted) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let server = thread::spawn(move || {
        let stream = listener.accept().unwrap();
        let _ = served.send(stream.peer_pid());
        // The connection stays open until the test has compared the pid.
        let _ = released.recv_timeout(Duration::from_secs(10));
        drop(stream);
    });
    let peer = spawn_role("connect", &path);
    let pid = accepted
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .expect("this system reports the peer's pid");
    assert_eq!(pid, peer.child.id());
    drop(release);
    server.join().unwrap();
}

#[test]
fn a_process_of_this_account_is_this_accounts_peer() {
    let (_folder, path) = endpoint();
    let listener = LocalListener::bind(&path).unwrap();
    let (served, accepted) = mpsc::channel();
    let server = thread::spawn(move || {
        let stream = listener.accept().unwrap();
        let _ = served.send(stream.peer_is_this_account());
    });
    let _peer = spawn_role("connect", &path);
    let same = accepted.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(same.expect("this system names the peer's account"));
    server.join().unwrap();
}

#[test]
fn the_connecting_side_names_the_listening_process() {
    let (_folder, path) = endpoint();
    let holder = spawn_role("listen", &path);
    let client = LocalStream::connect(&path).unwrap();
    assert_eq!(client.peer_pid().unwrap(), holder.child.id());
}

/// A listener at `path` whose one connection never reads, held open until
/// the returned sender is dropped.
fn silent_peer(path: &Path) -> (mpsc::Sender<()>, thread::JoinHandle<()>) {
    let listener = LocalListener::bind(path).unwrap();
    let (release, released) = mpsc::channel::<()>();
    let held = thread::spawn(move || {
        let stream = listener.accept().unwrap();
        // Bounded only so a write that never ends fails the test.
        let _ = released.recv_timeout(Duration::from_secs(30));
        drop(stream);
    });
    (release, held)
}

/// Writes to a peer that never reads until a write fails, and answers that
/// failure, the moment it came and how long the write that ran out of time
/// waited: the failing one, or the one before it that moved only part of its
/// bytes. No buffer on any system holds a gigabyte.
fn write_until_refused(client: &mut LocalStream) -> (std::io::ErrorKind, Instant, Duration) {
    let chunk = vec![7_u8; 64 * 1024];
    let mut written = 0_usize;
    let mut cut = None;
    while written < 1 << 30 {
        let started = Instant::now();
        match client.write(&chunk) {
            Ok(count) => {
                written += count;
                if count < chunk.len() {
                    cut = Some(started.elapsed());
                }
            }
            Err(error) => {
                let waited = cut.unwrap_or_else(|| started.elapsed());
                return (error.kind(), Instant::now(), waited);
            }
        }
    }
    panic!("a gigabyte reached a peer that never reads");
}

/// The peer stops reading, so its stream's buffer fills and holds the next
/// write; only the write timeout can end it.
#[test]
fn a_write_timeout_ends_a_write_a_silent_peer_holds_after_the_time() {
    let (_folder, path) = endpoint();
    let (release, held) = silent_peer(&path);
    let mut client = LocalStream::connect(&path).unwrap();
    let timeout = Duration::from_millis(300);
    client.set_write_timeout(Some(timeout)).unwrap();
    let (kind, _, waited) = write_until_refused(&mut client);
    assert_eq!(kind, std::io::ErrorKind::TimedOut);
    assert!(waited >= timeout, "returned early: {waited:?}");
    drop(release);
    held.join().unwrap();
}

/// A write cut short may leave the peer part of a message, so once the peer
/// reads again and there would be room, no handle on the stream writes.
#[test]
fn after_a_write_runs_out_of_time_the_stream_takes_no_more_writes() {
    let (_folder, path) = endpoint();
    let listener = LocalListener::bind(&path).unwrap();
    let (resume, resumed) = mpsc::channel::<()>();
    let peer = thread::spawn(move || {
        let mut stream = listener.accept().unwrap();
        resumed.recv_timeout(Duration::from_secs(30)).unwrap();
        let mut taken = Vec::new();
        let _ = stream.read_to_end(&mut taken);
    });
    let mut client = LocalStream::connect(&path).unwrap();
    client
        .set_write_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    let (kind, _, _) = write_until_refused(&mut client);
    assert_eq!(kind, std::io::ErrorKind::TimedOut);
    resume.send(()).unwrap();
    client
        .set_write_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let mut duplicate = client.duplicate();
    for stream in [&mut client, &mut duplicate] {
        let started = Instant::now();
        let error = stream.write(b"x").unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut, "{error:?}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the write waited"
        );
    }
    drop(duplicate);
    drop(client);
    peer.join().unwrap();
}

/// One exchange's deadline bounds the wait of each read and write, longer
/// timeouts included, and past it nothing waits.
#[test]
fn an_exchange_deadline_ends_its_reads_and_writes_once_it_has_passed() {
    let (_folder, path) = endpoint();
    let (release, held) = silent_peer(&path);
    assert_eq!(
        LocalStream::connect_until(&path, Instant::now())
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::TimedOut,
        "a deadline already passed tries no connect"
    );
    let deadline = Instant::now() + Duration::from_millis(300);
    let mut client = LocalStream::connect_until(&path, deadline).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    client
        .set_write_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let error = client.read(&mut [0_u8; 1]).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut, "{error:?}");
    assert!(Instant::now() >= deadline, "the read returned early");
    assert_eq!(
        client.write(b"x").unwrap_err().kind(),
        std::io::ErrorKind::TimedOut
    );
    drop(release);
    held.join().unwrap();

    let (_folder, path) = endpoint();
    let (release, held) = silent_peer(&path);
    let deadline = Instant::now() + Duration::from_millis(300);
    let mut client = LocalStream::connect_until(&path, deadline).unwrap();
    let (kind, at, _) = write_until_refused(&mut client);
    assert_eq!(kind, std::io::ErrorKind::TimedOut);
    assert!(at >= deadline, "the write returned early");
    drop(release);
    held.join().unwrap();
}

#[cfg(unix)]
#[test]
fn the_endpoint_is_private_to_the_account() {
    use std::os::unix::fs::PermissionsExt;

    let (_folder, path) = endpoint();
    let _listener = LocalListener::bind(&path).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(
        mode & 0o077,
        0,
        "group or other can reach the socket: {mode:o}"
    );
}

#[cfg(windows)]
#[test]
fn the_endpoint_is_private_to_the_account() {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SDDL_REVISION_1,
        SE_KERNEL_OBJECT,
    };
    use windows_sys::Win32::Security::DACL_SECURITY_INFORMATION;

    let (_folder, path) = endpoint();
    let _listener = LocalListener::bind(&path).unwrap();
    let client = LocalStream::connect(&path).unwrap();
    let mut descriptor = std::ptr::null_mut();
    // SAFETY: the handle is open for the borrow of `client`; every out
    // pointer is either null (not asked for) or a live local.
    let status = unsafe {
        GetSecurityInfo(
            client.as_raw_handle(),
            SE_KERNEL_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    assert_eq!(status, 0, "GetSecurityInfo failed");
    let mut text: *mut u16 = std::ptr::null_mut();
    // SAFETY: `descriptor` came from GetSecurityInfo above and `text` is a
    // live local the call fills.
    let converted = unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            descriptor,
            SDDL_REVISION_1,
            DACL_SECURITY_INFORMATION,
            &mut text,
            std::ptr::null_mut(),
        )
    };
    assert_ne!(converted, 0, "the descriptor could not be written as text");
    // SAFETY: the call wrote a NUL-terminated UTF-16 string at `text`.
    let sddl = unsafe {
        let mut length = 0;
        while *text.add(length) != 0 {
            length += 1;
        }
        let sddl = String::from_utf16_lossy(std::slice::from_raw_parts(text, length));
        LocalFree(text.cast());
        LocalFree(descriptor);
        sddl
    };
    assert!(sddl.starts_with("D:P"), "the DACL inherits rights: {sddl}");
    for everyone in [";WD)", ";BU)", ";AU)", ";IU)", ";AN)", ";S-1-1-0)"] {
        assert!(!sddl.contains(everyone), "others can connect: {sddl}");
    }
}
