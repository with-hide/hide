import { OpenerMark } from "./ViewComments";
import { downloadFailureText, readFailureText } from "./fileFailure";
import { useEffect, useMemo, useRef, useState } from "react";
import type { Actions } from "./actions";
import { adoptNodeDrafts, allBuffers, bufferDecision, bufferFor, claimLegacyBuffer, deleteBuffer, draftStorageHold, flushBuffer, identity, MAX_STORED_BYTES, queueBuffer, tabBufferKey, type BufferKey } from "./buffers";
import { useInterfaceTranslation } from "./i18n/client";
import { Button } from "./components/ui/button";
import { Hint } from "./components/ui/tooltip";
import { CodeMirrorEditor } from "./editor/CodeMirrorEditor";
import { PatchView } from "./editor/PatchView";
import { clearDraft, closingWithSave, latestDraft, noteDraft } from "./editor/draft";
import { downloadFile } from "./fileBytes";
import { relativeTo } from "./explorer";
import { changesFor, editorTabFor, frontCheckout, localDeviceId, type EditorDocumentSnapshot, type EditorTabSnapshot, type ViewDisplaySnapshot } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { FileViewer } from "./viewers/FileViewer";
import { useFileSource } from "./viewers/useFileBytes";
import { commandLabel } from "./shortcutLabels";
import { absolutePathRoot } from "./terminalLinks";
import { RegionCommentButton } from "./ViewComments";
import { diffNoticeText } from "./diffNotice";
import { isVideoPath } from "./viewers/video";
import type { ViewWorkspace } from "./viewLayout";

// The document surface (PRD B3-B7; S7 B4, B5, A9, A10). The core owns the
// documents and the displays that show them; this draws one display - its
// document from the `documents` section, or its diff from `changes.diffs` -
// and dispatches the events that change it. Text and Markdown edit in
// CodeMirror; a document the core classified as binary takes no edits, and a
// readonly one says why in one line.
//
// One document may show in several displays. It is still one buffer: its
// stored draft, autosave, saving mark and save-on-leave are kept once per
// document by `DocumentKeeper`, and every display's edits reach the core as
// that document's drafts.

const DEFAULT_SCALE = 1;

/** How long editing must be idle before the shell saves (PRD S3 D-10). */
export const AUTOSAVE_IDLE_MS = 600;

type ShellState = ReturnType<typeof useShellStore.getState>;

/** Where a document's unsaved draft is stored: this daemon host, the device, its checkout root and the real path (D-14, S5.5 B9). */
function documentBufferKey(state: ShellState, tabId: string): BufferKey | null {
  const tab = editorTabFor(state.editor, tabId);
  return tab && tab.kind === "file" ? tabBufferKey(state.daemon?.host_id, state.rest, tab) : null;
}

function autosaveDue(document: EditorDocumentSnapshot | null | undefined): boolean {
  return (
    !!document &&
    (document.document_kind === "text" || document.document_kind === "markdown") &&
    !document.readonly_reason &&
    !document.conflict &&
    // A save whose answer was lost blocks the next one until it is read
    // back; a running or waiting save takes the newest draft behind it.
    (!document.save || document.save.state === "saving" || document.save.state === "waiting") &&
    document.dirty
  );
}

/** One idle timer per document, whichever display the last edit came from. */
const autosaves = new Map<string, number>();

/** Saves a document once editing it has been idle for a moment. */
function scheduleAutosave(tabId: string, actions: Actions): void {
  window.clearTimeout(autosaves.get(tabId));
  autosaves.set(
    tabId,
    window.setTimeout(() => {
      autosaves.delete(tabId);
      const state = useShellStore.getState();
      if (!autosaveDue(state.documents[tabId]) || state.connection !== "live" || closingWithSave(tabId)) return;
      state.noteSaving(tabId, actions.saveFile(tabId) === true);
    }, AUTOSAVE_IDLE_MS),
  );
}

function cancelAutosave(tabId: string): void {
  window.clearTimeout(autosaves.get(tabId));
  autosaves.delete(tabId);
}

/** One edit, from whichever display of the document it was made in. */
function draftEdited(tabId: string, contents: string, actions: Actions): void {
  noteDraft(tabId, contents);
  // A buffer that cannot be stored keeps the edit alive and says so on the
  // tab; the session continues either way (D-14).
  const key = documentBufferKey(useShellStore.getState(), tabId);
  if (key) {
    queueBuffer(key, contents, (stored) => {
      if (stored !== null) useShellStore.getState().noteBufferWarning(tabId, !stored);
    });
  } else {
    useShellStore.getState().noteBufferWarning(tabId, true);
  }
  actions.updateDraft(tabId, contents);
  scheduleAutosave(tabId, actions);
}

/**
 * The bookkeeping of one file document on screen, mounted once however many
 * displays show it: the autosave window, the saving mark, the stored draft's
 * restore and removal, and the save of a dirty draft the operator leaves
 * behind. It draws nothing.
 */
export function DocumentKeeper({ tabId, actions }: { tabId: string; actions: Actions }) {
  const document = useShellStore((s) => s.documents[tabId] ?? null);
  const path = useShellStore((s) => editorTabFor(s.editor, tabId)?.path ?? null);
  const draftId = useShellStore((s) => {
    const key = documentBufferKey(s, tabId);
    return key ? identity(key) : null;
  });
  const connection = useShellStore((s) => s.connection);
  const latest = useRef(document);
  latest.current = document;
  const checked = useRef(false);
  const hasDocument = document !== null;

  // A saved document needs no stored draft, so its tab-only mark goes with
  // the save, and with it the hold on every other document.
  const clean = document ? !document.dirty : false;
  useEffect(() => {
    if (clean) useShellStore.getState().noteBufferWarning(tabId, false);
  }, [clean, tabId]);

  // Leaving the document ends its idle window: the timer goes, and a dirty
  // draft the operator left behind is saved rather than waiting for a return
  // that may never come (D-10). A document the operator closed carries its
  // draft in the close itself, so it is not saved a second time here.
  useEffect(
    () => () => {
      cancelAutosave(tabId);
      const current = latest.current;
      if (!current?.dirty || current.conflict || closingWithSave(tabId)) return;
      const stillOpen = useShellStore.getState().editor?.tabs.some((row) => row.id === tabId);
      if (stillOpen) actions.saveFile(tabId);
    },
    [tabId, actions],
  );

  // A conflict pauses autosave until the operator chooses; the choice (or the
  // next edit) resumes it, because the conflict clears and the document is
  // still dirty (D-10, B5).
  useEffect(() => {
    if (document?.conflict || connection !== "live") {
      cancelAutosave(tabId);
      useShellStore.getState().noteSaving(tabId, false);
      return;
    }
    if (document?.dirty) scheduleAutosave(tabId, actions);
  }, [document?.conflict, document?.dirty, tabId, connection, actions]);

  // The save landed when the core reports the document clean.
  useEffect(() => {
    if (document?.dirty === false) useShellStore.getState().noteSaving(tabId, false);
  }, [document?.dirty, tabId]);

  // A refused save never reaches the clean state, so the core's failure is
  // what takes the saving mark off the tab; the document stays dirty and the
  // reason is in the diagnostic log (B5). Only a file error settles a save;
  // a device or catalog error that lands while a slow device save runs
  // leaves its mark alone (S5.5 B45).
  const failureAt = useShellStore((s) => {
    const error = s.rest?.status?.last_error;
    return error && error.kind.startsWith("file.") ? error.occurred_at : null;
  });
  useEffect(() => {
    if (failureAt !== null) useShellStore.getState().noteSaving(tabId, false);
  }, [failureAt, tabId]);

  // A reconnect may have left an unsaved draft in IndexedDB (B8): the draft
  // is the newest edit, so it is restored over a clean core document and
  // dropped when the core already holds the same contents. A draft is never
  // dropped for its age (S5.5 B12). It waits for the document itself, which
  // arrives with the display that shows it.
  useEffect(() => {
    checked.current = false;
    const key = documentBufferKey(useShellStore.getState(), tabId);
    if (connection !== "live" || !key || !hasDocument) return;
    let live = true;
    const node = localDeviceId(useShellStore.getState().rest);
    void flushBuffer(key).then(() => adoptNodeDrafts(key.host, node)).then(() => claimLegacyBuffer(key, node)).then(allBuffers).then((buffers) => {
      if (!live) return;
      const buffer = bufferFor(buffers, key);
      checked.current = true;
      if (!buffer) return;
      const current = latest.current;
      if (!current) return;
      // A read-only or preview-only document takes no draft, so this one
      // cannot be restored into it; it stays a recovery item to export or
      // discard rather than being deleted here (D-14, B11).
      if (current.readonly_reason !== null || current.contents_utf8 === null) {
        useShellStore.getState().noteDiagnostic(`an unsaved draft for ${path ?? tabId} is kept for recovery: the document is not editable here`);
        return;
      }
      const decision = bufferDecision(buffer, current);
      if (decision === "restore") {
        noteDraft(tabId, buffer.contents);
        actions.updateDraft(tabId, buffer.contents);
      } else if (decision === "drop") void deleteBuffer(key);
    });
    return () => {
      live = false;
    };
  }, [tabId, draftId, path, actions, connection, hasDocument]);

  // A document the core reports clean holds the draft on disk, so its copy
  // goes: the core is clean only after a save of the draft landed, or when no
  // draft was ever made (the restore above runs first).
  useEffect(() => {
    const key = documentBufferKey(useShellStore.getState(), tabId);
    if (!key || !checked.current || document?.dirty) return;
    if (document?.document_kind !== "text" && document?.document_kind !== "markdown") return;
    void deleteBuffer(key);
  }, [document?.dirty, document?.document_kind, draftId, tabId]);

  return null;
}

/**
 * One open display (S7 B4): its header and its document or diff. `placeKey`
 * names the display and its document within its Workspace, so the selection
 * and scroll it had in that document come back with it, and a preview
 * retargeted to another document starts that one at its own place.
 */
export function DisplayEditor({ display, workspace, placeKey, actions }: { display: ViewDisplaySnapshot; workspace: ViewWorkspace; placeKey: string; actions: Actions }) {
  const tab = useShellStore((s) => editorTabFor(s.editor, display.tab_id));
  const document = useShellStore((s) => (display.tab_id ? (s.documents[display.tab_id] ?? null) : null));
  const scaleValue = useShellStore((s) => s.rest?.ui_state?.editor_text_scale);
  const scale = typeof scaleValue === "number" ? scaleValue : DEFAULT_SCALE;
  const { t } = useInterfaceTranslation();
  if (!tab) return <><PendingDocumentHeader display={display} actions={actions} /><Notice text={t("common.loading")} state="loading" /></>;
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col" data-editor={tab.id} data-editor-kind={display.kind} data-editor-display={display.id}>
      <EditorHeader display={display} tab={tab} document={document} actions={actions} />
      {display.kind === "diff" ? (
        <DiffBody display={display} workspace={workspace} wrap={tab.wrap} scale={scale} actions={actions} />
      ) : (
        <FileBody display={display} workspace={workspace} tab={tab} document={document} placeKey={placeKey} scale={scale} actions={actions} />
      )}
    </div>
  );
}

/**
 * A diff display's own entry of `changes.diffs`, found by its path and group
 * (S7 A5, A10). The core answers every diff display it shows, with its text
 * or its own reason there is none; the page's sentence is only for a display
 * the core sent nothing for.
 */
function DiffBody({ display, workspace, wrap, scale, actions }: { display: ViewDisplaySnapshot; workspace: ViewWorkspace; wrap: boolean; scale: number; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const changes = useShellStore((s) => changesFor(s.changes, s.rest?.navigator?.changes_root_path ?? null));
  if (!changes) return <Notice text={t("documents.readingDiff")} state="diff-loading" />;
  if (changes.unavailable_reason) return <Notice text={t("documents.historyUnavailable", { reason: readFailureText(changes.unavailable_reason, t) })} state="diff-unavailable" />;
  const committed = display.committed === true;
  const diff = (changes.diffs ?? []).find((row) => row.path === display.path && row.committed === committed) ?? null;
  if (!diff) {
    const group = committed ? changes.committed : changes.entries;
    if (!group.some((entry) => entry.path === display.path)) {
      return <Notice text={t("documents.historyFileGone")} state="diff-unavailable" />;
    }
    return <Notice text={t("documents.readingDiff")} state="diff-loading" />;
  }
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col" data-diff-path={display.path} data-diff-group={committed ? "committed" : "working"}>
      {diff.notice ? <div className="border-b border-border px-md py-xs text-caption text-warning" data-diff-notice={diff.notice.code}>{diffNoticeText(diff.notice, t)}</div> : null}
      {diff.text ? <PatchView text={diff.text} wrap={wrap} scale={scale} comments={{ display, workspace, actions }} /> : <div className="min-h-0 flex-1" data-diff-empty="true" />}
    </div>
  );
}

/** A checkout-relative path as the path row reads it (PRD right-lane B9): its folders and name, `›` between them; a path outside the checkout stays whole. */
function crumbs(path: string): string {
  return absolutePathRoot(path) ? path : path.split("/").filter(Boolean).join(" › ");
}

/** Provenance remains reachable while a document is loading or unavailable. */
export function PendingDocumentHeader({ display, actions }: { display: ViewDisplaySnapshot; actions: Actions }) {
  const root = useShellStore((s) => frontCheckout(s.rest)?.path ?? null);
  return <div className="flex h-(--size-tab-strip) shrink-0 items-center gap-sm border-b border-border px-sm text-caption text-subtle-foreground" data-document-header="true">
    <Hint label={display.path} reveals>
      <span className="min-w-0 flex-1 truncate text-left [direction:rtl]" data-editor-path="true"><bdi dir="ltr">{crumbs(root ? relativeTo(display.path, root) : display.path)}</bdi></span>
    </Hint>
    {display.opener ? <OpenerMark opener={display.opener} actions={actions} /> : null}
  </div>;
}

function EditorHeader({
  display,
  tab,
  document,
  actions,
}: {
  display: ViewDisplaySnapshot;
  tab: EditorTabSnapshot;
  document: EditorDocumentSnapshot | null;
  actions: Actions;
}) {
  const file = display.kind === "file";
  const isMarkdown = file && document?.document_kind === "markdown";
  const editable = document?.document_kind === "text" || isMarkdown;
  const { t } = useInterfaceTranslation();
  // A file or a diff is named from its checkout, as the Explorer names it; a path the
  // header still cannot fit loses its start, so the file name stays shown.
  const root = useShellStore((s) => frontCheckout(s.rest)?.path ?? null);
  const path = root ? relativeTo(display.path, root) : display.path;
  return (
    <div className="flex h-[var(--size-tab-strip)] shrink-0 items-center gap-sm border-b border-border pl-xs pr-md text-caption text-subtle-foreground" data-document-header="true">
      <Hint label={file ? display.path : display.committed ? t("documents.committedPath", { path: display.path }) : t("documents.uncommittedPath", { path: display.path })} reveals>
      <span className="min-w-0 flex-1 truncate text-left [direction:rtl]" data-editor-path="true">
        <bdi dir="ltr">{file ? crumbs(path) : display.committed ? t("documents.branchDiffPath", { path: crumbs(path) }) : t("documents.workingDiffPath", { path: crumbs(path) })}</bdi>
      </span>
      </Hint>
      {document?.dirty ? (
        <span className="text-warning" data-editor-dirty="true">
          {t("documents.unsaved")}
        </span>
      ) : null}
      {isMarkdown && editable ? (
        <Hint label={tab.markdown_live ? t("documents.liveHint") : t("documents.sourceHint")}>
          <Button
            variant="ghost"
            size="sm"
            className={tab.markdown_live ? "text-foreground" : "text-muted-foreground"}
            data-markdown-mode={tab.markdown_live ? "live" : "source"}
            onClick={() => actions.setFileView(tab.id, !tab.markdown_live, tab.wrap)}
          >
            {tab.markdown_live ? t("documents.live") : t("documents.source")}
          </Button>
        </Hint>
      ) : null}
      {display.kind === "diff" || (file && editable) ? (
        <Hint label={t("documents.wrapLines")}>
          <Button
            variant="ghost"
            size="sm"
            className={tab.wrap ? "text-foreground" : "text-muted-foreground"}
            data-editor-wrap={tab.wrap ? "true" : "false"}
            onClick={() => actions.setFileView(tab.id, tab.markdown_live, !tab.wrap)}
          >
            {t("documents.wrap")}
          </Button>
        </Hint>
      ) : null}
      {file && (document?.document_kind === "image" || document?.document_kind === "pdf") && !isVideoPath(display.path) ? (
        <RegionCommentButton displayId={display.id} kind={document.document_kind} />
      ) : null}
      {file ? (
        <Hint label={t("documents.findHint")} shortcut={commandLabel("find_in_pane")}>
          <Button variant="ghost" size="sm" className="text-muted-foreground" disabled={!editable} onClick={() => actions.requestEditorFind(display.id)}>
            {t("documents.find")}
          </Button>
        </Hint>
      ) : null}
      {display.preview ? (
        <Hint label={t("documents.view.keepOpen")} shortcut={commandLabel("keep_open")}>
          <Button variant="ghost" size="sm" className="text-muted-foreground" data-editor-preview="true" onClick={() => actions.keepViewOpen(display.id)}>
            {t("documents.preview")}
          </Button>
        </Hint>
      ) : null}
      {display.opener ? <OpenerMark opener={display.opener} actions={actions} /> : null}
    </div>
  );
}

function FileBody({
  display,
  workspace,
  tab,
  document,
  placeKey,
  scale,
  actions,
}: {
  display: ViewDisplaySnapshot;
  workspace: ViewWorkspace;
  tab: EditorTabSnapshot;
  document: EditorDocumentSnapshot | null;
  placeKey: string;
  scale: number;
  actions: Actions;
}) {
  const { t } = useInterfaceTranslation();
  const viewerComments = useMemo(() => ({ workspace, actions }), [workspace, actions]);
  const findRequest = useUiStore((s) => s.editorFindRequest);
  const findTarget = useUiStore((s) => s.editorFindDisplay === display.id);
  // A draft store that refused a write holds every other clean document
  // read-only until there is room again (B44); only open documents count, so
  // a closed one's old refusal does not hold the editor forever.
  const storageFull = useShellStore((s) => (s.editor?.tabs ?? []).some((row) => s.bufferWarnings.has(row.id)));
  const unstored = useShellStore((s) => s.bufferWarnings.has(tab.id));
  const hold = draftStorageHold({ storageFull, unstored, dirty: document?.dirty ?? false });
  if (!document) {
    return <Notice text={t("common.loading")} state="loading" />;
  }
  if (document.document_kind !== "text" && document.document_kind !== "markdown") {
    return <FileViewer document={document} displayId={display.id} comments={viewerComments} />;
  }
  // A bare browser cannot prove that the daemon is on the viewer's machine,
  // even when its URL is localhost through an SSH tunnel (D-12).
  if (document.contents_utf8 === null) {
    return <PreviewOnly document={document} />;
  }
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {document.readonly_reason ? (
        <div className="border-b border-border px-md py-xs text-caption text-warning" data-editor-readonly="true">
          {document.readonly_reason}
        </div>
      ) : null}
      {hold === "held" ? (
        <div role="status" className="border-b border-border px-md py-xs text-caption text-warning" data-editor-draft-hold="held">
          {t("documents.storageHold", { limit: MAX_STORED_BYTES / (1024 * 1024) })}
        </div>
      ) : null}
      {hold === "unstored" ? (
        <div role="status" className="flex items-center gap-sm border-b border-border px-md py-xs text-caption text-warning" data-editor-draft-hold="unstored">
          <span className="min-w-0 flex-1">{t("documents.unstored")}</span>
          <ExportDraftButton tabId={tab.id} path={tab.path} />
        </div>
      ) : null}
      {document.conflict ? (
        <ConflictBar tabId={tab.id} path={tab.path} removed={document.conflict.disk_revision === null} actions={actions} />
      ) : null}
      {document.save && document.save.state !== "saving" ? <SaveStatusBar tabId={tab.id} path={tab.path} save={document.save} actions={actions} /> : null}
      <CodeMirrorEditor
        key={placeKey}
        tabId={tab.id}
        placeKey={placeKey}
        document={document}
        scale={scale}
        wrap={tab.wrap}
        live={document.document_kind === "markdown" && tab.markdown_live}
        findRequest={findRequest}
        findTarget={findTarget}
        held={hold === "held"}
        onDraft={(contents) => draftEdited(tab.id, contents, actions)}
        onSave={() => actions.saveFile(tab.id)}
        onOpenPath={(path) => actions.openFile(path, false)}
        comments={{ display, workspace, actions }}
      />
    </div>
  );
}

function PreviewOnly({ document }: { document: EditorDocumentSnapshot }) {
  const { t } = useInterfaceTranslation();
  const [downloadError, setDownloadError] = useState<{ reason: string | null } | null>(null);
  const source = useFileSource();
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-sm px-md text-center text-caption text-muted-foreground" data-editor-preview-only="true">
      <span>{document.readonly_reason ?? t("documents.tooLarge")}</span>
      <Button
        variant="secondary"
        data-editor-download="true"
        onClick={() => {
          setDownloadError(null);
          void downloadFile(document.path, source).catch((error: unknown) => {
            if ((error as { name?: string }).name !== "AbortError") {
              setDownloadError({ reason: error instanceof Error ? error.message : null });
            }
          });
        }}
      >
        {t("documents.download")}
      </Button>
      {downloadError ? <span className="text-destructive" data-editor-download-failed="true">{downloadError.reason ? t("documents.downloadFailed", { reason: downloadFailureText(downloadError.reason, t) }) : t("documents.downloadFailedUnknown")}</span> : null}
    </div>
  );
}

/**
 * Saves the draft the operator sees to a file of their choosing, for a
 * document whose own save cannot land (B13-B15): the draft leaves through the
 * browser's download, never through the host's filesystem.
 */
function exportDraft(tabId: string, path: string) {
  const contents = latestDraft(tabId) ?? useShellStore.getState().documents[tabId]?.contents_utf8 ?? "";
  const url = URL.createObjectURL(new Blob([contents], { type: "text/plain;charset=utf-8" }));
  const link = window.document.createElement("a");
  link.href = url;
  link.download = `${path.split("/").pop() || "draft"}.draft`;
  link.click();
  URL.revokeObjectURL(url);
  // The exported text no longer depends on this tab, so a device removal
  // held for it can go ahead (B26, B44).
  useShellStore.getState().noteDraftExported(tabId, contents);
}

function ExportDraftButton({ tabId, path }: { tabId: string; path: string }) {
  const { t } = useInterfaceTranslation();
  return (
    <Button variant="ghost" size="sm" data-export-draft="true" onClick={() => exportDraft(tabId, path)}>
      {t("documents.exportDraft")}
    </Button>
  );
}

/**
 * A save that has no result yet: one waiting for the device's helper, or one
 * whose answer was lost, which is never resent and is read back to settle it
 * (B14). Or a save that was refused or not sent, with its reason, Export and
 * Retry at the same place; autosave waits for Retry rather than repeating a
 * refusal (S5.5 B15, B45).
 */
function SaveStatusBar({ tabId, path, save, actions }: { tabId: string; path: string; save: NonNullable<EditorDocumentSnapshot["save"]>; actions: Actions }) {
  // `not_applied` is an unanswered save read back unchanged: it has not
  // reached the file yet but may still, so it is not called unsaved. Retry
  // is safe there: a save that meets the late one lands as a conflict.
  const { t } = useInterfaceTranslation();
  const message = save.message ?? t("documents.saveUnknown");
  const settled = save.state === "refused" || save.state === "not_applied";
  return (
    <div role="status" className="flex flex-wrap items-center gap-sm border-b border-border px-md py-xs text-caption text-warning" data-editor-save-state={save.state}>
      <span className="min-w-0 flex-1 break-words">
        {save.state === "refused" ? t("documents.notSaved", { message }) : save.state === "not_applied" ? t("documents.notSavedYet", { message }) : t("documents.saveUnsettled", { message })}
      </span>
      <ExportDraftButton tabId={tabId} path={path} />
      {settled ? (
        <Button variant="ghost" size="sm" data-save-retry="true" onClick={() => actions.saveFile(tabId)}>
          {t("common.retry")}
        </Button>
      ) : null}
    </div>
  );
}

function ConflictBar({ tabId, path, removed, actions }: { tabId: string; path: string; removed: boolean; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="flex flex-wrap items-center gap-sm border-b border-border px-md py-xs text-caption text-warning" data-editor-conflict="true">
      <span className="flex-1">
        {removed ? t("documents.conflictRemoved") : t("documents.conflictChanged")}
      </span>
      <ExportDraftButton tabId={tabId} path={path} />
      <Button
        variant="ghost"
        size="sm"
        data-conflict-action="reload"
        onClick={() => {
          clearDraft(tabId);
          const key = documentBufferKey(useShellStore.getState(), tabId);
          if (key) void deleteBuffer(key);
          actions.resolveConflict(tabId, "reload");
        }}
      >
        {t("documents.reloadDisk")}
      </Button>
      <Button variant="ghost" size="sm" data-conflict-action="keep_editing" onClick={() => actions.resolveConflict(tabId, "keep_editing")}>
        {t("documents.keepEditing")}
      </Button>
    </div>
  );
}

function Notice({ text, state }: { text: string; state: string }) {
  return (
    <div className="flex flex-1 items-center justify-center px-md text-center text-caption text-muted-foreground" data-editor-state={state}>
      {text}
    </div>
  );
}
