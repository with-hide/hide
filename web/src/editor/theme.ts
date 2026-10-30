// The editor's chrome and syntax colors, taken from the design tokens the
// generated CSS already carries. No hex value is written here: every color is
// a `var(--<token>)`, so a token change moves the editor with the shell
// (`scripts/check-web-tokens.mjs` refuses a literal).

import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { Compartment } from "@codemirror/state";
import { EditorView, ViewPlugin } from "@codemirror/view";
import { tags as t } from "@lezer/highlight";

// CodeMirror draws a few things of its own (bracket and selection matches, fold
// markers) in a light or a dark variant; which one follows the page theme, so
// every open editor is told when the theme changes and a new one starts on it.
const appearance = new Compartment();
const openEditors = new Set<EditorView>();
const tracked = ViewPlugin.define((view) => {
  openEditors.add(view);
  return { destroy: () => openEditors.delete(view) };
});

function pageIsDark(): boolean {
  return !document.documentElement.classList.contains("light");
}

/** Re-theme every open editor after the page theme changed; content, selection and history stay. */
export function applyEditorTheme() {
  const effects = appearance.reconfigure(EditorView.darkTheme.of(pageIsDark()));
  for (const view of openEditors) view.dispatch({ effects });
}

// The base chrome: on the side panel's card, with a caret, a current line and
// a selection that read against it in either theme, and a search panel in the
// shell's palette. The selection takes the terminal's selection colour, the
// one text selection the shell draws, so it stays apart from the current line.
// Built once, so every editor shares one set of mounted style rules.
const chrome = EditorView.theme(
  {
    "&": {
      color: "var(--foreground)",
      backgroundColor: "var(--card)",
      height: "100%",
    },
    ".cm-content": {
      caretColor: "var(--primary)",
      fontFamily: "var(--font-mono)",
    },
    ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--primary)" },
    "&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection":
      { backgroundColor: "var(--terminal-selection)" },
    ".cm-activeLine": { backgroundColor: "var(--secondary)" },
    ".cm-gutters": {
      backgroundColor: "var(--card)",
      color: "var(--muted-foreground)",
      border: "none",
    },
    ".cm-activeLineGutter": { backgroundColor: "var(--secondary)", color: "var(--subtle-foreground)" },
    ".cm-panels": { backgroundColor: "var(--card)", color: "var(--foreground)" },
    ".cm-panels.cm-panels-top": { borderBottom: "var(--size-hairline) solid var(--border)" },
    ".cm-searchMatch": { backgroundColor: "var(--secondary)", outline: "var(--size-hairline) solid var(--border)" },
    ".cm-searchMatch.cm-searchMatch-selected": { backgroundColor: "var(--popover)" },
    ".cm-button": {
      backgroundImage: "none",
      backgroundColor: "var(--secondary)",
      color: "var(--foreground)",
      border: "var(--size-hairline) solid var(--border)",
    },
    ".cm-textfield": {
      backgroundColor: "var(--background)",
      color: "var(--foreground)",
      border: "var(--size-hairline) solid var(--border)",
    },
    ".cm-tooltip": { backgroundColor: "var(--popover)", border: "var(--size-hairline) solid var(--border)" },
  },
);

const highlighting = syntaxHighlighting(
  HighlightStyle.define([
    { tag: [t.comment, t.lineComment, t.blockComment], color: "var(--muted-foreground)" },
    { tag: [t.keyword, t.modifier, t.controlKeyword, t.operatorKeyword], color: "var(--file-purple)" },
    { tag: [t.string, t.special(t.string)], color: "var(--file-green)" },
    { tag: [t.number, t.bool, t.null], color: "var(--file-orange)" },
    { tag: [t.function(t.variableName), t.function(t.propertyName)], color: "var(--file-yellow)" },
    { tag: [t.typeName, t.className, t.namespace], color: "var(--file-blue)" },
    { tag: [t.propertyName, t.attributeName], color: "var(--file-blue)" },
    { tag: [t.variableName, t.definition(t.variableName)], color: "var(--foreground)" },
    { tag: [t.tagName], color: "var(--file-orange)" },
    { tag: [t.heading], color: "var(--file-blue)", fontWeight: "bold" },
    { tag: [t.emphasis], fontStyle: "italic" },
    { tag: [t.strong], fontWeight: "bold" },
    { tag: [t.link, t.url], color: "var(--file-blue)", textDecoration: "underline" },
    { tag: [t.invalid], color: "var(--destructive)" },
  ]),
);

/** The editor's theme, starting on the page's current Light or Dark variant. */
export const baseTheme = () => [appearance.of(EditorView.darkTheme.of(pageIsDark())), tracked, chrome, highlighting];

/** The text scale the core holds for the editor (`ui_state.editor_text_scale`). */
export function scaleTheme(scale: number) {
  return EditorView.theme({
    // Markdown Live's headings and frontmatter properties size from their own tokens times this.
    "&": { "--cm-editor-scale": `${scale}` },
    ".cm-content": {
      fontSize: `calc(var(--text-editor-document) * ${scale})`,
      lineHeight: "1.5",
    },
    ".cm-gutters": { fontSize: `calc(var(--text-editor-base) * ${scale})` },
  });
}

const COLUMN_INSET = "max(var(--spacing-none), calc((100% - var(--size-editor-measure) - var(--size-control-sm)) / 2))";

/**
 * A Markdown document in Live reads as a page, not as source: the interface's
 * sans-serif font, a looser line, one centred column (`--size-editor-measure`,
 * the comment gutter beside it) and no current-line band. Source mode keeps
 * the monospaced editor; an issue body (`MarkdownText.tsx`) takes only the
 * drawn styles below.
 */
export const liveDocumentTheme = EditorView.theme({
  "& .cm-scroller": { paddingLeft: COLUMN_INSET, paddingRight: COLUMN_INSET },
  "&.cm-editor .cm-content": {
    fontFamily: "var(--font-sans)",
    lineHeight: "1.75",
    paddingTop: "var(--spacing-lg)",
    paddingBottom: "var(--spacing-xxxl)",
  },
  "&.cm-editor .cm-activeLine": { backgroundColor: "transparent" },
});

/**
 * The frontmatter pane's column (`frontmatterLive.ts`): the body's, moved past
 * the comment gutter the pane does not have, so a key starts where the text does.
 */
export const liveFrontColumn = EditorView.theme({
  "& .cm-scroller": { paddingLeft: `calc(${COLUMN_INSET} + var(--size-control-sm))`, paddingRight: COLUMN_INSET },
});

const heading = (level: number, lineHeight: string, spaceAbove: string) => ({
  fontSize: `calc(var(--text-editor-heading-${level}) * var(--cm-editor-scale, 1))`,
  fontWeight: level === 1 ? "700" : "600",
  lineHeight,
  paddingTop: spaceAbove,
  color: "var(--foreground)",
});

/**
 * Markdown Live's drawn styles (PRD B6): the block and inline classes the plan
 * emits. Headings take the text colour and are told apart by size and weight;
 * their sizes are the editor heading tokens times the editor's text scale. A
 * hidden fence line collapses to no height.
 */
export const liveTheme = EditorView.theme({
  ".cm-md-heading-1": { ...heading(1, "1.35", "var(--spacing-sm)"), letterSpacing: "-0.01em" },
  ".cm-md-heading-2": {
    ...heading(2, "1.4", "var(--spacing-xl)"),
    paddingBottom: "var(--spacing-xs)",
    borderBottom: "var(--size-hairline) solid var(--border)",
  },
  ".cm-md-heading-3": heading(3, "1.4", "var(--spacing-lg)"),
  ".cm-md-heading-4": heading(4, "1.5", "var(--spacing-md)"),
  ".cm-md-heading-5": heading(5, "1.5", "var(--spacing-md)"),
  ".cm-md-heading-6": { ...heading(6, "1.5", "var(--spacing-md)"), color: "var(--subtle-foreground)" },
  // The syntax colours stay with Source; a drawn heading reads in its line's colour.
  "& [class*='cm-md-heading-'] span": { color: "inherit" },
  "& .cm-line .cm-md-link": { color: "var(--file-blue)" },
  "& .cm-line .cm-md-code-inline": {
    fontFamily: "var(--font-mono)",
    fontSize: "0.86em",
    padding: "var(--spacing-xxs) var(--spacing-xs)",
    borderRadius: "var(--radius-xs)",
    backgroundColor: "var(--secondary)",
  },
  ".cm-md-code": { fontFamily: "var(--font-mono)", fontSize: "0.9em", lineHeight: "1.6", backgroundColor: "var(--secondary)" },
  ".cm-md-link": { textDecoration: "underline" },
  ".cm-md-hidden-line": { display: "none" },
  ".cm-md-quote": {
    borderLeft: "var(--size-editor-quote-rule) solid var(--border)",
    paddingLeft: "var(--spacing-sm)",
    color: "var(--subtle-foreground)",
  },
  ".cm-md-bullet": { color: "var(--subtle-foreground)" },
  ".cm-md-checkbox": { verticalAlign: "middle" },
  // Drawn HTML and images. The page's reset makes an image a block and strips
  // list and table styling, so each of those is put back to what HTML means.
  // The editor keeps a text's newlines (`pre-wrap`); HTML collapses them.
  ".cm-md-html": { fontFamily: "var(--font-sans)", lineHeight: "1.5", whiteSpace: "normal" },
  ".cm-md-html pre": { whiteSpace: "pre-wrap" },
  ".cm-md-html-block": { display: "block" },
  ".cm-md-html img": { display: "inline-block", maxWidth: "100%", height: "auto", verticalAlign: "middle" },
  ".cm-md-html a[href]": { color: "var(--file-blue)", textDecoration: "underline" },
  ".cm-md-html p, .cm-md-html ul, .cm-md-html ol, .cm-md-html table, .cm-md-html blockquote, .cm-md-html pre": { margin: "0 0 var(--spacing-sm)" },
  ".cm-md-html h1": { fontSize: "var(--text-editor-heading-1)", fontWeight: "600", lineHeight: "1.3", margin: "0 0 var(--spacing-sm)" },
  ".cm-md-html h2": { fontSize: "var(--text-editor-heading-2)", fontWeight: "600", lineHeight: "1.3", margin: "0 0 var(--spacing-sm)" },
  ".cm-md-html h3": { fontSize: "var(--text-editor-heading-3)", fontWeight: "600", lineHeight: "1.3", margin: "0 0 var(--spacing-sm)" },
  ".cm-md-html h4, .cm-md-html h5, .cm-md-html h6": { fontSize: "var(--text-editor-heading-4)", fontWeight: "600", lineHeight: "1.4", margin: "0 0 var(--spacing-sm)" },
  ".cm-md-html ul": { listStyle: "disc", paddingLeft: "1.5em" },
  ".cm-md-html ol": { listStyle: "decimal", paddingLeft: "1.5em" },
  ".cm-md-html blockquote": { borderLeft: "var(--size-editor-quote-rule) solid var(--border)", paddingLeft: "var(--spacing-sm)", color: "var(--subtle-foreground)" },
  ".cm-md-html code, .cm-md-html kbd, .cm-md-html pre": { fontFamily: "var(--font-mono)", backgroundColor: "var(--secondary)" },
  ".cm-md-html hr": { border: "0", borderTop: "var(--size-hairline) solid var(--border)", margin: "var(--spacing-sm) 0" },
  ".cm-md-html table": { borderCollapse: "collapse" },
  ".cm-md-html th, .cm-md-html td": { border: "var(--size-hairline) solid var(--border)", padding: "0 var(--spacing-sm)" },
  ".cm-md-html summary": { cursor: "pointer" },
  // An image that did not load: its alt text, muted and small, nothing else.
  ".cm-md-alt": { color: "var(--muted-foreground)", fontSize: "var(--text-caption)" },
  ".cm-md-alt[data-state='loading']": { display: "none" },
});
