//! A page on a device's loopback, reached from this machine through the
//! device's node link (PRD wsl-device B9): a listener on this machine's
//! loopback whose every connection is one link stream to the node's own
//! loopback, over whichever link the device has. The forward owns its
//! listener and its connections; dropping it ends them, and a link that
//! ends ends every stream it carried.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use hide_node::ssh::{OpenError, RemoteHost};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use tokio::task::{JoinHandle, JoinSet};

/// Connections one page's forward carries at once; one past it is closed.
pub const MAX_CONNECTIONS: usize = 16;

pub struct PageForward {
    link: RemoteHost,
    local_addr: SocketAddr,
    task: JoinHandle<()>,
}

impl PageForward {
    /// Listens on this machine's loopback for `remote` on the node's.
    /// `alternate` is the other address family a `localhost` page may
    /// resolve to, tried when the node cannot reach `remote`, and a numeric
    /// HTTPS host keeps its address for the certificate.
    pub async fn start(
        link: RemoteHost,
        remote: SocketAddr,
        alternate: Option<SocketAddr>,
        preserve_numeric_host: bool,
    ) -> io::Result<Self> {
        if !remote.ip().is_loopback()
            || remote.port() == 0
            || alternate.is_some_and(|address| {
                !address.ip().is_loopback() || address.port() != remote.port()
            })
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a page's address must be loopback with a nonzero port",
            ));
        }
        let local_ip = if preserve_numeric_host {
            remote.ip()
        } else if remote.is_ipv6() {
            IpAddr::V6(Ipv6Addr::LOCALHOST)
        } else {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        };
        let (primary, ipv6, local_addr) = bind(local_ip, alternate.is_some()).await?;
        let task = tokio::spawn(accept(link.clone(), primary, ipv6, remote, alternate));
        Ok(Self {
            link,
            local_addr,
            task,
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Whether the link it rides still carries streams; a forward whose
    /// link ended reaches nothing and is built again on the next link.
    pub fn live(&self) -> bool {
        self.link.closed_reason().is_none()
    }
}

impl Drop for PageForward {
    fn drop(&mut self) {
        // The accept task owns the connections' set, which aborts them as
        // it drops; each connection's stream closes on the node with it.
        self.task.abort();
    }
}

/// A listener at `local_ip`, and when a `localhost` page may resolve to
/// either family, one on IPv6 loopback at the same port, so the page's
/// address names both.
async fn bind(
    local_ip: IpAddr,
    both_families: bool,
) -> io::Result<(TcpListener, Option<TcpListener>, SocketAddr)> {
    for _ in 0..16 {
        let primary = TcpListener::bind(SocketAddr::new(local_ip, 0)).await?;
        let primary_addr = primary.local_addr()?;
        if !both_families {
            return Ok((primary, None, primary_addr));
        }
        match TcpListener::bind(SocketAddr::new(
            IpAddr::V6(Ipv6Addr::LOCALHOST),
            primary_addr.port(),
        ))
        .await
        {
            Ok(ipv6) => return Ok((primary, Some(ipv6), primary_addr)),
            Err(error) if error.kind() == io::ErrorKind::AddrInUse => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AddrInUse,
        "could not reserve both localhost address families",
    ))
}

async fn accept(
    link: RemoteHost,
    primary: TcpListener,
    ipv6: Option<TcpListener>,
    remote: SocketAddr,
    alternate: Option<SocketAddr>,
) {
    let permits = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    let reported = Arc::new(AtomicBool::new(false));
    let mut carried = JoinSet::new();
    loop {
        let accepted = tokio::select! {
            accepted = primary.accept() => accepted,
            accepted = async {
                match &ipv6 {
                    Some(listener) => listener.accept().await,
                    None => std::future::pending().await,
                }
            } => accepted,
            Some(_) = carried.join_next(), if !carried.is_empty() => continue,
        };
        let local = match accepted {
            Ok((local, _)) => local,
            Err(error) => {
                eprintln!(
                    "{}",
                    serde_json::json!({"component":"page_forward","kind":"accept.failed","node":link.target(),"reason":error.to_string()})
                );
                break;
            }
        };
        let Ok(permit) = Arc::clone(&permits).try_acquire_owned() else {
            drop(local);
            continue;
        };
        let link = link.clone();
        let reported = Arc::clone(&reported);
        carried.spawn(async move {
            let _permit = permit;
            carry(link, local, remote, alternate, &reported).await;
        });
    }
}

/// One connection: a stream to the node's loopback, and the bytes both
/// ways until either end closes. The first refusal a forward meets goes
/// to the log; the page shows its own failure to load.
async fn carry(
    link: RemoteHost,
    mut local: TcpStream,
    remote: SocketAddr,
    alternate: Option<SocketAddr>,
    reported: &AtomicBool,
) {
    let target = link.target().to_owned();
    let opened = tokio::task::spawn_blocking(move || match (link.page_stream(remote), alternate) {
        (Err(OpenError::Refused(_)), Some(alternate)) => link.page_stream(alternate),
        (opened, _) => opened,
    })
    .await;
    let stream = match opened {
        Ok(Ok(stream)) => stream,
        Ok(Err(error)) => {
            if !reported.swap(true, Ordering::Relaxed) {
                eprintln!(
                    "{}",
                    serde_json::json!({"component":"page_forward","kind":"stream.refused","node":target,"reason":error.to_string()})
                );
            }
            return;
        }
        Err(_) => return,
    };
    let Ok(mut far) = crate::browser_relay::bridge(stream) else {
        return;
    };
    let _ = tokio::io::copy_bidirectional(&mut local, &mut far).await;
}
