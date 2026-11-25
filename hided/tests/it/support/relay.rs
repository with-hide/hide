//! A loopback TCP relay between a test's `ssh` and its `sshd`
//! ([`super::sshd`]) that can cut the connections it carries the way a
//! network that went away does (with-hide/hide#2882): no byte crosses any
//! more and neither side's close reaches the other, so the server keeps a
//! connection whose client is gone, with nothing coming in, as a Linux
//! server keeps one for about fifteen minutes after the other machine
//! roamed off its Wi-Fi. Stopping or killing the client's `ssh` cannot make
//! that state on one machine: its socket's close crosses the loopback at
//! once.
//!
//! A connection made after a cut is carried as any other. The relay owns
//! its listener, every connection and every thread it starts; dropping it
//! closes them all, so a cut connection ends with the test.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;

/// The most connections the relay carries over its life; a test makes a
/// handful (one per device link and per sign-in check).
const MAX_CONNECTIONS: usize = 64;

pub struct Relay {
    /// The port a client reaches the server through.
    pub port: u16,
    shared: Arc<Shared>,
    accepting: Option<std::thread::JoinHandle<()>>,
}

#[derive(Default)]
struct Shared {
    connections: Mutex<Vec<Arc<Carried>>>,
    stopping: AtomicBool,
}

/// One client connection and the server connection it was relayed to.
struct Carried {
    client: TcpStream,
    server: TcpStream,
    cut: AtomicBool,
}

impl Relay {
    /// Relays each connection to `127.0.0.1:target`.
    pub fn start(target: u16) -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        let shared = Arc::new(Shared::default());
        let accepting = {
            let shared = Arc::clone(&shared);
            std::thread::Builder::new()
                .name("relay-accept".into())
                .spawn(move || accept(&listener, target, &shared))?
        };
        Ok(Self {
            port,
            shared,
            accepting: Some(accepting),
        })
    }

    /// Cuts every connection the relay carries now: from here nothing
    /// crosses either way, and a side that closes is not told to the other.
    pub fn cut(&self) {
        for carried in lock(&self.shared.connections).iter() {
            carried.cut.store(true, Ordering::SeqCst);
        }
    }

    /// How many connections the relay has taken.
    pub fn taken(&self) -> usize {
        lock(&self.shared.connections).len()
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.shared.stopping.store(true, Ordering::SeqCst);
        for carried in lock(&self.shared.connections).iter() {
            let _ = carried.client.shutdown(Shutdown::Both);
            let _ = carried.server.shutdown(Shutdown::Both);
        }
        // Wakes the accept with a connection of its own.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(accepting) = self.accepting.take() {
            let _ = accepting.join();
        }
    }
}

fn accept(listener: &TcpListener, target: u16, shared: &Arc<Shared>) {
    for client in listener.incoming() {
        if shared.stopping.load(Ordering::SeqCst) {
            return;
        }
        let Ok(client) = client else { continue };
        if lock(&shared.connections).len() >= MAX_CONNECTIONS {
            eprintln!("relay: refused a connection past its cap of {MAX_CONNECTIONS}");
            continue;
        }
        let Ok(server) = TcpStream::connect(("127.0.0.1", target)) else {
            eprintln!("relay: the server at port {target} did not take a connection");
            continue;
        };
        // As the fixture's server does, so a small write crosses at once.
        let _ = client.set_nodelay(true);
        let _ = server.set_nodelay(true);
        let (Ok(client_reading), Ok(server_reading)) = (client.try_clone(), server.try_clone())
        else {
            continue;
        };
        let carried = Arc::new(Carried {
            client,
            server,
            cut: AtomicBool::new(false),
        });
        lock(&shared.connections).push(Arc::clone(&carried));
        for (reading, to_server) in [(client_reading, true), (server_reading, false)] {
            let carried = Arc::clone(&carried);
            let spawned = std::thread::Builder::new()
                .name("relay-copy".into())
                .spawn(move || copy(reading, &carried, to_server));
            if spawned.is_err() {
                eprintln!("relay: a copy thread did not start");
            }
        }
    }
}

/// Copies what `reading` reads to the other side of `carried` until it
/// ends. Once the connection is cut, what it reads is dropped and its end
/// is not passed on.
fn copy(mut reading: TcpStream, carried: &Carried, to_server: bool) {
    let mut writing = if to_server {
        &carried.server
    } else {
        &carried.client
    };
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = match reading.read(&mut buffer) {
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => 0,
        };
        let cut = carried.cut.load(Ordering::SeqCst);
        if read == 0 {
            if !cut {
                let _ = writing.shutdown(Shutdown::Write);
            }
            return;
        }
        if !cut && writing.write_all(&buffer[..read]).is_err() {
            return;
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
