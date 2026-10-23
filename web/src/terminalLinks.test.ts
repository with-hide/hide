import { describe, expect, it } from "vitest";
import type { CellRow } from "./selection";
import { linkCandidates, osc8Target, parseToken, TOKEN_CANDIDATES, type LinkCandidate } from "./terminalLinks";

/** A row of `width` cells holding `text`, padded with written spaces the way a Herdr frame draws it. */
function row(text: string, width: number): CellRow {
  const cells: CellRow = [];
  for (const char of text) {
    cells.push(char);
    // Hangul takes two columns; xterm keeps an empty trailing half.
    if (/\p{Script=Hangul}/u.test(char)) cells.push(null);
  }
  while (cells.length < width) cells.push(" ");
  return cells;
}

const texts = (groups: LinkCandidate[][]) => groups.map((group) => group.map((candidate) => candidate.text));

describe("parseToken", () => {
  it("reads a URL and drops the punctuation around it", () => {
    expect(parseToken("(https://example.com/a_(b)).")).toMatchObject({ text: "https://example.com/a_(b)", lead: 1, trail: 2, target: { kind: "url", url: "https://example.com/a_(b)" } });
    expect(parseToken("<https://example.com/x>,")?.target).toEqual({ kind: "url", url: "https://example.com/x" });
  });

  it("reads a path with the location compilers and agents write after it", () => {
    expect(parseToken("src/main.rs:10:5")?.target).toEqual({ kind: "path", path: "src/main.rs", line: 10, column: 5 });
    expect(parseToken("web/src/a.ts:42:")?.target).toEqual({ kind: "path", path: "web/src/a.ts", line: 42, column: null });
    expect(parseToken("src/a.ts(12,3):")?.target).toEqual({ kind: "path", path: "src/a.ts", line: 12, column: 3 });
    expect(parseToken("docs/README.md#L7-L9")?.target).toEqual({ kind: "path", path: "docs/README.md", line: 7, column: null });
    expect(parseToken("`README.md:3`")?.target).toEqual({ kind: "path", path: "README.md", line: 3, column: null });
  });

  it("takes a tool header's argument out of its parentheses", () => {
    expect(parseToken("Update(web/src/terminals.ts)")).toMatchObject({ text: "web/src/terminals.ts", lead: 7, trail: 1 });
  });

  it("reads absolute, home, relative and bare file names, Hangul included", () => {
    expect(parseToken("/tmp/out.png")?.target).toMatchObject({ kind: "path", path: "/tmp/out.png" });
    expect(parseToken("~/notes/할일.md")?.target).toMatchObject({ kind: "path", path: "~/notes/할일.md" });
    expect(parseToken("../x/y")?.target).toMatchObject({ kind: "path", path: "../x/y" });
    expect(parseToken(".gitignore")?.target).toMatchObject({ kind: "path", path: ".gitignore" });
    expect(parseToken("Cargo.lock")?.target).toMatchObject({ kind: "path", path: "Cargo.lock" });
  });

  it("refuses what only looks like a path: versions, shortened paths, schemes, prose", () => {
    for (const token of ["v1.2.3", "1.5", "…/terminals.ts", "mailto:a@b.c", "/", "and", "--flag", "ssh://host/x"]) {
      expect(parseToken(token), token).toBeNull();
    }
  });
});

describe("osc8Target", () => {
  it("reads http(s), file and the vscode://file links Codex writes", () => {
    expect(osc8Target("https://example.com")).toEqual({ kind: "url", url: "https://example.com/" });
    expect(osc8Target("file:///tmp/a%20b.txt")).toEqual({ kind: "path", path: "/tmp/a b.txt", line: null, column: null });
    expect(osc8Target("file:///C:/repo/report.HTML#L2")).toEqual({ kind: "path", path: "C:/repo/report.HTML", line: 2, column: null });
    expect(osc8Target("vscode://file/C:/repo/report.HTML:2")).toEqual({ kind: "path", path: "C:/repo/report.HTML", line: 2, column: null });
    expect(parseToken("C:/repo/report.HTML:2")?.target).toEqual({ kind: "path", path: "C:/repo/report.HTML", line: 2, column: null });
    expect(osc8Target("vscode://file/Users/example/p/src/a.ts:12:4")).toEqual({ kind: "path", path: "/Users/example/p/src/a.ts", line: 12, column: 4 });
  });

  it("leaves any other address alone and refuses a file on another host", () => {
    expect(osc8Target("mailto:a@example.com")).toBeUndefined();
    expect(osc8Target("cursor://file/x")).toBeUndefined();
    expect(osc8Target("file://server/share/x")).toBeNull();
  });
});

describe("linkCandidates", () => {
  it("finds each token of the hovered row with its columns", () => {
    const rows = [row("see src/a.ts:3 and https://example.com.", 60)];
    const groups = linkCandidates(rows, 0);
    expect(texts(groups)).toEqual([["src/a.ts:3", "src/a.ts:3"], ["https://example.com"]]);
    expect(groups[0]![0]!.spans).toEqual([{ row: 0, start: 4, end: 14 }]);
    expect(groups[1]![0]!.spans).toEqual([{ row: 0, start: 19, end: 38 }]);
  });

  it("joins a path the terminal wrapped at its last column, from either row", () => {
    const rows = [row("open web/src/termi", 18), row("nals.ts now", 18)];
    const fromTop = linkCandidates(rows, 0);
    const fromBottom = linkCandidates(rows, 1);
    expect(texts(fromTop)).toEqual([["web/src/terminals.ts", "web/src/termi"]]);
    // The lone second half is offered too, for when the joined path does not exist.
    expect(texts(fromBottom)[0]).toEqual(["web/src/terminals.ts", "nals.ts"]);
    expect(fromTop[0]![0]!.spans).toEqual([
      { row: 0, start: 5, end: 18 },
      { row: 1, start: 0, end: 7 },
    ]);
  });

  it("joins a path a TUI broke at its own margin and indented, and leaves the check to decide", () => {
    const rows = [row("⏺ Edited /Users/example/proj/web/src/ter", 50), row("  minalLinks.ts", 50)];
    expect(texts(linkCandidates(rows, 1))).toEqual([["/Users/example/proj/web/src/terminalLinks.ts", "minalLinks.ts"]]);
  });

  it("keeps a row that ends short apart from a next row at the left edge, as `ls` prints them", () => {
    const rows = [row("src/", 20), row("main.rs", 20)];
    expect(texts(linkCandidates(rows, 0))).toEqual([["src/"]]);
    expect(texts(linkCandidates(rows, 1))).toEqual([["main.rs"]]);
  });

  it("places a tool header's argument on the row it lands on when the wrap cut the header", () => {
    const rows = [row("see Upd", 7), row("ate(web/src/a.ts)", 30)];
    const [group] = linkCandidates(rows, 1);
    const argument = group!.find((candidate) => candidate.text === "web/src/a.ts")!;
    expect(argument.spans).toEqual([{ row: 1, start: 4, end: 16 }]);
  });

  it("joins a URL only across a row that reaches the last column", () => {
    const full = [row("https://example.com/very/lo", 27), row("ng/path", 27)];
    expect(texts(linkCandidates(full, 0))[0]).toEqual(["https://example.com/very/long/path", "https://example.com/very/lo"]);
    const margin = [row("https://example.com/a", 40), row("  next", 40)];
    expect(texts(linkCandidates(margin, 0))).toEqual([["https://example.com/a"]]);
  });

  it("reads a wide glyph as one character and never splits it", () => {
    const rows = [row("열기 docs/한글.md 끝", 30)];
    const [group] = linkCandidates(rows, 0);
    expect(group![0]!.text).toBe("docs/한글.md");
    expect(group![0]!.spans).toEqual([{ row: 0, start: 5, end: 17 }]);
  });

  it("stops at the frames and marks a TUI draws around text", () => {
    const rows = [row("│ ⎿  src/a.ts │", 20)];
    expect(texts(linkCandidates(rows, 0))).toEqual([["src/a.ts"]]);
  });

  it("offers nothing for a row of prose", () => {
    expect(linkCandidates([row("nothing to open here", 30)], 0)).toEqual([]);
  });
});


describe("bounded Korean interpretations", () => {
  it("cuts prose attached where the script changes and keeps the original spelling first", () => {
    for (const [token, path, end] of [
      // Claude Code draws `path`에 without its backticks.
      ["agents/runs/view-mode.html에", "agents/runs/view-mode.html", 26],
      ["docs/한글.md을", "docs/한글.md", 12],
      ["docs/한글.md)에서도", "docs/한글.md", 12],
      ["docs/한글.md`에서라도", "docs/한글.md", 12],
      ["docs/a.md)한국어", "docs/a.md", 9],
      ["docs/a.md에서도", "docs/a.md", 9],
      ["view-mode.htmlに", "view-mode.html", 14],
      ["view-mode.html中", "view-mode.html", 14],
      ["view-mode.html。", "view-mode.html", 14],
    ] as const) {
      const [group] = linkCandidates([row(token, 40)], 0);
      expect(group!.length).toBeLessThanOrEqual(6);
      expect(group![0]!.target).toMatchObject({ path: token, line: null });
      expect(group!.at(-1)!.target).toMatchObject({ path });
      expect(group!.at(-1)!.spans).toEqual([{ row: 0, start: 0, end }]);
    }
  });

  it("offers every spelling between the token's edges, longest first, and reads a location only where the spelling reads as a link", () => {
    const [group] = linkCandidates([row("(src/a.ts:12:5)에", 40)], 0);
    const name = (path: string) => ({ kind: "path", path, line: null, column: null });
    expect(group!.map((candidate) => [candidate.text, candidate.target])).toEqual([
      ["(src/a.ts:12:5)에", name("(src/a.ts:12:5)에")],
      ["(src/a.ts:12:5)", name("(src/a.ts:12:5)")],
      ["src/a.ts:12:5)에", name("src/a.ts:12:5)에")],
      ["(src/a.ts:12:5", name("(src/a.ts:12:5")],
      ["src/a.ts:12:5)", name("src/a.ts:12:5)")],
      ["src/a.ts:12:5", name("src/a.ts:12:5")],
      ["src/a.ts:12:5", { kind: "path", path: "src/a.ts", line: 12, column: 5 }],
    ]);
    expect(group![1]!.spans).toEqual([{ row: 0, start: 0, end: 15 }]);
    expect(group![5]!.spans).toEqual([{ row: 0, start: 1, end: 14 }]);
  });

  it("reads a particle after a URL's closing mark as prose, since a URL cannot be checked", () => {
    const [group] = linkCandidates([row("PR #682 (https://github.com/o/r/pull/682)이 끝", 60)], 0).filter((each) => each[0]!.target.kind === "url");
    expect(group!.map((candidate) => [candidate.text, candidate.target])).toEqual([["https://github.com/o/r/pull/682", { kind: "url", url: "https://github.com/o/r/pull/682" }]]);
    expect(group![0]!.spans).toEqual([{ row: 0, start: 9, end: 40 }]);
    expect(parseToken("(https://example.com/a_(b))에서도")).toMatchObject({ text: "https://example.com/a_(b)", lead: 1, trail: 4 });
    expect(parseToken("https://ko.wikipedia.org/wiki/서울이")?.target).toEqual({ kind: "url", url: new URL("https://ko.wikipedia.org/wiki/서울이").href });
  });

  it("never cuts a name written in those scripts or a word in another script", () => {
    for (const token of ["docs/회의록에", "~/메모", "docs/a.mdは-x", "한글.md", "docs/a.mdfoo"]) {
      const [group] = linkCandidates([row(token, 40)], 0);
      expect(group!.map((candidate) => candidate.target)).toEqual([{ kind: "path", path: token, line: null, column: null }]);
    }
  });

  it("ends a path at a closing bracket whatever follows it, after the whole token", () => {
    const [group] = linkCandidates([row("docs/a.md)foo", 40)], 0);
    expect(group!.map((candidate) => candidate.text)).toEqual(["docs/a.md)foo", "docs/a.md"]);
  });

  it("excludes wrapping punctuation and wide particles from actual cell ranges", () => {
    const [group] = linkCandidates([row("보드 보기 (docs/README.md)에 끝", 50)], 0);
    const path = group!.find((candidate) => candidate.text === "docs/README.md")!;
    expect(path.spans).toEqual([{ row: 0, start: 11, end: 25 }]);
    const wrapped = [row("한글 (docs/한", 13), row("글.md)에서도 끝", 20)];
    const candidate = linkCandidates(wrapped, 1)[0]!.find((each) => each.text === "docs/한글.md")!;
    expect(candidate.spans).toEqual([{ row: 0, start: 6, end: 13 }, { row: 1, start: 0, end: 5 }]);
  });

  it("maps surrogate pairs and combining sequences to whole cells", () => {
    const cells: CellRow = ["앞", null, " ", "(", ..."docs/", "😀", null, "e\u0301", ".", "m", "d", ")", "에", null];
    const path = linkCandidates([cells], 0)[0]!.find((each) => each.text === "docs/😀e\u0301.md")!;
    expect(path.spans).toEqual([{ row: 0, start: 4, end: 15 }]);
  });

  it("bounds every hovered token to sixteen joins and a fixed number of spellings, dropping the shortest", () => {
    const rows = Array.from({ length: 7 }, (_, index) => row("docs/" + String.fromCharCode(97 + index) + ".md)에", 12));
    const groups = linkCandidates(rows, 3);
    expect(groups).toHaveLength(1);
    // All seven rows are fully occupied: four starts times four ends make
    // sixteen distinct joined spellings around the middle row.
    const originals = groups[0]!.filter((candidate) => candidate.original);
    expect(originals).toHaveLength(16);
    expect(new Set(originals.map((candidate) => candidate.text)).size).toBe(16);
    expect(groups[0]!.length).toBeLessThanOrEqual(TOKEN_CANDIDATES);
    // A token made of nothing but edges stays within the same bound.
    const [dense] = linkCandidates([row("(" + "[docs/a.md],".repeat(12) + ")", 200)], 0);
    expect(dense!.length).toBeLessThanOrEqual(TOKEN_CANDIDATES);
    const lengths = dense!.map((candidate) => candidate.text.length);
    expect(lengths).toEqual([...lengths].sort((a, b) => b - a));
  });
});

/**
 * Links written against prose, by kind of mark rather than by mark: every
 * pairing here is answered by the Unicode class of the characters around the
 * link, so each row of these tables is a script or a mark the parser has no
 * rule of its own for.
 */
describe("edges by Unicode class", () => {
  /** A row as xterm lays it out, with a wide glyph's trailing half for each East Asian wide character. */
  function wideRow(text: string, width: number): CellRow {
    const cells: CellRow = [];
    for (const char of text) {
      cells.push(char);
      if (/[\p{scx=Hangul}\p{scx=Han}\p{scx=Hiragana}\p{scx=Katakana}\u3000-\u303F\uFF01-\uFF60]/u.test(char)) cells.push(null);
    }
    while (cells.length < width) cells.push(" ");
    return cells;
  }

  const PAIRS = [["(", ")"], ["[", "]"], ["{", "}"], ["<", ">"], ['"', '"'], ["'", "'"], ["`", "`"], ["“", "”"], ["‘", "’"], ["«", "»"], ["（", "）"], ["［", "］"], ["「", "」"], ["『", "』"], ["【", "】"], ["《", "》"], ["〈", "〉"], ["〔", "〕"]] as const;
  const BEFORE = ["", "열어뒀어요", "ここに", "见"];
  const AFTER = ["", "에서도", "を", "里"];
  const ENDS = ["", ".", ",", "。", "、", "！", "：", "?"];

  /** The URL, or the first spelling of the hovered token that is a path, read the way the provider reads it. */
  function linked(text: string, files: string[]): { text: string; start: number; end: number } | null {
    const cells = wideRow(text, text.length * 2 + 4);
    for (const group of linkCandidates([cells], 0)) {
      const hit = group.find((candidate) => candidate.target.kind === "url" || files.includes(candidate.target.path));
      if (hit) return { text: hit.text, start: hit.spans[0]!.start, end: hit.spans.at(-1)!.end };
    }
    return null;
  }

  it("links a URL in any script's brackets or quotes with prose and punctuation written against either side", () => {
    const url = "https://example.com/a/b?q=1";
    for (const [open, close] of PAIRS) for (const before of BEFORE) for (const after of AFTER) for (const end of ENDS) {
      const text = `${before}${open}${url}${close}${after}${end}`;
      const found = linked(text, []);
      expect(found?.text, text).toBe(url);
      // The link covers exactly the URL's cells, whatever widths come before it.
      const start = [...`${before}${open}`].reduce((width, char) => width + (wideRow(char, 0).length), 0);
      expect(found, text).toMatchObject({ start, end: start + url.length });
    }
  });

  it("links an existing path, with or without a location, the same way", () => {
    for (const [written, path] of [["/Users/me/.hide/state/TerminalClipboard/hide-07c5.png", "/Users/me/.hide/state/TerminalClipboard/hide-07c5.png"], ["src/a.ts:12:5", "src/a.ts"], ["docs/한글.md", "docs/한글.md"]] as const) {
      for (const [open, close] of PAIRS) for (const before of BEFORE) for (const after of AFTER) for (const end of ENDS) {
        const text = `${before}${open}${written}${close}${after}${end}`;
        expect(linked(text, [path])?.text, text).toBe(written);
      }
    }
  });

  it("links the address in the line that was reported, and only the address", () => {
    const text = "브라우저 탭에 열어뒀어요(http://127.0.0.1:8391). 이전에는";
    const groups = linkCandidates([wideRow(text, 80)], 0);
    expect(groups.flat().map((candidate) => [candidate.text, candidate.spans])).toEqual([["http://127.0.0.1:8391", [{ row: 0, start: 25, end: 46 }]]]);
  });

  it("links prose written straight against an address with no bracket between", () => {
    expect(linked("열어뒀어요http://127.0.0.1:8391.", [])?.text).toBe("http://127.0.0.1:8391");
    expect(linked("见src/a.ts:3", ["src/a.ts"])?.text).toBe("src/a.ts:3");
    expect(linked("파일은~/notes/a.md입니다", ["~/notes/a.md"])?.text).toBe("~/notes/a.md");
  });

  it("keeps a folder or file named in those scripts whole, since `/` and `.` join a name", () => {
    expect(linked("회의록/a.md", ["/a.md", "회의록/a.md"])?.text).toBe("회의록/a.md");
    expect(linked("회의록/a.md", ["/a.md"])).toBeNull();
    expect(linked("docs/회의록에", ["docs/", "docs"])).toBeNull();
  });

  it("starts a value after `=`", () => {
    expect(linked("--file=/tmp/a.png", ["/tmp/a.png"])?.text).toBe("/tmp/a.png");
    expect(linked("https://example.com/?a=b", [])?.text).toBe("https://example.com/?a=b");
  });

  it("finds the address inside a Markdown link, a JSON field and a tool header", () => {
    expect(linked("[the docs](https://example.com/docs)", [])?.text).toBe("https://example.com/docs");
    expect(linked("[a.png](/tmp/a.png)", ["/tmp/a.png"])?.text).toBe("/tmp/a.png");
    expect(linked('{"path":"/tmp/a.png"},', ["/tmp/a.png"])?.text).toBe("/tmp/a.png");
    expect(linked("Read(web/src/a.ts)", ["web/src/a.ts"])?.text).toBe("web/src/a.ts");
  });

  it("keeps a URL's own balanced brackets and apostrophes and cuts what does not belong to it", () => {
    expect(linked("(https://en.wikipedia.org/wiki/Mercury_(planet)).", [])?.text).toBe("https://en.wikipedia.org/wiki/Mercury_(planet)");
    expect(linked("https://example.com/it's", [])?.text).toBe("https://example.com/it's");
    expect(linked("'https://example.com/a'", [])?.text).toBe("https://example.com/a");
    expect(linked("https://example.com/a…", [])?.text).toBe("https://example.com/a…");
  });

  it("takes the whole name when the brackets are part of it and never stops at an opening bracket", () => {
    expect(linked("app/(auth)/login/page.tsx", ["app/(auth)/login/page.tsx", "app/"])?.text).toBe("app/(auth)/login/page.tsx");
    expect(linked("app/(auth)/login/page.tsx", ["app/"])).toBeNull();
    expect(linked("「docs/a.md」를", ["「docs/a.md」"])?.text).toBe("「docs/a.md」");
  });

  it("refuses a shortened path however it is wrapped", () => {
    for (const text of ["…/terminals.ts", "(…/terminals.ts)", "「…/a.ts」에서"]) {
      expect(linked(text, ["/terminals.ts", "terminals.ts", "/a.ts", "a.ts"]), text).toBeNull();
    }
  });
});
