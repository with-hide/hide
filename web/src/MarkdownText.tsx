// A Markdown text drawn read-only by the editor's own Markdown Live view
// (PRD overview-lenses-issues D-43): the same language pack, Live plan and
// theme a Markdown document uses, so an issue's body reads like the file
// viewer's preview. The language pack loads lazily, as a document's does; the
// text stands plain until it arrives.

import { syntaxTree } from "@codemirror/language";
import { Compartment, EditorState, Prec, StateField, type Extension, type Range, type Text } from "@codemirror/state";
import { Decoration, EditorView, WidgetType, type DecorationSet } from "@codemirror/view";
import type { SyntaxNode } from "@lezer/common";
import { useEffect, useRef, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { CodeBlock } from "./components/code-block";
import { CopyWords } from "./components/copy-words";
import type { CopyText } from "./components/human-turn-copy";
import { languageLoader } from "./editor/languages";
import { markdownLive } from "./editor/markdownLivePlugin";
import { fencedCode } from "./editor/markdownLive";
import type { InlinePart } from "./mobile/markdown";
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

/**
 * A token of the text drawn as a part instead of its letters, such as a
 * session a Desk reply names: everywhere it stands, or only at the offsets
 * `at` names (a file path, which a code block keeps as code). A table's cells
 * draw the ones without `at`. `after` is a part that does not fit a line of
 * words: the token's place keeps `node`, and `after` stands in a row of its
 * own right behind the paragraph that holds the token, with the other parts
 * that paragraph has. `item` is the part a table's cell or a list's item
 * draws in the token's place instead, with no row after it (Doop Q 가): there
 * the token stands among other names, so the part takes the name's place.
 */
export type MarkdownChip = InlinePart & { at?: readonly number[]; after?: ReactNode; item?: ReactNode };

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
    root.render(this.nodes.map((node, at) => <span key={at} className="contents">{node}</span>));
    roots.set(dom, root);
    return dom;
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
 * Where the editor's parse puts the stretch: in code (inline code or a code
 * block, which keeps its letters), in a table or a list item, or in running
 * text; `null` while the lazily loaded language has not parsed that far. The
 * text is read once, by the parse that draws it, never by a second parser.
 */
function placeOf(state: EditorState, from: number, to: number): "code" | "item" | "text" | null {
  const tree = syntaxTree(state);
  if (tree.length < to) return null;
  for (let node: SyntaxNode | null = tree.resolveInner(from, 1); node; node = node.parent) {
    if (node.name === "InlineCode" || node.name === "FencedCode" || node.name === "CodeBlock") return "code";
    if (node.name === "ListItem" || node.name === "Table") return "item";
  }
  return "text";
}

/**
 * Every place a chip's token stands in the text, replaced by the chip, and each paragraph's `after` parts in a row behind it;
 * where two chips would cover the same letters, the one that starts first, then the earlier chip, wins. A token in code keeps
 * its letters. In a table's cell or a list's item a chip with `item` draws it there and adds no row; until the parse has reached
 * a chip, it draws `node` and no row, so a row never comes and goes. A chip whose part did not change keeps its DOM (`ChipWidget.eq`).
 */
function chipSet(state: EditorState, chips: readonly MarkdownChip[]): DecorationSet {
  if (!chips.length) return Decoration.none;
  const text = state.doc.toString();
  const found: { from: number; to: number; index: number }[] = [];
  chips.forEach((chip, index) => {
    if (!chip.token) return;
    if (chip.at) {
      for (const at of chip.at) if (text.startsWith(chip.token, at)) found.push({ from: at, to: at + chip.token.length, index });
      return;
    }
    for (let at = text.indexOf(chip.token); at >= 0; at = text.indexOf(chip.token, at + chip.token.length)) found.push({ from: at, to: at + chip.token.length, index });
  });
  found.sort((a, b) => a.from - b.from || a.index - b.index);
  const ranges: Range<Decoration>[] = [];
  const rows = new Map<number, Set<number>>();
  let end = 0;
  for (const span of found) {
    if (span.from < end) continue;
    end = span.to;
    const chip = chips[span.index]!;
    // A token in code keeps its letters; one placed by offsets (`at`) was already placed outside code.
    const where = chip.at ? "text" : placeOf(state, span.from, span.to);
    if (where === "code") continue;
    ranges.push(Decoration.replace({ widget: new ChipWidget(span.index, where === "item" && chip.item !== undefined ? chip.item : chip.node) }).range(span.from, span.to));
    if (chip.after && where === "text") {
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

/** The chips, placed again when the text changes or the parse moves (the lazily loaded language parsing it), never on other updates. */
function chipDecorations(chips: readonly MarkdownChip[]): Extension {
  const field = StateField.define<DecorationSet>({
    create: (state) => chipSet(state, chips),
    update: (placed, transaction) => (transaction.docChanged || syntaxTree(transaction.state) !== syntaxTree(transaction.startState) ? chipSet(transaction.state, chips) : placed),
    provide: (own) => EditorView.decorations.from(own),
  });
  return [chipRow, field];
}

/** What a table's cells draw for the chips (`markdownLive`'s parts): every chip without offsets, as its `item` where it has one. */
function cellParts(chips: readonly MarkdownChip[]): InlinePart[] {
  return chips.filter((chip) => !chip.at).map((chip) => (chip.item !== undefined ? { token: chip.token, node: chip.item } : chip));
}

/** A code block drawn whole by `CodeBlock`, with `복사` in its gutter, in a root of its own unmounted after the editor lets it go. */
class CodeBlockWidget extends WidgetType {
  constructor(readonly code: string, readonly language: string | null, readonly copy: CopyText | undefined) {
    super();
  }
  eq(other: CodeBlockWidget) {
    return other.code === this.code && other.language === this.language && other.copy === this.copy;
  }
  toDOM() {
    const dom = document.createElement("div");
    dom.dataset.markdownCodeBlock = "true";
    const root = createRoot(dom);
    root.render(<CodeBlock code={this.code} language={this.language} action={this.copy && this.code ? <CopyWords quiet text={this.code} what="code block" copy={this.copy} /> : undefined} />);
    roots.set(dom, root);
    return dom;
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

/**
 * A code block that stands on lines of its own, read from the parse: a
 * fence's code (`fencedCode`) with its language, an indented block's lines
 * with their four spaces. `null` for one opened after a list mark or a
 * quote's `>`, which the Live view draws as a box of lines with the same
 * head row and `복사`.
 */
function codeBlockAt(doc: Text, node: SyntaxNode): { from: number; to: number; code: string; language: string | null } | null {
  const open = doc.lineAt(node.from);
  const end = doc.lineAt(node.to);
  if (doc.sliceString(open.from, node.from).trim() !== "") return null;
  if (node.name === "CodeBlock") return { from: open.from, to: end.to, code: doc.sliceString(open.from, end.to).replace(/^ {1,4}/gm, ""), language: null };
  return { from: open.from, to: end.to, ...fencedCode(doc, node) };
}

/** Every code block of the text drawn as a `CodeBlock`; a diagram drawn in its place keeps its drawing. */
function codeBlockDecorations(state: EditorState, copy: CopyText | undefined): DecorationSet {
  const ranges: Range<Decoration>[] = [];
  syntaxTree(state).iterate({
    enter: (node) => {
      if (node.name !== "FencedCode" && node.name !== "CodeBlock") return;
      const info = node.node.getChild("CodeInfo");
      if (info && state.doc.sliceString(info.from, info.to).trim().toLowerCase() === "mermaid") return false;
      const block = codeBlockAt(state.doc, node.node);
      if (block) ranges.push(Decoration.replace({ widget: new CodeBlockWidget(block.code, block.language, copy), block: true }).range(block.from, block.to));
      return false;
    },
  });
  return Decoration.set(ranges, true);
}

/**
 * The code blocks (Doop N 1 나), found again whenever the lazily loaded
 * language parses the text. A block replaces whole lines, which only a
 * state field may give; it outranks the Live plan's own code lines.
 */
function codeBlocks(copy: CopyText | undefined): Extension {
  const field = StateField.define<DecorationSet>({
    create: (state) => codeBlockDecorations(state, copy),
    update: (blocks, transaction) => (transaction.docChanged || syntaxTree(transaction.state) !== syntaxTree(transaction.startState) ? codeBlockDecorations(transaction.state, copy) : blocks),
    provide: (own) => EditorView.decorations.from(own),
  });
  return Prec.highest(field);
}

const NO_CHIPS: readonly MarkdownChip[] = [];

/** A stretch of the text lit for a moment: the sentence a question line pressed (Doop N3 가). */
export type MarkdownMark = { from: number; to: number };

const LIT = Decoration.mark({ class: "rounded-xs bg-open-ask/20", attributes: { "data-markdown-lit": "true" } });

function markDecorations(mark: MarkdownMark | null, length: number): Extension {
  if (!mark || mark.from >= mark.to || mark.to > length) return [];
  return EditorView.decorations.of(Decoration.set([LIT.range(mark.from, mark.to)]));
}

/**
 * `size="chat"` sets the text at Desk's conversation size; `chips` draws
 * tokens of the text as parts (keep the array stable between renders);
 * `copyCode` puts `복사` in every code block's head row (Doop E2, N 1 나),
 * a block in a quote or a list included, always in view; `mark` lights a
 * stretch of the text, by offsets into it. The text can be
 * selected and copied as a page's text is.
 */
export function MarkdownText({ text, size = "body", chips = NO_CHIPS, copyCode, mark = null }: { text: string; size?: "body" | "chat"; chips?: readonly MarkdownChip[]; copyCode?: CopyText; mark?: MarkdownMark | null }) {
  const host = useRef<HTMLDivElement>(null);
  // The lit stretch changes without drawing the text again.
  const lit = useRef<{ view: EditorView; compartment: Compartment } | null>(null);
  const markNow = useRef(mark);
  markNow.current = mark;
  useEffect(() => {
    const parent = host.current;
    if (!parent) return;
    const language = new Compartment();
    const marked = new Compartment();
    const view = new EditorView({
      parent,
      state: EditorState.create({
        doc: text,
        extensions: [baseTheme(), liveTheme, Prec.highest(size === "chat" ? [chat, prose] : prose), language.of([]), markdownLive({ reveal: false, parts: cellParts(chips), copy: copyCode }), chipDecorations(chips), codeBlocks(copyCode), marked.of(markDecorations(markNow.current, text.length)), EditorView.lineWrapping, EditorState.readOnly.of(true), EditorView.editable.of(false)],
      }),
    });
    lit.current = { view, compartment: marked };
    let live = true;
    void languageLoader("markdown", "issue.md")?.().then((extension) => {
      if (live) view.dispatch({ effects: language.reconfigure(extension) });
    });
    return () => {
      live = false;
      lit.current = null;
      view.destroy();
    };
  }, [text, size, chips, copyCode]);
  useEffect(() => {
    const now = lit.current;
    now?.view.dispatch({ effects: now.compartment.reconfigure(markDecorations(mark, now.view.state.doc.length)) });
  }, [mark]);
  return <div ref={host} className="min-w-0" data-markdown-text="true" />;
}
