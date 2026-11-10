//! The iOS app's way to the relay protocol: the same `hide-relay-protocol` crate hided and the browser link, behind
//! a narrow C interface (`include/hide_relay.h`) that the app's Swift calls. Nothing here parses or encrypts: every
//! function hands its bytes to the protocol crate and returns what it answered, so there is one implementation of the
//! Noise session, the chunking and the control messages. The socket and the order of messages stay in Swift.
//!
//! This is the only crate in the repository that needs `unsafe`: the C boundary. Each function checks its pointers,
//! never unwinds across the boundary and never keeps a pointer it was given.
#![allow(unsafe_code)]

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;

use hide_relay_protocol::{
    Error,
    admission::{ClientMessage, ServerMessage},
    frame::Frame,
    noise::Session,
};

/// The reasons a call can fail, as the integers the header names. `0` is success.
const BAD_ARGUMENT: i32 = 100;
const PANIC: i32 = 101;

fn code(error: Error) -> i32 {
    match error {
        Error::InvalidMessage => 1,
        Error::UnsupportedVersion => 2,
        Error::SizeLimit => 3,
        Error::InvalidKey => 4,
        Error::InvalidSignature => 5,
        Error::InvalidQr => 6,
        Error::RandomUnavailable => 7,
        Error::SessionClosed => 8,
        Error::InvalidState => 9,
        Error::AuthenticationFailed => 10,
        Error::Timeout => 11,
    }
}

/// Bytes the library owns: released with `hide_relay_buf_free`, never with the caller's allocator.
#[repr(C)]
pub struct HideBuf {
    pub ptr: *mut u8,
    pub len: usize,
}

impl HideBuf {
    const EMPTY: Self = Self {
        ptr: ptr::null_mut(),
        len: 0,
    };

    fn of(bytes: Vec<u8>) -> Self {
        let slice = Box::leak(bytes.into_boxed_slice());
        Self {
            ptr: slice.as_mut_ptr(),
            len: slice.len(),
        }
    }
}

/// An opaque Noise session; the app holds the pointer and frees it exactly once.
pub struct HideRelaySession(Session);

fn guarded(call: impl FnOnce() -> Result<(), i32>) -> i32 {
    match catch_unwind(AssertUnwindSafe(call)) {
        Ok(Ok(())) => 0,
        Ok(Err(code)) => code,
        Err(_) => PANIC,
    }
}

/// # Safety
/// `pointer` is null or points to `len` readable bytes.
unsafe fn input<'a>(pointer: *const u8, len: usize) -> Result<&'a [u8], i32> {
    if len == 0 {
        return Ok(&[]);
    }
    if pointer.is_null() {
        return Err(BAD_ARGUMENT);
    }
    Ok(unsafe { std::slice::from_raw_parts(pointer, len) })
}

/// # Safety
/// `out` is null or points to a writable `HideBuf`.
unsafe fn put(out: *mut HideBuf, bytes: Vec<u8>) -> Result<(), i32> {
    if out.is_null() {
        return Err(BAD_ARGUMENT);
    }
    unsafe { *out = HideBuf::of(bytes) };
    Ok(())
}

/// # Safety
/// `out` is null or points to a writable `HideBuf`.
unsafe fn clear(out: *mut HideBuf) -> Result<(), i32> {
    if out.is_null() {
        return Err(BAD_ARGUMENT);
    }
    unsafe { *out = HideBuf::EMPTY };
    Ok(())
}

/// # Safety
/// `session` is null or a live pointer from `hide_relay_session_initiator`.
unsafe fn session<'a>(session: *mut HideRelaySession) -> Result<&'a mut Session, i32> {
    unsafe { session.as_mut() }
        .map(|session| &mut session.0)
        .ok_or(BAD_ARGUMENT)
}

/// Opens an initiator toward the computer's 32-byte Noise key on the relay `connection` the relay gave.
///
/// # Safety
/// `remote_public` points to 32 readable bytes and `out` to a writable pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hide_relay_session_initiator(
    remote_public: *const u8,
    connection: u32,
    out: *mut *mut HideRelaySession,
) -> i32 {
    guarded(|| {
        if out.is_null() {
            return Err(BAD_ARGUMENT);
        }
        unsafe { *out = ptr::null_mut() };
        let key: &[u8; 32] = unsafe { input(remote_public, 32) }?
            .try_into()
            .map_err(|_| BAD_ARGUMENT)?;
        let session = Session::initiator(key, connection).map_err(code)?;
        unsafe { *out = Box::into_raw(Box::new(HideRelaySession(session))) };
        Ok(())
    })
}

/// # Safety
/// `session` is live and `out` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hide_relay_write_handshake(
    session_ptr: *mut HideRelaySession,
    out: *mut HideBuf,
) -> i32 {
    guarded(|| {
        let bytes = unsafe { session(session_ptr) }?
            .write_handshake()
            .map_err(code)?;
        unsafe { put(out, bytes) }
    })
}

/// # Safety
/// `session` is live and `frame` points to `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hide_relay_read_handshake(
    session_ptr: *mut HideRelaySession,
    frame: *const u8,
    len: usize,
) -> i32 {
    guarded(|| {
        let frame = unsafe { input(frame, len) }?;
        unsafe { session(session_ptr) }?
            .read_handshake(frame)
            .map_err(code)
    })
}

/// Encrypts one application message. `out` holds its frames, each as a 4-byte big-endian length and the frame, in the
/// order they must be sent.
///
/// # Safety
/// `session` is live, `message` points to `len` readable bytes and `out` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hide_relay_send_message(
    session_ptr: *mut HideRelaySession,
    message: *const u8,
    len: usize,
    out: *mut HideBuf,
) -> i32 {
    guarded(|| {
        let message = unsafe { input(message, len) }?;
        let frames = unsafe { session(session_ptr) }?
            .send_message(message)
            .map_err(code)?;
        let mut packed = Vec::with_capacity(frames.iter().map(|frame| frame.len() + 4).sum());
        for frame in frames {
            packed.extend_from_slice(
                &u32::try_from(frame.len())
                    .map_err(|_| code(Error::SizeLimit))?
                    .to_be_bytes(),
            );
            packed.extend_from_slice(&frame);
        }
        unsafe { put(out, packed) }
    })
}

/// Reads one frame. `out` is empty while the message is incomplete and holds the message once it is.
///
/// # Safety
/// `session` is live, `frame` points to `len` readable bytes and `out` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hide_relay_recv_message(
    session_ptr: *mut HideRelaySession,
    frame: *const u8,
    len: usize,
    now_ms: u64,
    out: *mut HideBuf,
) -> i32 {
    guarded(|| {
        unsafe { clear(out) }?;
        let frame = unsafe { input(frame, len) }?;
        match unsafe { session(session_ptr) }?
            .recv_message(frame, now_ms)
            .map_err(code)?
        {
            Some(message) => unsafe { put(out, message) },
            None => Ok(()),
        }
    })
}

/// # Safety
/// `session` is live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hide_relay_check_timeout(
    session_ptr: *mut HideRelaySession,
    now_ms: u64,
) -> i32 {
    guarded(|| {
        unsafe { session(session_ptr) }?
            .check_timeout(now_ms)
            .map_err(code)
    })
}

/// Ends the session's use and zeroizes its keys; the pointer stays valid until `hide_relay_session_free`.
///
/// # Safety
/// `session` is live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hide_relay_close(session_ptr: *mut HideRelaySession) -> i32 {
    guarded(|| {
        unsafe { session(session_ptr) }?.close();
        Ok(())
    })
}

/// # Safety
/// `session` is null or a pointer from `hide_relay_session_initiator`, freed once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hide_relay_session_free(session_ptr: *mut HideRelaySession) {
    if !session_ptr.is_null() {
        drop(unsafe { Box::from_raw(session_ptr) });
    }
}

/// # Safety
/// `buf` came from this library and is freed once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hide_relay_buf_free(buf: HideBuf) {
    if !buf.ptr.is_null() {
        drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(buf.ptr, buf.len)) });
    }
}

/// Validates a control message the app is about to send to the relay and returns its canonical text.
///
/// # Safety
/// `json` points to `len` readable bytes and `out` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hide_relay_client_message(
    json: *const u8,
    len: usize,
    out: *mut HideBuf,
) -> i32 {
    guarded(|| {
        let json = std::str::from_utf8(unsafe { input(json, len) }?)
            .map_err(|_| code(Error::InvalidMessage))?;
        let text = ClientMessage::decode(json)
            .map_err(code)?
            .encode()
            .map_err(code)?;
        unsafe { put(out, text.into_bytes()) }
    })
}

/// Validates a control message the relay sent and returns its canonical text.
///
/// # Safety
/// `json` points to `len` readable bytes and `out` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hide_relay_server_message(
    json: *const u8,
    len: usize,
    out: *mut HideBuf,
) -> i32 {
    guarded(|| {
        let json = std::str::from_utf8(unsafe { input(json, len) }?)
            .map_err(|_| code(Error::InvalidMessage))?;
        let text = ServerMessage::decode(json)
            .map_err(code)?
            .encode()
            .map_err(code)?;
        unsafe { put(out, text.into_bytes()) }
    })
}

/// The relay connection number a binary frame is addressed to.
///
/// # Safety
/// `bytes` points to `len` readable bytes and `connection` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hide_relay_frame_connection(
    bytes: *const u8,
    len: usize,
    connection: *mut u32,
) -> i32 {
    guarded(|| {
        if connection.is_null() {
            return Err(BAD_ARGUMENT);
        }
        let frame = Frame::decode(unsafe { input(bytes, len) }?).map_err(code)?;
        unsafe { *connection = frame.connection };
        Ok(())
    })
}

/// A digest of the sources this library was built from, set by `scripts/build-relay-ios.sh` (`docs/BUILD.md`), so a
/// release check can compare the library inside a signed app with the checkout it claims to come from.
/// Without it the library says `unrecorded`, which the release check refuses.
static SOURCE: &str = match option_env!("HIDE_RELAY_IOS_SOURCE") {
    Some(digest) => digest,
    None => "unrecorded",
};

/// The marker `hide-relay-ios-source:<digest>`, a NUL-ended string that lives for the whole process.
#[unsafe(no_mangle)]
pub extern "C" fn hide_relay_source() -> *const u8 {
    static MARK: std::sync::OnceLock<std::ffi::CString> = std::sync::OnceLock::new();
    MARK.get_or_init(|| {
        std::ffi::CString::new(format!("hide-relay-ios-source:{SOURCE}"))
            .expect("no NUL in a digest")
    })
    .as_ptr()
    .cast()
}
