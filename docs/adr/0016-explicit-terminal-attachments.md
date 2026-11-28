# 0016. Explicit terminal attachments

- Status: Accepted
- Date: 2026-09-20
- Code: `hide-core/src/runtime/attachments.rs`, `hide-host/src/attachments.rs`, `hided/src/attachments.rs`, `hide-node-link/src/attachments.rs`, `web/src/attachments.ts`

## Decision

### Explicit terminal attachments

The browser cannot name a dropped file's or a pasted image's path, so hided's own staging (`hided/src/attachments.rs`, see hided and the WebSocket boundary) is the only source of a `terminal_attachment` intent: it writes the uploaded bytes to disk and sends the event carrying the original pane ID, a generated request UUID and the staged paths.
hided reports `terminal_attachment_ready` itself once a clipboard image's bytes are staged at the exact path the core reads it from.
No clipboard image bytes enter the JSON event or the render mutex.

`runtime/attachments.rs` owns admission, captured terminal and remote connection generations, one active worker, retry, cancellation and an ordered 64 KiB input reservation.
Subsequent input to the originating pane is held by the pane's node until file preparation and transfer succeed, then the node pastes each quoted path to the pane's holder, a space after each, and writes the held input behind them on the same writer (`TerminalControl::AttachmentHold`, `AttachmentDeliver`); the holder wraps each path in bracketed paste only when the program turned that mode on, which the screen cannot see (with-hide/hide issue 2952).
No Enter is synthesized, and an upload failure never forwards held input, because it may contain an Enter that would submit an incomplete prompt.
Retry keeps immutable prepared bytes; cancel discards held input explicitly.
Missing, closed, released or reconnected targets retire the intent and cannot forward its data to a replacement session or fall back to the local host.
A local pane with no terminal session yet is still a target: the web shows a pane once its layout arrives, before its session attaches, so a drop can reach the core first (issue 735).
That intent waits for the pane's session, taking the first session the pane starts as its own, with the paste and typed input held under the same 64 KiB reservation, and both are written when the session attaches.
The wait is bounded by `COMMIT_GRACE` from admission (`hide-node-link/src/attachments.rs`, how long hided keeps a committed file from eviction, so the pasted path still names a file); the coordinator's operation tick checks the deadline with no event needed and fails the intent as unavailable, with a `terminal.attachment.terminal_wait_expired` diagnostic.
Only admission, completion and actionable failure transitions publish a notice through `status.async_operations`; terminal-specific actions render in the originating pane.

The node that holds the picked files reads them (`hide-host/src/attachments.rs`, the `read_attachments` call, asking the core before each file whether to go on, so a cancel stops the read between files): only explicitly selected regular files, refusing final-component symlinks, special files, control-character paths and files that change while read; `terminal_attachments.rs` keeps the limits' admission and constructs ordered quoted terminal input.
The bounds are eight files, 20 MiB per file and 40 MiB per intent; clipboard decoding additionally allows at most 16 megapixels.
A device stages the immutable bytes through its node's link, the same way over every device link (`Call::StageAttachment`, one file per call so each stays under the link's line cap, answered by `hide-host/src/attachments.rs`, `Staging`), not a shell command, a subprocess or an SSH-only channel, and answers only device paths.
Device staging lives in the node's private `<state>/terminal-attachments` folder, with generated exclusive filenames, 0700 folder and 0600 file permissions, ownership checks and same-byte verification before adopting a completed file on retry; the folder an earlier build staged into, `~/.hide-terminal-attachments`, is swept of expired files and removed once empty.
The node tracks each request until its last file arrives: a link that ends with a request part way removes that request's files, and `Call::RemoveAttachments` removes a request's files when the core cancels, retires or finishes it.
A file still being written when either happens is removed by its own write as it finishes, so neither can leave a file that appears after it, and a write whose worker unwinds drops its request the same way (`hide-host/src/attachments.rs`, `Staging` and its `Writing` guard).
Each file's call is bounded at 60 seconds, a cancel stops the request between files and removes what it staged, and a device whose link is down refuses the request without waking it.
Neither filesystem nor network I/O runs under `Mutex<Runtime>`.

Both staging locations admit at most 128 files and 256 MiB.
Cleanup scans only their immediate generated entries on the next explicit attachment intent and expires entries older than 24 hours; there is no background janitor and successful paths may therefore survive longer while idle.
Local clipboard PNGs remain available after a successful local paste, while remote success releases the local PNG after upload; the core's own node removes it (`Call::RemoveClipboard`, refused for any path that is not a clipboard image), so the core touches no file.
Cancellation, retirement and destruction attempt to remove only that intent's exact generated files; failed remote cleanup is diagnostic and remains bounded by the next-intent expiry policy.
Original user-selected files are never deleted.
This boundary has no provider-specific draft, composer or attachment shelf.
