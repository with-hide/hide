// An agent's message on the phone drawn as Markdown: parsed once per message
// with the grammar the editor's Markdown view uses (@lezer/markdown with GFM)
// and drawn as plain elements, never an editor, so a page of messages costs a
// few milliseconds. Nothing on the page navigates: a link is its text and an
// image its description, as the terminal rows keep OSC 8 links as text. Code
// wraps at the phone's width like everything else, without highlighting.
//
// A pull request's body is drawn the same way with `html` on (PRD
// record-card-readability D-11, B19, B20): the HTML GitHub draws in a body
// (folded details, pictures, line breaks, emphasis, code, keys, sub and
// superscript, links to http and https) becomes those elements, and so do
// Markdown's own links and pictures. Each tag is read on its own against an
// allow-list and built as a React element, never inserted as markup; a tag
// that is not on the list, or carries an attribute or an address that is not,
// stays as the text it was, and an HTML comment is not drawn.

import type { SyntaxNode } from "@lezer/common";
import { GFM, parser } from "@lezer/markdown";
import { useState, type ReactNode } from "react";

const markdown = parser.configure(GFM);

/** The syntax the drawing stands in for: `**`, `#`, `-`, `>`, backticks, brackets, pipes, a fence's language. */
const MARKS = new Set(["EmphasisMark", "CodeMark", "HeaderMark", "ListMark", "QuoteMark", "LinkMark", "StrikethroughMark", "TableDelimiter", "CodeInfo"]);

/** What a link or an image carries besides its text. */
const TARGETS = new Set(["URL", "LinkTitle", "LinkLabel"]);

const HEADINGS = /^(ATXHeading[1-6]|SetextHeading[12])$/;

/** One parse: the source and whether its HTML is drawn. */
type Source = { text: string; html: boolean };

export function renderMarkdown(text: string, options: { html?: boolean } = {}): ReactNode {
  return blocks(markdown.parse(text).topNode, { text, html: options.html === true });
}

function childrenOf(node: SyntaxNode): SyntaxNode[] {
  const children: SyntaxNode[] = [];
  for (let child = node.firstChild; child; child = child.nextSibling) children.push(child);
  return children;
}

/** A container's blocks; the whitespace between them is layout, not text. An HTML block opens or closes an element around the blocks that follow it. */
function blocks(node: SyntaxNode, source: Source): ReactNode[] {
  if (!source.html) return childrenOf(node).map((child, index) => block(child, source, index));
  const tree = new HtmlTree();
  childrenOf(node).forEach((child) => {
    if (child.name === "HTMLBlock" || child.name === "CommentBlock") tree.html(source.text.slice(child.from, child.to), true);
    else tree.add(block(child, source, tree.key()));
  });
  return tree.done();
}

function block(node: SyntaxNode, source: Source, key: number): ReactNode {
  const name = node.name;
  const { text } = source;
  if (MARKS.has(name)) return null;
  if (HEADINGS.test(name)) {
    // The PR card's heading chips find a `##` by this mark (PRD pr-card-and-session-record-redesign B12).
    return (
      <p key={key} className="whitespace-pre-line font-semibold" data-markdown-heading={name.slice(-1)}>
        {trimmed(inline(node, source))}
      </p>
    );
  }
  switch (name) {
    case "BulletList":
      return (
        <ul key={key} className="list-disc space-y-xs pl-lg">
          {blocks(node, source)}
        </ul>
      );
    case "OrderedList": {
      const mark = node.getChild("ListItem")?.getChild("ListMark");
      const start = mark ? Number.parseInt(text.slice(mark.from, mark.to), 10) : 1;
      return (
        <ol key={key} start={Number.isFinite(start) ? start : 1} className="list-decimal space-y-xs pl-lg">
          {blocks(node, source)}
        </ol>
      );
    }
    case "ListItem":
      return (
        <li key={key} className="space-y-xs">
          {blocks(node, source)}
        </li>
      );
    case "Blockquote":
      return (
        <blockquote key={key} className="space-y-xs border-l-2 border-border pl-md text-muted-foreground">
          {blocks(node, source)}
        </blockquote>
      );
    case "FencedCode":
    case "CodeBlock":
      return (
        <pre key={key} className="m-none whitespace-pre-wrap wrap-anywhere rounded-md bg-secondary px-md py-sm font-mono">
          {codeOf(node, text)}
        </pre>
      );
    case "HorizontalRule":
      return <hr key={key} className="border-border" />;
    case "Table":
      return (
        <table key={key} className="w-full border-collapse">
          <tbody>{childrenOf(node).map((row, index) => tableRow(row, source, index))}</tbody>
        </table>
      );
    default:
      // A paragraph, a task, and anything the grammar knows that has no drawing of its own (raw HTML, a link reference).
      return (
        <p key={key} className="whitespace-pre-line">
          {inline(node, source)}
        </p>
      );
  }
}

function tableRow(row: SyntaxNode, source: Source, key: number): ReactNode {
  if (row.name !== "TableHeader" && row.name !== "TableRow") return null;
  const Cell = row.name === "TableHeader" ? "th" : "td";
  return (
    <tr key={key}>
      {row.getChildren("TableCell").map((cell, index) => (
        <Cell key={index} className={`border border-border px-sm py-xs text-left align-top wrap-anywhere ${Cell === "th" ? "font-semibold" : ""}`}>
          {inline(cell, source)}
        </Cell>
      ))}
    </tr>
  );
}

/** The code between a fence's marks, or an indented block's lines. */
function codeOf(node: SyntaxNode, text: string): string {
  const lines = node.getChildren("CodeText");
  const first = lines[0];
  const last = lines.at(-1);
  if (!first || !last) return "";
  const code = text.slice(first.from, last.to);
  return node.name === "CodeBlock" ? code.replace(/^ {1,4}/gm, "") : code;
}

/** A run of inline text: what lies between the child nodes is the text itself. With HTML drawn, an inline tag opens or closes an element around what follows it in the run. */
function inline(node: SyntaxNode, source: Source, skip: ReadonlySet<string> = MARKS): ReactNode[] {
  const { text } = source;
  const tree = source.html ? new HtmlTree() : null;
  const out: ReactNode[] = [];
  const add = (part: ReactNode) => (tree ? tree.add(part) : out.push(part));
  let at = node.from;
  for (const child of childrenOf(node)) {
    if (child.from > at) add(text.slice(at, child.from));
    if (tree && (child.name === "HTMLTag" || child.name === "Comment")) tree.html(text.slice(child.from, child.to), false);
    else if (!skip.has(child.name) && !MARKS.has(child.name)) add(span(child, source, tree ? tree.key() : out.length));
    at = child.to;
  }
  if (node.to > at) add(text.slice(at, node.to));
  return tree ? tree.done() : out;
}

const LINK_PARTS = new Set([...MARKS, ...TARGETS]);

function span(node: SyntaxNode, source: Source, key: number): ReactNode {
  const { text } = source;
  switch (node.name) {
    case "Emphasis":
      return <em key={key}>{inline(node, source)}</em>;
    case "StrongEmphasis":
      return (
        <strong key={key} className="font-semibold">
          {inline(node, source)}
        </strong>
      );
    case "Strikethrough":
      return <s key={key}>{inline(node, source)}</s>;
    case "InlineCode":
      return (
        <code key={key} className="rounded-xs bg-secondary px-xxs font-mono">
          {inline(node, source)}
        </code>
      );
    case "Link":
    case "Image": {
      const url = node.getChild("URL");
      const href = source.html && url ? (node.name === "Image" ? imageAddress : webAddress)(text.slice(url.from, url.to)) : null;
      const words = inline(node, source, LINK_PARTS);
      if (href && node.name === "Image") return <Picture key={key} src={href} alt={words.map((part) => (typeof part === "string" ? part : "")).join("")} />;
      if (href) return <WebLink key={key} href={href}>{words}</WebLink>;
      return <span key={key}>{words}</span>;
    }
    case "Autolink":
      return <span key={key}>{inline(node, source)}</span>;
    case "HardBreak":
      return <br key={key} />;
    case "Escape":
      return text.slice(node.from + 1, node.to);
    default:
      // A bare URL, an HTML tag, an entity, a task's box: as written.
      return text.slice(node.from, node.to);
  }
}

/** A heading without the space after its `#` or the line before its underline. */
function trimmed(parts: ReactNode[]): ReactNode[] {
  const first = parts[0];
  if (typeof first === "string") parts[0] = first.trimStart();
  const end = parts.length - 1;
  const last = parts[end];
  if (typeof last === "string") parts[end] = last.trimEnd();
  return parts;
}

// --- the HTML a pull request body may draw ---------------------------------

/** The tags drawn, each with the attributes it may carry; anything else stays as its text. */
const ALLOWED: Record<string, readonly string[]> = {
  details: ["open"],
  summary: [],
  img: ["src", "alt", "width", "height", "title"],
  br: [],
  p: [],
  b: [],
  strong: [],
  i: [],
  em: [],
  code: [],
  kbd: [],
  sub: [],
  sup: [],
  a: ["href", "title"],
};
const VOID = new Set(["img", "br"]);
const ENTITIES: Record<string, string> = { amp: "&", lt: "<", gt: ">", quot: '"', apos: "'", "#39": "'" };

type Tag = { name: string; close: boolean; attrs: Record<string, string> };

/** The most elements open at once; a deeper opening tag stays as its text, so a body cannot nest without bound. */
const MAX_OPEN = 32;

function parsedAddress(raw: string): URL | null {
  try {
    return new URL(raw.trim());
  } catch {
    return null;
  }
}

/** The page an address names, when it is an http or https one; never a script, a file or a relative path. */
function webAddress(raw: string): string | null {
  const url = parsedAddress(raw);
  return url && (url.protocol === "http:" || url.protocol === "https:") ? url.href : null;
}

/**
 * A picture's address, which the card loads by itself as it opens: https on a
 * public host only, so a body cannot make the operator's machine ask this
 * computer or its network for anything. The parser has already turned a
 * number-spelled address into dotted form. A public name that resolves to a
 * private address is past what a name can tell.
 */
function imageAddress(raw: string): string | null {
  const url = parsedAddress(raw);
  return url && url.protocol === "https:" && !privateHost(url.hostname) ? url.href : null;
}

function privateHost(hostname: string): boolean {
  // A trailing dot names the same host (`localhost.`), so it is read without one.
  const name = hostname.replace(/\.+$/, "").replace(/^\[|\]$/g, "").toLowerCase();
  if (name === "localhost" || /\.(localhost|local|internal|lan|home)$/.test(name)) return true;
  if (name.includes(":")) return name === "::" || name === "::1" || /^(fc|fd|fe8|fe9|fea|feb)/.test(name) || name.startsWith("::ffff:");
  const v4 = /^(\d+)\.(\d+)\.\d+\.\d+$/.exec(name);
  if (!v4) return !name.includes(".");
  const a = Number(v4[1]);
  const b = Number(v4[2]);
  return a === 0 || a === 10 || a === 127 || (a === 100 && b >= 64 && b <= 127) || (a === 169 && b === 254) || (a === 172 && b >= 16 && b <= 31) || (a === 192 && b === 168);
}

/** One tag read against the allow-list, or null when it stays text: an unknown tag, an attribute it may not carry, an address that is not the web's. */
function readTag(raw: string): Tag | null {
  const match = /^<\s*(\/)?\s*([a-zA-Z][a-zA-Z0-9]*)([^>]*?)\s*\/?\s*>$/.exec(raw);
  if (!match) return null;
  const name = match[2]!.toLowerCase();
  const allowed = Object.hasOwn(ALLOWED, name) ? ALLOWED[name] : undefined;
  if (!allowed) return null;
  const close = match[1] === "/";
  const attrs: Record<string, string> = {};
  const rest = match[3] ?? "";
  if (close) return rest.trim() ? null : { name, close, attrs };
  const pattern = /\s*([^\s"'>/=]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>`]+)))?/gy;
  let read = 0;
  for (let found = pattern.exec(rest); found && found[0]; found = pattern.exec(rest)) {
    read = pattern.lastIndex;
    const attr = found[1]!.toLowerCase();
    if (!allowed.includes(attr)) return null;
    const value = (found[2] ?? found[3] ?? found[4] ?? "").replace(/&(amp|lt|gt|quot|apos|#39);/g, (_, entity: string) => ENTITIES[entity]!);
    if (attr === "src" || attr === "href") {
      const address = (attr === "src" ? imageAddress : webAddress)(value);
      if (!address) return null;
      attrs[attr] = address;
    } else if (attr === "width" || attr === "height") {
      if (!/^\d{1,4}$/.test(value)) return null;
      attrs[attr] = value;
    } else attrs[attr] = value;
  }
  if (rest.slice(read).trim()) return null;
  if (name === "img" && !attrs.src) return null;
  if (name === "a" && !attrs.href) return null;
  return { name, close, attrs };
}

/** An HTML run's pieces: comments, tags and the text between them. */
function htmlPieces(raw: string): string[] {
  return raw.split(/(<!--[\s\S]*?(?:-->|$)|<\/?[a-zA-Z][^<>]*>)/).filter((piece) => piece !== "");
}

type Frame = { tag: Tag | null; children: ReactNode[] };

/**
 * Builds elements from a run where tags open and close around other parts:
 * an opening tag starts a frame, its closing tag ends the nearest frame of the
 * same name (and any left open inside it), and whatever is still open at the
 * end closes there. A closing tag with nothing to close stays as its text.
 */
class HtmlTree {
  private stack: Frame[] = [{ tag: null, children: [] }];
  private next = 0;

  key(): number {
    this.next += 1;
    return this.next;
  }

  add(part: ReactNode): void {
    if (part === null || part === "") return;
    this.stack.at(-1)!.children.push(part);
  }

  /** A run of raw HTML: from an HTML block (`block`, whose text between tags is layout when blank) or one inline tag. */
  html(raw: string, block: boolean): void {
    for (const piece of htmlPieces(raw)) {
      if (piece.startsWith("<!--")) continue;
      const tag = piece.startsWith("<") ? readTag(piece) : null;
      if (!tag) {
        // An HTML block's text is not Markdown: its line breaks and indents are layout, as GitHub draws them.
        const words = block && !piece.startsWith("<") ? piece.replace(/\s+/g, " ") : piece;
        if (words.trim()) this.add(words);
        continue;
      }
      if (VOID.has(tag.name)) {
        if (!tag.close) this.add(element(tag, [], this.key()));
        continue;
      }
      if (!tag.close) {
        if (this.stack.length > MAX_OPEN) this.add(piece);
        else this.stack.push({ tag, children: [] });
        continue;
      }
      let at = this.stack.length - 1;
      while (at > 0 && this.stack[at]!.tag?.name !== tag.name) at -= 1;
      if (at < 1) this.add(piece);
      else while (this.stack.length > at) this.pop();
    }
  }

  done(): ReactNode[] {
    while (this.stack.length > 1) this.pop();
    return this.stack[0]!.children;
  }

  private pop(): void {
    const frame = this.stack.pop()!;
    this.add(element(frame.tag!, frame.children, this.key()));
  }
}

function element(tag: Tag, children: ReactNode[], key: number): ReactNode {
  switch (tag.name) {
    case "details":
      return <details key={key} open={"open" in tag.attrs} className="space-y-sm rounded-md border border-border px-md py-xs" data-markdown-html="details">{children}</details>;
    case "summary":
      return <summary key={key} className="cursor-pointer font-medium" data-markdown-html="summary">{children}</summary>;
    case "img":
      return <Picture key={key} src={tag.attrs.src!} alt={tag.attrs.alt ?? ""} title={tag.attrs.title} width={tag.attrs.width} height={tag.attrs.height} />;
    case "br":
      return <br key={key} />;
    case "p":
      return <p key={key} className="whitespace-pre-line">{children}</p>;
    case "b":
    case "strong":
      return <strong key={key} className="font-semibold">{children}</strong>;
    case "i":
    case "em":
      return <em key={key}>{children}</em>;
    case "code":
      return <code key={key} className="rounded-xs bg-secondary px-xxs font-mono">{children}</code>;
    case "kbd":
      return <kbd key={key} className="rounded-xs border border-border bg-secondary px-xxs font-mono text-micro">{children}</kbd>;
    case "sub":
      return <sub key={key}>{children}</sub>;
    case "sup":
      return <sup key={key}>{children}</sup>;
    case "a":
      return <WebLink key={key} href={tag.attrs.href!} title={tag.attrs.title}>{children}</WebLink>;
    default:
      return null;
  }
}

/** A link that leaves for the operator's browser: the desktop host and the browser both open a new window's address outside the shell. */
function WebLink({ href, title, children }: { href: string; title?: string; children: ReactNode }) {
  return <a href={href} title={title} target="_blank" rel="noopener noreferrer" className="text-primary underline-offset-2 hover:underline" data-markdown-html="a">{children}</a>;
}

/** A picture, which reads its description where it cannot be read (B19). */
function Picture({ src, alt, title, width, height }: { src: string; alt: string; title?: string; width?: string; height?: string }) {
  const [failed, setFailed] = useState(false);
  if (failed) return <span className="text-muted-foreground" data-markdown-html="img-alt">{alt}</span>;
  return <img src={src} alt={alt} title={title} width={width} height={height} loading="lazy" referrerPolicy="no-referrer" onError={() => setFailed(true)} className="inline max-w-full rounded-xs" data-markdown-html="img" />;
}
