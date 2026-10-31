//! The one layer that reads an agent's session files. Every feature that
//! reads a session (the label worker's turns and the doorbell, usage, the
//! thread view, the catalog, Memory, links, the phone's conversation, the
//! hooks) takes its records from here and never opens or splits a session
//! file itself.
//!
//! - [`file`]: opening a session file without waiting on it, and the
//!   identity of the file opened, spelled one way for every reader that
//!   keeps it.
//! - [`frame`]: where a record ends. A record longer than its reader's cap
//!   is passed over piece by piece and never held, so a pasted image never
//!   refuses a session; a last record without its newline is held until it
//!   is written; a record that is not JSON is the caller's to count.
//! - [`window`]: the bounded windows a reader takes of a file: the bytes
//!   appended since its last position (a replaced or truncated file starts
//!   over and says why), a page of whole records before an offset, the tail,
//!   and the whole file under the 64 MiB cap.
//! - [`facts`]: per agent, what a session's own records or files say about
//!   it: its native id, its checkout, when it was made, and the record that
//!   names them.
//!
//! What a record means stays with the feature that reads it (conversation
//! events, turn marks, usage events, thread updates); how it is read is the
//! same for all of them.

pub(crate) mod facts;
pub(crate) mod file;
pub(crate) mod frame;
pub(crate) mod window;

pub(crate) use file::{FileIdentity, open, open_nofollow};
