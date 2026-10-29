import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ProbedPath } from "./host";
import type { CellRow } from "./selection";
import { useShellStore } from "./store";
import { linkCandidates, TOKEN_CANDIDATES } from "./terminalLinks";
import type { SnapshotRest } from "./snapshot";
import type { ILink, ILinkProvider, Terminal } from "@xterm/xterm";
import { PROBE_CACHE_CAP, PROBE_NEW_LIMIT, PROBE_TTL_MS, clearProbeCache, osc8Handler, owningCheckout, paneContext, pathLookups, probePaths, registerTerminalLinks, resolveGroups } from "./terminalLinkProvider";

function row(text: string, width: number): CellRow {
  const cells: CellRow = [];
  for (const char of text) { cells.push(char); if (/\p{Script=Hangul}/u.test(char)) cells.push(null); }
  while (cells.length < width) cells.push(" ");
  return cells;
}

/** A host that knows exactly `files` and `dirs`, each its own physical path. */
function host(files: string[], dirs: string[] = []) {
  return vi.fn(async (paths: string[]): Promise<ProbedPath[]> =>
    paths.map((path) => (files.includes(path) ? { real: path, kind: "file" } : dirs.includes(path) ? { real: path, kind: "directory" } : null)),
  );
}

beforeEach(() => clearProbeCache());

describe("paneContext", () => {
  // The core runs on `core`; this screen's machine `mac` dials it, so its
  // panes are a device's in the core's snapshot (PRD core-host-node-remote-core B13).
  const pane = "remote:mac:pane:w1:p1";
  const rest = {
    navigator: {
      devices: [{ id: "core", kind: "local" }, { id: "mac", kind: "remote" }],
      workspaces: [{ id: "w-core", device_id: "core", checkouts: [{ id: "c-core", path: "/c/proj", tabs: [{ panes: [{ id: "w9:p1", cwd: "/c/proj" }] }] }] }],
    },
    status: {
      remote: [{
        target_id: "mac",
        session: { workspaces: [{ id: "w-mac", device_id: "mac", checkouts: [{ id: "c-mac", path: "/m/proj", tabs: [{ panes: [{ id: pane, cwd: "/m/proj/sub" }] }] }] }] },
      }],
    },
  } as unknown as SnapshotRest;

  it("reads a pane of the screen's own machine, whichever machine runs the core", () => {
    expect(paneContext(rest, pane, "mac")).toEqual({ cwd: "/m/proj/sub", root: "/m/proj" });
    expect(paneContext(rest, "w9:p1", "core")).toEqual({ cwd: "/c/proj", root: "/c/proj" });
  });

  it("reads no pane of another machine than the screen's", () => {
    expect(paneContext(rest, "w9:p1", "mac")).toBeNull();
    expect(paneContext(rest, pane, "core")).toBeNull();
  });
});

describe("pathLookups", () => {
  it("looks a relative path up under the pane's folder, then the checkout's root", () => {
    expect(pathLookups("src/a.ts", "/w/proj/sub", "/w/proj")).toEqual(["/w/proj/sub/src/a.ts", "/w/proj/src/a.ts"]);
    expect(pathLookups("../b.ts", "/w/proj/sub", "/w/proj")).toEqual(["/w/proj/b.ts", "/w/b.ts"]);
    expect(pathLookups("a.ts", "/w/proj", "/w/proj")).toEqual(["/w/proj/a.ts"]);
  });

  it("takes an absolute or home path as written", () => {
    expect(pathLookups("/tmp/./x/../out.png", "/w", "/w")).toEqual(["/tmp/out.png"]);
    expect(pathLookups("~/notes.md", "/w", "/w")).toEqual(["~/notes.md"]);
  });

  it("has nowhere to look for a relative path without a folder", () => {
    expect(pathLookups("src/a.ts", null, null)).toEqual([]);
  });

  it("keeps the drive when resolving paths in a Windows pane", async () => {
    expect(pathLookups("src/report.HTML", "C:/repo/sub", "C:/repo")).toEqual(["C:/repo/sub/src/report.HTML", "C:/repo/src/report.HTML"]);
    expect(pathLookups("C:/repo/../report.HTML", null, null)).toEqual(["C:/report.HTML"]);
    expect(pathLookups("../../report.HTML", "C:/repo", null)).toEqual(["C:/report.HTML"]);
    expect(pathLookups("../../report.HTML", "//server/share/repo", null)).toEqual(["//server/share/report.HTML"]);
    const chosen = await resolveGroups(linkCandidates([row("src/report.HTML:2", 40)], 0), { cwd: "C:/repo", root: "C:/repo" }, host(["C:/repo/src/report.HTML"]));
    expect(chosen[0]?.resolved.found?.real).toBe("C:/repo/src/report.HTML");
    // A drive path a Windows shell printed with backslashes is one link, asked in the wire spelling.
    const printed = "img C:\\Users\\example\\AppData\\Local\\Temp\\shot.png";
    const temp = await resolveGroups(linkCandidates([row(printed, 80)], 0), { cwd: "C:/repo", root: "C:/repo" }, host(["C:/Users/example/AppData/Local/Temp/shot.png"]));
    expect(temp[0]?.resolved.found?.real).toBe("C:/Users/example/AppData/Local/Temp/shot.png");
  });
});

describe("resolveGroups", () => {
  it("links the longest spelling that exists and drops a token that names nothing", async () => {
    const rows = [row("open web/src/termi", 18), row("nals.ts or nowhere.ts", 30)];
    const probe = host(["/w/web/src/terminals.ts"]);
    const chosen = await resolveGroups(linkCandidates(rows, 1), { cwd: "/w", root: "/w" }, probe);
    expect(chosen.map(({ candidate }) => candidate.text)).toEqual(["web/src/terminals.ts"]);
    expect(chosen[0]!.resolved.found).toEqual({ real: "/w/web/src/terminals.ts", kind: "file" });
    // Every lookup of the row went out in one call.
    expect(probe).toHaveBeenCalledTimes(1);
  });

  it("falls back to the unjoined piece when the joined path does not exist", async () => {
    const rows = [row("see src/a.ts", 12), row("src/b.ts", 12)];
    const chosen = await resolveGroups(linkCandidates(rows, 0), { cwd: "/w", root: "/w" }, host(["/w/src/a.ts", "/w/src/b.ts"]));
    expect(chosen.map(({ candidate }) => candidate.text)).toEqual(["src/a.ts"]);
  });

  it("offers URLs alone where paths cannot be checked: a device pane or a page without the desktop host", async () => {
    const rows = [row("https://example.com src/a.ts", 40)];
    const probe = host(["/w/src/a.ts"]);
    expect((await resolveGroups(linkCandidates(rows, 0), null, probe)).map(({ candidate }) => candidate.text)).toEqual(["https://example.com"]);
    expect((await resolveGroups(linkCandidates(rows, 0), { cwd: "/w", root: "/w" }, null)).map(({ candidate }) => candidate.text)).toEqual(["https://example.com"]);
    expect(probe).not.toHaveBeenCalled();
  });
});

describe("probePaths", () => {
  it("asks once per path within the cache's time and again after it", async () => {
    const probe = host(["/a"]);
    await probePaths(["/a", "/b"], probe, 1_000);
    await probePaths(["/a", "/b"], probe, 1_000 + PROBE_TTL_MS - 1);
    expect(probe).toHaveBeenCalledTimes(1);
    const answers = await probePaths(["/a"], probe, 1_000 + PROBE_TTL_MS);
    expect(probe).toHaveBeenCalledTimes(2);
    expect(answers.get("/a")).toEqual({ real: "/a", kind: "file" });
  });

  it("splits a long request into the host's batches and keeps at most the cap", async () => {
    const probe = host([]);
    const paths = Array.from({ length: PROBE_CACHE_CAP + 10 }, (_, index) => `/p${index}`);
    await probePaths(paths, probe, 0);
    expect(probe.mock.calls.every(([batch]) => batch.length <= 64)).toBe(true);
    expect(probe.mock.calls.flatMap(([batch]) => batch)).toHaveLength(PROBE_NEW_LIMIT);
    // A later call fills the cache past its cap and evicts the oldest answer.
    await probePaths(["/extra"], probe, 1);
    probe.mockClear();
    await probePaths(["/p0"], probe, 1);
    expect(probe).toHaveBeenCalledTimes(1);
  });

  it("leaves a failed or refused call's paths unanswered, apart from a path that does not exist, and caches nothing from it", async () => {
    const failing = vi.fn(async () => {
      throw new Error("gone");
    });
    const refused = vi.fn(async (): Promise<ProbedPath[]> => []);
    expect((await probePaths(["/a"], failing, 0)).has("/a")).toBe(false);
    expect((await probePaths(["/a"], refused, 0)).has("/a")).toBe(false);
    const probe = host(["/a"]);
    const answers = await probePaths(["/a", "/b"], probe, 1);
    expect(answers.get("/a")).toEqual({ real: "/a", kind: "file" });
    expect(answers.has("/b") && answers.get("/b")).toBeNull();
  });
});

describe("owningCheckout", () => {
  const checkouts = [{ path: "/w/proj" }, { path: "/w/proj/.worktrees/feature" }, { path: "/links/other" }];
  const roots = new Map<string, ProbedPath>([
    ["/w/proj", { real: "/w/proj", kind: "directory" }],
    ["/w/proj/.worktrees/feature", { real: "/w/proj/.worktrees/feature", kind: "directory" }],
    // A checkout registered through a symlink is matched by its physical root.
    ["/links/other", { real: "/real/other", kind: "directory" }],
  ]);

  it("gives a path to the checkout with the longest physical root that holds it", () => {
    expect(owningCheckout("/w/proj/src/a.ts", checkouts, roots)).toEqual({ checkout: checkouts[0], root: "/w/proj" });
    expect(owningCheckout("/w/proj/.worktrees/feature/a.ts", checkouts, roots)?.checkout).toBe(checkouts[1]);
    expect(owningCheckout("/real/other/x.md", checkouts, roots)).toEqual({ checkout: checkouts[2], root: "/real/other" });
    expect(owningCheckout("/w/proj", checkouts, roots)?.checkout).toBe(checkouts[0]);
  });

  it("gives nothing for a path outside every checkout or beside one with a shared prefix", () => {
    expect(owningCheckout("/tmp/out.png", checkouts, roots)).toBeNull();
    expect(owningCheckout("/w/project-two/a.ts", checkouts, roots)).toBeNull();
  });
});


describe("original spelling and bounded hover work", () => {
  it("records logical lookups separately from cache misses only in the opt-in QA seam", async () => {
    const probe = host(["/w/a.md"]);
    vi.stubGlobal("window", { __hideProbe: {} });
    try {
      await probePaths(["/w/a.md", "/w/a.md", "/w/b.md"], probe, 100);
      expect(useShellStore.getState().diagnostics.at(-1)).toBe("terminal link: path_probe unique=2 cache_hits=0 cache_misses=2 admitted=2 skipped=0");
      await probePaths(["/w/a.md", "/w/b.md"], probe, 101);
      expect(useShellStore.getState().diagnostics.at(-1)).toBe("terminal link: path_probe unique=2 cache_hits=2 cache_misses=0 admitted=0 skipped=0");
      expect(probe).toHaveBeenCalledTimes(1);
    } finally { vi.unstubAllGlobals(); }
  });

  it.each([
    ["docs/a.md)에", ["docs/a.md", "docs/a.md)"], "docs/a.md)"],
    ["docs/a.md)에", ["docs/a.md", "docs/a.md)", "docs/a.md)에"], "docs/a.md)에"],
    ["(docs/a.md)에", ["docs/a.md", "docs/a.md)에"], "docs/a.md)에"],
    ["(docs/a.md)에", ["docs/a.md", "docs/a.md)"], "docs/a.md)"],
    ["docs/a.md)에.", ["docs/a.md", "docs/a.md)"], "docs/a.md)"],
    ["(docs/a.md)에", ["docs/a.md", "docs/a.md)", "(docs/a.md)"], "(docs/a.md)"],
    ["(docs/a.md)에.", ["docs/a.md", "docs/a.md)", "(docs/a.md)"], "(docs/a.md)"],
    ["(docs/a.md)에,", ["docs/a.md", "docs/a.md)", "(docs/a.md)"], "(docs/a.md)"],
    ["(src/a.ts:12:4)에", ["src/a.ts", "src/a.ts:12:4"], "src/a.ts:12:4"],
    ["src/a.ts:12:4)에", ["src/a.ts", "src/a.ts:12:4"], "src/a.ts:12:4"],
    ["docs/a.md)", ["docs/a.md", "docs/a.md)"], "docs/a.md)"],
    ["src/a.ts:12:4", ["src/a.ts", "src/a.ts:12:4"], "src/a.ts:12:4"],
    ["src/a.ts:12:4", ["src/a.ts"], "src/a.ts"],
    ["docs/a(b).md", ["docs/a(b).md"], "docs/a(b).md"],
    ['docs/a"b.md', ['docs/a"b.md'], 'docs/a"b.md'],
    ["docs/a.md)없는말", ["docs/a.md"], "docs/a.md"],
    ["agents/runs/view-mode.html에", ["agents/runs/view-mode.html"], "agents/runs/view-mode.html"],
    ["agents/runs/view-mode.html에", ["agents/runs/view-mode.html", "agents/runs/view-mode.html에"], "agents/runs/view-mode.html에"],
  ])("resolves %s to the longest real spelling", async (token, files, expected) => {
    const chosen = await resolveGroups(linkCandidates([row(token, 80)], 0), { cwd: "/w", root: "/r" }, host(files.map((file) => "/w/" + file)));
    expect(chosen.map(({ resolved }) => resolved.found!.real)).toEqual(expected ? ["/w/" + expected] : []);
    if (chosen[0]?.resolved.target.kind === "path") {
      expect(chosen[0].resolved.target.line).toBe(expected === "src/a.ts" ? 12 : null);
      expect(chosen[0].resolved.target.column).toBe(expected === "src/a.ts" ? 4 : null);
    }
  });

  it("chooses each composite literal before shorter spellings or a location, with exact cells", async () => {
    // Longest first; of two the same length, the one that keeps the token's start.
    const spellings = ["(src/a.ts:12:5)에", "(src/a.ts:12:5)", "src/a.ts:12:5)에", "src/a.ts:12:5)", "src/a.ts:12:5", "src/a.ts"];
    for (let index = 0; index < spellings.length; index += 1) {
      clearProbeCache();
      const chosen = await resolveGroups(linkCandidates([row(spellings[0]!, 40)], 0), { cwd: "/w", root: "/r" }, host(spellings.slice(index).map((file) => "/w/" + file)));
      expect(chosen).toHaveLength(1);
      const { candidate, resolved } = chosen[0]!;
      expect(resolved.found!.real).toBe("/w/" + spellings[index]);
      expect(resolved.target).toMatchObject({ line: index === 5 ? 12 : null, column: index === 5 ? 5 : null });
      const text = index === 5 ? spellings[4]! : spellings[index]!;
      const start = text.startsWith("(") ? 0 : 1;
      expect(candidate.spans).toEqual([{ row: 0, start, end: start + text.length + (text.endsWith("에") ? 1 : 0) }]);
    }
  });

  it("keeps raw leading and closing literals with a location and trailing sentence punctuation", async () => {
    for (const punctuation of [".", ",", ")", "]", "!", "?"]) {
      clearProbeCache();
      const token = "(src/a.ts:12:5)에" + punctuation;
      const groups = linkCandidates([row(token, 40)], 0);
      expect(groups[0]!.length).toBeLessThanOrEqual(TOKEN_CANDIDATES);
      const chosen = await resolveGroups(groups, { cwd: "/w", root: "/r" }, host(["/w/(src/a.ts:12:5)", "/w/src/a.ts:12:5)", "/w/src/a.ts:12:5", "/w/src/a.ts"]));
      expect(chosen[0]!.resolved.found!.real).toBe("/w/(src/a.ts:12:5)");
      expect(chosen[0]!.candidate.spans).toEqual([{ row: 0, start: 0, end: 15 }]);
    }
  });

  it("does not substitute a shorter file after a higher-precedence check failed", async () => {
    const probe = vi.fn(async (paths: string[]): Promise<ProbedPath[]> => {
      if (paths.includes("/w/docs/a.md)에")) throw new Error("unavailable");
      return paths.map((real) => ({ real, kind: "file" }));
    });
    expect(await resolveGroups(linkCandidates([row("docs/a.md)에", 40)], 0), { cwd: "/w", root: "/w" }, probe)).toEqual([]);
  });

  it("waits for the original spelling when shorter filesystem answers arrive first", async () => {
    const releases = new Map<string, () => void>();
    let settled = false;
    const probe = (paths: string[]) => Promise.all(paths.map((real) => new Promise<ProbedPath>((resolve) => {
      releases.set(real, () => resolve({ real, kind: "file" }));
    })));
    const pending = resolveGroups(linkCandidates([row("docs/a.md)에", 40)], 0), { cwd: "/w", root: "/w" }, probe).then((chosen) => { settled = true; return chosen; });
    releases.get("/w/docs/a.md")!();
    releases.get("/w/docs/a.md)")!();
    await Promise.resolve();
    expect(settled).toBe(false);
    releases.get("/w/docs/a.md)에")!();
    expect((await pending)[0]?.resolved.found?.real).toBe("/w/docs/a.md)에");
  });

  it("spends the row budget on originals before interpretations and reports overflow without paths", async () => {
    const text = Array.from({ length: 300 }, (_, index) => "docs/f" + index + ".md)에").join(" ");
    const probe = host([]);
    const chosen = await resolveGroups(linkCandidates([row(text, text.length * 2)], 0), { cwd: "/w", root: "/r" }, probe);
    expect(chosen).toEqual([]);
    const sent = probe.mock.calls.flatMap(([batch]) => batch);
    expect(sent).toHaveLength(512);
    expect(new Set(sent).size).toBe(512);
    expect(probe.mock.calls).toHaveLength(8);
    expect(sent.every((path) => path.endsWith(")에"))).toBe(true);
    expect(useShellStore.getState().diagnostics.at(-1)).toBe("terminal link: path_budget_exceeded unique=1800 cache_misses=1800 limit=512 skipped=1288");
  });

  it("does not select inferred cache hits when an original was skipped", async () => {
    await probePaths(["/w/docs/end.md"], host(["/w/docs/end.md"]));
    const text = [...Array.from({ length: 512 }, (_, index) => "docs/f" + index + ".md"), "docs/end.md)에"].join(" ");
    const chosen = await resolveGroups(linkCandidates([row(text, text.length * 2)], 0), { cwd: "/w", root: "/w" }, host([]));
    expect(chosen).toEqual([]);
  });

  it("charges only cache misses and keeps warm work at zero host calls", async () => {
    const paths = Array.from({ length: 512 }, (_, index) => "/p" + index);
    const probe = host([]);
    await probePaths(paths, probe, 0);
    probe.mockClear();
    const answers = await probePaths([...paths, "/new"], probe, 1);
    expect(answers.size).toBe(513);
    expect(probe).toHaveBeenCalledExactlyOnceWith(["/new"]);
  });
});

it("a retained terminal link opens under its current address after a native move", () => {
  let pane = "w1:p1";
  const actions = { openLink: vi.fn(), openTerminalPath: vi.fn() };
  const handler = osc8Handler(() => pane, { hovered: false }, () => actions, () => false);
  pane = "w2:p2";
  handler.activate({ button: 0, metaKey: false, ctrlKey: false } as MouseEvent, "https://example.test/moved", { start: { x: 1, y: 1 }, end: { x: 27, y: 1 } });
  expect(actions.openLink).toHaveBeenCalledWith("https://example.test/moved", false, "w2:p2");
});

/** A one-row terminal showing `text`, with the link provider it was given. */
function terminalShowing(text: string): { term: Terminal; links: () => Promise<ILink[]> } {
  let provider: ILinkProvider | undefined;
  const cells = [...text];
  const line = { getCell: (x: number) => ({ getWidth: () => 1, getChars: () => cells[x] ?? "" }) };
  const term = {
    cols: text.length,
    buffer: { active: { length: 1, getLine: (y: number) => (y === 0 ? line : undefined) } },
    hasSelection: () => false,
    registerLinkProvider: (given: ILinkProvider) => {
      provider = given;
      return { dispose() {} };
    },
  } as unknown as Terminal;
  return { term, links: () => new Promise((resolve) => provider!.provideLinks(1, (links) => resolve(links ?? []))) };
}

// xterm activates a link on the release of any button, and on Windows the
// right-click menu opens after that release, so a right-click on a link
// opened it beside the menu (with-hide/hide-private#252).
it("only the primary button's release opens a link, never a right-click's or a middle-click's", async () => {
  const actions = { openLink: vi.fn(), openTerminalPath: vi.fn() };
  const { term, links } = terminalShowing("see https://example.test/a");
  registerTerminalLinks(term, "w1:p1", { hovered: false }, () => actions);
  const [link] = await links();
  const osc8 = osc8Handler("w1:p1", { hovered: false }, () => actions, () => false);
  for (const button of [2, 1]) {
    link!.activate({ button, metaKey: false, ctrlKey: false } as MouseEvent, link!.text);
    osc8.activate({ button, metaKey: false, ctrlKey: false } as MouseEvent, "https://example.test/b", link!.range);
  }
  expect(actions.openLink).not.toHaveBeenCalled();
  link!.activate({ button: 0, metaKey: false, ctrlKey: false } as MouseEvent, link!.text);
  osc8.activate({ button: 0, metaKey: false, ctrlKey: false } as MouseEvent, "https://example.test/b", link!.range);
  expect(actions.openLink.mock.calls).toEqual([["https://example.test/a", false, "w1:p1"], ["https://example.test/b", false, "w1:p1"]]);
});
