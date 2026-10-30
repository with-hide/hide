// A Markdown text drawn read-only by the editor's own Markdown Live view
// (PRD overview-lenses-issues D-43): the same language pack, Live plan and
// theme a Markdown document uses, so an issue's body reads like the file
// viewer's preview. The language pack loads lazily, as a document's does; the
// text stands plain until it arrives.

import { Compartment, EditorState, Prec } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { useEffect, useRef } from "react";
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

/** `size="chat"` sets the text at Desk's conversation size. */
export function MarkdownText({ text, size = "body" }: { text: string; size?: "body" | "chat" }) {
  const host = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const parent = host.current;
    if (!parent) return;
    const language = new Compartment();
    const view = new EditorView({
      parent,
      state: EditorState.create({
        doc: text,
        extensions: [baseTheme(), liveTheme, Prec.highest(size === "chat" ? [chat, prose] : prose), language.of([]), markdownLive({ reveal: false }), EditorView.lineWrapping, EditorState.readOnly.of(true), EditorView.editable.of(false)],
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
  }, [text, size]);
  return <div ref={host} className="min-w-0" data-markdown-text="true" />;
}
