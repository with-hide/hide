import { useEffect, useImperativeHandle, useLayoutEffect, useRef, type ClipboardEvent, type KeyboardEvent, type Ref } from "react";
import { createPortal } from "react-dom";
import { markedMention } from "../../desk";
import { chipDelete, chipStep, draftChips, replaceRange, type DraftChip } from "../../deskComposer";
import { cn } from "../../lib/utils";
import type { DeskDraft } from "../../ui";
import { DeskMentionChip } from "./desk-chat";

/** What the composer asks of the box: the keyboard, and the caret at a place in the draft's text. */
export type MentionInputHandle = { focus: () => void; place: (caret: number) => void };

// The box is an editable element whose text nodes the browser owns while the
// operator types, Hangul composition included; React never renders into it.
// Each chip is a span the box holds as one uneditable piece, with the chip
// drawn into it through a portal, so the box's own text and the chips never
// share a React tree. The draft's text is read back from the box after every
// input, a chip read as its token, and the box is laid out again only when
// the draft differs from what it shows (a pick, a send, a chip taken out).

const TOKEN = "deskToken";

/** The draft's text as the box shows it: its text, each chip as its token; a line break the browser left in an emptied box is no text. */
function readNodes(nodes: NodeListOf<ChildNode> | ChildNode[], out: string[]) {
  for (const node of Array.from(nodes)) {
    if (node.nodeType === Node.TEXT_NODE) {
      out.push((node.nodeValue ?? "").replace(/\u00a0/g, " "));
    } else if (node instanceof HTMLElement) {
      const token = node.dataset[TOKEN];
      if (token !== undefined) out.push(token);
      else if (node.tagName !== "BR") readNodes(node.childNodes, out);
    }
  }
}

function readBox(box: HTMLElement): string {
  const out: string[] = [];
  readNodes(box.childNodes, out);
  return out.join("");
}

/** The tokens of the chips the box shows, in order. */
function shownTokens(box: HTMLElement): string[] {
  return Array.from(box.querySelectorAll<HTMLElement>(`[data-desk-token]`)).map((span) => span.dataset[TOKEN]!);
}

/** Where a point of the box falls in the draft's text. */
function offsetOf(box: HTMLElement, node: Node, offset: number): number {
  const range = document.createRange();
  range.setStart(box, 0);
  range.setEnd(node, offset);
  const out: string[] = [];
  readNodes(Array.from(range.cloneContents().childNodes), out);
  return out.join("").length;
}

/** The selection's start and end in the draft's text, when it is in the box. */
function selectionIn(box: HTMLElement): { start: number; end: number } | null {
  const selection = document.getSelection();
  if (!selection || selection.rangeCount === 0) return null;
  const range = selection.getRangeAt(0);
  if (!box.contains(range.startContainer) || !box.contains(range.endContainer)) return null;
  return { start: offsetOf(box, range.startContainer, range.startOffset), end: offsetOf(box, range.endContainer, range.endOffset) };
}

/** Puts the caret at `caret` in the draft's text: inside a text node, or just before or after a chip. */
function placeCaret(box: HTMLElement, caret: number) {
  const selection = document.getSelection();
  if (!selection) return;
  const range = document.createRange();
  let at = 0;
  let placed = false;
  for (const node of Array.from(box.childNodes)) {
    if (node.nodeType === Node.TEXT_NODE) {
      const length = node.nodeValue?.length ?? 0;
      if (caret <= at + length) {
        range.setStart(node, caret - at);
        placed = true;
        break;
      }
      at += length;
    } else if (node instanceof HTMLElement && node.dataset[TOKEN] !== undefined) {
      const length = node.dataset[TOKEN]!.length;
      if (caret <= at) {
        range.setStartBefore(node);
        placed = true;
        break;
      }
      if (caret <= at + length) {
        range.setStartAfter(node);
        placed = true;
        break;
      }
      at += length;
    } else if (node instanceof HTMLElement && node.tagName === "BR" && node === box.lastChild) {
      range.setStartBefore(node);
      placed = true;
      break;
    }
  }
  if (!placed) {
    range.selectNodeContents(box);
    range.collapse(false);
  }
  range.collapse(true);
  selection.removeAllRanges();
  selection.addRange(range);
}

/**
 * Desk's text box with chips (D-85, B49): a picked session or project stands
 * in the words as one chip, a session's blue with its status and agent marks,
 * a project's green with ⌂, which one ⌫ takes out whole and ← or → steps
 * over. The draft's text holds each as its token, so the send carries the
 * words as written; a Hangul syllable being composed is left to the browser.
 * Shift+↵ starts a line, a paste comes in as plain text, and the keys the
 * composer needs (↵, the `@` menu's) reach `onKeyDown` first.
 */
export function MentionInput({ draft, label, placeholder, autoFocus = false, className, onChange, onCaret, onKeyDown, ref }: {
  draft: DeskDraft;
  label: string;
  placeholder: string;
  autoFocus?: boolean;
  className?: string;
  /** The words changed: the new text and where the caret stands in it. */
  onChange: (text: string, caret: number) => void;
  /** The caret moved without the words changing. */
  onCaret?: (caret: number) => void;
  onKeyDown?: (event: KeyboardEvent<HTMLDivElement>) => void;
  ref?: Ref<MentionInputHandle>;
}) {
  const box = useRef<HTMLDivElement>(null);
  const spans = useRef(new Map<string, HTMLSpanElement>());
  const pending = useRef<number | null>(null);
  const chips = draftChips(draft);
  const keyed: (DraftChip & { key: string; span: HTMLSpanElement })[] = [];
  const seen = new Map<string, number>();
  for (const chip of chips) {
    const token = markedMention(chip.mention);
    const nth = seen.get(token) ?? 0;
    seen.set(token, nth + 1);
    const key = `${token}#${nth}`;
    let span = spans.current.get(key);
    if (!span) {
      span = document.createElement("span");
      span.contentEditable = "false";
      span.dataset[TOKEN] = token;
      span.className = "inline-flex max-w-full align-baseline select-all";
      spans.current.set(key, span);
    }
    keyed.push({ ...chip, key, span });
  }

  // Lays the box out again when it shows other words or other chips than the draft.
  useLayoutEffect(() => {
    const element = box.current;
    if (!element) return;
    const tokens = keyed.map((chip) => chip.span.dataset[TOKEN]!);
    // A line break the browser leaves where words were taken out is no text, and would hold the caret in front of a chip.
    const stray = Array.from(element.childNodes).some((node, index, all) => node.nodeName === "BR" && !(index === all.length - 1 && draft.text.endsWith("\n")));
    const current = !stray && readBox(element) === draft.text && shownTokens(element).join("\n") === tokens.join("\n") && (draft.text !== "" || element.childNodes.length === 0);
    if (!current) {
      const focused = document.activeElement === element;
      const caret = pending.current ?? (focused ? selectionIn(element)?.end ?? draft.text.length : null);
      element.replaceChildren();
      let at = 0;
      for (const chip of keyed) {
        if (chip.start > at) element.append(document.createTextNode(draft.text.slice(at, chip.start)));
        element.append(chip.span);
        at = chip.end;
      }
      if (at < draft.text.length) element.append(document.createTextNode(draft.text.slice(at)));
      // A line break at the very end shows only with something after it.
      if (draft.text.endsWith("\n")) element.append(document.createElement("br"));
      if (caret !== null) placeCaret(element, Math.min(caret, draft.text.length));
    } else if (pending.current !== null) {
      placeCaret(element, pending.current);
    }
    pending.current = null;
    const kept = new Set(keyed.map((chip) => chip.key));
    for (const key of spans.current.keys()) if (!kept.has(key)) spans.current.delete(key);
  });

  // Focused from code, the caret goes to the end of the words, as a text box's does, unless a place waits for it.
  const focusBox = () => {
    const element = box.current;
    if (!element || document.activeElement === element) return;
    element.focus();
    if (pending.current === null) placeCaret(element, readBox(element).length);
  };

  useEffect(() => {
    if (autoFocus) focusBox();
  }, [autoFocus]);

  useEffect(() => {
    const follow = () => {
      const element = box.current;
      if (!element || document.activeElement !== element) return;
      const selection = selectionIn(element);
      if (selection) onCaret?.(selection.end);
    };
    document.addEventListener("selectionchange", follow);
    return () => document.removeEventListener("selectionchange", follow);
  }, [onCaret]);

  useImperativeHandle(ref, () => ({
    focus: focusBox,
    place: (caret: number) => {
      pending.current = caret;
      // The draft may not change, so no layout follows: place it once the frame is drawn.
      requestAnimationFrame(() => {
        const element = box.current;
        if (pending.current === null || !element) return;
        placeCaret(element, pending.current);
        pending.current = null;
      });
    },
  }), []);

  const commit = (next: { draft: DeskDraft; caret: number }) => {
    pending.current = next.caret;
    onChange(next.draft.text, next.caret);
  };

  const keyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    onKeyDown?.(event);
    const element = box.current;
    if (!element || event.defaultPrevented || event.nativeEvent.isComposing || event.altKey || event.metaKey || event.ctrlKey) return;
    const selection = selectionIn(element);
    if (!selection) return;
    const collapsed = selection.start === selection.end;
    if ((event.key === "Backspace" || event.key === "Delete") && collapsed) {
      const next = chipDelete(draft, selection.start, event.key === "Delete");
      if (next) {
        event.preventDefault();
        commit(next);
      }
    } else if ((event.key === "ArrowLeft" || event.key === "ArrowRight") && collapsed && !event.shiftKey) {
      const caret = chipStep(draft, selection.start, event.key === "ArrowRight");
      if (caret !== null) {
        event.preventDefault();
        placeCaret(element, caret);
        onCaret?.(caret);
      }
    } else if (event.key === "Enter" && event.shiftKey) {
      event.preventDefault();
      commit(replaceRange(draft, selection.start, selection.end, "\n"));
    }
  };

  const paste = (event: ClipboardEvent<HTMLDivElement>) => {
    const element = box.current;
    event.preventDefault();
    if (!element) return;
    const selection = selectionIn(element) ?? { start: draft.text.length, end: draft.text.length };
    const words = event.clipboardData.getData("text/plain").replace(/\r\n?/g, "\n");
    if (words) commit(replaceRange(draft, selection.start, selection.end, words));
  };

  return (
    <div className="relative min-w-0">
      {draft.text === "" ? <span aria-hidden="true" className="pointer-events-none absolute inset-x-0 top-0 truncate py-xxs text-body text-muted-foreground">{placeholder}</span> : null}
      <div
        ref={box}
        role="textbox"
        aria-multiline="true"
        aria-label={label}
        aria-placeholder={placeholder}
        contentEditable
        suppressContentEditableWarning
        spellCheck={false}
        className={cn("block w-full min-w-0 overflow-y-auto whitespace-pre-wrap break-words py-xxs text-body text-foreground outline-none [overflow-wrap:anywhere]", className)}
        onInput={() => {
          const element = box.current;
          if (!element) return;
          onChange(readBox(element), selectionIn(element)?.end ?? readBox(element).length);
        }}
        onKeyDown={keyDown}
        onPaste={paste}
        onDrop={(event) => event.preventDefault()}
        data-desk-composer-input="true"
        data-desk-chips={keyed.length}
      />
      {keyed.map((chip) => createPortal(<DeskMentionChip mention={chip.mention} />, chip.span, chip.key))}
    </div>
  );
}
