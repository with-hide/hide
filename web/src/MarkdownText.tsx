// A Markdown text drawn read-only by the editor's own Markdown Live view
// (PRD overview-lenses-issues D-43): the same language pack, Live plan and
// theme a Markdown document uses, so an issue's body reads like the file
// viewer's preview. The language pack loads lazily, as a document's does; the
// text stands plain until it arrives. One editor draws the text for as long as
// it stands: a changed text (an answer streaming in) is one edit of the words
// that changed, so the parts already drawn keep their nodes.

import { syntaxTree } from "@codemirror/language";
import { Compartment, EditorState, Prec, StateField, type Extension, type Range } from "@codemirror/state";
import { Decoration, EditorView, ViewPlugin, WidgetType, type DecorationSet, type ViewUpdate } from "@codemirror/view";
import { useLayoutEffect, useRef, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { CopyWords } from "./components/copy-words";
import type { CopyText } from "./components/human-turn-copy";
import { languageLoader } from "./editor/languages";
import { markdownLive } from "./editor/markdownLivePlugin";
import { baseTheme, liveTheme } from "./editor/theme";

// The chrome sets the editor's monospace on `.cm-content` and the card's
// background; prose outranks both, so the text sits on whatever surface holds
// it (a Task page's background as well as an issue panel's card). A theme
// listed later loses to an earlier one, hence the precedence.
const prose = EditorView.theme({
  "&": { backgroundColor: "transparent", height: "auto" },
  "&.cm-editor .cm-content": { fontFamily: "var(--font-sans)", fontSize: "var(--text-body)", lineHeight: "1.5", padding: "0" },
  ".cm-line": { padding: "0" },
  "&.cm-focused": { outline: "none" },
});

// Desk's conversation reads larger and looser than a panel (PRD
// session-docs-and-assistant D-75): the subhead size at a relaxed leading.
const chat = EditorView.theme({
  "&.cm-editor .cm-content": { fontSize: "var(--text-subhead)", lineHeight: "1.625" },
});

// A code block's `복사` stands at the top right of its first line, inside the block's own ground.
const codeCopy = EditorView.theme({
  ".cm-md-code-head": { position: "relative" },
  ".cm-md-code-copy": { position: "absolute", top: "var(--spacing-xxs)", right: "var(--spacing-xs)", fontFamily: "var(--font-sans)", lineHeight: "normal" },
});

/**
 * A token of the text drawn as a part instead of its letters, such as a
 * session a Desk reply names: everywhere it stands, or only at the offsets
 * `at` names (a file path, which a code block keeps as code). `after` is a
 * part that does not fit a line of words: the token's place keeps `node`, and
 * `after` stands in a row of its own right behind the paragraph that holds the
 * token, with the other parts that paragraph has.
 */
export type MarkdownChip = { token: string; node: ReactNode; at?: readonly number[]; after?: ReactNode };

const roots = new WeakMap<HTMLElement, Root>();

/** A chip in the read-only text: its own React root, unmounted after the editor lets it go, since that happens while React commits. */
class ChipWidget extends WidgetType {
  constructor(readonly index: number, readonly node: ReactNode) {
    super();
  }
  eq(other: ChipWidget) {
    return other.index === this.index && other.node === this.node;
  }
  toDOM() {
    const dom = document.createElement("span");
    dom.dataset.markdownChip = String(this.index);
    const root = createRoot(dom);
    root.render(this.node);
    roots.set(dom, root);
    return dom;
  }
  /** Another chip in this one's place is drawn into its node and root, so the place never stands empty. */
  updateDOM(dom: HTMLElement) {
    const root = roots.get(dom);
    if (!root) return false;
    dom.dataset.markdownChip = String(this.index);
    root.render(this.node);
    return true;
  }
  destroy(dom: HTMLElement) {
    const root = roots.get(dom);
    roots.delete(dom);
    if (root) queueMicrotask(() => root.unmount());
  }
}

/** A row of parts under a paragraph, in a root of its own, unmounted after the editor lets it go. */
class ChipRowWidget extends WidgetType {
  constructor(readonly indexes: string, readonly nodes: readonly ReactNode[]) {
    super();
  }
  eq(other: ChipRowWidget) {
    return other.indexes === this.indexes && other.nodes.every((node, at) => node === this.nodes[at]);
  }
  toDOM() {
    const dom = document.createElement("div");
    dom.className = "cm-md-chip-row";
    dom.dataset.markdownChipRow = this.indexes;
    const root = createRoot(dom);
    root.render(this.parts());
    roots.set(dom, root);
    return dom;
  }
  updateDOM(dom: HTMLElement) {
    const root = roots.get(dom);
    if (!root) return false;
    dom.dataset.markdownChipRow = this.indexes;
    root.render(this.parts());
    return true;
  }
  private parts() {
    return this.nodes.map((node, at) => <span key={at} className="contents">{node}</span>);
  }
  destroy(dom: HTMLElement) {
    const root = roots.get(dom);
    roots.delete(dom);
    if (root) queueMicrotask(() => root.unmount());
  }
  ignoreEvent() {
    return true;
  }
}

// The row's room: the words' spacing and the parts side by side, wrapping to the next line when they run out.
const chipRow = EditorView.theme({
  ".cm-md-chip-row": { display: "flex", flexWrap: "wrap", gap: "var(--spacing-sm)", padding: "var(--spacing-xs) 0" },
});

/** Where the paragraph holding `at` ends: before its blank line, or the text's end. */
function paragraphEnd(text: string, at: number): number {
  const blank = text.indexOf("\n\n", at);
  return blank < 0 ? text.length : blank;
}

/**
 * Every place a chip's token stands in the text, replaced by the chip, and each paragraph's `after` parts in a row behind it;
 * where two chips would cover the same letters, the one that starts first, then the earlier chip, wins.
 */
function chipSet(text: string, chips: readonly MarkdownChip[]): DecorationSet {
  const spans: { from: number; to: number; index: number }[] = [];
  chips.forEach((chip, index) => {
    if (!chip.token) return;
    if (chip.at) {
      for (const at of chip.at) if (text.startsWith(chip.token, at)) spans.push({ from: at, to: at + chip.token.length, index });
      return;
    }
    for (let at = text.indexOf(chip.token); at >= 0; at = text.indexOf(chip.token, at + chip.token.length)) spans.push({ from: at, to: at + chip.token.length, index });
  });
  spans.sort((a, b) => a.from - b.from || a.index - b.index);
  const ranges: Range<Decoration>[] = [];
  const rows = new Map<number, Set<number>>();
  let end = 0;
  for (const span of spans) {
    if (span.from < end) continue;
    const chip = chips[span.index]!;
    ranges.push(Decoration.replace({ widget: new ChipWidget(span.index, chip.node) }).range(span.from, span.to));
    end = span.to;
    if (chip.after) {
      const close = paragraphEnd(text, span.from);
      rows.set(close, (rows.get(close) ?? new Set()).add(span.index));
    }
  }
  for (const [at, held] of rows) {
    const indexes = [...held].sort((a, b) => a - b);
    ranges.push(Decoration.widget({ widget: new ChipRowWidget(indexes.join(" "), indexes.map((index) => chips[index]!.after)), block: true, side: 1 }).range(at));
  }
  return Decoration.set(ranges, true);
}

/** The chips, found again in the words each time they change. */
function chipDecorations(chips: readonly MarkdownChip[]): Extension {
  if (!chips.length) return [];
  const field = StateField.define<DecorationSet>({
    create: (state) => chipSet(state.doc.toString(), chips),
    update: (set, transaction) => (transaction.docChanged ? chipSet(transaction.state.doc.toString(), chips) : set),
    provide: (own) => EditorView.decorations.from(own),
  });
  return [chipRow, field];
}

/** A code block's `복사`, in a root of its own, unmounted after the editor lets it go. */
class CodeCopyWidget extends WidgetType {
  constructor(readonly code: string, readonly copy: CopyText) {
    super();
  }
  eq(other: CodeCopyWidget) {
    return other.code === this.code && other.copy === this.copy;
  }
  toDOM() {
    const dom = document.createElement("span");
    dom.className = "cm-md-code-copy";
    dom.dataset.markdownCodeCopy = "true";
    const root = createRoot(dom);
    root.render(this.words());
    roots.set(dom, root);
    return dom;
  }
  /** A block whose code grew (an answer streaming it) keeps its `복사`, which copies the code it has now. */
  updateDOM(dom: HTMLElement) {
    const root = roots.get(dom);
    if (!root) return false;
    root.render(this.words());
    return true;
  }
  private words() {
    return <CopyWords text={this.code} what="code block" copy={this.copy} />;
  }
  destroy(dom: HTMLElement) {
    const root = roots.get(dom);
    roots.delete(dom);
    if (root) queueMicrotask(() => root.unmount());
  }
  ignoreEvent() {
    return true;
  }
}

/** Each fenced code block's code with `복사` on its first line, read from the parse; a diagram drawn in its place has none. */
function codeCopyDecorations(view: EditorView, copy: CopyText): DecorationSet {
  const ranges: Range<Decoration>[] = [];
  const doc = view.state.doc;
  syntaxTree(view.state).iterate({
    enter: (node) => {
      if (node.name !== "FencedCode") return;
      const info = node.node.getChild("CodeInfo");
      if (info && doc.sliceString(info.from, info.to).trim().toLowerCase() === "mermaid") return false;
      const text = node.node.getChild("CodeText");
      if (!text || text.from === text.to) return false;
      const line = doc.lineAt(text.from);
      ranges.push(Decoration.line({ class: "cm-md-code-head" }).range(line.from));
      ranges.push(Decoration.widget({ widget: new CodeCopyWidget(doc.sliceString(text.from, text.to), copy), side: -1 }).range(line.from));
      return false;
    },
  });
  return Decoration.set(ranges, true);
}

/** `복사` on every code block, found again whenever the lazily loaded language parses the text. */
function codeCopyButtons(copy: CopyText): Extension {
  return [
    codeCopy,
    ViewPlugin.define(
      (view) => ({
        decorations: codeCopyDecorations(view, copy),
        update(this: { decorations: DecorationSet }, update: ViewUpdate) {
          if (update.docChanged || syntaxTree(update.state) !== syntaxTree(update.startState)) this.decorations = codeCopyDecorations(update.view, copy);
        },
      }),
      { decorations: (plugin) => plugin.decorations },
    ),
  ];
}

const NO_CHIPS: readonly MarkdownChip[] = [];

/** The one edit that turns `from` into `to`: the stretch between their common start and their common end. */
function changeBetween(from: string, to: string): { from: number; to: number; insert: string } {
  const most = Math.min(from.length, to.length);
  let start = 0;
  while (start < most && from.charCodeAt(start) === to.charCodeAt(start)) start++;
  let end = 0;
  while (end < most - start && from.charCodeAt(from.length - 1 - end) === to.charCodeAt(to.length - 1 - end)) end++;
  return { from: start, to: from.length - end, insert: to.slice(start, to.length - end) };
}

/** The parts drawn from the caller's chips and `복사`, swapped as one when either changes. */
function partsOf(chips: readonly MarkdownChip[], copyCode: CopyText | undefined): Extension {
  return [chipDecorations(chips), copyCode ? codeCopyButtons(copyCode) : []];
}

type Drawn = { view: EditorView; parts: Compartment; text: string; chips: readonly MarkdownChip[]; copyCode: CopyText | undefined };

/**
 * `size="chat"` sets the text at Desk's conversation size; `chips` draws
 * tokens of the text as parts (keep the array stable between renders, or every
 * part is drawn again); `copyCode` puts `복사` on every code block (Doop E2),
 * always in view. The text can be selected and copied as a page's text is. A
 * new `text` is written into the editor already drawn as one edit, so a
 * streamed answer adds its words without drawing again what it said before.
 */
export function MarkdownText({ text, size = "body", chips = NO_CHIPS, copyCode }: { text: string; size?: "body" | "chat"; chips?: readonly MarkdownChip[]; copyCode?: CopyText }) {
  const host = useRef<HTMLDivElement>(null);
  const drawn = useRef<Drawn | null>(null);
  const latest = useRef({ text, chips, copyCode });
  latest.current = { text, chips, copyCode };
  // The editor lives as long as the text's size; the language pack lights it up once.
  useLayoutEffect(() => {
    const parent = host.current;
    if (!parent) return;
    const now = latest.current;
    const language = new Compartment();
    const parts = new Compartment();
    const view = new EditorView({
      parent,
      state: EditorState.create({
        doc: now.text,
        extensions: [baseTheme(), liveTheme, Prec.highest(size === "chat" ? [chat, prose] : prose), language.of([]), markdownLive({ reveal: false }), parts.of(partsOf(now.chips, now.copyCode)), EditorView.lineWrapping, EditorState.readOnly.of(true), EditorView.editable.of(false)],
      }),
    });
    drawn.current = { view, parts, ...now };
    let live = true;
    void languageLoader("markdown", "issue.md")?.().then((extension) => {
      if (live) view.dispatch({ effects: language.reconfigure(extension) });
    });
    return () => {
      live = false;
      drawn.current = null;
      view.destroy();
    };
  }, [size]);
  // A changed text is one edit of the stretch that changed; changed parts are swapped in the same transaction.
  useLayoutEffect(() => {
    const now = drawn.current;
    if (!now) return;
    const swap = now.chips !== chips || now.copyCode !== copyCode;
    if (now.text === text && !swap) return;
    now.view.dispatch({
      changes: now.text === text ? undefined : changeBetween(now.text, text),
      effects: swap ? now.parts.reconfigure(partsOf(chips, copyCode)) : undefined,
    });
    drawn.current = { ...now, text, chips, copyCode };
  }, [text, chips, copyCode]);
  return <div ref={host} className="min-w-0" data-markdown-text="true" />;
}
