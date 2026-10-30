// A document's leading YAML frontmatter, found by text alone so the rule is
// testable without a browser. Live mode draws the front half in its own pane
// (PRD S3 D-11); source mode and every other document kind keep one buffer.

export type Frontmatter = {
  /** The block, delimiters included, ending in a newline. */
  front: string;
  /** Everything after it; the body keeps the document's remaining lines. */
  body: string;
};

/**
 * The frontmatter at the very top of `contents`, or null when there is none.
 *
 * Only a first line that is exactly `---` opens a block and only a later
 * `---` line closes it, which is the rule YAML frontmatter is written with;
 * an unclosed block is body text, because there is no block to draw.
 */
export function splitFrontmatter(contents: string): Frontmatter | null {
  const lines = contents.split("\n");
  if ((lines[0] ?? "").trimEnd() !== "---") return null;
  for (let index = 1; index < lines.length; index += 1) {
    if ((lines[index] ?? "").trimEnd() !== "---") continue;
    const front = `${lines.slice(0, index + 1).join("\n")}\n`;
    return { front, body: contents.slice(front.length) };
  }
  return null;
}

/** How a property's value reads: a one-word value is a chip, a path stays monospaced, anything else is text. */
export type PropertyLook = "chip" | "path" | "text";

/** One mark of the Live properties list, by offset into the frontmatter block. */
export type PropertyMark =
  /** A `---` line, which the list does not draw. */
  | { kind: "delimiter"; from: number }
  /** A `key: value` line. */
  | { kind: "property"; from: number }
  /** A line under a property (a list item, a folded string), indented to the values. */
  | { kind: "more"; from: number }
  /** A line the selection is on: its source, as typed. */
  | { kind: "source"; from: number }
  | { kind: "key"; from: number; to: number }
  /** The `: ` after a key and the quotes around a value. */
  | { kind: "hide"; from: number; to: number }
  | { kind: "value"; from: number; to: number; look: PropertyLook };

const PROPERTY = /^([A-Za-z0-9_][\w.-]*):( +|$)(.*)$/;
const CHIP = /^[\w.+-]{1,32}$/;

function lookOf(value: string): PropertyLook {
  if (!/\s/.test(value) && value.includes("/")) return "path";
  return CHIP.test(value) ? "chip" : "text";
}

/**
 * The Live drawing of a frontmatter block (`front` as `splitFrontmatter`
 * returns it) as a list of properties: the delimiters hidden, each key in a
 * muted column, its value without the colon and the quotes. A line for which
 * `revealed(index)` is true (0-based) keeps its source, so the caret edits
 * what the file holds. The text itself is never changed.
 */
export function frontmatterPlan(front: string, revealed: (line: number) => boolean): PropertyMark[] {
  const marks: PropertyMark[] = [];
  const lines = front.split("\n");
  const last = lines.length - (lines[lines.length - 1] === "" ? 2 : 1);
  let from = 0;
  lines.forEach((line, index) => {
    const start = from;
    from += line.length + 1;
    if (index > last) return;
    if (revealed(index)) {
      marks.push({ kind: "source", from: start });
      return;
    }
    if (index === 0 || index === last) {
      marks.push({ kind: "delimiter", from: start });
      return;
    }
    const match = PROPERTY.exec(line);
    if (!match) {
      marks.push({ kind: "more", from: start });
      return;
    }
    const [, key = "", gap = "", raw = ""] = match;
    marks.push({ kind: "property", from: start });
    const keyEnd = start + key.length;
    marks.push({ kind: "key", from: start, to: keyEnd });
    marks.push({ kind: "hide", from: keyEnd, to: keyEnd + 1 + gap.length });
    let valueFrom = keyEnd + 1 + gap.length;
    let value = raw.trimEnd();
    const quote = value[0];
    if (value.length >= 2 && (quote === '"' || quote === "'") && value.endsWith(quote) && !value.slice(1, -1).includes(quote)) {
      marks.push({ kind: "hide", from: valueFrom, to: valueFrom + 1 });
      marks.push({ kind: "hide", from: valueFrom + value.length - 1, to: valueFrom + value.length });
      valueFrom += 1;
      value = value.slice(1, -1);
    }
    if (value) marks.push({ kind: "value", from: valueFrom, to: valueFrom + value.length, look: lookOf(value) });
  });
  return marks;
}
