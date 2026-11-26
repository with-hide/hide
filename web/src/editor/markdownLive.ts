// Markdown Live's reading of a document (PRD B6, D-07): which ranges carry a
// block or inline style, and which markup characters are hidden while the
// selection sits elsewhere. The plan is a pure function of the syntax tree,
// the text and the selection, so it is testable without a browser, and the
// CodeMirror plugin is only the part that turns it into decorations.
//
// Live draws the rendered document and hides the markup: heading
// hashes, emphasis and code delimiters, link syntax and its URL, quote
// prefixes, list indentation and the code fences. A line whose selection
// touches it keeps its source, which is how the operator edits the markup.
//
// An HTML block, an inline `<img>` and a Markdown image are drawn as what they
// say; the plan only names their source (`html`, `image`), and the plugin
// sanitizes and loads it. Other inline HTML (`<br>`, `<kbd>`, `<sub>`) stays
// source. A top-level fenced block whose language is `mermaid` is drawn as its
// diagram the same way (`mermaid`), and is revealed like an HTML block; so is
// a top-level GFM table (`table`), which the plugin draws with the renderer
// the phone uses.

import type { SyntaxNode, Tree } from "@lezer/common";
import type { Text } from "@codemirror/state";
import { isMermaidInfo } from "../markdownMermaid";

export type LiveKind = "hide" | "bullet" | "listItem" | "checkbox" | "heading" | "codeHead" | "codeBlock" | "fenceLine" | "quote" | "link" | "codeInline" | "html" | "image" | "mermaid" | "table";

export type LiveRange = {
  from: number;
  to: number;
  kind: LiveKind;
  /** Heading level 1-6. */
  level?: number;
  /** `listItem`: how many lists hold the item's list (0 for a top-level item). */
  depth?: number;
  /** Task checkbox state. */
  checked?: boolean;
  /** `html`: the source to draw, and whether it takes whole lines (an HTML block) or sits in a line (a tag). */
  html?: string;
  block?: boolean;
  /** `image`: a Markdown image's source address, alt text and title. */
  src?: string;
  alt?: string;
  title?: string;
  /** `mermaid`: the diagram's source, the text between the fences; `table`: the table's source; `codeHead`: the block's code as it is copied; `listItem`: the mark it draws (`•`, or an ordered item's number). */
  source?: string;
  /** `codeHead`: the language the fence names, null when it names none. */
  language?: string | null;
  /** `codeBlock`: the line is the block's first code line, its last, or its only one, so the block is drawn as one box under its head. */
  edge?: "first" | "last" | "only";
};

/** The Live view's whole-document byte bound: above
 * it the parse is off and the document reads as source (D-07). */
export const MARKDOWN_LIVE_BYTE_LIMIT = 256 * 1024;

const HEADINGS: Record<string, number> = {
  ATXHeading1: 1,
  ATXHeading2: 2,
  ATXHeading3: 3,
  ATXHeading4: 4,
  ATXHeading5: 5,
  ATXHeading6: 6,
  SetextHeading1: 1,
  SetextHeading2: 2,
};

/**
 * `embedsRevealed` says whether the selection reveals an HTML block or an
 * image's source: a file opens with the caret at 0, which the operator did not
 * put there, so they stay drawn until the operator has moved it.
 */
export function markdownLivePlan(tree: Tree, doc: Text, selection: { from: number; to: number }, embedsRevealed = true): LiveRange[] {
  const ranges: LiveRange[] = [];
  const overlaps = (from: number, to: number) => selection.from <= to && selection.to >= from;
  const lineTouched = (pos: number) => {
    const line = doc.lineAt(pos);
    return overlaps(line.from, line.to);
  };
  const lineSpanTouched = (from: number, to: number) => embedsRevealed && overlaps(doc.lineAt(from).from, doc.lineAt(to).to);

  tree.iterate({
    enter: (ref) => {
      const node = ref.node;
      const name = node.name;
      const parent = node.parent?.name;

      const level = HEADINGS[name];
      if (level !== undefined) {
        const line = doc.lineAt(node.from);
        ranges.push({ from: line.from, to: line.to, kind: "heading", level });
        return;
      }

      if (name === "HeaderMark") {
        // The heading's `#`s and the space after them, so the text starts at
        // the body's edge; or a Setext underline on its own line.
        const space = doc.sliceString(node.to, node.to + 1) === " " ? 1 : 0;
        if (!lineTouched(node.from)) ranges.push({ from: node.from, to: node.to + space, kind: "hide" });
        return;
      }

      if (name === "ListMark") {
        // A bullet list draws one bullet in place of `-`, `*` or `+`; an
        // ordered list keeps its number. The list is the grandparent: a
        // ListItem holds the mark and its own list holds the item.
        const list = node.parent?.parent?.name;
        if (list === "BulletList" && !lineTouched(node.from)) {
          ranges.push({ from: node.from, to: node.to, kind: "bullet" });
        }
        // The item's lead, from the line's start through the mark and the
        // spaces after it, for a text that hangs an item's wrapped lines under
        // its words (a read-only text); an item opened after a quote's `>` has none.
        const line = doc.lineAt(node.from);
        if (doc.sliceString(line.from, node.from).trim() === "" && !lineTouched(node.from)) {
          let to = node.to;
          while (to < line.to && to - node.to < 4 && doc.sliceString(to, to + 1) === " ") to += 1;
          let depth = -1;
          for (let up: SyntaxNode | null = node.parent; up; up = up.parent) if (up.name === "BulletList" || up.name === "OrderedList") depth += 1;
          ranges.push({ from: line.from, to, kind: "listItem", depth, source: list === "BulletList" ? "•" : doc.sliceString(node.from, node.to) });
        }
        return;
      }

      if (name === "TaskMarker") {
        // `[ ]` or `[x]` becomes a checkbox the operator can click, so it is
        // drawn even while the caret is on its line.
        const text = doc.sliceString(node.from, node.to).toLowerCase();
        ranges.push({ from: node.from, to: node.to, kind: "checkbox", checked: text.includes("x") });
        return;
      }

      if (name === "FencedCode" && parent === "Document") {
        const diagram = mermaidSource(node, doc);
        if (diagram !== null) {
          const first = doc.lineAt(node.from);
          const last = doc.lineAt(node.to);
          if (!(embedsRevealed && overlaps(first.from, last.to))) {
            ranges.push({ from: first.from, to: last.to, kind: "mermaid", source: diagram });
            return false;
          }
        }
        return;
      }

      if (name === "Table") {
        // A table inside a quote or a list item shares its lines with that
        // container's marks, so only a top-level table is drawn; it is one
        // unit like a diagram, revealed by a selection anywhere in it.
        if (parent === "Document") {
          const first = doc.lineAt(node.from);
          const last = doc.lineAt(node.to);
          if (!(embedsRevealed && overlaps(first.from, last.to))) {
            ranges.push({ from: first.from, to: last.to, kind: "table", source: doc.sliceString(node.from, node.to) });
            return false;
          }
        }
        return;
      }

      if (name === "CodeMark") {
        if (parent === "InlineCode") {
          if (!lineTouched(node.from)) ranges.push({ from: node.from, to: node.to, kind: "hide" });
          return;
        }
        // A fence is revealed by a selection anywhere in its block, which is
        // the unit a fenced code block is edited in. The opening fence of a
        // block with code draws as the block's head row; the closing one hides.
        const block = enclosing(node, "FencedCode");
        if (block && !overlaps(block.from, block.to)) {
          const line = doc.lineAt(node.from);
          const fenced = node.parent;
          if (fenced?.firstChild?.from === node.from && fenced.getChild("CodeText")) {
            const { code, language } = fencedCode(doc, fenced);
            ranges.push({ from: node.from, to: line.to, kind: "codeHead", language, source: code });
            return;
          }
          ranges.push({ from: node.from, to: node.to, kind: "hide" });
          ranges.push({ from: line.from, to: line.to, kind: "fenceLine" });
        }
        return;
      }

      if (name === "CodeInfo") {
        // The head row says the language in place of the opening fence's words.
        const block = enclosing(node, "FencedCode");
        if (block && !overlaps(block.from, block.to) && !node.parent?.getChild("CodeText")) {
          ranges.push({ from: node.from, to: node.to, kind: "hide" });
        }
        return;
      }

      if (name === "CodeText") {
        const block = enclosing(node, "FencedCode");
        if (block && !overlaps(block.from, block.to)) {
          const first = doc.lineAt(node.from).number;
          const last = doc.lineAt(node.to).number;
          for (let number = first; number <= last; number += 1) {
            const line = doc.line(number);
            const edge = first === last ? "only" : number === first ? "first" : number === last ? "last" : undefined;
            ranges.push({ from: line.from, to: line.to, kind: "codeBlock", ...(edge ? { edge } : {}) });
          }
        }
        return;
      }

      if (name === "InlineCode") {
        const inner = betweenMarks(node, "CodeMark");
        if (inner) ranges.push({ from: inner.from, to: inner.to, kind: "codeInline" });
        return;
      }

      if (name === "EmphasisMark" || name === "StrikethroughMark") {
        if (!lineTouched(node.from)) ranges.push({ from: node.from, to: node.to, kind: "hide" });
        return;
      }

      if (name === "QuoteMark") {
        const quote = enclosing(node, "Blockquote");
        if (!quote || !overlaps(quote.from, quote.to)) {
          const line = doc.lineAt(node.from);
          ranges.push({ from: node.from, to: node.to, kind: "hide" });
          ranges.push({ from: line.from, to: line.to, kind: "quote" });
        }
        return;
      }

      if (name === "HTMLBlock") {
        // A block inside a quote or a list item shares its lines with that
        // container's marks, so only a top-level block is drawn.
        if (parent === "Document") {
          const first = doc.lineAt(node.from);
          const last = doc.lineAt(node.to);
          if (doc.sliceString(first.from, node.from).trim() === "" && !(embedsRevealed && overlaps(first.from, last.to))) {
            ranges.push({ from: first.from, to: last.to, kind: "html", block: true, html: doc.sliceString(node.from, node.to) });
          }
        }
        return false;
      }

      if (name === "HTMLTag") {
        const tag = doc.sliceString(node.from, node.to);
        if (/^<img[\s/>]/i.test(tag) && !lineSpanTouched(node.from, node.to)) {
          ranges.push({ from: node.from, to: node.to, kind: "html", block: false, html: tag });
        }
        return;
      }

      if (name === "Image") {
        if (!lineSpanTouched(node.from, node.to)) {
          const image = imageParts(node, doc);
          if (image) ranges.push({ from: node.from, to: node.to, kind: "image", ...image });
        }
        return false;
      }

      if (name === "Link") {
        if (!overlaps(node.from, node.to)) {
          for (let child = node.firstChild; child; child = child.nextSibling) {
            if (child.name === "LinkMark" || child.name === "URL" || child.name === "LinkTitle") {
              ranges.push({ from: child.from, to: child.to, kind: "hide" });
            }
          }
          const text = linkText(node);
          if (text) ranges.push({ from: text.from, to: text.to, kind: "link" });
        }
        return;
      }
    },
  });

  ranges.sort((left, right) => left.from - right.from || left.to - right.to);
  return ranges;
}

function enclosing(node: SyntaxNode, name: string): { from: number; to: number } | null {
  let current: SyntaxNode | null = node;
  while (current) {
    if (current.name === name) return { from: current.from, to: current.to };
    current = current.parent;
  }
  return null;
}

/** A fenced block's diagram when its language is `mermaid` and it holds text, else null. */
/**
 * A fenced block's code as it is copied, and the language its fence names.
 * The lines are those between the fences (to the text's end when the block
 * is never closed), each without the container prefix the opening fence's
 * column holds (a quote's `>`, a list item's indent), so a block in a quote
 * or a list copies as its code alone.
 */
export function fencedCode(doc: Text, node: SyntaxNode): { code: string; language: string | null } {
  const open = doc.lineAt(node.from);
  const column = node.from - open.from;
  const marks = node.getChildren("CodeMark");
  const close = marks.length > 1 ? doc.lineAt(marks[marks.length - 1]!.from) : null;
  const last = close && close.number > open.number ? close.number - 1 : doc.lineAt(node.to).number;
  const lines: string[] = [];
  for (let number = open.number + 1; number <= last; number += 1) {
    const text = doc.line(number).text;
    const prefix = /^[ \t>]*/.exec(text)?.[0].length ?? 0;
    lines.push(text.slice(Math.min(column, prefix)));
  }
  const info = node.getChild("CodeInfo");
  return { code: lines.join("\n"), language: info ? fenceLanguage(doc.sliceString(info.from, info.to)) : null };
}

/** The language a fence's info words name, its first word, or null when they name none. */
export function fenceLanguage(info: string): string | null {
  return info.trim().split(/\s+/)[0] || null;
}

function mermaidSource(node: SyntaxNode, doc: Text): string | null {
  const info = node.getChild("CodeInfo");
  const lines = node.getChildren("CodeText");
  const first = lines[0];
  const last = lines.at(-1);
  if (!info || !first || !last || !isMermaidInfo(doc.sliceString(info.from, info.to))) return null;
  const source = doc.sliceString(first.from, last.to);
  return source.trim() === "" ? null : source;
}

/** A Markdown image's address, alt text and title, or null when it has no address. */
function imageParts(node: SyntaxNode, doc: Text): { src: string; alt: string; title?: string } | null {
  const marks: SyntaxNode[] = [];
  let url: SyntaxNode | null = null;
  let title: SyntaxNode | null = null;
  for (let child = node.firstChild; child; child = child.nextSibling) {
    if (child.name === "LinkMark") marks.push(child);
    else if (child.name === "URL") url = child;
    else if (child.name === "LinkTitle") title = child;
  }
  const open = marks[0];
  const close = marks[1];
  if (!url || !open || !close || close.from < open.to) return null;
  const strip = (text: string) => (text.startsWith("<") && text.endsWith(">") ? text.slice(1, -1) : text);
  const quoted = title ? doc.sliceString(title.from, title.to).slice(1, -1) : undefined;
  return {
    src: strip(doc.sliceString(url.from, url.to)),
    alt: doc.sliceString(open.to, close.from),
    ...(quoted ? { title: quoted } : {}),
  };
}

/** The link's visible text: the range between `[` and `]`, the first two of
 * the four link marks. */
function linkText(node: SyntaxNode): { from: number; to: number } | null {
  const marks: SyntaxNode[] = [];
  for (let child = node.firstChild; child; child = child.nextSibling) {
    if (child.name === "LinkMark") marks.push(child);
  }
  const open = marks[0];
  const close = marks[1];
  if (!open || !close || close.from <= open.to) return null;
  return { from: open.to, to: close.from };
}

/** The range between a node's first and last `mark` child, if it has both. */
function betweenMarks(node: SyntaxNode, mark: string): { from: number; to: number } | null {
  const marks: SyntaxNode[] = [];
  for (let child = node.firstChild; child; child = child.nextSibling) {
    if (child.name === mark) marks.push(child);
  }
  const open = marks[0];
  const close = marks[marks.length - 1];
  if (!open || !close || close.from <= open.to) return null;
  return { from: open.to, to: close.from };
}
