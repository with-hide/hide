import { describe, expect, it } from "vitest";
import { frontmatterPlan, splitFrontmatter } from "./frontmatter";

describe("splitFrontmatter", () => {
  it("finds the block at the top and leaves the rest to the body", () => {
    expect(splitFrontmatter("---\ntitle: one\n---\n# Body\n")).toEqual({
      front: "---\ntitle: one\n---\n",
      body: "# Body\n",
    });
  });

  it("answers null without a leading block", () => {
    expect(splitFrontmatter("# Body\n")).toBeNull();
    expect(splitFrontmatter("\n---\ntitle: one\n---\n")).toBeNull();
    expect(splitFrontmatter("---\ntitle: one\n")).toBeNull();
  });

  it("keeps an empty block and a bodyless document", () => {
    expect(splitFrontmatter("---\n---\n")).toEqual({ front: "---\n---\n", body: "" });
  });

  it("does not mistake a longer rule for a delimiter", () => {
    expect(splitFrontmatter("----\ntitle: one\n----\n")).toBeNull();
  });

  it("closes at the first delimiter after the opening line", () => {
    expect(splitFrontmatter("---\na: 1\n---\nb: 2\n---\n")).toEqual({
      front: "---\na: 1\n---\n",
      body: "b: 2\n---\n",
    });
  });
});

describe("frontmatterPlan", () => {
  const front = '---\ntopic: "Engine bundle 1"\nstatus: draft\nsource: "agents/a/qa-log.md"\ntags:\n  - one\n---\n';
  const none = () => false;

  it("hides the delimiters, the colon and the quotes, and reads each value's look", () => {
    const marks = frontmatterPlan(front, none);
    const text = (mark: { from: number; to: number }) => front.slice(mark.from, mark.to);
    expect(marks.filter((mark) => mark.kind === "delimiter").map((mark) => mark.from)).toEqual([0, front.length - 4]);
    expect(marks.flatMap((mark) => (mark.kind === "key" ? [text(mark)] : []))).toEqual(["topic", "status", "source", "tags"]);
    expect(marks.flatMap((mark) => (mark.kind === "value" ? [[text(mark), mark.look]] : []))).toEqual([
      ["Engine bundle 1", "text"],
      ["draft", "chip"],
      ["agents/a/qa-log.md", "path"],
    ]);
    expect(marks.flatMap((mark) => (mark.kind === "hide" ? [text(mark)] : []))).toEqual([": ", '"', '"', ": ", ": ", '"', '"', ":"]);
    expect(marks.filter((mark) => mark.kind === "more")).toEqual([{ kind: "more", from: front.indexOf("  - one") }]);
  });

  it("keeps a revealed line's source", () => {
    const marks = frontmatterPlan(front, (line) => line === 0 || line === 2);
    expect(marks.filter((mark) => mark.kind === "source").map((mark) => mark.from)).toEqual([0, front.indexOf("status")]);
    expect(marks.some((mark) => mark.kind === "key" && front.slice(mark.from, mark.to) === "status")).toBe(false);
  });

  it("leaves a value with a quote inside it quoted", () => {
    const marks = frontmatterPlan('---\nsay: "a" and "b"\n---\n', none);
    expect(marks.filter((mark) => mark.kind === "hide")).toHaveLength(1);
  });
});
