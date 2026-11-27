// The page's half of a file or diff View's comments (`comments.ts`): which
// comments and note the editor shows and under which row, the rows the
// operator picked turned into a note, and a comment's lines found again when
// the saved text changes (PRD view-opener-feedback B12, B31, D-14).

import type { EditorView } from "@codemirror/view";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import type { Actions } from "../actions";
import { useInterfaceTranslation } from "../i18n/client";
import type { ViewCommentAnchor, ViewDisplaySnapshot } from "../snapshot";
import { anchorMeta, CommentCard, NoteFormCard, useCommentsTaken, useViewFeedback } from "../ViewComments";
import { commentsOn, COMMENTS_PER_VIEW, linesAnchor, openNote, relocation, useFeedbackStore } from "../viewFeedback";
import type { ViewWorkspace } from "../viewLayout";
import { setComments, type CommentBlock, type CommentHost } from "./comments";
import type { PatchLine } from "./patch";

type Side = "old" | "new";

/** How a View numbers the rows it shows: a file's lines, or a diff's old and new lines. */
export type LineModel = {
  /** The number and side of row `row` (1-based), or null for a row that has none. */
  at(row: number): { line: number; side?: Side } | null;
  /** The row that shows `line` on `side`, or null. */
  row(line: number, side?: Side): number | null;
  /** A side's lines, indexed by number - 1; a diff leaves the lines it does not show empty. */
  lines(side?: Side): (string | undefined)[];
};

/** A file shown from line `offset + 1` (the body under a frontmatter pane starts after it). */
export function fileModel(text: string, offset = 0): LineModel {
  const all = (text.endsWith("\n") ? text.slice(0, -1) : text).split("\n");
  return {
    at: (row) => (row >= 1 ? { line: row + offset } : null),
    row: (line) => (line - offset >= 1 && line <= all.length ? line - offset : null),
    lines: () => all,
  };
}

/** A diff's rows: an added or context row is numbered in the new file, a removed one in the old (B24). */
export function diffModel(rows: readonly PatchLine[]): LineModel {
  const old: (string | undefined)[] = [];
  const next: (string | undefined)[] = [];
  const oldRow = new Map<number, number>();
  const newRow = new Map<number, number>();
  rows.forEach((row, index) => {
    const text = row.text.slice(1);
    if (row.newLine !== null) {
      next[row.newLine - 1] = text;
      newRow.set(row.newLine, index + 1);
    }
    if (row.oldLine !== null) {
      old[row.oldLine - 1] = text;
      if (row.newLine === null) oldRow.set(row.oldLine, index + 1);
    }
  });
  return {
    at(row) {
      const line = rows[row - 1];
      if (!line) return null;
      if (line.newLine !== null) return { line: line.newLine, side: "new" };
      if (line.oldLine !== null) return { line: line.oldLine, side: "old" };
      return null;
    },
    row: (line, side) => (side === "old" ? oldRow.get(line) : newRow.get(line)) ?? null,
    lines: (side) => (side === "old" ? old : next),
  };
}

/** The anchor rows `from`..`to` make: a diff range of removed rows alone is on the old side, any other on the new. */
export function pickedAnchor(model: LineModel, from: number, to: number): ViewCommentAnchor | null {
  const numbered = [];
  for (let row = from; row <= to; row += 1) {
    const at = model.at(row);
    if (at) numbered.push(at);
  }
  if (numbered.length === 0) return null;
  const diff = numbered.some((at) => at.side !== undefined);
  const side: Side | undefined = !diff ? undefined : numbered.every((at) => at.side === "old") ? "old" : "new";
  const lines = numbered.filter((at) => at.side === side).map((at) => at.line);
  const lines_ = model.lines(side).map((line) => line ?? "");
  return linesAnchor(lines_, Math.min(...lines), Math.max(...lines), side);
}

function sideOf(anchor: ViewCommentAnchor): Side | undefined {
  return anchor.kind === "lines" ? anchor.side : undefined;
}

/** The row a comment's block goes under: its last line, else its first, else the end. */
function blockRow(model: LineModel, anchor: ViewCommentAnchor, rows: number, diff: boolean): number {
  if (anchor.kind !== "lines") return rows;
  const side = diff ? sideOf(anchor) ?? "new" : undefined;
  return model.row(anchor.end, side) ?? model.row(anchor.start, side) ?? Math.min(anchor.end, rows);
}

/**
 * Keeps an editor's comments in step with the core, and gives the editor
 * the host it reports to. `saved` is the text the comments are found in:
 * a file's saved contents or the diff's text, never a keystroke's. The
 * blocks are drawn as portals the caller renders, so a card keeps the page's
 * context (its tooltips, its language).
 */
export function useLineComments({
  view,
  display,
  workspace,
  actions,
  model,
  rows,
  saved,
}: {
  view: EditorView | null;
  display: ViewDisplaySnapshot;
  workspace: ViewWorkspace;
  actions: Actions;
  model: LineModel;
  rows: number;
  saved: string;
}): { host: { current: CommentHost }; blocks: ReactNode } {
  const { t } = useInterfaceTranslation();
  const feedback = useViewFeedback();
  const taken = useCommentsTaken();
  const form = useFeedbackStore((s) => s.forms[display.id] ?? null);
  const diff = display.kind === "diff";
  const comments = useMemo(() => commentsOn(feedback, display.id).filter((comment) => comment.anchor.kind === "lines"), [feedback, display.id]);
  const full = commentsOn(feedback, display.id).length >= COMMENTS_PER_VIEW;
  const lineForm = form && form.anchor.kind === "lines" ? form : null;

  // The elements the editor made for its blocks, each drawn by a portal below.
  const [mounted, setMounted] = useState<ReadonlyMap<HTMLElement, CommentBlock>>(() => new Map());

  const host = useRef<CommentHost>(null as unknown as CommentHost);
  host.current = {
    pick(from, to) {
      if (!taken) return;
      const anchor = pickedAnchor(model, from, to);
      if (anchor) openNote({ displayId: display.id, anchor, commentId: null, note: "" });
    },
    commentable: (row) => taken && model.at(row) !== null,
    label: (row) => t("feedback.addOnLine", { line: model.at(row)?.line ?? row }),
    render(block, element) {
      setMounted((now) => new Map(now).set(element, block));
      return () =>
        setMounted((now) => {
          const next = new Map(now);
          next.delete(element);
          return next;
        });
    },
  };

  // The blocks and the commented rows, sent to the editor when they change.
  const shown = useMemo(() => {
    const blocks: CommentBlock[] = [];
    const marked: number[] = [];
    for (const comment of comments) {
      const anchor = comment.anchor;
      if (anchor.kind !== "lines") continue;
      const side = diff ? sideOf(anchor) ?? "new" : undefined;
      if (!comment.changed) {
        for (let line = anchor.start; line <= anchor.end; line += 1) {
          const row = model.row(line, side);
          if (row !== null) marked.push(row);
        }
      }
      if (lineForm?.commentId === comment.id) continue;
      blocks.push({ key: comment.id, version: [comment.number, comment.note, comment.changed, comment.not_delivered, anchor.start].join("\u0000"), row: blockRow(model, anchor, rows, diff), kind: "comment" });
    }
    if (lineForm) {
      blocks.push({ key: `form:${lineForm.commentId ?? "new"}`, version: JSON.stringify(lineForm.anchor), row: blockRow(model, lineForm.anchor, rows, diff), kind: "form" });
      if (lineForm.anchor.kind === "lines" && lineForm.commentId === null) {
        for (let line = lineForm.anchor.start; line <= lineForm.anchor.end; line += 1) {
          const row = model.row(line, diff ? sideOf(lineForm.anchor) ?? "new" : undefined);
          if (row !== null) marked.push(row);
        }
      }
    }
    return { blocks, marked };
  }, [comments, lineForm, model, rows, diff]);

  useEffect(() => {
    view?.dispatch({ effects: setComments.of(shown) });
  }, [view, shown]);

  // A comment's lines found again in the saved text: moved with them, or
  // Changed when they are gone (D-14). Each answer is sent once.
  const sent = useRef(new Set<string>());
  useEffect(() => {
    for (const comment of comments) {
      const side = diff && comment.anchor.kind === "lines" ? sideOf(comment.anchor) ?? "new" : undefined;
      const lines = model.lines(side).map((line) => line ?? "\u0000");
      const moved = relocation(comment, lines);
      if (!moved) continue;
      const key = `${comment.id}:${JSON.stringify(moved)}`;
      if (sent.current.has(key)) continue;
      sent.current.add(key);
      actions.commentView(workspace, { action: "relocate", comment_id: comment.id, changed: moved.changed, ...(moved.anchor ? { anchor: moved.anchor } : {}) });
    }
    // `saved` is what `model` was built from; the comments move only when it does.
  }, [saved, comments]);

  const byId = new Map(comments.map((comment) => [comment.id, comment]));
  const blocks = [...mounted].map(([element, block]) => {
    if (block.kind === "form") {
      if (!lineForm) return null;
      return createPortal(<NoteFormCard form={lineForm} meta={anchorMeta(t, lineForm.anchor, display.path)} full={full} workspace={workspace} actions={actions} />, element, block.key);
    }
    const comment = byId.get(block.key);
    return comment ? createPortal(<CommentCard comment={comment} workspace={workspace} displayId={display.id} actions={actions} />, element, block.key) : null;
  });

  return { host, blocks };
}
