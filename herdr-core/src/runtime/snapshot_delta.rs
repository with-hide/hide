use std::sync::Arc;

use super::*;

/// Turns a taken delta into the bytes the shell reads.
///
/// It takes the payload and nothing else. That signature is the guarantee the
/// runtime mutex is not held here: there is no runtime in scope to lock. The
/// caller takes a payload under the lock, drops the guard, and calls this.
pub fn serialize_snapshot_delta(
    payload: &crate::model::SnapshotDeltaPayload,
) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&crate::model::SnapshotDeltaWire::borrow(payload))
}

/// Revision bookkeeping for the delta snapshot wire. Revisions are stamped
/// lazily at read time. The rest and editor sections are compared against
/// the last stamped copy; the changes section and each View document carry
/// an edit number ([`crate::model::Edited`]) taken wherever they may change,
/// so a read compares two numbers instead of several diffs or a whole file.
/// A change between reads always leaves a number the stamp does not hold,
/// so it is never swallowed.
#[derive(Default)]
pub(super) struct DeltaState {
    revision: u64,
    rest_revision: u64,
    usage_revision: u64,
    usage_edit: u64,
    last_usage: Option<Arc<crate::usage::model::UsageSnapshot>>,
    editor_revision: u64,
    changes_revision: u64,
    memory_revision: u64,
    /// `None` until Memory has a Project open or a pane's received cards; the
    /// section is never unset after that.
    last_memory: Option<Arc<crate::model::MemorySnapshot>>,
    link_summaries_revision: u64,
    last_link_summaries: Option<Arc<crate::links::LinkSummariesSnapshot>>,
    link_panel_revision: u64,
    last_link_panel: Option<Arc<crate::links::LinkPanelSnapshot>>,
    session_record_revision: u64,
    last_session_record: Option<Arc<crate::links::ledger::SessionRecordSnapshot>>,
    session_work_search_revision: u64,
    last_session_work_search: Option<Arc<crate::links::ledger::SessionSearchSnapshot>>,
    /// Reference-counted so a delta can carry the section out of the lock
    /// without copying it. The runtime never mutates one in place: a changed
    /// section becomes a new `Arc`, which leaves any payload already handed
    /// out holding the state it was taken at.
    last_rest: Option<Arc<crate::model::RestSections>>,
    last_editor: Option<Arc<crate::model::EditorSnapshot>>,
    last_changes: Option<Arc<crate::model::ChangesSnapshot>>,
    /// The edit number `last_changes` was copied at.
    last_changes_edit: u64,
    /// The View documents on screen, each stamped with its own revision
    /// (PRD S7 contract 3.1), so a keystroke re-sends only its document.
    /// Empty in a shell without View areas.
    documents: HashMap<String, StampedDocument>,
    documents_visible: Vec<String>,
    documents_visible_revision: u64,
}

struct StampedDocument {
    revision: u64,
    /// The edit number `document` was copied at.
    edit: u64,
    document: Arc<EditorDocumentSnapshot>,
}

impl Runtime {
    /// Takes one delta response for the snapshot wire: sections whose revision
    /// passed `have_revision`.
    /// Reading is idempotent - the same cursors return the same delta again -
    /// so a caller that failed to apply a response recovers by re-reading with
    /// its unadvanced cursor.
    ///
    /// This is the half that needs the runtime, and it is deliberately the
    /// only half: it stamps revisions and copies out what the wire needs, and
    /// `serialize_snapshot_delta` turns that into bytes with the lock already
    /// released. Serializing here would put the whole navigator, ui state and
    /// terminal output through `serde_json` while every attach thread and the
    /// shell's next read wait on the mutex.
    pub fn snapshot_delta_payload(
        &mut self,
        have_revision: u64,
    ) -> crate::model::SnapshotDeltaPayload {
        use crate::model::{RestSections, SnapshotDeltaPayload};

        // A front Workspace moved by Herdr or by a device's own focus, not by
        // an event, is followed here, before anything is stamped.
        self.sync_workspace_view();
        // What a screen is about to be shown as the keyboard's pane is what
        // the nodes compare its next key with.
        self.sync_terminal_focus();
        // A plan's review moved by a command, the restore or a deleted
        // session is laid on the rows shown, before anything reads them.
        self.refresh_plan_reviews();
        // A mission's map follows its workers, its Conductor's pane and its
        // pull requests; the section is sent again only when it moved.
        self.refresh_missions();
        // The open mission's events, sent for that mission only.
        self.refresh_mission_events();
        // A pane's plan follows its session and the plan book.
        self.refresh_plans();
        self.note_snapshot_sent();
        if self.delta.usage_edit != self.usage_edit {
            self.delta.revision += 1;
            self.delta.usage_revision = self.delta.revision;
            self.delta.usage_edit = self.usage_edit;
            self.delta.last_usage = Some(Arc::new(self.snapshot.usage.clone()));
        }
        let rest = RestSections::capture(
            &self.snapshot,
            self.delta.last_rest.as_deref(),
            self.delta.revision + 1,
        );
        if rest.revision() > self.delta.rest_revision {
            self.delta.revision = rest.revision();
            self.delta.rest_revision = self.delta.revision;
            self.delta.last_rest = Some(Arc::new(rest));
        }
        if self.delta.last_editor.as_deref() != Some(&self.snapshot.editor) {
            self.delta.revision += 1;
            self.delta.editor_revision = self.delta.revision;
            self.delta.last_editor = Some(Arc::new(self.snapshot.editor.clone()));
        }
        let changes_edit = self.snapshot.changes.edit_number();
        if self.delta.last_changes.is_none() || self.delta.last_changes_edit != changes_edit {
            self.delta.revision += 1;
            self.delta.changes_revision = self.delta.revision;
            self.delta.last_changes = Some(Arc::new(crate::model::ChangesSnapshot::clone(
                &self.snapshot.changes,
            )));
            self.delta.last_changes_edit = changes_edit;
        }
        let views = self.separate_view_areas();
        if views {
            self.stamp_view_documents();
        }
        if let Some(memory) = &self.snapshot.memory
            && self.delta.last_memory.as_deref() != Some(memory)
        {
            self.delta.revision += 1;
            self.delta.memory_revision = self.delta.revision;
            self.delta.last_memory = Some(Arc::new(memory.clone()));
        }
        if let Some(summaries) = &self.snapshot.link_summaries
            && self.delta.last_link_summaries.as_deref() != Some(summaries)
        {
            self.delta.revision += 1;
            self.delta.link_summaries_revision = self.delta.revision;
            self.delta.last_link_summaries = Some(Arc::new(summaries.clone()));
        }
        if let Some(panel) = &self.snapshot.link_panel
            && self.delta.last_link_panel.as_deref() != Some(panel)
        {
            self.delta.revision += 1;
            self.delta.link_panel_revision = self.delta.revision;
            self.delta.last_link_panel = Some(Arc::new(panel.clone()));
        }
        // Worker answers are immutable Arcs; comparing identity costs no
        // request-body walk on a terminal notification.
        if let Some(record) = &self.snapshot.session_record
            && self
                .delta
                .last_session_record
                .as_ref()
                .is_none_or(|last| !Arc::ptr_eq(last, record))
        {
            self.delta.revision += 1;
            self.delta.session_record_revision = self.delta.revision;
            self.delta.last_session_record = Some(Arc::clone(record));
        }
        if let Some(search) = &self.snapshot.session_work_search
            && self
                .delta
                .last_session_work_search
                .as_ref()
                .is_none_or(|last| !Arc::ptr_eq(last, search))
        {
            self.delta.revision += 1;
            self.delta.session_work_search_revision = self.delta.revision;
            self.delta.last_session_work_search = Some(Arc::clone(search));
        }
        // A cursor from the future has no valid meaning in-process; treat it
        // as a fresh reader so the response converges on full state.
        let have_revision = if have_revision > self.delta.revision {
            0
        } else {
            have_revision
        };

        SnapshotDeltaPayload {
            have_revision,
            current_rest: Arc::clone(self.delta.last_rest.as_ref().expect("rest is stamped")),
            schema_version: self.snapshot.schema_version,
            revision: self.delta.revision,
            usage: self
                .delta
                .last_usage
                .as_ref()
                .filter(|_| self.delta.usage_revision > have_revision)
                .map(Arc::clone),
            // The retained copies were compared against the live snapshot
            // above and rebuilt where they differed, so each is the live
            // section and costs a refcount instead of a copy. All three are
            // stamped by that block, so a missing one is a broken invariant
            // and not a section to send as null.
            rest: (self.delta.rest_revision > have_revision).then(|| {
                Arc::clone(
                    self.delta
                        .last_rest
                        .as_ref()
                        .expect("the rest section is stamped before a delta is taken"),
                )
            }),
            editor: (self.delta.editor_revision > have_revision).then(|| {
                Arc::clone(
                    self.delta
                        .last_editor
                        .as_ref()
                        .expect("the editor section is stamped before a delta is taken"),
                )
            }),
            changes: (self.delta.changes_revision > have_revision).then(|| {
                Arc::clone(
                    self.delta
                        .last_changes
                        .as_ref()
                        .expect("the changes section is stamped before a delta is taken"),
                )
            }),
            documents: views
                .then(|| self.view_documents_delta(have_revision))
                .flatten(),
            memory: self
                .delta
                .last_memory
                .as_ref()
                .filter(|_| self.delta.memory_revision > have_revision)
                .map(Arc::clone),
            link_summaries: self
                .delta
                .last_link_summaries
                .as_ref()
                .filter(|_| self.delta.link_summaries_revision > have_revision)
                .map(Arc::clone),
            link_panel: self
                .delta
                .last_link_panel
                .as_ref()
                .filter(|_| self.delta.link_panel_revision > have_revision)
                .map(Arc::clone),
            session_record: self
                .delta
                .last_session_record
                .as_ref()
                .filter(|_| self.delta.session_record_revision > have_revision)
                .map(Arc::clone),
            session_work_search: self
                .delta
                .last_session_work_search
                .as_ref()
                .filter(|_| self.delta.session_work_search_revision > have_revision)
                .map(Arc::clone),
            // Cloning a section clones the summary's `Arc` and at most the
            // capped answers, never the cards.
            find: self.snapshot.find.clone(),
        }
    }

    /// Stamps each visible View document edited since its last stamped
    /// copy, and the visible set when it changed. Per visible document a
    /// read costs two map lookups and one number comparison; a document is
    /// copied only when it was edited.
    fn stamp_view_documents(&mut self) {
        let visible = self.visible_view_documents();
        for tab_id in &visible {
            let Some(document) = self.editor_documents.get(tab_id) else {
                continue;
            };
            let edit = document.edit_number();
            if self
                .delta
                .documents
                .get(tab_id)
                .is_some_and(|stamped| stamped.edit == edit)
            {
                continue;
            }
            self.delta.revision += 1;
            self.delta.documents.insert(
                tab_id.clone(),
                StampedDocument {
                    revision: self.delta.revision,
                    edit,
                    document: Arc::new(EditorDocumentSnapshot::clone(document)),
                },
            );
        }
        // A document off screen is dropped, so it is sent again, whole, when
        // it comes back: the shell drops it too.
        self.delta
            .documents
            .retain(|tab_id, _| visible.contains(tab_id));
        if visible != self.delta.documents_visible {
            self.delta.revision += 1;
            self.delta.documents_visible_revision = self.delta.revision;
            self.delta.documents_visible = visible;
        }
    }

    /// The `documents` section for a reader at `have_revision`: absent when
    /// nothing in it changed past that revision, complete for a fresh reader.
    fn view_documents_delta(&self, have_revision: u64) -> Option<crate::model::DocumentsDelta> {
        let changed: Vec<_> = self
            .delta
            .documents_visible
            .iter()
            .filter_map(|tab_id| {
                let stamped = self.delta.documents.get(tab_id)?;
                (stamped.revision > have_revision)
                    .then(|| (tab_id.clone(), Arc::clone(&stamped.document)))
            })
            .collect();
        (have_revision == 0
            || self.delta.documents_visible_revision > have_revision
            || !changed.is_empty())
        .then(|| crate::model::DocumentsDelta {
            visible: self.delta.documents_visible.clone(),
            changed,
        })
    }
}
