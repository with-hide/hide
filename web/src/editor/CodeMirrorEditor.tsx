// The CodeMirror 6 host (PRD B4, D-07). The core owns the document; this view
// renders it, reports an edit as a `file_draft`, and replaces its own text
// only when the core's contents are the ones this view last sent (an echo) or
// a real external change. A stale echo of an earlier keystroke is dropped
// rather than applied.
//
// One document may show in several displays at once (S7 B4, D-03): every
// view of it joins the document's channel (`sync.ts`), so an edit in one view
// reaches the others at once as a minimal change outside their undo history,
// and each of them waits for the same echo. Each display keeps its own
// selection and scroll, remembered by display across remounts.
//
// Markdown Live draws a document's leading frontmatter in its own pane above
// the body as a list of properties (D-11, `frontmatterLive.ts`), so that document is held by two views that partition it:
// the pane scrolls inside its fixed height while the body keeps the rest. The
// split and the merge are not user events and never enter either history, so
// undo stays inside the half it happened in.

import { Compartment, EditorState, Transaction, type Extension, type StateEffect } from "@codemirror/state";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { bracketMatching, indentOnInput } from "@codemirror/language";
import { openSearchPanel, search, searchKeymap } from "@codemirror/search";
import {
  EditorView,
  drawSelection,
  dropCursor,
  highlightActiveLine,
  highlightActiveLineGutter,
  highlightSpecialChars,
  keymap,
  lineNumbers,
  type ViewUpdate,
} from "@codemirror/view";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { viewComments } from "./comments";
import { fileModel, useLineComments } from "./useLineComments";
import type { PatchComments } from "./PatchView";
import { useInterfaceTranslation } from "../i18n/client";
import { explorerContext, localDeviceId, type EditorDocumentSnapshot } from "../snapshot";
import { useShellStore } from "../store";
import { splitFrontmatter } from "./frontmatter";
import { lineOffset, onLineRequest, takeLine } from "./lineRequest";
import { languageLoader } from "./languages";
import { markdownLive, type LiveFiles } from "./markdownLivePlugin";
import { editorPhrases } from "./phrases";
import { echoDecision, joinDocument, minimalChange, peerState, type PeerState } from "./sync";
import { frontmatterLive } from "./frontmatterLive";
import { baseTheme, liveDocumentTheme, liveFrontColumn, liveTheme, scaleTheme } from "./theme";

const languageConf = new Compartment();
const readonlyConf = new Compartment();
const wrapConf = new Compartment();
const scaleConf = new Compartment();
const liveConf = new Compartment();
const numbersConf = new Compartment();
const phrasesConf = new Compartment();

/** A display's place in its document: its selection and the first line it shows. */
type Place = { anchor: number; head: number; top: number };

/**
 * Places by display and document (`placeKey`), the newest `PLACE_CAP` kept: a
 * display id is never reused within its Workspace, and a preview display
 * retargeted to another document starts that document at its own place.
 */
const places = new Map<string, Place>();
const PLACE_CAP = 256;

function rememberPlace(key: string, place: Place): void {
  places.delete(key);
  places.set(key, place);
  if (places.size <= PLACE_CAP) return;
  const oldest = places.keys().next();
  if (!oldest.done) places.delete(oldest.value);
}

/** The document position of the first line the view shows. */
function topLine(view: EditorView): number {
  const height = view.scrollDOM.getBoundingClientRect().top - view.documentTop;
  return view.lineBlockAtHeight(Math.max(0, height)).from;
}

/** Brings a view's text to `text` by the smallest change, outside its undo history. */
function applyText(view: EditorView, text: string): void {
  const change = minimalChange(view.state.doc.toString(), text);
  if (change) view.dispatch({ changes: change, annotations: Transaction.addToHistory.of(false) });
}

// Live reads as a page and numbers no lines; the comment gutter stays beside the text.
const liveExtensions = (live: boolean, files: LiveFiles): Extension => (live ? [markdownLive({ files }), liveTheme, liveDocumentTheme] : []);
const numberExtensions = (live: boolean): Extension => (live ? [] : [lineNumbers(), highlightActiveLineGutter()]);

function readonlyExtensions(readonly: boolean): Extension {
  return readonly ? [EditorState.readOnly.of(true), EditorView.editable.of(false)] : [];
}

export function CodeMirrorEditor({
  tabId,
  placeKey,
  document,
  scale,
  wrap,
  live,
  findRequest,
  findTarget,
  held = false,
  onDraft,
  onSave,
  onOpenPath,
  comments,
}: {
  tabId: string;
  /** The display this view draws and its document, scoped to its Workspace: the selection and scroll are kept under it. */
  placeKey: string;
  document: EditorDocumentSnapshot;
  scale: number;
  wrap: boolean;
  live: boolean;
  findRequest: number;
  /** Whether a new find request opens this view's search panel; one display answers ⌘F. */
  findTarget: boolean;
  /** Read-only while unsaved drafts cannot be stored (S5.5 B44). */
  held?: boolean;
  onDraft: (contents: string) => void;
  /** Saves the document: the editor's own Mod-s (docs/UI_BEHAVIOR.md, Keyboard shortcuts per system). */
  onSave: () => void;
  /** A file a drawn HTML link names inside the checkout, opened in the Workspace. */
  onOpenPath: (path: string) => void;
  /** The View it draws, which takes the operator's comments (PRD view-opener-feedback B12). */
  comments: PatchComments;
}) {
  const { t } = useInterfaceTranslation();
  const phrasesRef = useRef(editorPhrases(t));
  phrasesRef.current = editorPhrases(t);
  const bodyHost = useRef<HTMLDivElement>(null);
  const frontHost = useRef<HTMLDivElement>(null);
  const bodyView = useRef<EditorView | null>(null);
  const frontView = useRef<EditorView | null>(null);
  // The last text this view sent the core or received from another view of
  // the document; the core's echo of anything else is a stale keystroke and
  // is not written back over the buffer.
  const pending = useRef<string | null>(null);
  // This view's own text across a pane toggle, which recreates the views: an
  // edit the core has not echoed yet stays in them.
  const carried = useRef<{ tabId: string; state: PeerState } | null>(null);
  const onDraftRef = useRef(onDraft);
  onDraftRef.current = onDraft;
  const onSaveRef = useRef(onSave);
  onSaveRef.current = onSave;

  const contents = document.contents_utf8 ?? "";
  const contentsRef = useRef(contents);
  contentsRef.current = contents;
  const pathRef = useRef(document.path);
  pathRef.current = document.path;
  const onOpenPathRef = useRef(onOpenPath);
  onOpenPathRef.current = onOpenPath;
  // Where this document's relative images and links start: its folder, the
  // checkout in front, and the device that checkout is on, read when a widget
  // is drawn.
  const files = useMemo<LiveFiles>(
    () => ({
      base: () => {
        const rest = useShellStore.getState().rest;
        const { device, checkout } = explorerContext(rest);
        if (!checkout) return null;
        const source = device === localDeviceId(rest) ? null : { device, root: checkout.path };
        return { docPath: pathRef.current, root: checkout.path, source };
      },
      openUrl: (url) => {
        window.open(url, "_blank", "noopener,noreferrer");
      },
      openPath: (path) => onOpenPathRef.current(path),
    }),
    [],
  );
  const readonly = held || document.readonly_reason !== null || document.document_kind !== "text" && document.document_kind !== "markdown";
  // The pane belongs to Live mode's markdown only; source mode and every
  // other kind keep one buffer (D-11).
  const split = document.document_kind === "markdown" && live && splitFrontmatter(contents) !== null;
  // The body under a frontmatter pane starts after its lines; a comment is numbered in the whole file.
  const offset = split ? (splitFrontmatter(contents)?.front.split("\n").length ?? 1) - 1 : 0;
  const model = useMemo(() => fileModel(contents, offset), [contents, offset]);
  const [shownBody, setShownBody] = useState<EditorView | null>(null);
  const { host: commentHost, blocks: commentBlocks } = useLineComments({ view: shownBody, ...comments, model, rows: model.lines().length - offset, saved: contents });

  const combined = () =>
    (frontView.current?.state.doc.toString() ?? "") + (bodyView.current?.state.doc.toString() ?? "");

  // One view per display, and one pair while the frontmatter pane shows:
  // switching displays remounts (the parent keys by display), so the history
  // belongs to the display and its place is remembered for its return. A
  // layout effect, so the place is read while the view is still attached.
  useLayoutEffect(() => {
    if (!bodyHost.current) return undefined;
    // This view before a pane toggle, or another view of the document on
    // screen, may hold an edit the core has not echoed yet; the view starts
    // from it rather than from older text.
    const own = carried.current?.tabId === tabId ? carried.current.state : null;
    carried.current = null;
    const start = own ?? peerState(tabId);
    const initial = start?.text ?? contentsRef.current;
    pending.current = start?.pending ?? null;
    const parts = split ? splitFrontmatter(initial) : null;
    let channel: ReturnType<typeof joinDocument> | null = null;
    // Both halves report one draft: the changed half comes from this update,
    // the untouched one from its own view, and the block stays first (D-11).
    const listeners = (half: "front" | "body") =>
      EditorView.updateListener.of((update: ViewUpdate) => {
        if (!update.docChanged) return;
        // An undo or a redo is the operator's edit too, and the core must hear
        // it or the next save writes the text they just undid. A change
        // another view of the document applied carries no user event.
        if (
          !update.transactions.some(
            (tr) =>
              tr.isUserEvent("input") ||
              tr.isUserEvent("delete") ||
              tr.isUserEvent("move") ||
              tr.isUserEvent("undo") ||
              tr.isUserEvent("redo"),
          )
        )
          return;
        const front = half === "front" ? update.state.doc.toString() : (frontView.current?.state.doc.toString() ?? "");
        const body = half === "body" ? update.state.doc.toString() : (bodyView.current?.state.doc.toString() ?? "");
        pending.current = front + body;
        channel?.publish(front + body);
        onDraftRef.current(front + body);
      });
    // Ctrl+S saves inside a document on Windows and Linux, as in every editor,
    // without the window taking Ctrl+S from a terminal (XOFF, forward search).
    // On macOS the window's own ⌘S has answered before this keymap runs.
    const save = keymap.of([{ key: "Mod-s", run: () => { onSaveRef.current(); return true; } }]);
    const shared = [
      highlightSpecialChars(),
      history(),
      drawSelection(),
      dropCursor(),
      indentOnInput(),
      bracketMatching(),
      highlightActiveLine(),
      baseTheme(),
      phrasesConf.of(phrasesRef.current),
      readonlyConf.of(readonlyExtensions(readonly)),
      wrapConf.of(wrap ? EditorView.lineWrapping : []),
      scaleConf.of(scaleTheme(scale)),
    ];
    const bodyEditor = new EditorView({
      state: EditorState.create({
        doc: parts ? parts.body : initial,
        extensions: [
          ...shared,
          viewComments(commentHost),
          numbersConf.of(numberExtensions(live)),
          search({ top: true }),
          save,
          keymap.of([...defaultKeymap, ...historyKeymap, ...searchKeymap, indentWithTab]),
          languageConf.of([]),
          liveConf.of(liveExtensions(live, files)),
          listeners("body"),
        ],
      }),
      parent: bodyHost.current,
    });
    bodyView.current = bodyEditor;
    setShownBody(bodyEditor);
    let frontEditor: EditorView | null = null;
    if (parts && frontHost.current) {
      frontEditor = new EditorView({
        state: EditorState.create({
          doc: parts.front,
          extensions: [
            ...shared,
            save,
            keymap.of([...defaultKeymap, ...historyKeymap, indentWithTab]),
            frontmatterLive,
            liveFrontColumn,
            listeners("front"),
          ],
        }),
        parent: frontHost.current,
      });
      frontView.current = frontEditor;
    }
    const place = places.get(placeKey);
    if (place) {
      const limit = bodyEditor.state.doc.length;
      bodyEditor.dispatch({
        selection: { anchor: Math.min(place.anchor, limit), head: Math.min(place.head, limit) },
        userEvent: "select",
        effects: EditorView.scrollIntoView(Math.min(place.top, limit), { y: "start" }),
      });
    }
    // A terminal link's line (`lineRequest.ts`), taken when this view mounts
    // for the open or when the open only brought this view forward; a line
    // inside the frontmatter pane moves that pane.
    const frontLines = parts ? parts.front.split("\n").length - 1 : 0;
    const goToLine = () => {
      const asked = takeLine(pathRef.current);
      if (!asked) return;
      const target = frontEditor && asked.line <= frontLines ? frontEditor : bodyEditor;
      const offset = lineOffset(target.state.doc, target === bodyEditor ? asked.line - frontLines : asked.line, asked.column);
      target.dispatch({ selection: { anchor: offset }, userEvent: "select", effects: EditorView.scrollIntoView(offset, { y: "center" }) });
    };
    goToLine();
    const stopLines = onLineRequest(goToLine);
    // The view scrolls to its place in its first measure, which runs in the
    // frame this callback follows; a view taken down before then (a double
    // mount, a quick toggle) never showed it, so its top would overwrite the
    // place it was given.
    let drawn = false;
    const frame = window.requestAnimationFrame(() => {
      drawn = true;
    });
    // Another view's edit: the same text here at once, as its pending echo.
    const receive = (text: string) => {
      pending.current = text;
      if (frontEditor) {
        const incoming = splitFrontmatter(text);
        applyText(frontEditor, incoming ? incoming.front : "");
        applyText(bodyEditor, incoming ? incoming.body : text);
      } else {
        applyText(bodyEditor, text);
      }
    };
    channel = joinDocument(tabId, receive, () => ({ text: combined(), pending: pending.current }));
    return () => {
      window.cancelAnimationFrame(frame);
      stopLines();
      channel?.leave();
      carried.current = { tabId, state: { text: combined(), pending: pending.current } };
      const selection = bodyEditor.state.selection.main;
      if (drawn || !place) rememberPlace(placeKey, { anchor: selection.anchor, head: selection.head, top: topLine(bodyEditor) });
      bodyView.current = null;
      frontView.current = null;
      setShownBody(null);
      bodyEditor.destroy();
      frontEditor?.destroy();
    };
    // The views are created once per display and pane state; the effects
    // below reconfigure them.
  }, [tabId, split, placeKey]);

  // The core's contents, when they are not what these views hold: an echo of
  // the last draft settles it, a stale echo is dropped, and anything else is
  // a real change (a reload) that replaces both halves.
  useEffect(() => {
    const editor = bodyView.current;
    if (!editor) return;
    const incoming = document.contents_utf8 ?? "";
    const decision = echoDecision(incoming, combined(), pending.current);
    if (decision === "settle") {
      pending.current = null;
      return;
    }
    if (decision === "drop") return;
    pending.current = null;
    const parts = split ? splitFrontmatter(incoming) : null;
    const front = frontView.current;
    if (parts && front) {
      front.dispatch({ changes: { from: 0, to: front.state.doc.length, insert: parts.front } });
      editor.dispatch({ changes: { from: 0, to: editor.state.doc.length, insert: parts.body } });
    } else {
      editor.dispatch({ changes: { from: 0, to: editor.state.doc.length, insert: incoming } });
    }
  }, [document.contents_utf8, split]);

  // The language pack loads only for a document that asks for one; the
  // dispatch is ignored when the view was closed while the import was in flight.
  useEffect(() => {
    const editor = bodyView.current;
    if (!editor) return;
    let cancelled = false;
    const loader = languageLoader(document.language, document.path);
    const apply = (extension: Extension | null) => {
      if (cancelled || bodyView.current !== editor) return;
      editor.dispatch({ effects: languageConf.reconfigure(extension ?? []) });
    };
    if (!loader) apply(null);
    else void loader().then(apply).catch(() => apply(null));
    return () => {
      cancelled = true;
    };
  }, [document.language, document.path, split, placeKey]);

  // A pane setting reaches both halves, because the frontmatter is the same
  // document and the operator's scale, wrap and readonly choice is one choice.
  const both = (effect: StateEffect<unknown>) => {
    bodyView.current?.dispatch({ effects: effect });
    frontView.current?.dispatch({ effects: effect });
  };

  useEffect(() => {
    both(readonlyConf.reconfigure(readonlyExtensions(readonly)));
  }, [readonly]);

  useEffect(() => {
    both(phrasesConf.reconfigure(editorPhrases(t)));
  }, [t]);

  useEffect(() => {
    both(wrapConf.reconfigure(wrap ? EditorView.lineWrapping : []));
  }, [wrap]);

  useEffect(() => {
    both(scaleConf.reconfigure(scaleTheme(scale)));
  }, [scale]);

  useEffect(() => {
    bodyView.current?.dispatch({ effects: [liveConf.reconfigure(liveExtensions(live, files)), numbersConf.reconfigure(numberExtensions(live))] });
  }, [live, files]);

  // A request is an increment: the counter survives a display switch, so only
  // a new ⌘F opens a panel, only in the display it was meant for, and a
  // freshly mounted display does not inherit the last one's find input.
  const seenFind = useRef(findRequest);
  useEffect(() => {
    if (findRequest <= seenFind.current) return;
    seenFind.current = findRequest;
    if (findTarget && bodyView.current) openSearchPanel(bodyView.current);
  }, [findRequest, findTarget]);

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden" data-editor-codemirror="true">
      {split ? (
        <div className="shrink-0 overflow-hidden border-b border-border" data-editor-frontmatter="true">
          <div ref={frontHost} data-editor-frontmatter-editor="true" />
        </div>
      ) : null}
      <div ref={bodyHost} className="min-h-0 flex-1 overflow-hidden" data-editor-body="true" />
      {commentBlocks}
    </div>
  );
}
