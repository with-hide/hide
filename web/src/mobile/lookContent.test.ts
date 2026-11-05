import { describe, expect, it } from "vitest";
import { addChunk, beginAssembly, clampOffset, fileTag, finishAssembly, LOOK_IMAGE_CAP, LOOK_TEXT_CAP, resultContent, splitPath, countState, zoomBy } from "./lookContent";
import { replyAbout, splitLooks, type LookHead, type LookRow } from "./protocol";

const head = (over: Partial<LookHead> = {}): LookHead => ({ type: "look_head", request_id: "r", view_id: "v", mode: "text", size: 3, total: 3, truncated: false, chunks: 1, ...over });
const b64 = (bytes: number[]) => btoa(String.fromCharCode(...bytes));
const utf8 = (text: string) => b64([...new TextEncoder().encode(text)]);

describe("a read put back from its chunks", () => {
  it("refuses a head that claims more than its cap or contradicts itself", () => {
    expect(beginAssembly(head({ size: LOOK_TEXT_CAP + 1, chunks: 11 }))).toBeNull();
    expect(beginAssembly(head({ mode: "image", size: LOOK_IMAGE_CAP + 1, chunks: 103 }))).toBeNull();
    expect(beginAssembly(head({ size: 0, chunks: 1 }))).toBeNull();
    expect(beginAssembly(head({ size: 5, chunks: 0 }))).toBeNull();
    expect(beginAssembly(head({ size: 5, chunks: 100000 }))).toBeNull();
    expect(beginAssembly(head({ mode: "image", size: LOOK_IMAGE_CAP, chunks: 103 }))).not.toBeNull();
  });

  it("takes chunks in order only, and never more bytes than the head said", () => {
    const start = beginAssembly(head({ size: 6, chunks: 2 }))!;
    expect(addChunk(start, 1, utf8("abc"))).toBeNull();
    const one = addChunk(start, 0, utf8("abc"))!;
    expect(addChunk(one, 0, utf8("abc"))).toBeNull();
    expect(addChunk(one, 1, utf8("abcd"))).toBeNull();
    expect(addChunk(one, 2, utf8("a"))).toBeNull();
    expect(finishAssembly(one)).toBeNull();
    expect(finishAssembly(addChunk(one, 1, utf8("def"))!)).toMatchObject({ state: "text", text: "abcdef" });
  });

  it("decodes Korean text across chunk boundaries and drops the half of a character the cap cut", () => {
    const bytes = [...new TextEncoder().encode("가나")];
    const cut = bytes.slice(0, 5);
    const whole = addChunk(beginAssembly(head({ size: 6, chunks: 1 }))!, 0, b64(bytes))!;
    expect(finishAssembly(whole)).toMatchObject({ text: "가나", truncated: false });
    const half = addChunk(beginAssembly(head({ size: 5, total: 900, truncated: true, chunks: 1 }))!, 0, b64(cut))!;
    expect(finishAssembly(half)).toEqual({ state: "text", markdown: false, text: "가", truncated: true });
  });

  it("calls a Markdown read a document to draw", () => {
    const done = addChunk(beginAssembly(head({ mode: "markdown", size: 2, chunks: 1 }))!, 0, utf8("# "))!;
    expect(finishAssembly(done)).toMatchObject({ state: "text", markdown: true });
  });

  it("makes an image of bytes that name a format and nothing of any other", () => {
    const png = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
    const done = addChunk(beginAssembly(head({ mode: "image", size: 8, chunks: 1 }))!, 0, b64(png))!;
    expect(finishAssembly(done)).toEqual({ state: "image", url: `data:image/png;base64,${b64(png)}`, size: 8 });
    const text = addChunk(beginAssembly(head({ mode: "image", size: 5, chunks: 1 }))!, 0, utf8("hello"))!;
    expect(finishAssembly(text)).toBeNull();
  });

  it("draws a vector image as an image, which is how a page's content policy keeps its scripts out", () => {
    const svg = '<?xml version="1.0"?>\n<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"/>';
    const done = addChunk(beginAssembly(head({ mode: "image", size: svg.length, chunks: 1 }))!, 0, utf8(svg))!;
    expect(finishAssembly(done)).toMatchObject({ state: "image", url: expect.stringMatching(/^data:image\/svg\+xml;base64,/) });
  });

  it("finishes an empty file as an empty text", () => {
    expect(finishAssembly(beginAssembly(head({ size: 0, total: 0, chunks: 0 }))!)).toMatchObject({ state: "text", text: "" });
  });
});

describe("the states that are not content", () => {
  it("maps what hided answers to what the viewer draws", () => {
    expect(resultContent("page", "https://a.example/")).toEqual({ state: "page", url: "https://a.example/" });
    for (const state of ["other", "too_large", "closed", "device_unreachable"] as const) expect(resultContent(state, undefined)).toEqual({ state });
    // A refusal, an agent that left and an unreadable checkout read as a read that failed, with 다시.
    for (const state of ["failed", "gone", "refused", "unavailable"] as const) expect(resultContent(state, undefined)).toEqual({ state: "failed" });
  });
});

describe("the changed files", () => {
  it("splits a path into its name and folder, and tags the file by its extension", () => {
    expect(splitPath("hide-platform/src/path.rs")).toEqual({ name: "path.rs", folder: "hide-platform/src" });
    expect(splitPath("README")).toEqual({ name: "README", folder: "" });
    expect(fileTag("path.rs")).toBe("RS");
    expect(fileTag("Makefile")).toBe("");
    expect(fileTag(".gitignore")).toBe("");
  });

  it("calls a file binary only when hided found git cannot count it, and a new file left uncounted is neither", () => {
    expect(countState({ path: "a.png", status: "modified", added: null, removed: null, binary: true })).toBe("binary");
    expect(countState({ path: "a.rs", status: "modified", added: 0, removed: 0, binary: false })).toBe("counted");
    expect(countState({ path: "src/fresh.ts", status: "untracked", added: null, removed: null, binary: false })).toBe("uncounted");
  });
});

describe("an image zoomed with two fingers", () => {
  it("never gets smaller than the fitted image or larger than the most", () => {
    expect(zoomBy(1, 0.5)).toBe(1);
    expect(zoomBy(2, 1.5)).toBe(3);
    expect(zoomBy(7, 3)).toBe(8);
  });

  it("moves only as far as its edge reaches the frame's", () => {
    expect(clampOffset(500, 1, 400)).toBe(0);
    expect(clampOffset(500, 2, 400)).toBe(200);
    expect(clampOffset(-500, 2, 400)).toBe(-200);
    expect(clampOffset(50, 2, 400)).toBe(50);
  });
});

describe("the list and the reply", () => {
  const row = (id: string, unseen: boolean): LookRow => ({ id, kind: "doc", name: id, unseen, at: 0 });

  it("keeps hided's order within not seen and seen", () => {
    const parts = splitLooks([row("a", false), row("b", true), row("c", false), row("d", true)]);
    expect(parts.unseen.map((view) => view.id)).toEqual(["b", "d"]);
    expect(parts.seen.map((view) => view.id)).toEqual(["a", "c"]);
  });

  it("puts the name of what is on screen in brackets in front of the words, without anything that would refuse the reply", () => {
    expect(replyAbout("경로 구분자 수정", "고쳐 줘")).toBe("[경로 구분자 수정] 고쳐 줘");
    expect(replyAbout("a]b[c\nd", "x")).toBe("[abc d] x");
    expect(replyAbout("n".repeat(100), "x")).toBe(`[${"n".repeat(60)}] x`);
    expect(replyAbout("\n", "x")).toBe("x");
  });
});
