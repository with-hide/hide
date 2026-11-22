//! Ending a TCP stream from another thread than the one using it.
//!
//! `TcpStream::shutdown` ends the stream for later calls on every system,
//! and on macOS and Linux it also wakes a read or a write another thread is
//! blocked in. On Windows it does not: a `recv` waiting for a silent peer, or
//! a `send` waiting for a peer that stopped reading, sleeps on until the peer
//! acts or its own timeout runs out (with-hide/hide#2655). [`abort`] is the
//! one call that ends both on all three, and `tests/it/sockets.rs` checks it
//! on each.

use std::net::{Shutdown, TcpStream};

/// Ends `socket` both ways, through whichever handle on it (a `try_clone`
/// shares the stream): a read blocked on it returns an end or an error, a
/// write blocked on it fails, and so does every later one. Its peer reads the
/// end after the bytes already sent. A socket that already ended is left as it
/// is.
pub fn abort(socket: &TcpStream) {
    // The stream is shut before anything is woken, so a call that was about
    // to block fails at once instead of sleeping past the wake.
    let _ = socket.shutdown(Shutdown::Both);
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawSocket;
        use windows_sys::Win32::Foundation::HANDLE;
        use windows_sys::Win32::System::IO::CancelIoEx;

        // SAFETY: the socket is open while it is borrowed, a socket is a
        // handle, and a null overlapped pointer cancels every request
        // pending on it, a blocked `recv` or `send` of another thread
        // included; with none pending the call changes nothing.
        let _ = unsafe { CancelIoEx(socket.as_raw_socket() as HANDLE, std::ptr::null()) };
    }
}
