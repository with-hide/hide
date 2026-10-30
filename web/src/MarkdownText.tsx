// A Markdown text drawn read-only by the editor's own Markdown Live view
// (PRD overview-lenses-issues D-43): the same language pack, Live plan and
// theme a Markdown document uses, so an issue's body reads like the file
// viewer's preview. The language pack loads lazily, as a document's does; the
// text stands plain until it arrives.

import { Compartment, EditorState, Prec, type Extension, type Range } from "@codemirror/state";
import { Decoration, EditorView, WidgetType } from "@codemirror/view";
import { useEffect, useRef, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
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

/** A token of the text drawn as a part instead of its letters, such as a session a Desk reply names. */
export type MarkdownChip = { token: string; node: ReactNode };

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

/** Every place a chip's token stands in the text, replaced by the chip. */
function chipDecorations(text: string, chips: readonly MarkdownChip[]): Extension {
  const ranges: Range<Decoration>[] = [];
  chips.forEach((chip, index) => {
    if (!chip.token) return;
    for (let at = text.indexOf(chip.token); at >= 0; at = text.indexOf(chip.token, at + chip.token.length)) {
      ranges.push(Decoration.replace({ widget: new ChipWidget(index, chip.node) }).range(at, at + chip.token.length));
    }
  });
  return EditorView.decorations.of(Decoration.set(ranges, true));
}

const NO_CHIPS: readonly MarkdownChip[] = [];

/** `size="chat"` sets the text at Desk's conversation size; `chips` draws tokens of the text as parts (keep the array stable between renders). */
export function MarkdownText({ text, size = "body", chips = NO_CHIPS }: { text: string; size?: "body" | "chat"; chips?: readonly MarkdownChip[] }) {
  const host = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const parent = host.current;
    if (!parent) return;
    const language = new Compartment();
    const view = new EditorView({
      parent,
      state: EditorState.create({
        doc: text,
        extensions: [baseTheme(), liveTheme, Prec.highest(size === "chat" ? [chat, prose] : prose), language.of([]), markdownLive({ reveal: false }), chipDecorations(text, chips), EditorView.lineWrapping, EditorState.readOnly.of(true), EditorView.editable.of(false)],
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
  }, [text, size, chips]);
  return <div ref={host} className="min-w-0" data-markdown-text="true" />;
}
