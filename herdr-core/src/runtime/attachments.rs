use super::*;
use crate::model::AsyncOperationSnapshot;
use crate::terminal_attachments::{self as ingress, AttachmentFile};
use hide_node_link::terminal::{AttachmentInputOutcome, TerminalControl};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(super) struct PendingAttachment {
    pub operation: AsyncOperationSnapshot,
    pane_id: String,
    terminal_generation: Option<u64>,
    /// The local pane had no terminal session at the drop. The web shows a
    /// pane once its layout arrives, before its session attaches, so the
    /// paste waits for that session until the operation's deadline.
    awaiting_terminal: bool,
    remote: Option<(String, u64)>,
    /// The files are on the pane's own machine already, staged there by its
    /// own screen: their paths are pasted as they are (`staged_on`).
    on_pane_machine: bool,
    clipboard: bool,
    clipboard_preparing: bool,
    bracketed: bool,
    paths: Vec<String>,
    prepared: Option<Arc<Vec<AttachmentFile>>>,
    /// The paste text, made when the files are ready and held while the
    /// terminal is awaited.
    paste: Option<Vec<u8>>,
    /// Input bytes the node refused at the held input's cap; the keys under
    /// it wait on the node, behind the paste.
    rejected_bytes: usize,
    /// The node was asked to write the paste and has not answered.
    writing: bool,
    cancelled: Arc<AtomicBool>,
}

impl Runtime {
    pub(crate) fn take_attachment_worker(&mut self) -> Option<thread::JoinHandle<()>> {
        if let Some(pending) = &mut self.attachment {
            pending.cancelled.store(true, Ordering::Release);
            pending.operation.stage = "cancelling".to_owned();
            let intent = pending.operation.id.clone();
            self.terminals
                .control(TerminalControl::AttachmentRefuse { intent });
        }
        // A failed intent can outlive its transfer thread and still own staged
        // files. Start one cleanup worker now so destruction takes and joins it.
        if self
            .attachment
            .as_ref()
            .is_some_and(|pending| !pending.clipboard_preparing)
            && self
                .attachment_worker
                .as_ref()
                .is_none_or(|worker| worker.is_finished())
        {
            self.start_attachment_cleanup();
        }
        self.attachment_worker.take()
    }

    pub(super) fn reconcile_attachment_target(&mut self) -> bool {
        let Some(pending) = self.attachment.as_ref() else {
            return false;
        };
        if pending.operation.stage == "retired" {
            return false;
        }
        if self.attachment_target_valid(pending) {
            return self.attachment_terminal_arrived();
        }
        let pending = self.attachment.as_mut().expect("checked above");
        pending.cancelled.store(true, Ordering::Release);
        pending.operation.stage = "retired".to_owned();
        let intent = pending.operation.id.clone();
        self.terminals
            .control(TerminalControl::AttachmentRefuse { intent });
        self.fail_attachment("The original terminal closed or reconnected. Held input was discarded and nothing was sent. Cancel and paste again.", false);
        if !self
            .attachment
            .as_ref()
            .is_some_and(|pending| pending.clipboard_preparing)
            && self
                .attachment_worker
                .as_ref()
                .is_none_or(|worker| worker.is_finished())
        {
            self.start_attachment_cleanup();
        }
        true
    }

    /// A paste that began before the pane's first terminal session takes
    /// that session as its own; a later one retires it like any reconnect.
    pub(super) fn adopt_attachment_terminal(&mut self, pane_id: &str, generation: u64) {
        if let Some(pending) = self.attachment.as_mut().filter(|pending| {
            pending.pane_id == pane_id
                && pending.awaiting_terminal
                && pending.terminal_generation.is_none()
        }) {
            pending.terminal_generation = Some(generation);
        }
    }

    /// Ends the wait once the awaited session is there, and writes a paste
    /// that was ready before it.
    fn attachment_terminal_arrived(&mut self) -> bool {
        let controlling = self
            .attachment
            .as_ref()
            .is_some_and(|pending| self.terminal_controlling(&pending.pane_id));
        let Some(pending) = self
            .attachment
            .as_mut()
            .filter(|pending| pending.awaiting_terminal && controlling)
        else {
            return false;
        };
        pending.awaiting_terminal = false;
        crate::diagnostic!(serde_json::json!({
            "kind": "terminal.attachment.terminal_arrived",
            "request_id": pending.operation.id,
            "pane_id": pending.pane_id,
            "waited_ms": unix_milliseconds().saturating_sub(pending.operation.started_at_unix_ms),
        }));
        if pending.operation.stage == "terminal" {
            self.deliver_attachment();
        }
        true
    }

    /// Fails a paste whose terminal did not attach while its staged files are
    /// kept, the same unavailable failure a drop on a dead pane gets.
    pub(super) fn expire_attachment_wait(&mut self, now_unix_ms: u64) -> bool {
        let Some(pending) = self.attachment.as_mut().filter(|pending| {
            pending.awaiting_terminal
                && pending.operation.phase == "pending"
                && pending.operation.stage != "cancelling"
                && pending
                    .operation
                    .deadline_at_unix_ms
                    .is_some_and(|deadline| now_unix_ms >= deadline)
        }) else {
            return false;
        };
        pending.awaiting_terminal = false;
        pending.paste = None;
        pending.cancelled.store(true, Ordering::Release);
        crate::diagnostic!(serde_json::json!({
            "kind": "terminal.attachment.terminal_wait_expired",
            "request_id": pending.operation.id,
            "pane_id": pending.pane_id,
            "generation": pending.terminal_generation,
            "waited_ms": now_unix_ms.saturating_sub(pending.operation.started_at_unix_ms),
        }));
        self.fail_attachment(TERMINAL_UNAVAILABLE, false);
        true
    }

    pub(super) fn tick_attachment(&mut self) -> bool {
        if self
            .attachment
            .as_ref()
            .is_some_and(|pending| pending.operation.stage == "retry_wait")
            && self
                .attachment_worker
                .as_ref()
                .is_none_or(|worker| worker.is_finished())
        {
            self.start_attachment_worker();
            return true;
        }
        false
    }

    fn attachment_target_valid(&self, pending: &PendingAttachment) -> bool {
        if self.close_operation_holds_pane(&pending.pane_id) {
            return false;
        }
        if self.terminal_generation(&pending.pane_id) != pending.terminal_generation {
            return false;
        }
        if !self
            .snapshot
            .terminal
            .panes
            .iter()
            .any(|pane| pane.pane_id == pending.pane_id)
        {
            return false;
        }
        if let Some((target, generation)) = &pending.remote {
            if self.remote_connection_generations.get(target) != Some(generation) {
                return false;
            }
            if !self
                .snapshot
                .status
                .remote
                .iter()
                .any(|remote| remote.target_id == *target && remote.state == "connected")
            {
                return false;
            }
        }
        match self
            .terminal_states
            .get(&pending.pane_id)
            .map(|state| state.state.as_str())
        {
            Some("controlling") => true,
            Some("observing") => false,
            _ => pending.awaiting_terminal,
        }
    }

    /// The generation of the pane's current or last session, as its node
    /// reported it.
    fn terminal_generation(&self, pane_id: &str) -> Option<u64> {
        self.terminal_states
            .get(pane_id)
            .map(|state| state.generation)
            .filter(|generation| *generation > 0)
    }

    fn terminal_controlling(&self, pane_id: &str) -> bool {
        self.terminal_states
            .get(pane_id)
            .is_some_and(|state| state.state == "controlling")
    }

    /// Drops the paste and lets the node drop what it held behind it.
    fn clear_attachment(&mut self) {
        if let Some(pending) = self.attachment.take() {
            self.terminals.control(TerminalControl::AttachmentRelease {
                intent: pending.operation.id,
            });
        }
    }

    pub(super) fn begin_attachment(&mut self, payload: AttachmentPayload) -> bool {
        let mut operation = AsyncOperationSnapshot {
            related_scope_ids: Vec::new(),
            id: payload.request_id.clone(),
            kind: "terminal.attachment".to_owned(),
            target_id: payload.pane_id.clone(),
            scope_id: self
                .pane_operation_scope(&payload.pane_id)
                .unwrap_or_else(|| payload.pane_id.clone()),
            phase: "pending".to_owned(),
            stage: if payload.clipboard {
                "clipboard"
            } else {
                "preparing"
            }
            .to_owned(),
            started_at_unix_ms: unix_milliseconds(),
            deadline_at_unix_ms: None,
            message: Some("Preparing files…".to_owned()),
            retryable: false,
        };
        if self
            .attachment
            .as_ref()
            .is_some_and(|pending| pending.operation.id == payload.request_id)
        {
            return false;
        }
        let refusal = if !ingress::valid_request_id(&payload.request_id) {
            Some("Invalid attachment request identity.")
        } else if self.attachment.is_some()
            || self
                .attachment_worker
                .as_ref()
                .is_some_and(|worker| !worker.is_finished())
        {
            Some(
                "Another file transfer is pending. Finish or cancel that transfer before pasting more files.",
            )
        } else if payload
            .paths
            .iter()
            .any(|path| path.len() > ingress::MAX_PATH_BYTES)
        {
            Some(
                "A selected file path exceeds the 4096-byte path limit. Move the file to a shorter path.",
            )
        } else if payload.paths.len() > ingress::MAX_FILES
            || (!payload.clipboard && payload.paths.is_empty())
        {
            Some("Choose between 1 and 8 regular files.")
        } else {
            None
        };
        if let Some(message) = refusal {
            operation.phase = "refused".to_owned();
            operation.message = Some(message.to_owned());
            self.attachment_rejection = Some(operation);
            self.sync_async_operations();
            return true;
        }
        // The paste text lands only when the upload ends, which can be long
        // after; the operator is typing into the pane meanwhile.
        self.note_delivery_key(&payload.pane_id, "attachment");
        let remote = self
            .remote_file_transports
            .keys()
            .find(|target| remote_pane_source_id(target, &payload.pane_id).is_some())
            .and_then(|target| {
                self.remote_connection_generations
                    .get(target)
                    .map(|generation| (target.clone(), *generation))
            });
        // Only the pane's own machine may say the files are already there.
        let on_pane_machine = match payload.staged_on.as_deref() {
            None => false,
            Some(node) => {
                if remote.as_ref().is_none_or(|(target, _)| target != node) {
                    operation.phase = "refused".to_owned();
                    operation.message = Some("Invalid attachment request identity.".to_owned());
                    self.attachment_rejection = Some(operation);
                    self.sync_async_operations();
                    return true;
                }
                true
            }
        };
        let awaiting_terminal = remote.is_none()
            && !payload.pane_id.starts_with("remote:")
            && !self.terminal_controlling(&payload.pane_id);
        if awaiting_terminal {
            operation.deadline_at_unix_ms = Some(operation.started_at_unix_ms.saturating_add(
                u64::try_from(ingress::COMMIT_GRACE.as_millis()).unwrap_or(u64::MAX),
            ));
        }
        let pending = PendingAttachment {
            operation,
            terminal_generation: self.terminal_generation(&payload.pane_id),
            awaiting_terminal,
            pane_id: payload.pane_id,
            remote,
            on_pane_machine,
            clipboard: payload.clipboard,
            clipboard_preparing: payload.clipboard,
            bracketed: payload.bracketed_paste,
            paths: payload.paths,
            prepared: None,
            paste: None,
            rejected_bytes: 0,
            writing: false,
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        let valid = self.attachment_target_valid(&pending)
            && (!pending.pane_id.starts_with("remote:") || pending.remote.is_some());
        if valid && pending.awaiting_terminal {
            crate::diagnostic!(serde_json::json!({
                "kind": "terminal.attachment.awaiting_terminal",
                "request_id": pending.operation.id,
                "pane_id": pending.pane_id,
                "generation": pending.terminal_generation,
            }));
        }
        // Keys typed for the pane from now on wait behind the paste.
        self.terminals.control(TerminalControl::AttachmentHold {
            pane: pending.pane_id.clone(),
            intent: pending.operation.id.clone(),
        });
        self.attachment = Some(pending);
        self.attachment_rejection = None;
        if !valid {
            self.attachment
                .as_mut()
                .expect("assigned above")
                .clipboard_preparing = false;
            self.fail_attachment(TERMINAL_UNAVAILABLE, false);
        } else if !payload.clipboard {
            self.start_attachment_worker();
        }
        self.sync_async_operations();
        true
    }

    pub(super) fn attachment_ready(&mut self, payload: AttachmentCompletionPayload) -> bool {
        let Some(pending) = self.attachment.as_mut().filter(|pending| {
            pending.operation.id == payload.request_id
                && pending.pane_id == payload.pane_id
                && pending.clipboard_preparing
                && ["clipboard", "cancelling", "retired"]
                    .contains(&pending.operation.stage.as_str())
        }) else {
            return false;
        };
        pending.clipboard_preparing = false;
        if ["cancelling", "retired"].contains(&pending.operation.stage.as_str()) {
            self.clear_attachment();
            self.sync_async_operations();
            return true;
        }
        if pending.operation.phase == "failed" {
            return true;
        }
        if let Some(error) = payload.error {
            self.fail_attachment(&error, false);
        } else {
            pending.paths = vec![
                ingress::clipboard_path(&self.state_path, &payload.request_id)
                    .to_string_lossy()
                    .into_owned(),
            ];
            self.start_attachment_worker();
        }
        self.sync_async_operations();
        true
    }

    fn fail_attachment(&mut self, message: &str, retryable: bool) {
        if let Some(pending) = self.attachment.as_mut() {
            pending.operation.phase = "failed".to_owned();
            pending.operation.retryable = retryable;
            pending.operation.message = Some(format!(
                "{message} Files were not pasted.{}{}",
                if pending.operation.stage == "retired" {
                    ""
                } else if !retryable {
                    " Typed input is held. Cancel to discard it."
                } else {
                    " Typed input is held until retry succeeds or you cancel."
                },
                if pending.rejected_bytes > 0 {
                    format!(
                        " {} further input bytes were refused at the 64 KiB limit.",
                        pending.rejected_bytes
                    )
                } else {
                    String::new()
                }
            ));
        }
        self.sync_async_operations();
    }

    /// The node refused a key for the paste's pane: the paste is cancelling,
    /// or the input held behind it reached its cap. Returns whether the
    /// snapshot changed.
    pub(super) fn attachment_input_refused(
        &mut self,
        intent: &str,
        outcome: AttachmentInputOutcome,
        bytes: usize,
    ) -> bool {
        let Some(pending) = self
            .attachment
            .as_mut()
            .filter(|pending| pending.operation.id == intent)
        else {
            return false;
        };
        match outcome {
            AttachmentInputOutcome::Cancelling => {
                let message = "Cancellation is finishing. New terminal input was refused; wait for this notice to close before typing.";
                if pending.operation.message.as_deref() == Some(message) {
                    return false;
                }
                pending.operation.message = Some(message.to_owned());
                self.sync_async_operations();
                true
            }
            AttachmentInputOutcome::Limit => {
                pending.rejected_bytes = pending.rejected_bytes.saturating_add(bytes);
                pending.cancelled.store(true, Ordering::Release);
                self.fail_attachment(
                    "Held input reached its 64 KiB limit. Cancel and paste again.",
                    false,
                );
                true
            }
        }
    }

    /// The node wrote the paste and the input held behind it, or its
    /// session could not take them.
    pub(super) fn attachment_delivered(&mut self, intent: &str, written: bool) -> bool {
        let Some(pending) = self
            .attachment
            .as_mut()
            .filter(|pending| pending.operation.id == intent && pending.writing)
        else {
            return false;
        };
        pending.writing = false;
        if !written {
            self.fail_attachment(
                "Terminal input was not accepted. Check the connection and retry.",
                true,
            );
            return true;
        }
        let pane_id = pending.pane_id.clone();
        // The paste is composer text the operator has not sent, and an Enter
        // held behind it does not send it either.
        self.note_delivery_key(&pane_id, "attachment");
        // The node dropped its hold with the write; the release lets the
        // router forget where the paste went.
        self.clear_attachment();
        self.sync_async_operations();
        true
    }

    pub(super) fn attachment_action(&mut self, payload: AttachmentActionPayload) -> bool {
        if self.attachment_rejection.as_ref().is_some_and(|rejection| {
            rejection.id == payload.request_id && rejection.target_id == payload.pane_id
        }) {
            self.attachment_rejection = None;
            self.sync_async_operations();
            return true;
        }
        let Some(pending) = self.attachment.as_ref().filter(|pending| {
            pending.operation.id == payload.request_id && pending.pane_id == payload.pane_id
        }) else {
            return false;
        };
        match payload.action.as_str() {
            "cancel" => {
                pending.cancelled.store(true, Ordering::Release);
                if self
                    .attachment_worker
                    .as_ref()
                    .is_some_and(|worker| !worker.is_finished())
                    || pending.clipboard_preparing
                {
                    let pending = self.attachment.as_mut().expect("matched above");
                    pending.operation.phase = "pending".to_owned();
                    pending.operation.stage = "cancelling".to_owned();
                    pending.operation.message =
                        Some("Cancelling transfer and discarding held input…".to_owned());
                    let intent = pending.operation.id.clone();
                    self.terminals
                        .control(TerminalControl::AttachmentRefuse { intent });
                } else {
                    self.start_attachment_cleanup();
                }
            }
            "retry" if pending.operation.retryable && pending.operation.phase == "failed" => {
                if !self.attachment_target_valid(pending) {
                    self.fail_attachment(
                        "The terminal connection changed. Cancel and paste again.",
                        false,
                    );
                } else if self
                    .attachment_worker
                    .as_ref()
                    .is_some_and(|worker| !worker.is_finished())
                {
                    let pending = self.attachment.as_mut().expect("matched above");
                    pending.operation.stage = "retry_wait".to_owned();
                    pending.operation.phase = "pending".to_owned();
                    pending.operation.retryable = false;
                    pending.operation.message = Some(
                        "Waiting for the previous transfer to finish before retrying…".to_owned(),
                    );
                } else {
                    self.start_attachment_worker();
                }
            }
            _ => return false,
        }
        self.sync_async_operations();
        true
    }

    fn start_attachment_cleanup(&mut self) {
        let Some(context) = self.worker_context.clone() else {
            self.clear_attachment();
            return;
        };
        let node = self.own_node();
        let Some(pending) = self.attachment.as_mut() else {
            return;
        };
        let request_id = pending.operation.id.clone();
        let clipboard = pending.clipboard;
        let files = pending.prepared.take();
        let transport = pending
            .remote
            .as_ref()
            .filter(|_| !pending.on_pane_machine)
            .and_then(|(target, _)| self.remote_file_transports.get(target))
            .cloned();
        let state_path = self.state_path.clone();
        pending.operation.stage = "cancelling".to_owned();
        let intent = request_id.clone();
        pending.operation.phase = "pending".to_owned();
        pending.operation.retryable = false;
        pending.operation.message =
            Some("Cancelling transfer and discarding held input…".to_owned());
        self.terminals
            .control(TerminalControl::AttachmentRefuse { intent });
        match thread::Builder::new()
            .name("hide-attachment-cleanup".to_owned())
            .spawn(move || {
                if clipboard {
                    ingress::remove_clipboard(node.as_ref(), &state_path, &request_id);
                }
                if let (Some(transport), Some(files)) = (transport, files) {
                    transport.remove_attachments(&request_id, &files);
                }
                let Some(runtime) = context.runtime.upgrade() else {
                    return;
                };
                if let Ok(mut runtime) = runtime.lock()
                    && runtime
                        .attachment
                        .as_ref()
                        .is_some_and(|pending| pending.operation.id == request_id)
                {
                    runtime.clear_attachment();
                    runtime.sync_async_operations();
                }
                context.notifier.notify();
            }) {
            Ok(worker) => self.attachment_worker = Some(worker),
            Err(_) => {
                self.clear_attachment();
                crate::diagnostic!(
                    serde_json::json!({"kind":"terminal.attachment.cleanup_worker_failed"})
                );
            }
        }
    }

    fn start_attachment_worker(&mut self) {
        let Some(context) = self.worker_context.clone() else {
            self.fail_attachment("The file transfer worker is unavailable.", false);
            return;
        };
        if self
            .attachment_worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
        {
            return;
        }
        let Some(pending) = self.attachment.as_mut() else {
            return;
        };
        pending.operation.phase = "pending".to_owned();
        pending.operation.stage = "transfer".to_owned();
        pending.operation.retryable = false;
        pending.operation.message = Some(
            if pending.remote.is_some() && !pending.on_pane_machine {
                "Uploading files… Temporary files expire after 24 hours on a later paste."
            } else if pending.clipboard {
                "Preparing image… Temporary files expire after 24 hours on a later paste."
            } else {
                "Preparing files…"
            }
            .to_owned(),
        );
        pending.cancelled = Arc::new(AtomicBool::new(false));
        let cancelled = pending.cancelled.clone();
        let request_id = pending.operation.id.clone();
        let paths = pending.paths.clone();
        let prepared = pending.prepared.clone();
        let on_pane_machine = pending.on_pane_machine;
        let transport = pending
            .remote
            .as_ref()
            .filter(|_| !pending.on_pane_machine)
            .and_then(|(target, _)| self.remote_file_transports.get(target))
            .cloned();
        let clipboard = pending.clipboard;
        let state_path = self.state_path.clone();
        // The picked files are on the core's own machine until a screen can
        // attach from another node.
        let node = self.own_node();
        match thread::Builder::new()
            .name("hide-terminal-attachment".to_owned())
            .spawn(move || {
                // Files already on the pane's machine are neither read nor
                // copied: their paths are the paste.
                let prepared = match prepared {
                    Some(files) => Ok(files),
                    None if on_pane_machine => Ok(Arc::new(Vec::new())),
                    None => ingress::read_sources(node.as_ref(), &paths, &cancelled).map(Arc::new),
                };
                let result = if on_pane_machine {
                    Ok(paths.clone())
                } else {
                    prepared
                        .as_ref()
                        .map_err(Clone::clone)
                        .and_then(|files| match &transport {
                            Some(transport) => {
                                transport.stage_attachments(&request_id, files, &cancelled)
                            }
                            None => Ok(files.iter().map(|file| file.path.clone()).collect()),
                        })
                };
                let remote_uploaded = transport.is_some() && result.is_ok();
                let cleanup_files = prepared.as_ref().ok().cloned();
                let Some(runtime) = context.runtime.upgrade() else {
                    if clipboard {
                        ingress::remove_clipboard(node.as_ref(), &state_path, &request_id);
                    }
                    if let (Some(transport), Some(files)) = (transport, cleanup_files) {
                        transport.remove_attachments(&request_id, &files);
                    }
                    return;
                };
                let changed = match runtime.lock() {
                    Ok(mut runtime) => {
                        runtime.finish_attachment(&request_id, prepared.ok(), result)
                    }
                    Err(_) => return,
                };
                let cleaned_cancellation = cancelled.load(Ordering::Acquire);
                if clipboard && (cleaned_cancellation || remote_uploaded) {
                    ingress::remove_clipboard(node.as_ref(), &state_path, &request_id);
                }
                if cleaned_cancellation
                    && let (Some(transport), Some(files)) = (&transport, &cleanup_files)
                {
                    transport.remove_attachments(&request_id, files);
                }
                // All I/O is finished before releasing worker ownership. An immediate
                // retry is either admitted now or handed off here, even without a live tick.
                let draining_cleanup = match runtime.lock() {
                    Ok(mut runtime) => runtime.complete_attachment_worker(cleaned_cancellation),
                    Err(_) => false,
                };
                // Drop already owns this worker's JoinHandle. Complete any cancellation
                // that arrived after the first check here, never in an unjoined child.
                if draining_cleanup {
                    if clipboard {
                        ingress::remove_clipboard(node.as_ref(), &state_path, &request_id);
                    }
                    if let (Some(transport), Some(files)) = (&transport, &cleanup_files) {
                        transport.remove_attachments(&request_id, files);
                    }
                }
                if changed {
                    context.notifier.notify();
                }
            }) {
            Ok(worker) => self.attachment_worker = Some(worker),
            Err(_) => {
                self.fail_attachment("Could not start the file transfer worker. Retry.", true)
            }
        }
        self.sync_async_operations();
    }

    fn complete_attachment_worker(&mut self, cleaned_cancellation: bool) -> bool {
        // None means destruction took the handle and is joining this exact worker.
        let draining = self.attachment_worker.take().is_none();
        let needs_cleanup = self.attachment.as_ref().is_some_and(|pending| {
            ["cancelling", "retired"].contains(&pending.operation.stage.as_str())
        });
        if needs_cleanup {
            if draining || cleaned_cancellation {
                self.clear_attachment();
                self.sync_async_operations();
            } else {
                self.start_attachment_cleanup();
            }
        } else if !draining
            && self
                .attachment
                .as_ref()
                .is_some_and(|pending| pending.operation.stage == "retry_wait")
        {
            self.start_attachment_worker();
        }
        draining && needs_cleanup && !cleaned_cancellation
    }

    fn finish_attachment(
        &mut self,
        request_id: &str,
        prepared: Option<Arc<Vec<AttachmentFile>>>,
        result: Result<Vec<String>, String>,
    ) -> bool {
        let Some(pending) = self
            .attachment
            .as_ref()
            .filter(|pending| pending.operation.id == request_id)
        else {
            return false;
        };
        if pending.operation.stage == "cancelling" {
            self.clear_attachment();
            self.sync_async_operations();
            return true;
        }
        if !self.attachment_target_valid(pending) || pending.operation.stage == "retired" {
            let pending = self.attachment.as_mut().expect("matched");
            pending.cancelled.store(true, Ordering::Release);
            pending.prepared = None;
            pending.operation.stage = "retired".to_owned();
            let intent = pending.operation.id.clone();
            self.terminals
                .control(TerminalControl::AttachmentRefuse { intent });
            self.fail_attachment("The original terminal closed or reconnected. No files or held input were sent. Paste again in the new terminal.", false);
            return true;
        }
        let pending = self.attachment.as_mut().expect("matched");
        pending.prepared = prepared;
        if pending.rejected_bytes > 0 {
            self.fail_attachment(
                "Held input exceeded the 64 KiB limit. Cancel and paste again.",
                false,
            );
            return true;
        }
        match result {
            Err(error) => {
                crate::diagnostic!(
                    serde_json::json!({"kind":"terminal.attachment.failed", "request_id":request_id, "pane_id":pending.pane_id, "error":error})
                );
                self.fail_attachment(&error, true);
            }
            Ok(paths) => {
                match ingress::paste_bytes(&paths, pending.bracketed, pending.remote.is_some()) {
                    Ok(paste) => {
                        pending.paste = Some(paste);
                        self.deliver_attachment();
                    }
                    Err(error) => {
                        self.fail_attachment(&error, false);
                        return true;
                    }
                }
            }
        }
        self.sync_async_operations();
        true
    }

    /// Asks the node to write the ready paste, and the input held behind it,
    /// to the pane's session, or keeps both while that session is awaited.
    fn deliver_attachment(&mut self) {
        let controlling = self
            .attachment
            .as_ref()
            .is_some_and(|pending| self.terminal_controlling(&pending.pane_id));
        let Some(pending) = self.attachment.as_mut() else {
            return;
        };
        let Some(paste) = pending.paste.as_ref() else {
            return;
        };
        if controlling {
            let control = TerminalControl::AttachmentDeliver {
                intent: pending.operation.id.clone(),
                generation: pending.terminal_generation.unwrap_or_default(),
                paste: hide_node_link::terminal::encode_base64(paste),
            };
            pending.writing = true;
            self.terminals.control(control);
            return;
        }
        if pending.awaiting_terminal {
            pending.operation.stage = "terminal".to_owned();
            pending.operation.message = Some("Waiting for the terminal to attach…".to_owned());
            self.sync_async_operations();
            return;
        }
        self.fail_attachment(TERMINAL_UNAVAILABLE, false);
    }
}

const TERMINAL_UNAVAILABLE: &str =
    "The original terminal is unavailable. Reconnect, cancel this transfer and paste again.";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::tests::{
        RecordedTerminals, record_terminals, report_terminal, terminal_state,
    };
    use hide_node_link::terminal::TerminalReport;
    const ID: &str = "01234567-0123-0123-0123-0123456789ab";
    const OTHER_ID: &str = "01234567-0123-0123-0123-0123456789ac";

    fn runtime() -> Runtime {
        let folder = tempfile::tempdir().unwrap();
        let mut runtime = Runtime::new(
            CoreOptions {
                schema_version: SCHEMA_VERSION,
                home: None,
                node_id: crate::node::test_node(),
                herdr_socket_path: None,
                herdr_bin_path: None,
                app_state_path: folder
                    .path()
                    .join("state.json")
                    .to_string_lossy()
                    .into_owned(),
                host_helper_root: None,
                host_cli_dir: None,
                workspace_views_path: None,
                shortcut_import_path: None,
                local_issues_path: None,
                effects_held: false,
                machine_name: None,
                account_state_folder: false,
            },
            environment::EnvironmentReport {
                statuses: Vec::new(),
                home_path: None,
                codex_home: None,
            },
            std::sync::Arc::new(hide_node::Local::of_process()),
            crate::node::test_devices(),
        );
        runtime.test_dirs.push(folder);
        runtime.ensure_terminal_pane("pane-one");
        runtime
    }

    fn begin(runtime: &mut Runtime, id: &str, pane: &str) {
        assert!(runtime.begin_attachment(AttachmentPayload {
            request_id: id.to_owned(),
            pane_id: pane.to_owned(),
            bracketed_paste: true,
            clipboard: true,
            paths: Vec::new(),
            staged_on: None,
        }));
    }

    /// Only a pane's own machine may say a paste's files are already there:
    /// naming another machine is refused before any key is held.
    #[test]
    fn files_said_to_be_on_another_machine_than_the_panes_are_refused() {
        let mut runtime = runtime();
        assert!(runtime.begin_attachment(AttachmentPayload {
            request_id: ID.to_owned(),
            pane_id: "pane-one".to_owned(),
            bracketed_paste: true,
            clipboard: false,
            paths: vec!["/Users/example/shot.png".to_owned()],
            staged_on: Some("mini".to_owned()),
        }));
        assert!(runtime.attachment.is_none());
        let refused = runtime.attachment_rejection.as_ref().expect("refused");
        assert_eq!(refused.phase, "refused");
        assert_eq!(
            refused.message.as_deref(),
            Some("Invalid attachment request identity.")
        );
    }

    /// The routes the runtime reaches its node by, with `pane-one`
    /// attached in control.
    fn controlled(runtime: &mut Runtime) -> Arc<RecordedTerminals> {
        let terminals = record_terminals(runtime);
        report_terminal(runtime, "pane-one", terminal_state("controlling", 1));
        terminals.take();
        terminals
    }

    /// The paste each AttachmentDeliver asked for, in order.
    fn delivered(terminals: &RecordedTerminals) -> Vec<Vec<u8>> {
        terminals
            .take()
            .into_iter()
            .filter_map(|control| match control {
                TerminalControl::AttachmentDeliver { intent, paste, .. } => {
                    assert_eq!(intent, ID);
                    Some(hide_node_link::terminal::decode_base64(&paste).unwrap())
                }
                _ => None,
            })
            .collect()
    }

    fn written(runtime: &mut Runtime, intent: &str) -> bool {
        runtime.ingest_terminal_reports(vec![TerminalReport::AttachmentDelivered {
            intent: intent.to_owned(),
            written: true,
        }])
    }

    /// The node holds the keys typed behind a paste and writes them after it
    /// (`hide-node` `a_paste_is_written_before_the_keys_typed_behind_it`);
    /// the core asks it to hold, and to write once the files are staged.
    #[test]
    fn failed_attachment_holds_enter_and_success_releases_in_order_without_extra_publication() {
        let mut runtime = runtime();
        let terminals = controlled(&mut runtime);
        begin(&mut runtime, ID, "pane-one");
        assert!(terminals.take().contains(&TerminalControl::AttachmentHold {
            pane: "pane-one".into(),
            intent: ID.into()
        }));
        runtime.finish_attachment(
            ID,
            None,
            Err("A file exceeds the 20 MiB attachment limit.".to_owned()),
        );
        assert!(delivered(&terminals).is_empty());
        assert!(
            runtime
                .attachment
                .as_ref()
                .unwrap()
                .operation
                .message
                .as_ref()
                .unwrap()
                .contains("20 MiB")
        );
        runtime.finish_attachment(ID, None, Ok(vec!["/remote/private/image.png".to_owned()]));
        assert_eq!(
            delivered(&terminals),
            [b"\x1b[200~\"/remote/private/image.png\"\x1b[201~".to_vec()]
        );
        assert!(written(&mut runtime, ID));
        assert!(runtime.attachment.is_none());
        // A written paste is released too, so the router forgets which node
        // held it rather than keep one route per paste.
        assert!(
            terminals
                .take()
                .contains(&TerminalControl::AttachmentRelease { intent: ID.into() })
        );
    }

    #[test]
    fn input_limit_is_visible_and_never_sends_partial_input() {
        let mut runtime = runtime();
        let terminals = controlled(&mut runtime);
        begin(&mut runtime, ID, "pane-one");
        assert!(
            runtime.ingest_terminal_reports(vec![TerminalReport::AttachmentInput {
                intent: ID.to_owned(),
                outcome: AttachmentInputOutcome::Limit,
                bytes: 1,
            }])
        );
        let pending = runtime.attachment.as_ref().unwrap();
        assert!(
            pending
                .operation
                .message
                .as_ref()
                .unwrap()
                .contains("64 KiB")
        );
        runtime.finish_attachment(ID, None, Ok(vec!["/remote/file.png".to_owned()]));
        assert!(delivered(&terminals).is_empty());
        assert!(!runtime.attachment.as_ref().unwrap().operation.retryable);
    }

    #[test]
    fn competing_paste_dismissal_preserves_original_and_cancel_discards_held_input() {
        let mut runtime = runtime();
        let terminals = controlled(&mut runtime);
        begin(&mut runtime, ID, "pane-one");
        begin(&mut runtime, OTHER_ID, "pane-one");
        assert_eq!(
            runtime.attachment_rejection.as_ref().unwrap().phase,
            "refused"
        );
        terminals.take();
        runtime.attachment_action(AttachmentActionPayload {
            request_id: OTHER_ID.to_owned(),
            pane_id: "pane-one".to_owned(),
            action: "cancel".to_owned(),
        });
        assert!(
            terminals.take().is_empty(),
            "dismissing the refused paste touched the original's held input"
        );
        runtime.attachment_action(AttachmentActionPayload {
            request_id: ID.to_owned(),
            pane_id: "pane-one".to_owned(),
            action: "cancel".to_owned(),
        });
        assert!(
            terminals
                .take()
                .contains(&TerminalControl::AttachmentRefuse { intent: ID.into() })
        );
        runtime.attachment_ready(AttachmentCompletionPayload {
            request_id: ID.to_owned(),
            pane_id: "pane-one".to_owned(),
            error: None,
        });
        assert!(runtime.attachment.is_none());
        let controls = terminals.take();
        assert!(
            controls.contains(&TerminalControl::AttachmentRelease { intent: ID.into() }),
            "{controls:?}"
        );
        assert!(
            !controls
                .iter()
                .any(|control| matches!(control, TerminalControl::AttachmentDeliver { .. }))
        );
    }

    /// The paste is composer text the operator has not sent. That an Enter
    /// held behind it is not a send either is the node's
    /// (`a_paste_is_written_before_the_keys_typed_behind_it`).
    #[test]
    fn a_pasted_attachment_is_a_draft_for_the_doorbell_even_with_an_enter_held_behind_it() {
        let mut runtime = runtime();
        let _terminals = controlled(&mut runtime);
        let payload: crate::sidebar::SessionSnapshotPayload = serde_json::from_value(
            serde_json::json!({"agents":[{"id":"a","pane_id":"pane-one","agent":"claude","agent_status":"idle","state_change_seq":1,"lineage_session":"s"}]}),
        )
        .unwrap();
        runtime.observe_delivery(crate::node::TEST_NODE, &payload, None, None);
        let clocks = |runtime: &mut Runtime| {
            let observation = runtime.delivery_observations.get_mut("pane-one").unwrap();
            let read = (
                observation.last_input_at_unix_ms,
                observation.last_submit_at_unix_ms,
            );
            observation.last_input_at_unix_ms = 0;
            observation.last_submit_at_unix_ms = 0;
            read
        };
        clocks(&mut runtime);
        begin(&mut runtime, ID, "pane-one");
        runtime.finish_attachment(ID, None, Ok(vec!["/remote/file.png".to_owned()]));
        assert!(written(&mut runtime, ID));
        let (input, submit) = clocks(&mut runtime);
        assert!(input > 0 && submit == 0, "pasted text is an unsent draft");
    }

    #[test]
    fn changed_generation_retires_intent_and_unknown_remote_never_falls_back_locally() {
        let mut runtime = runtime();
        let terminals = controlled(&mut runtime);
        begin(&mut runtime, ID, "pane-one");
        terminals.take();
        report_terminal(&mut runtime, "pane-one", terminal_state("controlling", 99));
        assert_eq!(
            runtime.attachment.as_ref().unwrap().operation.stage,
            "retired"
        );
        assert!(
            terminals
                .take()
                .contains(&TerminalControl::AttachmentRefuse { intent: ID.into() })
        );
        runtime.attachment_ready(AttachmentCompletionPayload {
            request_id: ID.to_owned(),
            pane_id: "pane-one".to_owned(),
            error: None,
        });
        assert!(runtime.attachment.is_none());
        runtime.ensure_terminal_pane("remote:missing:pane");
        begin(&mut runtime, OTHER_ID, "remote:missing:pane");
        assert_eq!(
            runtime.attachment.as_ref().unwrap().operation.phase,
            "failed"
        );
        assert!(!runtime.attachment.as_ref().unwrap().operation.retryable);
        assert!(delivered(&terminals).is_empty());
    }

    #[test]
    fn cancel_after_completion_and_retirement_release_the_admission_slot() {
        for retire in [false, true] {
            let mut runtime = runtime();
            let _terminals = controlled(&mut runtime);
            begin(&mut runtime, ID, "pane-one");
            runtime.attachment.as_mut().unwrap().clipboard_preparing = false;
            runtime.attachment.as_mut().unwrap().operation.stage = "transfer".to_owned();
            let gate = Arc::new(std::sync::Barrier::new(2));
            let worker_gate = gate.clone();
            runtime.attachment_worker = Some(thread::spawn(move || {
                worker_gate.wait();
            }));
            runtime.finish_attachment(ID, None, Err("Transfer failed.".to_owned()));
            if retire {
                report_terminal(&mut runtime, "pane-one", terminal_state("controlling", 99));
                assert_eq!(
                    runtime.attachment.as_ref().unwrap().operation.stage,
                    "retired"
                );
            } else {
                runtime.attachment_action(AttachmentActionPayload {
                    request_id: ID.to_owned(),
                    pane_id: "pane-one".to_owned(),
                    action: "cancel".to_owned(),
                });
            }
            gate.wait();
            assert!(!runtime.complete_attachment_worker(false));
            assert!(runtime.attachment.is_none());
            runtime.ensure_terminal_pane("pane-two");
            begin(&mut runtime, OTHER_ID, "pane-two");
            assert_eq!(
                runtime.attachment.as_ref().unwrap().operation.phase,
                "pending"
            );
        }
    }

    /// A runtime attached to Herdr whose pane is on screen before its
    /// terminal session, the order the web shows a new pane in.
    fn live_pane() -> (Runtime, Arc<RecordedTerminals>) {
        let mut runtime = crate::runtime::tests::live_runtime();
        let terminals = record_terminals(&mut runtime);
        runtime.ensure_terminal_pane("pane-one");
        runtime
            .terminal_sizes
            .insert("pane-one".to_owned(), (24, 80));
        (runtime, terminals)
    }

    const STAGED: &str = "/state/attachments/image.png";

    /// Issue 735: a drop reached the core 3 ms before the pane's first
    /// session was requested, failed as unavailable, and nothing was pasted.
    /// The paste now waits for that session and is written to it, whichever
    /// of the files and the session is first; the input typed behind it
    /// follows it there at the node.
    #[test]
    fn a_drop_before_the_panes_first_session_is_written_to_that_session() {
        for files_first in [true, false] {
            let (mut runtime, terminals) = live_pane();
            begin(&mut runtime, ID, "pane-one");
            assert_eq!(
                runtime.attachment.as_ref().unwrap().operation.phase,
                "pending"
            );
            if files_first {
                runtime.finish_attachment(ID, None, Ok(vec![STAGED.to_owned()]));
                assert_eq!(
                    runtime.attachment.as_ref().unwrap().operation.phase,
                    "pending"
                );
            }
            report_terminal(&mut runtime, "pane-one", terminal_state("controlling", 3));
            if !files_first {
                runtime.finish_attachment(ID, None, Ok(vec![STAGED.to_owned()]));
            }
            let controls = terminals.take();
            let deliveries = controls
                .iter()
                .filter_map(|control| match control {
                    TerminalControl::AttachmentDeliver {
                        generation, paste, ..
                    } => Some((
                        *generation,
                        hide_node_link::terminal::decode_base64(paste).unwrap(),
                    )),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(
                deliveries,
                [(
                    3,
                    b"\x1b[200~\"/state/attachments/image.png\"\x1b[201~".to_vec()
                )],
                "files_first={files_first}"
            );
            assert!(written(&mut runtime, ID));
            assert!(runtime.attachment.is_none(), "files_first={files_first}");
        }
    }

    /// The wait ends without any event: the coordinator's tick fails a paste
    /// whose terminal never attached once its staged files stop being kept,
    /// the slot is free for the next drop, and a late session gets nothing.
    #[test]
    fn a_drop_whose_terminal_never_attaches_fails_at_the_commit_grace() {
        let (mut runtime, terminals) = live_pane();
        begin(&mut runtime, ID, "pane-one");
        runtime.attachment.as_mut().unwrap().clipboard_preparing = false;
        runtime.finish_attachment(ID, None, Ok(vec![STAGED.to_owned()]));
        let operation = &runtime.attachment.as_ref().unwrap().operation;
        let deadline = operation.started_at_unix_ms + ingress::COMMIT_GRACE.as_millis() as u64;
        assert_eq!(operation.deadline_at_unix_ms, Some(deadline));

        runtime.tick_async_operations(deadline - 1);
        assert_eq!(
            runtime.attachment.as_ref().unwrap().operation.phase,
            "pending"
        );
        let (_, records) = crate::diagnostics::capture(|| runtime.tick_async_operations(deadline));
        assert!(
            records
                .iter()
                .any(|record| record["kind"] == "terminal.attachment.terminal_wait_expired"),
            "{records:?}"
        );
        assert!(runtime.attachment.is_none());

        report_terminal(&mut runtime, "pane-one", terminal_state("controlling", 1));
        assert!(delivered(&terminals).is_empty());
        begin(&mut runtime, OTHER_ID, "pane-one");
        assert_eq!(
            runtime.attachment.as_ref().unwrap().operation.phase,
            "pending"
        );
    }

    /// While the session is awaited, the pane leaving or its session being
    /// replaced still retires the paste, as it does after a session attached.
    #[test]
    fn an_awaited_pane_that_leaves_or_reattaches_retires_the_paste() {
        for leaves in [true, false] {
            let (mut runtime, terminals) = live_pane();
            runtime
                .terminal_states
                .insert("pane-one".to_owned(), terminal_state("starting", 7));
            begin(&mut runtime, ID, "pane-one");
            if leaves {
                runtime.snapshot.terminal.panes.clear();
            } else {
                runtime
                    .terminal_states
                    .insert("pane-one".to_owned(), terminal_state("starting", 8));
            }
            assert!(runtime.reconcile_attachment_target());
            let pending = runtime.attachment.as_ref().unwrap();
            assert_eq!(pending.operation.stage, "retired", "leaves={leaves}");
            assert!(
                terminals
                    .take()
                    .contains(&TerminalControl::AttachmentRefuse { intent: ID.into() })
            );
            runtime.finish_attachment(ID, None, Ok(vec![STAGED.to_owned()]));
            assert!(delivered(&terminals).is_empty());
        }
    }

    #[test]
    fn destruction_drains_its_joined_worker_without_spawning_cleanup_or_retry() {
        let mut runtime = runtime();
        let terminals = controlled(&mut runtime);
        begin(&mut runtime, ID, "pane-one");
        runtime.attachment.as_mut().unwrap().clipboard_preparing = false;
        let gate = Arc::new(std::sync::Barrier::new(2));
        let worker_gate = gate.clone();
        let worker = thread::spawn(move || {
            worker_gate.wait();
        });
        runtime.attachment_worker = Some(worker);
        let joined = runtime.take_attachment_worker().unwrap();
        assert!(
            runtime.complete_attachment_worker(false),
            "the original joined worker owns remaining cleanup"
        );
        assert!(runtime.attachment.is_none() && runtime.attachment_worker.is_none());
        gate.wait();
        joined.join().unwrap();
        assert!(delivered(&terminals).is_empty());
    }
}
