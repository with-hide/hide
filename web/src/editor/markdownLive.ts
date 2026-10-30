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
// source.

import type { SyntaxNode, Tree } from "@lezer/common";
import type { Text } from "@codemirror/state";

export type LiveKind = "hide" | "bullet" | "checkbox" | "heading" | "codeBlock" | "fenceLine" | "quote" | "link" | "codeInline" | "html" | "image";

export type LiveRange = {
  from: number;
  to: number;
  kind: LiveKind;
  /** Heading level 1-6. */
  level?: number;
  /** Task checkbox state. */
  checked?: boolean;
  /** `html`: the source to draw, and whether it takes whole lines (an HTML block) or sits in a line (a tag). */
  html?: string;
  block?: boolean;
  /** `image`: a Markdown image's source address, alt text and title. */
  src?: string;
  alt?: string;
  title?: string;
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
        return;
      }

      if (name === "TaskMarker") {
        // `[ ]` or `[x]` becomes a checkbox the operator can click, so it is
        // drawn even while the caret is on its line.
        const text = doc.sliceString(node.from, node.to).toLowerCase();
        ranges.push({ from: node.from, to: node.to, kind: "checkbox", checked: text.includes("x") });
        return;
      }

      if (name === "CodeMark") {
        if (parent === "InlineCode") {
          if (!lineTouched(node.from)) ranges.push({ from: node.from, to: node.to, kind: "hide" });
          return;
        }
        // A fence is revealed by a selection anywhere in its block, which is
        // the unit a fenced code block is edited in.
        const block = enclosing(node, "FencedCode");
        if (block && !overlaps(block.from, block.to)) {
          const line = doc.lineAt(node.from);
          ranges.push({ from: node.from, to: node.to, kind: "hide" });
          ranges.push({ from: line.from, to: line.to, kind: "fenceLine" });
        }
        return;
      }

      if (name === "CodeInfo") {
        const block = enclosing(node, "FencedCode");
        if (block && !overlaps(block.from, block.to)) {
          ranges.push({ from: node.from, to: node.to, kind: "hide" });
        }
        return;
      }

      if (name === "CodeText") {
        const block = enclosing(node, "FencedCode");
        if (block && !overlaps(block.from, block.to)) {
          for (let pos = node.from; pos < node.to; ) {
            const line = doc.lineAt(pos);
            ranges.push({ from: line.from, to: line.to, kind: "codeBlock" });
            pos = line.to + 1;
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
