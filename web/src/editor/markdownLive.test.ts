import { GFM, parser } from "@lezer/markdown";
import { Text } from "@codemirror/state";
import { describe, expect, it } from "vitest";
import { markdownLivePlan, MARKDOWN_LIVE_BYTE_LIMIT, type LiveRange } from "./markdownLive";

const markdown = parser.configure([GFM]);

function plan(source: string, selection = 0): LiveRange[] {
  const doc = Text.of(source.split("\n"));
  const tree = markdown.parse(source);
  return markdownLivePlan(tree, doc, { from: selection, to: selection });
}

/** The ranges as `kind:slice` strings, which is what a reader can check. */
function shown(source: string, selection = 0): string[] {
  const doc = Text.of(source.split("\n"));
  return plan(source, selection).map((range) => `${range.kind}:${doc.sliceString(range.from, range.to)}`);
}

// The caret sits on the first line in every case that wants markup hidden, so
// the block under test is elsewhere and its own line is not the touched one.
const AWAY = "intro\n\n";

describe("markdown live plan", () => {
  it("styles a heading line and hides its hashes", () => {
    expect(shown(`${AWAY}# Title`)).toEqual(["hide:# ", "heading:# Title"]);
  });

  it("reveals the heading's own markup while the caret is on its line", () => {
    expect(shown("# Title", 3)).toEqual(["heading:# Title"]);
  });

  it("draws a bullet in place of the list marker", () => {
    expect(shown(`${AWAY}- one\n- two`)).toEqual(["bullet:-", "bullet:-"]);
  });

  it("leaves an ordered list's number alone", () => {
    expect(shown(`${AWAY}1. one`)).toEqual([]);
  });

  it("draws a checkbox for a task item in either state", () => {
    const ranges = plan(`${AWAY}- [ ] open\n- [x] done`);
    expect(ranges.filter((range) => range.kind === "checkbox").map((range) => range.checked)).toEqual([false, true]);
  });

  it("hides emphasis, code and link markup while the caret is elsewhere", () => {
    expect(shown(`${AWAY}a **bold** b`)).toEqual(["hide:**", "hide:**"]);
    expect(shown(`${AWAY}a \`code\` b`)).toEqual(["hide:`", "codeInline:code", "hide:`"]);
    expect(shown(`${AWAY}see [text](https://example.com) now`)).toEqual([
      "hide:[",
      "link:text",
      "hide:]",
      "hide:(",
      "hide:https://example.com",
      "hide:)",
    ]);
  });

  it("collapses a fenced code block to its body and styles those lines", () => {
    expect(shown(`${AWAY}\`\`\`rust\nfn main() {}\n\`\`\`\n`)).toEqual([
      "hide:```",
      "fenceLine:```rust",
      "hide:rust",
      "codeBlock:fn main() {}",
      "hide:```",
      "fenceLine:```",
    ]);
  });

  it("reveals the whole fenced block while the caret is inside it", () => {
    expect(shown("```rust\nfn main() {}\n```\n", 12)).toEqual([]);
  });

  it("hides a quote prefix and marks the quoted line", () => {
    expect(shown(`${AWAY}> quoted`)).toEqual(["hide:>", "quote:> quoted"]);
  });

  it("reads the live bound from the same number the editor caps at", () => {
    expect(MARKDOWN_LIVE_BYTE_LIMIT).toBe(256 * 1024);
  });
});

describe("markdown live plan: HTML and images", () => {
  const html = (source: string, selection = 0) =>
    plan(source, selection)
      .filter((range) => range.kind === "html" || range.kind === "image")
      .map((range) => ({ kind: range.kind, block: range.block, text: range.html ?? range.src }));

  it("draws a top-level HTML block over its whole lines", () => {
    expect(html(`${AWAY}<p align="center">\n  <img src="a.png">\n</p>\n`)).toEqual([
      { kind: "html", block: true, text: '<p align="center">\n  <img src="a.png">\n</p>' },
    ]);
  });

  it("shows the block's source while the selection is anywhere in it, edges included", () => {
    const source = `intro\n\n<p>\nhi\n</p>\n\nafter`;
    expect(html(source, source.indexOf("hi"))).toEqual([]);
    expect(html(source, source.indexOf("<p>"))).toEqual([]);
    expect(html(source, source.indexOf("</p>") + 4)).toEqual([]);
    expect(html(source, 0)).toHaveLength(1);
  });

  it("leaves a block inside a quote or a list item as source", () => {
    expect(html(`${AWAY}> <p>a</p>`)).toEqual([]);
    expect(html(`${AWAY}- item\n\n  <p>a</p>`)).toEqual([]);
  });

  it("draws a Markdown image with its address, alt text and title", () => {
    expect(plan(`${AWAY}![the logo](docs/a.png "Logo")`).filter((range) => range.kind === "image")).toMatchObject([
      { src: "docs/a.png", alt: "the logo", title: "Logo" },
    ]);
    expect(plan(`${AWAY}![x](<my file.png>)`).filter((range) => range.kind === "image")).toMatchObject([{ src: "my file.png", alt: "x" }]);
  });

  it("reveals an image's source while the caret is on its line", () => {
    const source = "intro ![a](b.png) end\n\nother";
    expect(html(source, 3)).toEqual([]);
    expect(html(source, source.indexOf("other"))).toEqual([{ kind: "image", block: undefined, text: "b.png" }]);
  });

  it("draws an inline <img> tag and leaves other inline HTML as source", () => {
    expect(html(`${AWAY}a <img src="x.png" width="20"> b <kbd>k</kbd> c<br>d`)).toEqual([
      { kind: "html", block: false, text: '<img src="x.png" width="20">' },
    ]);
  });

  it("draws an image inside a link without touching the link's own marks", () => {
    const ranges = shown(`${AWAY}[![badge](b.svg)](https://x.dev)`);
    expect(ranges).toContain("image:![badge](b.svg)");
    expect(ranges).toContain("hide:https://x.dev");
  });

  it("never reveals anything for a read-only text with no selection", () => {
    const source = "![a](b.png)\n\n<p>x</p>";
    const doc = Text.of(source.split("\n"));
    const nowhere = markdownLivePlan(markdown.parse(source), doc, { from: -1, to: -1 });
    expect(nowhere.map((range) => range.kind)).toEqual(["image", "html"]);
  });

  it("keeps a block and an image drawn under the caret until the operator has moved the caret, and still hides other markup there", () => {
    const source = "![a](b.png)\n\n<p>x</p>\n\n# Head";
    const doc = Text.of(source.split("\n"));
    const unfocused = markdownLivePlan(markdown.parse(source), doc, { from: 0, to: 0 }, false);
    expect(unfocused.map((range) => range.kind)).toEqual(["image", "html", "hide", "heading"]);
    const focused = markdownLivePlan(markdown.parse(source), doc, { from: 0, to: 0 }, true);
    expect(focused.map((range) => range.kind)).toEqual(["html", "hide", "heading"]);
  });
});
