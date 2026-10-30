// The operator's comments in a file or diff View (PRD view-opener-feedback
// B12, B31): a `+` on the line under the pointer or the cursor, a drag down
// the gutter for a range, and the comment chord on selected text, each
// opening a note under the range's last line; a commented line keeps a bar in
// the gutter and its comment under it. The core owns the comments; this
// extension only draws what it is given (`setComments`) and reports the rows
// the operator picked (`CommentHost.pick`).
//
// The widgets hold React trees the host renders; nothing here runs per
// keystroke beyond CodeMirror mapping the widgets through an edit.

import { RangeSet, StateEffect, StateField, type Extension } from "@codemirror/state";
import { Decoration, EditorView, gutter, GutterMarker, keymap, ViewPlugin, WidgetType, type DecorationSet } from "@codemirror/view";

/** The chord that comments on the selected lines: ⇧⌘. on macOS, Ctrl+Shift+. elsewhere (docs/UI_BEHAVIOR.md). */
export const COMMENT_CHORD = "Mod-Shift-.";

/** One block under a row: a comment's card, or the open note. */
export type CommentBlock = { key: string; version: string; row: number; kind: "comment" | "form" };

/** What the editor asks of the page. */
export type CommentHost = {
  /** The operator picked rows `from`..`to` (1-based, inclusive) of the shown text. */
  pick(from: number, to: number): void;
  /** Draws a block into `element`; the returned function takes it down. */
  render(block: CommentBlock, element: HTMLElement): () => void;
  /** Whether a row can take a comment (a diff's hunk header cannot). */
  commentable(row: number): boolean;
  /** The words of the `+` for a row. */
  label(row: number): string;
};

type Shown = { blocks: CommentBlock[]; marked: number[] };

/** What the page shows now: the blocks and the commented rows. */
export const setComments = StateEffect.define<Shown>();
const setHover = StateEffect.define<number | null>();
const setDrag = StateEffect.define<{ from: number; to: number } | null>();

class Block extends WidgetType {
  constructor(
    readonly block: CommentBlock,
    readonly host: { current: CommentHost },
  ) {
    super();
  }

  eq(other: Block): boolean {
    return other.block.key === this.block.key && other.block.version === this.block.version;
  }

  toDOM(): HTMLElement {
    const element = document.createElement("div");
    element.className = "cm-view-comment";
    element.dataset.viewCommentBlock = this.block.kind;
    takedowns.set(element, this.host.current.render(this.block, element));
    return element;
  }

  destroy(element: HTMLElement): void {
    takedowns.get(element)?.();
    takedowns.delete(element);
  }

  ignoreEvent(): boolean {
    return true;
  }

  get estimatedHeight(): number {
    return this.block.kind === "form" ? 140 : 72;
  }
}

const takedowns = new WeakMap<HTMLElement, () => void>();

class Bar extends GutterMarker {
  toDOM(): HTMLElement {
    const bar = document.createElement("span");
    bar.className = "cm-view-comment-bar";
    bar.setAttribute("aria-hidden", "true");
    return bar;
  }
}

class Plus extends GutterMarker {
  constructor(
    readonly row: number,
    readonly label: string,
  ) {
    super();
  }

  eq(other: Plus): boolean {
    return other.row === this.row && other.label === this.label;
  }

  /** A box one text line tall holding the button, so on a wrapped or taller line it sits beside the line number, on the first visual row. */
  toDOM(): HTMLElement {
    const slot = document.createElement("span");
    slot.className = "cm-view-comment-slot";
    const button = document.createElement("button");
    button.type = "button";
    button.className = "cm-view-comment-add";
    button.textContent = "+";
    button.setAttribute("aria-label", this.label);
    button.dataset.viewCommentAdd = String(this.row);
    slot.append(button);
    return slot;
  }
}

const BAR = new Bar();

/** The comments, the commented rows, the row under the pointer and a drag in progress. */
type State = { shown: Shown; hover: number | null; drag: { from: number; to: number } | null; blocks: DecorationSet };

function blocksOf(view: { doc: { lines: number; line(n: number): { to: number } } }, shown: Shown, host: { current: CommentHost }): DecorationSet {
  const ranges = shown.blocks
    .filter((block) => block.row >= 1)
    .map((block) => ({ block, at: view.doc.line(Math.min(block.row, view.doc.lines)).to }))
    .sort((a, b) => a.at - b.at || (a.block.kind === "form" ? 1 : 0) - (b.block.kind === "form" ? 1 : 0))
    .map(({ block, at }) => Decoration.widget({ widget: new Block(block, host), block: true, side: 1 }).range(at));
  return Decoration.set(ranges, true);
}

/** The rows `from`..`to` as the selection names them: a selection ending at a line's start leaves that line out. */
export function selectedRows(doc: { lineAt(pos: number): { number: number; from: number } }, from: number, to: number): { from: number; to: number } {
  const first = doc.lineAt(from).number;
  const end = doc.lineAt(to);
  const last = to > from && end.from === to ? Math.max(first, end.number - 1) : end.number;
  return { from: first, to: last };
}

/**
 * The extension, given a host whose `current` the page keeps up to date. A
 * drag starts on the comment gutter or a line number and follows the
 * pointer until it is released anywhere on the page.
 */
export function viewComments(host: { current: CommentHost }): Extension {
  const field = StateField.define<State>({
    create: (state) => ({ shown: { blocks: [], marked: [] }, hover: null, drag: null, blocks: blocksOf(state, { blocks: [], marked: [] }, host) }),
    update(value, tr) {
      let next = value;
      for (const effect of tr.effects) {
        if (effect.is(setComments)) next = { ...next, shown: effect.value, blocks: blocksOf(tr.state, effect.value, host) };
        else if (effect.is(setHover)) next = { ...next, hover: effect.value };
        else if (effect.is(setDrag)) next = { ...next, drag: effect.value };
      }
      if (next === value && tr.docChanged) return { ...value, blocks: value.blocks.map(tr.changes) };
      return next;
    },
    provide: (f) => EditorView.decorations.from(f, (value) => value.blocks),
  });

  const rowAt = (view: EditorView, clientY: number): number | null => {
    const block = view.lineBlockAtHeight(clientY - view.documentTop);
    if (!block) return null;
    return view.state.doc.lineAt(block.from).number;
  };

  const startDrag = (view: EditorView, event: MouseEvent): boolean => {
    if (event.button !== 0) return false;
    const from = rowAt(view, event.clientY);
    if (from === null || !host.current.commentable(from)) return false;
    event.preventDefault();
    view.dispatch({ effects: setDrag.of({ from, to: from }) });
    const move = (moved: MouseEvent) => {
      const to = rowAt(view, moved.clientY);
      if (to !== null && to !== view.state.field(field).drag?.to) view.dispatch({ effects: setDrag.of({ from, to }) });
    };
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      const drag = view.state.field(field).drag;
      view.dispatch({ effects: setDrag.of(null) });
      if (drag) host.current.pick(Math.min(drag.from, drag.to), Math.max(drag.from, drag.to));
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
    return true;
  };

  const commentGutter = gutter({
    class: "cm-view-comment-gutter",
    markers(view) {
      const { shown, hover, drag } = view.state.field(field);
      const marks: { row: number; marker: GutterMarker }[] = [];
      const rows = new Set(shown.marked);
      if (drag) for (let row = Math.min(drag.from, drag.to); row <= Math.max(drag.from, drag.to); row += 1) rows.add(row);
      const cursor = view.hasFocus ? view.state.doc.lineAt(view.state.selection.main.head).number : null;
      for (const row of [hover, cursor]) {
        if (row !== null && row <= view.state.doc.lines && !rows.has(row) && host.current.commentable(row)) marks.push({ row, marker: new Plus(row, host.current.label(row)) });
      }
      for (const row of rows) if (row >= 1 && row <= view.state.doc.lines) marks.push({ row, marker: BAR });
      marks.sort((a, b) => a.row - b.row);
      const seen = new Set<number>();
      return RangeSet.of(
        marks.filter(({ row }) => !seen.has(row) && seen.add(row)).map(({ row, marker }) => marker.range(view.state.doc.line(row).from)),
      );
    },
    lineMarkerChange: (update) => update.transactions.some((tr) => tr.effects.some((effect) => effect.is(setComments) || effect.is(setHover) || effect.is(setDrag))) || update.selectionSet || update.focusChanged,
    initialSpacer: () => new Plus(0, ""),
    domEventHandlers: {
      mousedown: (view, _line, event) => startDrag(view, event as MouseEvent),
    },
  });

  // The pointer over the gutters shows `+` on its row; leaving them hides it.
  const hover = ViewPlugin.define((view) => {
    const gutters = () => view.dom.querySelector<HTMLElement>(".cm-gutters");
    const move = (event: MouseEvent) => {
      const row = rowAt(view, event.clientY);
      if (row !== view.state.field(field).hover) view.dispatch({ effects: setHover.of(row) });
    };
    const leave = () => {
      if (view.state.field(field).hover !== null) view.dispatch({ effects: setHover.of(null) });
    };
    let bound: HTMLElement | null = null;
    const bind = () => {
      const element = gutters();
      if (element === bound) return;
      bound?.removeEventListener("mousemove", move);
      bound?.removeEventListener("mouseleave", leave);
      bound = element;
      bound?.addEventListener("mousemove", move);
      bound?.addEventListener("mouseleave", leave);
    };
    bind();
    return {
      update: bind,
      destroy() {
        bound?.removeEventListener("mousemove", move);
        bound?.removeEventListener("mouseleave", leave);
      },
    };
  });

  // A press on a line number takes a range too, as GitHub's gutter does.
  const numbers = EditorView.domEventHandlers({
    mousedown(event, view) {
      const target = event.target as HTMLElement | null;
      if (!target?.closest(".cm-lineNumbers, .cm-patch-old, .cm-patch-new")) return false;
      return startDrag(view, event);
    },
  });

  const chord = keymap.of([
    {
      key: COMMENT_CHORD,
      run(view) {
        const range = view.state.selection.main;
        const rows = selectedRows(view.state.doc, range.from, range.to);
        let from = rows.from;
        while (from <= rows.to && !host.current.commentable(from)) from += 1;
        if (from > rows.to) return false;
        host.current.pick(from, rows.to);
        return true;
      },
    },
  ]);

  // A block is as wide as the visible text, not the longest line, and stays
  // beside the gutters while the lines scroll sideways, so an unwrapped View
  // keeps its note's buttons in sight; Markdown Live's centred column insets
  // the scroller, and the block keeps inside it. Measured only when the
  // geometry moves.
  const fit = ViewPlugin.define((view) => {
    const measure = () =>
      view.requestMeasure({
        read: () => {
          const gutters = view.dom.querySelector<HTMLElement>(".cm-gutters")?.offsetWidth ?? 0;
          const style = getComputedStyle(view.scrollDOM);
          const inset = Number.parseFloat(style.paddingLeft) + Number.parseFloat(style.paddingRight);
          return { left: gutters, width: Math.max(0, view.scrollDOM.clientWidth - gutters - inset) };
        },
        write: ({ left, width }) => {
          view.dom.style.setProperty("--cm-view-comment-left", `${left}px`);
          view.dom.style.setProperty("--cm-view-comment-width", `${width}px`);
        },
      });
    measure();
    return { update: (update) => (update.geometryChanged ? measure() : undefined) };
  });

  return [field, commentGutter, hover, numbers, chord, fit, commentTheme];
}

const commentTheme = EditorView.theme({
  ".cm-view-comment-gutter": { width: "var(--size-control-sm)" },
  ".cm-view-comment-gutter .cm-gutterElement": { display: "flex", alignItems: "flex-start", justifyContent: "center", padding: "0" },
  ".cm-view-comment-slot": { display: "flex", alignItems: "center", justifyContent: "center", height: "1lh" },
  ".cm-view-comment-add": {
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    width: "var(--size-badge-height)",
    height: "var(--size-badge-height)",
    borderRadius: "var(--radius-xs)",
    border: "none",
    background: "var(--primary)",
    color: "var(--primary-foreground)",
    font: "inherit",
    lineHeight: "1",
    cursor: "pointer",
  },
  ".cm-view-comment-add:focus-visible": { outline: "var(--size-hairline) solid var(--ring)" },
  ".cm-view-comment-bar": { display: "block", alignSelf: "stretch", width: "var(--size-tab-indicator)", marginLeft: "auto", background: "var(--primary)" },
  ".cm-view-comment": {
    position: "sticky",
    left: "var(--cm-view-comment-left, 0)",
    boxSizing: "border-box",
    width: "var(--cm-view-comment-width, 100%)",
    padding: "var(--spacing-xs) var(--spacing-md) var(--spacing-xs) var(--spacing-xl)",
    fontFamily: "var(--font-sans)",
    fontSize: "var(--text-body)",
    whiteSpace: "normal",
    cursor: "default",
  },
});
