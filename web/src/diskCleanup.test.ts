import { emptyScope } from "../test/legacyAgentScope";
// The disk cleanup sheet's rules (PRD disk-layers): the selection math, the
// filters, the folded row, the footer and the entrance numbers, read against
// small fixtures. Expected answers are the PRD's Behaviors (B2, B4, B10-B16, B22).

import { describe, expect, it } from "vitest";
import { initializeInterfaceI18n } from "./i18n/instance";
import {
  EMPTY_SELECTION,
  SMALL_CHECKOUT_BYTES,
  BUSY_KEY,
  bundleRefs,
  bundleState,
  cellReasonText as cellReasonTextIn,
  entranceBytes,
  exclusionText as exclusionTextIn,
  filterCounts,
  footerOf as footerOfIn,
  layerLines as layerLinesIn,
  layoutRows,
  needsConfirm,
  planOf,
  pruneSelection,
  reasonText as reasonTextIn,
  reclaimable,
  cleanupElsewhere,
  resultLines as resultLinesIn,
  sheetModel as sheetModelIn,
  toggleBundle,
  toggleCell,
  visibleRows,
  type SheetRow,
} from "./diskCleanup";
import type { Checkout, CleanupCellResult, CleanupExclusionCode, CleanupInUse, CleanupRow, DiskCleanup, DiskLayers, Workspace } from "./snapshot";

const GB = 1024 ** 3;
const MB = 1024 ** 2;

// The Korean wording is what the sheet shipped with; the model reads the same under it.
const t = initializeInterfaceI18n("ko").getFixedT(null, "translation");
const english = initializeInterfaceI18n("en").getFixedT(null, "translation");
const sheetModel = (workspace: Workspace, elsewhere: Parameters<typeof sheetModelIn>[1]) => sheetModelIn(workspace, elsewhere, t);
const footerOf = (rows: readonly SheetRow[], selection: Parameters<typeof footerOfIn>[1], state?: Parameters<typeof footerOfIn>[4]) => footerOfIn(rows, selection, t, "ko", state);
const layerLines = (workspace: Workspace) => layerLinesIn(workspace, t);
const resultLines = (cleanup: DiskCleanup) => resultLinesIn(cleanup, t);
const exclusionText = (row: CleanupRow) => exclusionTextIn(row, t);
const reasonText = (code: Parameters<typeof reasonTextIn>[0], count?: number | null) => reasonTextIn(code, count, t);
const cellReasonText = (code: Parameters<typeof cellReasonTextIn>[0]) => cellReasonTextIn(code, t);

function layers(build: number, deps: number, other = 0): DiskLayers {
  const cell = (bytes: number) => ({ bytes, folders: bytes > 0 ? 1 : 0, largest_name: bytes > 0 ? "x" : null });
  return { build_cache: cell(build), dependencies: cell(deps), other: cell(other), source_bytes: 10 * MB };
}

type Options = { main?: boolean; merged?: boolean; working?: number; build?: number; deps?: number; other?: number; unmeasured?: boolean; unavailable?: boolean };

function checkout(name: string, options: Options = {}): Checkout {
  const build = options.build ?? 2 * GB;
  const deps = options.deps ?? 300 * MB;
  const total = build + deps + (options.other ?? 0) + 10 * MB;
  const disk = options.unmeasured
    ? { total_bytes: null, unavailable_reason: null }
    : options.unavailable
      ? { total_bytes: null, unavailable_reason: "limit" }
      : { total_bytes: total, unavailable_reason: null, layers: layers(build, deps, options.other ?? 0) };
  return { agent_scope: { ...emptyScope(), has_working: (options.working ?? 0) > 0 },
    id: name,
    workspace_id: "p",
    label: name,
    path: `/r/${name}`,
    branch: name,
    purpose: null,
    is_worktree: !options.main,
    exists: true,
    has_panes: false,
    agent_summary: { working: options.working ?? 0 } as Checkout["agent_summary"],
    worktree: { is_main: options.main ?? false, merged: options.merged ?? null, disk } as Checkout["worktree"],
    landed: options.merged ?? false,
    pull_request: null,
    tabs: [],
    active_tab_id: null,
    strip: [],
    next_tab_label: "Tab 2",
  } as unknown as Checkout;
}

function core(path: string, extra: Partial<CleanupRow> = {}): CleanupRow {
  return { path, branch: null, head: null, is_main: false, exclusion_code: null, exclusion_count: null, in_use: null, pane_count: 0, result: null, result_code: null, bytes: null, ...extra };
}

function workspace(checkouts: Checkout[], cleanup: Partial<DiskCleanup> | null, rows: Record<string, Partial<CleanupRow>> = {}, id = "p"): Workspace {
  const review: DiskCleanup | null = cleanup
    ? {
        id: 1,
        workspace_id: id,
        repository_root: "/r",
        phase: "review",
        main_head: null,
        message: null,
        usage_error: null,
        usage_ready: true,
        free_bytes: null,
        progress: null,
        cell_results: [],
        free_before: null,
        free_after: null,
        ...cleanup,
        rows: checkouts.map((c) => core(c.path, { is_main: c.worktree?.is_main ?? false, exclusion_code: c.worktree?.is_main ? "main" : null, in_use: (c.agent_summary?.working ?? 0) > 0 ? { code: "agent_working", name: null, port: null } : null, ...rows[c.label] })),
      }
    : null;
  return { agent_scope: emptyScope(), id, label: id, path: "/r", device_id: "local", is_git: true, registered: true, temporary: false, pinned: false, checkouts, inactive_checkouts: { expanded: false, checkout_ids: [] }, cleanup: review } as Workspace;
}

function ready(checkouts: Checkout[], rows: Record<string, Partial<CleanupRow>> = {}): SheetRow[] {
  return sheetModel(workspace(checkouts, {}, rows), null).rows;
}

const merged = { merged: true };

describe("filters (B12, D-22)", () => {
  const rows = ready([
    checkout("main", { main: true }),
    checkout("done-one", merged),
    checkout("busy", { ...merged, working: 1 }),
    checkout("idle"),
  ], { idle: {}, "done-one": {} });

  it("splits into finished, resting and working, and main is never finished", () => {
    expect(filterCounts(rows)).toEqual({ all: 4, done: 1, resting: 2, working: 1 });
    expect(visibleRows(rows, "done").map((r) => r.label)).toEqual(["done-one"]);
    expect(visibleRows(rows, "resting").map((r) => r.label).sort()).toEqual(["idle", "main"]);
  });

  it("puts a checkout in use in Working even when its pull request finished", () => {
    expect(rows.find((r) => r.label === "busy")?.bucket).toBe("working");
  });

  it("takes an in-use signal from the core review, not only from the agent", () => {
    const withPort = ready([checkout("srv")], { srv: { in_use: { code: "port", name: null, port: 5173 } } });
    expect(withPort[0]?.bucket).toBe("working");
    expect(withPort[0]?.inUse).toBe("포트 5173 서버");
  });
});

describe("layout (B4, D-21)", () => {
  it("puts main first, the rest by size, and folds checkouts under 1 GB", () => {
    const rows = ready([
      checkout("main", { main: true, build: 1 * GB }),
      checkout("small-a", { build: 100 * MB, deps: 0 }),
      checkout("large", { build: 9 * GB }),
      checkout("mid", { build: 3 * GB }),
      checkout("small-b", { build: 200 * MB, deps: 0 }),
    ]);
    const layout = layoutRows(rows);
    expect(layout.main?.label).toBe("main");
    expect(layout.big.map((r) => r.label)).toEqual(["large", "mid"]);
    expect(layout.small.map((r) => r.label)).toEqual(["small-b", "small-a"]);
    expect(layout.smallBytes).toBe(layout.small.reduce((sum, r) => sum + (r.total ?? 0), 0));
    expect(layout.small.every((r) => (r.total ?? 0) < SMALL_CHECKOUT_BYTES)).toBe(true);
  });

  it("keeps an unmeasured checkout out of the fold", () => {
    const layout = layoutRows(ready([checkout("main", { main: true }), checkout("later", { unmeasured: true })]));
    expect(layout.big.map((r) => r.label)).toEqual(["later"]);
    expect(layout.small).toEqual([]);
  });
});

describe("cell availability (B5, B6, B14, B15, B25)", () => {
  it("is a skeleton with nothing selectable while the review has not answered", () => {
    const pending = sheetModel(workspace([checkout("a")], { phase: "loading", usage_ready: false }), null);
    expect(pending.state).toBe("pending");
    expect(pending.rows[0]?.cache.build_cache.selectable).toBe(false);
  });

  it("leaves an unmeasured row unselectable and an unreadable row named", () => {
    const model = ready([checkout("wait", { unmeasured: true }), checkout("big", { unavailable: true })]);
    expect(model.map((r) => r.measure)).toEqual(["pending", "unavailable"]);
    expect(model.every((r) => !r.cache.build_cache.selectable)).toBe(true);
    expect(model[1]?.cache.build_cache.why).toBe("크기를 재지 못함");
  });

  it("blocks a checkout in use with the reason and lets an open pane alone through, main included", () => {
    const rows = ready([checkout("main", { main: true }), checkout("run")], { run: { in_use: { code: "process", name: "cargo", port: null } } });
    expect(rows[0]?.cache.build_cache.selectable).toBe(true);
    expect(rows[1]?.cache.build_cache.selectable).toBe(false);
    expect(rows[1]?.cache.build_cache.why).toBe("터미널에서 cargo 실행 중");
    expect(rows[1]?.worktree?.why).toBe("터미널에서 cargo 실행 중");
  });

  it("has no worktree cell for main and gives the reason a worktree cannot be ticked", () => {
    const rows = ready([checkout("main", { main: true }), checkout("wip"), checkout("ok", merged)], {
      wip: { exclusion_code: "dirty", exclusion_count: 3 },
      ok: {},
    });
    expect(rows[0]?.worktree).toBeNull();
    expect(rows[1]?.worktree).toEqual({ selectable: false, why: "바뀐 파일 3", panes: 0 });
    expect(rows[2]?.worktree?.selectable).toBe(true);
  });

  it("makes nothing selectable when in-use could not be read, without a banner state of its own", () => {
    const model = sheetModel(workspace([checkout("a")], { usage_error: "Herdr is not connected" }), null);
    expect(model.state).toBe("unreadable");
    expect(model.rows[0]?.cache.build_cache.selectable).toBe(false);
    expect(model.rows[0]?.worktree?.selectable).toBe(false);
  });

  it("is busy while another project's cleanup removes", () => {
    const other = workspace([checkout("a")], { phase: "removing" }, {}, "other");
    const mine = workspace([checkout("a")], null);
    expect(cleanupElsewhere([other, mine], "p")).toBe("removing");
    expect(cleanupElsewhere([other], "other")).toBeNull();
    expect(sheetModel(mine, "removing").state).toBe("busy");
    expect(t(BUSY_KEY.removing)).toBe("다른 정리가 진행 중");
  });

  it("is busy while another project's review still reads, and ready again once that worker ends", () => {
    // The core refuses a review while a worker runs, so this project would wait on "검토하는 중…" for good without the busy state.
    const reading = workspace([checkout("a")], { phase: "loading", usage_ready: false }, {}, "other");
    const mine = workspace([checkout("a")], null);
    expect(cleanupElsewhere([reading, mine], "p")).toBe("loading");
    expect(sheetModel(mine, "loading").state).toBe("busy");
    expect(t(BUSY_KEY.loading)).toBe("다른 프로젝트를 검토하는 중");
    // The other review ends and stays open with nobody closing it: it no longer holds this project back.
    const finished = workspace([checkout("a")], { phase: "review" }, {}, "other");
    expect(cleanupElsewhere([finished, mine], "p")).toBeNull();
    const reviewed = workspace([checkout("a")], { phase: "review" });
    expect(sheetModel(reviewed, cleanupElsewhere([finished, reviewed], "p")).state).toBe("ready");
  });

  it("opens the cache cells once the in-use answer is in and the worktree cells only with the whole review", () => {
    const early = sheetModel(workspace([checkout("main", { main: true }), checkout("ok", merged)], { phase: "loading", usage_ready: true }), null);
    expect(early.state).toBe("pending");
    expect(early.rows[1]?.cache.build_cache.selectable).toBe(true);
    expect(early.rows[1]?.worktree?.selectable).toBe(false);
    const waiting = sheetModel(workspace([checkout("ok", merged)], { phase: "loading", usage_ready: false }), null);
    expect(waiting.rows[0]?.cache.build_cache.selectable).toBe(false);
  });

  it("draws the rows from the checkouts when the review failed, with the reason for the notice", () => {
    const failed = sheetModel(workspace([checkout("a"), checkout("b")], { phase: "failed", message: "no herdr", usage_error: "no herdr", usage_ready: false }), null);
    expect(failed.state).toBe("unreadable");
    expect(failed.rows).toHaveLength(2);
    expect(failed.rows.every((row) => !row.cache.build_cache.selectable && row.total !== null)).toBe(true);
  });

  it("leaves a layer with nothing in it unselectable", () => {
    const rows = ready([checkout("none", { deps: 0 })]);
    expect(rows[0]?.cache.dependencies.selectable).toBe(false);
    expect(rows[0]?.cache.build_cache.selectable).toBe(true);
  });
});

describe("selection (D-09, B10, B11)", () => {
  const rows = ready([
    checkout("main", { main: true }),
    checkout("a", merged),
    checkout("b", merged),
    checkout("run", { ...merged, working: 1 }),
  ]);
  const visible = rows;

  it("ticks a cell alone and reads the row checkbox as partial", () => {
    let selection = toggleCell(EMPTY_SELECTION, { path: "/r/a", column: "build_cache" });
    const row = bundleRefs(visible.filter((r) => r.label === "a"), selection, ["build_cache", "dependencies"]);
    expect(bundleState(selection, row)).toBe("indeterminate");
    selection = toggleBundle(selection, row);
    expect(bundleState(selection, row)).toBe("checked");
    selection = toggleBundle(selection, row);
    expect(bundleState(selection, row)).toBe("unchecked");
  });

  it("turns a partial bundle fully on, then off", () => {
    const column = bundleRefs(visible, EMPTY_SELECTION, ["dependencies"]);
    expect(column.map((ref) => ref.path)).toEqual(["/r/main", "/r/a", "/r/b"]);
    let selection = toggleCell(EMPTY_SELECTION, column[0]!);
    expect(bundleState(selection, column)).toBe("indeterminate");
    selection = toggleBundle(selection, column);
    expect(bundleState(selection, column)).toBe("checked");
    expect(toggleBundle(selection, column).cells.size).toBe(0);
  });

  it("skips a disabled cell in every bundle, so the row in use stays unticked", () => {
    const all = bundleRefs(visible, EMPTY_SELECTION, ["build_cache", "dependencies"]);
    expect(all.some((ref) => ref.path === "/r/run")).toBe(false);
    const selection = toggleBundle(EMPTY_SELECTION, all);
    expect(selection.cells.size).toBe(all.length);
    expect([...selection.cells].some((key) => key.startsWith("/r/run"))).toBe(false);
  });

  it("covers the worktree column separately and never through the top-left bundle", () => {
    const worktrees = bundleRefs(visible, EMPTY_SELECTION, ["worktree"]);
    expect(worktrees.map((ref) => ref.path)).toEqual(["/r/a", "/r/b"]);
    const topLeft = toggleBundle(EMPTY_SELECTION, bundleRefs(visible, EMPTY_SELECTION, ["build_cache", "dependencies"]));
    expect(topLeft.worktrees.size).toBe(0);
  });

  it("includes a row's caches once its worktree is ticked and gives them back when it is unticked", () => {
    let selection = toggleCell(EMPTY_SELECTION, { path: "/r/a", column: "build_cache" });
    selection = toggleCell(selection, { path: "/r/a", column: "worktree" });
    const cachesOfA = bundleRefs(visible.filter((r) => r.label === "a"), selection, ["build_cache", "dependencies"]);
    expect(cachesOfA).toEqual([]);
    expect(bundleState(selection, cachesOfA)).toBe("none");
    const plan = planOf(visible, selection);
    expect(plan.cells).toEqual([]);
    expect(plan.worktrees.map((w) => w.label)).toEqual(["a"]);
    // The total counts the folder once: its caches are part of it.
    expect(plan.bytes).toBe(visible.find((r) => r.label === "a")?.total);
    selection = toggleCell(selection, { path: "/r/a", column: "worktree" });
    expect(planOf(visible, selection).cells.map((c) => `${c.path}:${c.layer}`)).toEqual(["/r/a:build_cache"]);
  });

  it("drops a ticked cell that stopped being selectable from the plan", () => {
    const selection = toggleCell(EMPTY_SELECTION, { path: "/r/run", column: "build_cache" });
    expect(planOf(visible, selection).cells).toEqual([]);
  });

  it("has no worktree left to confirm when the one ticked turned in use meanwhile", () => {
    const before = ready([checkout("ok", merged)], { ok: {} });
    const selection = toggleCell(EMPTY_SELECTION, { path: "/r/ok", column: "worktree" });
    expect(needsConfirm(planOf(before, selection))).toBe(true);
    const after = ready([checkout("ok", merged)], { ok: { in_use: { code: "process", name: "vite", port: null } } });
    expect(planOf(after, selection).worktrees).toEqual([]);
    expect(needsConfirm(planOf(after, selection))).toBe(false);
  });

  it("counts the panes that close with a chosen worktree and keeps them out of a cache-only plan", () => {
    const panes = ready([checkout("finished", merged), checkout("empty", merged)], { finished: { pane_count: 2 }, empty: {} });
    expect(panes.map((row) => [row.label, row.worktree?.panes])).toEqual([["finished", 2], ["empty", 0]]);
    const both = toggleBundle(EMPTY_SELECTION, bundleRefs(panes, EMPTY_SELECTION, ["worktree"]));
    expect(planOf(panes, both).worktrees.map((w) => [w.label, w.panes])).toEqual([["finished", 2], ["empty", 0]]);
    const caches = toggleBundle(EMPTY_SELECTION, bundleRefs(panes, EMPTY_SELECTION, ["build_cache"]));
    expect(planOf(panes, caches).worktrees).toEqual([]);
  });

  it("chooses a worktree whose only panes are a finished agent's and a shell, not one with a waiting agent", () => {
    const rows = ready([checkout("finished", merged), checkout("blocked", merged)], { finished: { pane_count: 2 }, blocked: { pane_count: 1, in_use: { code: "agent_waiting", name: null, port: null } } });
    expect(rows.find((row) => row.label === "finished")?.worktree?.selectable).toBe(true);
    const blocked = rows.find((row) => row.label === "blocked");
    expect(blocked?.worktree?.selectable).toBe(false);
    expect(blocked?.bucket).toBe("working");
    expect(blocked?.inUse).toBe("에이전트가 입력을 기다림");
  });

  it("unticks what a filter hides (B12)", () => {
    let selection = toggleBundle(EMPTY_SELECTION, bundleRefs(visible, EMPTY_SELECTION, ["build_cache"]));
    selection = toggleCell(selection, { path: "/r/a", column: "worktree" });
    const kept = pruneSelection(selection, visibleRows(rows, "resting"));
    expect([...kept.cells]).toEqual(["/r/main\u0000build_cache"]);
    expect(kept.worktrees.size).toBe(0);
  });
});

describe("the folded small row (D-21, B10)", () => {
  const rows = ready([
    checkout("main", { main: true }),
    checkout("big", { build: 5 * GB }),
    checkout("s1", { build: 100 * MB, deps: 0 }),
    checkout("s2", { build: 200 * MB, deps: 0 }),
  ]);
  const layout = layoutRows(rows);

  it("ticks and clears its rows' caches as one, partial when only some are on, and never their worktrees", () => {
    const fold = bundleRefs(layout.small, EMPTY_SELECTION, ["build_cache", "dependencies"]);
    expect(fold.map((ref) => `${ref.path}:${ref.column}`)).toEqual(["/r/s2:build_cache", "/r/s1:build_cache"]);
    let selection = toggleCell(EMPTY_SELECTION, fold[0]!);
    expect(bundleState(selection, fold)).toBe("indeterminate");
    selection = toggleBundle(selection, fold);
    expect(bundleState(selection, fold)).toBe("checked");
    expect(selection.worktrees.size).toBe(0);
  });

  it("keeps the folded rows' selection when the fold opens, and the top-left bundle and footer include them", () => {
    const fold = bundleRefs(layout.small, EMPTY_SELECTION, ["build_cache"]);
    const selection = toggleBundle(EMPTY_SELECTION, fold);
    const all = bundleRefs(rows, selection, ["build_cache", "dependencies"]);
    expect(all.some((ref) => ref.path === "/r/s1")).toBe(true);
    expect(planOf(rows, selection).cells.filter((cell) => cell.layer === "build_cache")).toHaveLength(2);
  });
});

describe("footer (B13, B16, B17, B18)", () => {
  const rows = ready([
    checkout("main", { main: true, build: 0, deps: 0 }),
    checkout("done", { ...merged, build: 3 * GB, deps: 1 * GB }),
    checkout("done-2", { ...merged, build: 2 * GB, deps: 0 }),
  ]);

  it("says there is nothing to clear when no cell can be ticked", () => {
    const nothing = ready([checkout("main", { main: true, build: 0, deps: 0 })]);
    expect(footerOf(nothing, EMPTY_SELECTION)).toMatchObject({ empty: true, summary: "비울 캐시가 없어요" });
  });

  it("asks for a choice when cells exist but none is ticked", () => {
    expect(footerOf(rows, EMPTY_SELECTION)).toMatchObject({ empty: true, summary: "고른 항목 없음" });
  });

  it("says only how many cells and how much, with no explaining sentence", () => {
    const selection = toggleBundle(EMPTY_SELECTION, bundleRefs(rows, EMPTY_SELECTION, ["build_cache", "dependencies"]));
    expect(footerOf(rows, selection)).toEqual({ empty: false, summary: "항목 3개 · 6.0 GB" });
    expect(needsConfirm(planOf(rows, selection))).toBe(false);
  });

  it("says what it waits for instead of claiming there is nothing to clear while the review is out or unreadable", () => {
    const pending = sheetModel(workspace([checkout("a")], { phase: "loading", usage_ready: false }), null);
    expect(footerOf(pending.rows, EMPTY_SELECTION, pending.state).summary).toBe("검토하는 중…");
    const unreadable = sheetModel(workspace([checkout("a")], { usage_error: "no herdr" }), null);
    expect(footerOf(unreadable.rows, EMPTY_SELECTION, unreadable.state).summary).toBe("쓰는 중인지 확인해야 고를 수 있어요");
  });

  it("adds the worktree count only when a worktree is ticked, and asks the confirmation", () => {
    const one = toggleCell(EMPTY_SELECTION, { path: "/r/done", column: "worktree" });
    expect(footerOf(rows, one).summary).toBe("워크트리 1 · 4.0 GB");
    expect(needsConfirm(planOf(rows, one))).toBe(true);
    const withCache = toggleCell(one, { path: "/r/done-2", column: "build_cache" });
    expect(footerOf(rows, withCache).summary).toBe("항목 1개 · 워크트리 1 · 6.0 GB");
    const both = toggleBundle(EMPTY_SELECTION, bundleRefs(rows, EMPTY_SELECTION, ["worktree"]));
    expect(footerOf(rows, both).summary).toBe("워크트리 2 · 6.0 GB");
    expect(footerOf(rows, both)).not.toHaveProperty("destructive");
  });
});

describe("the entrance (B1, B2, B6, D-29)", () => {
  const measured = (free: number | null, checkouts: Checkout[]): Workspace => ({ ...workspace(checkouts, null), disk: { total_bytes: 50 * GB, unavailable_reason: null, measuring: false, free_bytes: free } });
  const layerTotals = { build_cache: 30 * GB, dependencies: 5 * GB, source: 3 * GB, other: 2 * GB, shared_git: 1 * GB };

  it("counts the caches of finished linked checkouts no agent works in", () => {
    const value = measured(1 * GB, [
      checkout("main", { main: true, build: 10 * GB }),
      checkout("done-a", { ...merged, build: 3 * GB, deps: 1 * GB }),
      checkout("done-busy", { ...merged, working: 1, build: 5 * GB }),
      checkout("open", { build: 7 * GB }),
    ]);
    expect(reclaimable(value)).toBe(4 * GB);
  });

  describe("with one checkout that could not be measured", () => {
    // The core leaves `total_bytes` out while any checkout is unavailable and states the subtotal, the layers and the free space of the rest.
    const partial: Workspace = {
      ...workspace([checkout("main", { main: true }), checkout("done-a", { ...merged, build: 3 * GB, deps: 1 * GB }), checkout("huge", { unavailable: true })], null),
      disk: { total_bytes: null, unavailable_reason: "over the limit", measuring: false, confirmed_bytes: 40 * GB, free_bytes: 1.6 * GB, layers: layerTotals },
    };

    it("keeps the entrance as a subtotal instead of removing it, and never as a total", () => {
      expect(entranceBytes(partial)).toEqual({ bytes: 40 * GB, partial: true });
      expect(entranceBytes({ ...partial, disk: { ...partial.disk!, total_bytes: 45 * GB } })).toEqual({ bytes: 45 * GB, partial: false });
      expect(entranceBytes({ ...partial, disk: { total_bytes: null, unavailable_reason: "x", measuring: false } })).toBeNull();
      expect(entranceBytes({ ...partial, disk: { ...partial.disk!, measuring: true } })).toBeNull();
    });

    it("keeps the layer tooltip in the PRD's words and the low-free warning from the measured rows", () => {
      expect(layerLines(partial)?.map((line) => [line.label, line.bytes])).toEqual([
        ["빌드 캐시", 30 * GB],
        ["의존성", 5 * GB],
        ["워크트리 소스", 3 * GB],
        ["기타 · 지우지 않음", 2 * GB],
        ["공유 Git", 1 * GB],
      ]);
      expect(reclaimable(partial)).toBe(4 * GB);
    });
  });
});

describe("reasons in the operator's words (B15, B22)", () => {
  it("words every exclusion code, and the count of changed files", () => {
    const codes: [CleanupExclusionCode, string][] = [
      ["locked", "잠김"],
      ["current", "지금 보고 있는 체크아웃"],
      ["pane_open", "이 체크아웃에 속하지 않은 터미널이 열려 있음"],
      ["dirty", "바뀐 파일 있음"],
      ["not_merged", "기본 브랜치에 머지되지 않음"],
      ["merge_unverified", "머지를 확인하지 못함"],
      ["nested_repository", "안에 다른 저장소가 있음"],
      ["detached", "분리된 HEAD (브랜치 없음)"],
      ["contains_worktree", "안에 다른 워크트리가 있음"],
      ["unverified", "확인하지 못함"],
      ["main_unavailable", "기본 브랜치를 읽을 수 없음"],
      ["alias", "폴더를 읽을 수 없음"],
      ["missing", "폴더를 읽을 수 없음"],
      ["unavailable", "폴더를 읽을 수 없음"],
      ["in_use", "쓰는 중"],
    ];
    for (const [code, text] of codes) expect(exclusionText(core("/r/x", { exclusion_code: code })), code).toBe(text);
    expect(exclusionText(core("/r/x", { exclusion_code: "dirty", exclusion_count: 3 }))).toBe("바뀐 파일 3");
    expect(exclusionText(core("/r/x", { exclusion_code: "main" }))).toBeNull();
    expect(exclusionText(core("/r/x"))).toBeNull();
  });

  it("words every result code of a worktree that stayed", () => {
    expect(reasonText("changed")).toBe("확인한 뒤에 바뀜");
    expect(reasonText("not_found")).toBe("이미 없음");
    expect(reasonText("unverified")).toBe("확인하지 못함");
    expect(reasonText("close_refused")).toBe("터미널을 닫지 못함");
    expect(reasonText("remove_refused")).toBe("Git이 워크트리를 지우지 못함");
    // A code this build does not know reads as a plain refusal, never as the raw code.
    expect(reasonText("from_a_newer_core" as CleanupExclusionCode)).toBe("지울 수 없음");
  });

  it("words every cell code the core sends and the in-use kinds", () => {
    const cells = { in_use: "확인한 뒤에 쓰는 중이 됨", changed: "확인한 뒤에 바뀜", tracked_files: "추적 파일이 있음", nested_repository: "안에 다른 저장소가 있음", symlink: "심볼릭 링크라 건너뜀", not_found: "이미 없음", unverified: "확인하지 못함", io: "폴더를 지우지 못함" } as const;
    for (const [code, text] of Object.entries(cells)) expect(cellReasonText(code as keyof typeof cells), code).toBe(text);
    const inUse = (code: CleanupInUse["code"], name: string | null = null, port: number | null = null) => exclusionText(core("/r/x", { in_use: { code, name, port } }));
    expect(inUse("agent_working")).toBe("에이전트 작업 중");
    expect(inUse("agent_waiting")).toBe("에이전트가 입력을 기다림");
    expect(inUse("descendant_busy")).toBe("자식 에이전트가 작업 중");
    expect(inUse("process", "cargo")).toBe("터미널에서 cargo 실행 중");
    expect(inUse("port", null, 5173)).toBe("포트 5173 서버");
    expect(inUse("unverified")).toBe("쓰는 중인지 확인하지 못함");
  });
});

describe("result (B22)", () => {
  // The core's real shape: codes and sizes only, no sentence anywhere.
  const cell = (path: string, layer: CleanupCellResult["layer"], outcome: CleanupCellResult["outcome"], bytes: number, folders: number, reason_code: CleanupCellResult["reason_code"]): CleanupCellResult => ({ path, layer, outcome, bytes, folders, reason_code });
  const cleanup: DiskCleanup = {
    id: 1,
    workspace_id: "p",
    repository_root: "/r",
    phase: "complete",
    main_head: null,
    message: null,
    usage_error: null,
    usage_ready: true,
    free_bytes: null,
    progress: null,
    free_before: 1.6 * GB,
    free_after: 18.3 * GB,
    rows: [
      core("/r/gone", { branch: "feat/gone", result: "removed", bytes: 4 * GB }),
      core("/r/kept", { branch: "feat/kept", result: "skipped", result_code: "changed" }),
      core("/r/dirty", { branch: "feat/dirty", result: "skipped", result_code: "dirty", exclusion_count: 2 }),
      core("/r/refused", { branch: "feat/refused", result: "failed", result_code: "remove_refused" }),
      core("/r/a", { branch: "feat/a" }),
    ],
    cell_results: [
      cell("/r/d", "build_cache", "removed", 1 * GB, 1, "tracked_files"),
      cell("/r/a", "build_cache", "removed", 8 * GB, 2, null),
      cell("/r/a", "dependencies", "removed", 1 * GB, 1, null),
      cell("/r/b", "build_cache", "skipped", 2 * GB, 1, "in_use"),
      cell("/r/c", "dependencies", "failed", 0, 1, "io"),
    ],
  };

  it("lists each removed, skipped and failed line from codes, and never invents a size for a skip", () => {
    const lines = resultLines(cleanup);
    const by = (key: string) => lines.find((line) => line.key === key);
    expect(by("cells:/r/a:removed")).toMatchObject({ label: "feat/a", what: "빌드 캐시 · 의존성", bytes: 9 * GB, reason: null });
    expect(by("cells:/r/b:skipped")).toMatchObject({ outcome: "skipped", reason: "확인한 뒤에 쓰는 중이 됨", bytes: null });
    expect(by("cells:/r/c:failed")).toMatchObject({ outcome: "failed", reason: "폴더를 지우지 못함" });
    // A cell that emptied some folders and kept one says so beside what it removed.
    expect(by("cells:/r/d:removed")).toMatchObject({ bytes: 1 * GB, reason: "추적 파일이 있음" });
  });

  it("names a removed worktree by its branch and sizes it from the core's bytes, so a reopened result matches", () => {
    const lines = resultLines(cleanup);
    const by = (key: string) => lines.find((line) => line.key === key);
    expect(by("worktree:/r/gone")).toMatchObject({ label: "feat/gone", outcome: "removed", bytes: 4 * GB, reason: null });
  });

  it("tells a skipped worktree from one Git refused", () => {
    const lines = resultLines(cleanup);
    const by = (key: string) => lines.find((line) => line.key === key);
    expect(by("worktree:/r/kept")).toMatchObject({ outcome: "skipped", reason: "확인한 뒤에 바뀜", bytes: null });
    expect(by("worktree:/r/dirty")).toMatchObject({ outcome: "skipped", reason: "바뀐 파일 2" });
    expect(by("worktree:/r/refused")).toMatchObject({ outcome: "failed", reason: "Git이 워크트리를 지우지 못함" });
  });

  it("words the same model in the interface language", () => {
    const rows = sheetModelIn(workspace([checkout("a", { working: 1 }), checkout("b")], { phase: "review", usage_ready: true }), null, english).rows;
    expect(rows[0]?.inUse).toBe("Agent is working");
    expect(footerOfIn(rows, EMPTY_SELECTION, english, "en").summary).toBe("No cells selected");
    expect(reasonTextIn("dirty", 3, english)).toBe("3 changed files");
    expect(reasonTextIn("dirty", 1, english)).toBe("1 changed file");
    expect(exclusionTextIn(core("/r/x", { in_use: { code: "port", name: null, port: 5173 } }), english)).toBe("Server on port 5173");
    expect(cellReasonTextIn("symlink", english)).toBe("Skipped a symbolic link");
  });
});
