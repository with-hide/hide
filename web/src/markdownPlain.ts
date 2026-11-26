// A Markdown text as the words it shows, for a line-clamped preview that has
// no room for the editor's Live view (PRD overview-lenses-issues B8): the
// same parser the Markdown document uses drops the marks, and each line is
// trimmed with the blank ones gone. And the one block of a Markdown text that
// holds a sentence, which the thread's question line shows (Doop N 2).

import { GFM, parser } from "@lezer/markdown";

const markdown = parser.configure(GFM);

/** The syntax that is punctuation, not words. */
const MARKS = new Set(["HeaderMark", "EmphasisMark", "ListMark", "QuoteMark", "CodeMark", "CodeInfo", "LinkMark", "URL", "TaskMarker", "StrikethroughMark"]);

export function markdownPlainText(text: string): string {
  let plain = "";
  let at = 0;
  markdown.parse(text).iterate({
    enter: (node) => {
      if (!MARKS.has(node.name)) return;
      plain += text.slice(at, node.from);
      at = node.to;
      return false;
    },
  });
  plain += text.slice(at);
  return plain
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean)
    .join("\n");
}

// The marks Hide AI's quote check drops before it looks for a question's
// sentence in the answer: emphasis, code and quotation marks a model may add
// or leave out around a copied sentence. The same reading as `quote_form` in
// hide-core/src/labels/context_label.rs; change the two together, or a
// sentence the core kept is not found here and the whole answer shows.
const QUOTE_MARKS = new Set(["*", "_", "`", '"', "'", "\u201C", "\u201D", "\u2018", "\u2019"]);

/** `text` as the quote check reads it, with where each of its characters stands in `text`. */
function quoteForm(text: string): { form: string; at: number[] } {
  let form = "";
  const at: number[] = [];
  for (let index = 0; index < text.length; index += 1) {
    const char = text[index]!;
    if (QUOTE_MARKS.has(char)) continue;
    if (/\s/.test(char)) {
      if (form !== "" && !form.endsWith(" ")) {
        form += " ";
        at.push(index);
      }
      continue;
    }
    form += char;
    at.push(index);
  }
  return form.endsWith(" ") ? { form: form.slice(0, -1), at: at.slice(0, -1) } : { form, at };
}

/**
 * The block of the Markdown `text` that holds the sentence `quote`, as Hide
 * AI's quote check finds it: the list item it stands in, else the paragraph
 * or other block between blank lines, as Markdown reads them. Null when the
 * sentence is not there or runs across blocks, so the caller shows the whole
 * text rather than a cut it guessed.
 */
export function markdownBlockHolding(text: string, quote: string): string | null {
  const sentence = quoteForm(quote).form;
  if (sentence === "") return null;
  const { form, at } = quoteForm(text);
  const found = form.indexOf(sentence);
  if (found < 0) return null;
  const from = at[found]!;
  const to = at[found + sentence.length - 1]! + 1;
  let held = null as { from: number; to: number } | null;
  markdown.parse(text).iterate({
    enter: (node) => {
      if (node.name === "Document") return undefined;
      if (node.from > from || node.to < to) return false;
      const block = node.name === "ListItem" || node.node.parent?.name === "Document";
      if (block && (!held || node.to - node.from < held.to - held.from)) held = { from: node.from, to: node.to };
      return undefined;
    },
  });
  return held ? text.slice(held.from, held.to) : null;
}
