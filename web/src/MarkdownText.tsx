// A Markdown text drawn read-only by the editor's own Markdown Live view
// (PRD overview-lenses-issues D-43): the same language pack, Live plan and
// theme a Markdown document uses, so an issue's body reads like the file
// viewer's preview. The language pack loads lazily, as a document's does; the
// text stands plain until it arrives.

import { syntaxTree } from "@codemirror/language";
import { Compartment, EditorState, Prec, type Extension, type Range } from "@codemirror/state";
import { Decoration, EditorView, ViewPlugin, WidgetType, type DecorationSet, type ViewUpdate } from "@codemirror/view";
import { useEffect, useRef, type ReactNode } from "react";
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
 * `at` names (a file path, which a code block keeps as code).
 */
export type MarkdownChip = { token: string; node: ReactNode; at?: readonly number[] };

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

/** Every place a chip's token stands in the text, replaced by the chip; where two chips would cover the same letters, the one that starts first, then the earlier chip, wins. */
function chipDecorations(text: string, chips: readonly MarkdownChip[]): Extension {
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
  let end = 0;
  for (const span of spans) {
    if (span.from < end) continue;
    ranges.push(Decoration.replace({ widget: new ChipWidget(span.index, chips[span.index]!.node) }).range(span.from, span.to));
    end = span.to;
  }
  return EditorView.decorations.of(Decoration.set(ranges));
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
    root.render(<CopyWords text={this.code} what="code block" copy={this.copy} />);
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

/**
 * `size="chat"` sets the text at Desk's conversation size; `chips` draws
 * tokens of the text as parts (keep the array stable between renders);
 * `copyCode` puts `복사` on every code block (Doop E2), always in view. The
 * text can be selected and copied as a page's text is.
 */
export function MarkdownText({ text, size = "body", chips = NO_CHIPS, copyCode }: { text: string; size?: "body" | "chat"; chips?: readonly MarkdownChip[]; copyCode?: CopyText }) {
  const host = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const parent = host.current;
    if (!parent) return;
    const language = new Compartment();
    const view = new EditorView({
      parent,
      state: EditorState.create({
        doc: text,
        extensions: [baseTheme(), liveTheme, Prec.highest(size === "chat" ? [chat, prose] : prose), language.of([]), markdownLive({ reveal: false }), chipDecorations(text, chips), copyCode ? codeCopyButtons(copyCode) : [], EditorView.lineWrapping, EditorState.readOnly.of(true), EditorView.editable.of(false)],
      }),
    });
    let live = true;
    void languageLoader("markdown", "issue.md")?.().then((extension) => {
      if (live) view.dispatch({ effects: language.reconfigure(extension) });
    });
    return () => {
      live = false;
      view.destroy();
    };
  }, [text, size, chips, copyCode]);
  return <div ref={host} className="min-w-0" data-markdown-text="true" />;
}
