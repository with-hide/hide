import { EditorState, type Extension } from "@codemirror/state";
import { Decoration, EditorView, gutter, GutterMarker } from "@codemirror/view";
import { useEffect, useMemo, useRef, useState } from "react";
import type { Actions } from "../actions";
import type { ViewDisplaySnapshot } from "../snapshot";
import type { ViewWorkspace } from "../viewLayout";
import { viewComments } from "./comments";
import { patchLines } from "./patch";
import { diffModel, useLineComments } from "./useLineComments";
import { baseTheme, scaleTheme } from "./theme";

class NumberMarker extends GutterMarker {
  constructor(readonly value: number) { super(); }
  toDOM(): HTMLElement {
    const span = document.createElement("span");
    span.textContent = String(this.value);
    return span;
  }
}

const patchTheme = EditorView.theme({
  "&": { overflow: "hidden" },
  ".cm-scroller": { overflow: "auto", fontFamily: "var(--font-mono)" },
  ".cm-content": { paddingTop: "0" },
  ".cm-gutter": { width: "var(--size-editor-diff-line-number-col)", textAlign: "right" },
  ".cm-gutterElement": { paddingRight: "var(--spacing-xs)" },
  ".cm-patch-added": { backgroundColor: "color-mix(in srgb, var(--diff-added) 15%, transparent)" },
  ".cm-patch-removed": { backgroundColor: "color-mix(in srgb, var(--diff-removed) 15%, transparent)" },
  ".cm-patch-hunk": { backgroundColor: "var(--secondary)", color: "var(--file-blue)" },
  ".cm-patch-header": { color: "var(--muted-foreground)" },
});

// Unwrapped, a row keeps its full width so an added or removed row's band
// runs past the viewport under a horizontal scroll.
const unwrappedTheme = EditorView.theme({
  ".cm-content": { minWidth: "max-content", whiteSpace: "pre" },
  ".cm-line": { width: "max-content", minWidth: "100%" },
});

/** The View a diff shows, which takes the operator's comments (PRD view-opener-feedback B12). */
export type PatchComments = { display: ViewDisplaySnapshot; workspace: ViewWorkspace; actions: Actions };

export function PatchView({ text, wrap, scale, comments }: { text: string; wrap: boolean; scale: number; comments: PatchComments }) {
  const [view, setView] = useState<EditorView | null>(null);
  const rows = useMemo(() => patchLines(text), [text]);
  const model = useMemo(() => diffModel(rows), [rows]);
  const { host: commentHost, blocks: commentBlocks } = useLineComments({ view, ...comments, model, rows: rows.length, saved: text });
  const extension = useMemo(() => viewComments(commentHost), [commentHost]);
  return (
    <>
      <PatchEditor text={text} rows={rows} wrap={wrap} scale={scale} extension={extension} onView={setView} />
      {commentBlocks}
    </>
  );
}

/** A diff that takes no comments: one no View display holds, such as a Hide AI branch's file (PRD hide-ai-branch-first-apply B4). */
export function PlainPatchView({ text, wrap, scale }: { text: string; wrap: boolean; scale: number }) {
  const rows = useMemo(() => patchLines(text), [text]);
  return <PatchEditor text={text} rows={rows} wrap={wrap} scale={scale} extension={null} onView={null} />;
}

function PatchEditor({ text, rows, wrap, scale, extension, onView }: { text: string; rows: ReturnType<typeof patchLines>; wrap: boolean; scale: number; extension: Extension | null; onView: ((view: EditorView | null) => void) | null }) {
  const host = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!host.current || !text) return;
    const doc = text.endsWith("\n") ? text.slice(0, -1) : text;
    let from = 0;
    const decorations = Decoration.set(rows.flatMap((row) => {
      const position = from;
      from += row.text.length + 1;
      return row.kind === "context" || row.kind === "meta"
        ? []
        : [Decoration.line({ attributes: { class: `cm-patch-${row.kind}` } }).range(position)];
    }));
    const state = EditorState.create({
      doc,
      extensions: [
        ...baseTheme(),
        patchTheme,
        wrap ? EditorView.lineWrapping : unwrappedTheme,
        scaleTheme(scale),
        EditorState.readOnly.of(true),
        EditorView.editable.of(false),
        // Read-only, yet it takes the keyboard like an editor, so an opened
        // diff answers ⌘W, copy and scrolling rather than leaving them to
        // the row it was opened from.
        EditorView.contentAttributes.of({ tabindex: "0" }),
        ...(extension ? [extension] : []),
        gutter({ class: "cm-patch-old", lineMarker: (_view, line) => {
          const number = rows[_view.state.doc.lineAt(line.from).number - 1]?.oldLine;
          return number === null || number === undefined ? null : new NumberMarker(number);
        } }),
        gutter({ class: "cm-patch-new", lineMarker: (_view, line) => {
          const number = rows[_view.state.doc.lineAt(line.from).number - 1]?.newLine;
          return number === null || number === undefined ? null : new NumberMarker(number);
        } }),
        EditorView.decorations.of(decorations),
      ],
    });
    const editor = new EditorView({ state, parent: host.current });
    onView?.(editor);
    return () => {
      onView?.(null);
      editor.destroy();
    };
  }, [text, wrap, scale, rows, extension, onView]);
  return <div ref={host} className="min-h-0 min-w-0 flex-1" data-patch-view="true" />;
}
