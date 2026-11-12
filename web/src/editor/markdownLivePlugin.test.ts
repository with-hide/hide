// @vitest-environment jsdom
import { markdown } from "@codemirror/lang-markdown";
import { ensureSyntaxTree } from "@codemirror/language";
import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { GFM } from "@lezer/markdown";
import { afterEach, describe, expect, it, vi } from "vitest";
import { markdownLive } from "./markdownLivePlugin";

const views: EditorView[] = [];
afterEach(() => views.splice(0).forEach((view) => view.destroy()));

function open(doc: string, options: Parameters<typeof markdownLive>[0] = {}): EditorView {
  const state = EditorState.create({ doc, extensions: [markdown({ extensions: [GFM] }), markdownLive(options)] });
  ensureSyntaxTree(state, doc.length, 5000);
  const view = new EditorView({ state, parent: document.body });
  views.push(view);
  // The tree the state field first sees may be partial; one empty transaction after parsing finishes lets it rebuild.
  view.dispatch({});
  return view;
}

const blocks = (view: EditorView) => Array.from(view.dom.querySelectorAll(".cm-md-html-block"));

describe("markdown live widgets", () => {
  const doc = "<p>first</p>\n\nmiddle\n\n<p>second</p>\n\n<div align=\"center\">\n\nend";

  it("draws a block, and keeps a block that cleans to nothing as source", () => {
    const view = open(doc);
    expect(blocks(view).map((block) => block.textContent)).toEqual(["first", "second"]);
    expect(view.dom.textContent).toContain('<div align="center">');
  });

  it("keeps the opening caret from revealing, then reveals the block the operator moves into", () => {
    const view = open(doc);
    expect(blocks(view)).toHaveLength(2);
    view.dispatch({ selection: { anchor: doc.indexOf("second") }, userEvent: "select.pointer" });
    expect(blocks(view).map((block) => block.textContent)).toEqual(["first"]);
    expect(view.dom.textContent).toContain("<p>second</p>");
  });

  it("does not reveal for a selection no operator made", () => {
    const view = open(doc);
    view.dispatch({ selection: { anchor: doc.indexOf("second") } });
    expect(blocks(view)).toHaveLength(2);
  });

  it("keeps a block's DOM while an edit lands outside it", () => {
    const view = open(doc);
    // The caret is at the end, outside both blocks, as it is when the operator types there.
    view.dispatch({ selection: { anchor: doc.length }, userEvent: "select.pointer" });
    const before = blocks(view);
    expect(before).toHaveLength(2);
    view.dispatch({ changes: { from: doc.indexOf("middle"), insert: "x" }, userEvent: "input.type" });
    const after = blocks(view);
    expect(after).toHaveLength(2);
    expect(after[0]).toBe(before[0]);
    expect(after[1]).toBe(before[1]);
  });

  it("draws the same for a read-only text, which never reveals", () => {
    const view = open(doc, { reveal: false });
    view.dispatch({ selection: { anchor: doc.indexOf("second") }, userEvent: "select.pointer" });
    expect(blocks(view)).toHaveLength(2);
  });
});

describe("markdown live tables", () => {
  const table = "| 장치 | 질문 |\n|:---|---:|\n| 70년 | 왜 [지금](https://example.com/now)? |";
  const doc = `intro\n\n${table}\n\nend`;
  const drawn = async (view: EditorView) => {
    await vi.waitFor(() => expect(view.dom.querySelector(".cm-md-table table")).not.toBeNull());
    return view.dom.querySelector<HTMLElement>(".cm-md-table")!;
  };

  it("draws a table as a header row and cells in place of its source", async () => {
    const view = open(doc);
    const block = await drawn(view);
    expect(Array.from(block.querySelectorAll("th")).map((cell) => cell.textContent)).toEqual(["장치", "질문"]);
    expect(Array.from(block.querySelectorAll("td")).map((cell) => cell.textContent)).toEqual(["70년", "왜 지금?"]);
    expect(view.contentDOM.textContent).not.toContain("|:---|");
  });

  it("shows the source when the operator presses the table, and draws it again when the caret leaves", async () => {
    const view = open(doc);
    const block = await drawn(view);
    block.querySelector("td")!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
    expect(view.state.selection.main.head).toBe(doc.indexOf("| 장치"));
    expect(view.dom.querySelector(".cm-md-table")).toBeNull();
    expect(view.contentDOM.textContent).toContain("|:---|---:|");
    view.dispatch({ selection: { anchor: doc.length }, userEvent: "select.pointer" });
    await drawn(view);
  });

  it("opens a link in a cell on modifier-click only, leaving the caret where it was", async () => {
    const opened: string[] = [];
    const view = open(doc, { files: { base: () => null, openUrl: (url) => opened.push(url), openPath: () => {} } });
    const link = (await drawn(view)).querySelector("a")!;
    link.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0, metaKey: true }));
    expect(opened).toEqual(["https://example.com/now"]);
    expect(view.state.selection.main.head).toBe(0);
    const click = new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 });
    link.dispatchEvent(click);
    expect(click.defaultPrevented).toBe(true);
  });

  it("keeps the table's DOM while an edit lands outside it", async () => {
    const view = open(doc);
    view.dispatch({ selection: { anchor: doc.length }, userEvent: "select.pointer" });
    const before = await drawn(view);
    view.dispatch({ changes: { from: doc.length, insert: "x" }, userEvent: "input.type" });
    expect(view.dom.querySelector(".cm-md-table")).toBe(before);
  });
});
