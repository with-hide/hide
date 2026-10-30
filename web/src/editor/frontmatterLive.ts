// Markdown Live's frontmatter pane drawn as a list of properties (D-11): the
// `---` lines hidden, each key in a muted column and its value without the
// colon and the quotes, a one-word value as a chip and a path monospaced
// (`frontmatterPlan`). The line the caret is on shows its source while the
// pane holds the keyboard, so a file that opens with the caret at 0 still
// draws its first line.

import type { Range } from "@codemirror/state";
import { Decoration, EditorView, ViewPlugin, type DecorationSet, type ViewUpdate } from "@codemirror/view";
import { frontmatterPlan } from "./frontmatter";

/** The properties' line height, which also sizes the pane (`CodeMirrorEditor.tsx`). */
export const PROPERTY_LINE_HEIGHT = 1.6;
/** How many property lines the pane shows before it scrolls inside itself (D-11). */
export const PROPERTY_LINES = 8;

const LINE: Record<"delimiter" | "property" | "more" | "source", Decoration> = {
  delimiter: Decoration.line({ class: "cm-fm-hidden" }),
  property: Decoration.line({ class: "cm-fm-property" }),
  more: Decoration.line({ class: "cm-fm-more" }),
  source: Decoration.line({ class: "cm-fm-source" }),
};
const KEY = Decoration.mark({ class: "cm-fm-key" });
const HIDE = Decoration.replace({});
const VALUE = {
  chip: Decoration.mark({ class: "cm-fm-value cm-fm-chip" }),
  path: Decoration.mark({ class: "cm-fm-value cm-fm-path" }),
  text: Decoration.mark({ class: "cm-fm-value" }),
};

function build(view: EditorView): DecorationSet {
  const { doc, selection } = view.state;
  const shown = new Set<number>();
  if (view.hasFocus) {
    for (const range of selection.ranges) {
      for (let line = doc.lineAt(range.from).number; line <= doc.lineAt(range.to).number; line += 1) shown.add(line - 1);
    }
  }
  const ranges: Range<Decoration>[] = [];
  for (const mark of frontmatterPlan(doc.toString(), (line) => shown.has(line))) {
    switch (mark.kind) {
      case "key":
        ranges.push(KEY.range(mark.from, mark.to));
        break;
      case "hide":
        ranges.push(HIDE.range(mark.from, mark.to));
        break;
      case "value":
        ranges.push(VALUE[mark.look].range(mark.from, mark.to));
        break;
      default:
        ranges.push(LINE[mark.kind].range(mark.from));
    }
  }
  return Decoration.set(ranges, true);
}

const properties = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = build(view);
    }
    update(update: ViewUpdate) {
      if (update.docChanged || update.selectionSet || update.focusChanged) this.decorations = build(update.view);
    }
  },
  { decorations: (plugin) => plugin.decorations },
);

const propertiesTheme = EditorView.theme({
  "&.cm-editor": {
    height: "auto",
    maxHeight: `calc(var(--text-editor-property) * var(--cm-editor-scale, 1) * ${PROPERTY_LINE_HEIGHT * PROPERTY_LINES})`,
  },
  "&.cm-editor .cm-content": {
    fontFamily: "var(--font-sans)",
    fontSize: "calc(var(--text-editor-property) * var(--cm-editor-scale, 1))",
    lineHeight: `${PROPERTY_LINE_HEIGHT}`,
    paddingTop: "var(--spacing-lg)",
    paddingBottom: "var(--spacing-md)",
    color: "var(--subtle-foreground)",
  },
  "&.cm-editor .cm-activeLine": { backgroundColor: "transparent" },
  ".cm-line.cm-fm-property, .cm-line.cm-fm-more": {
    paddingLeft: "calc(var(--size-editor-property-key) + var(--spacing-md))",
    textIndent: "calc(-1 * (var(--size-editor-property-key) + var(--spacing-md)))",
  },
  ".cm-line.cm-fm-more": { textIndent: "0" },
  ".cm-line.cm-fm-source": { fontFamily: "var(--font-mono)", color: "var(--foreground)" },
  ".cm-fm-hidden": { display: "none" },
  ".cm-fm-key": {
    display: "inline-block",
    width: "calc(var(--size-editor-property-key) + var(--spacing-md))",
    textIndent: "0",
    overflow: "hidden",
    textOverflow: "ellipsis",
    verticalAlign: "top",
    whiteSpace: "nowrap",
    color: "var(--muted-foreground)",
  },
  ".cm-fm-chip": {
    padding: "0 var(--spacing-sm)",
    borderRadius: "var(--radius-xl)",
    backgroundColor: "var(--secondary)",
    color: "var(--foreground)",
    fontSize: "var(--text-caption)",
  },
  ".cm-fm-path": { fontFamily: "var(--font-mono)", fontSize: "var(--text-caption)" },
});

/** The frontmatter pane's Live drawing: the properties and their styles. */
export const frontmatterLive = [properties, propertiesTheme];
