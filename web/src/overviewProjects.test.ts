// The Overview's Projects tab as data (PRD overview-device-tasks B47-B55):
// each row's 지금 counts, its disk cell as the sweep left it, what folds as
// quiet, the size order, and the top line. The expected answers are the
// PRD's Behaviors.

import { describe, expect, it } from "vitest";
import type { TaskState } from "./agentScope";
import { diskCell, diskSummary, nowCounts, nowParts, orderedRows, projectRows } from "./overviewProjects";
import type { BoardProject } from "./projectBoard";
import type { ProjectDisk, Workspace } from "./snapshot";

const GB = 1024 ** 3;

type Seed = { states?: TaskState[]; disk?: Partial<ProjectDisk> | null; merged?: { cache: number; deps: number; working?: boolean }[]; git?: boolean; backlog?: number };

function project(id: string, seed: Seed = {}): BoardProject {
  const merged = (seed.merged ?? []).map((one, index) => ({
    id: `${id}-m${index}`, is_worktree: true, is_primary: false, exists: true, landed: true,
    agent_scope: { has_working: one.working ?? false },
    worktree: { is_main: false, missing: false, disk: { layers: { build_cache: { bytes: one.cache }, dependencies: { bytes: one.deps } } } },
  }));
  const workspace = {
    id, label: id, is_git: seed.git ?? true,
    checkouts: [{ id: `${id}-main`, is_worktree: false, is_primary: true, exists: true, agent_scope: { has_working: false } }, ...merged],
    tasks: { source: null, tasks: Array.from({ length: seed.backlog ?? 0 }, (_, index) => ({ key: `local:/${id}#${index + 1}`, id: `L-${index + 1}`, title: "t", open: true, url: null })), overflow: false },
    disk: seed.disk === null ? undefined : { total_bytes: null, unavailable_reason: null, measuring: false, ...seed.disk },
    agent_scope: { tasks: { cards: (seed.states ?? []).map((state, index) => ({ root: `${id}-${index}`, state, members: [], checkouts: [], recency: "" })), needs_you: 0, recency: "" } },
  } as unknown as Workspace;
  return { workspace, agents: [], device: null };
}

describe("a row's 지금 (B48)", () => {
  it("counts Needs You with the blocked ones, working, waiting and finished, and the rest as resting", () => {
    expect(nowCounts(["needs_you", "blocked", "working", "waiting", "done", "resting", "cleanup"].map((state) => ({ state: state as TaskState })))).toEqual({ needs: 2, working: 1, waiting: 1, done: 1, resting: 2, total: 7 });
    expect(nowCounts([])).toEqual({ needs: 0, working: 0, waiting: 0, done: 0, resting: 0, total: 0 });
  });

  it("names only the counts that are not zero, in the cell's order", () => {
    expect(nowParts({ needs: 1, working: 2, waiting: 0, done: 0, resting: 3, total: 6 })).toEqual([{ key: "needs", value: 1 }, { key: "working", value: 2 }]);
    expect(nowParts({ needs: 0, working: 0, waiting: 1, done: 4, resting: 0, total: 5 })).toEqual([{ key: "waiting", value: 1 }, { key: "done", value: 4 }]);
    expect(nowParts({ needs: 0, working: 0, waiting: 0, done: 0, resting: 2, total: 2 })).toEqual([]);
  });

  it("carries the project's backlog count, and marks a project that is not a Git repository as a folder", () => {
    const [git, folder] = projectRows([project("git", { backlog: 3 }), project("folder", { git: false })]);
    expect(git).toMatchObject({ backlog: 3, folder: false });
    expect(folder).toMatchObject({ backlog: 0, folder: true });
  });
});

describe("처리할 일 and the quiet fold (B49, B50)", () => {
  it("counts the merged checkouts left on disk, and folds a resting project only when nothing is left to clean", () => {
    const [quiet, dirty, busy] = projectRows([project("quiet", { states: ["resting"] }), project("dirty", { states: ["resting"], merged: [{ cache: GB, deps: GB }] }), project("busy", { states: ["working"] })]);
    expect(quiet).toMatchObject({ merged: 0, quiet: true });
    expect(dirty).toMatchObject({ merged: 1, quiet: false });
    expect(busy).toMatchObject({ quiet: false });
  });

  it("keeps the device's order for the active rows and puts the quiet ones apart", () => {
    const rows = projectRows([project("q1"), project("a1", { states: ["working"] }), project("q2"), project("a2", { states: ["needs_you"] })]);
    const { active, quiet } = orderedRows(rows, false);
    expect(active.map((row) => row.project.id)).toEqual(["a1", "a2"]);
    expect(quiet.map((row) => row.project.id)).toEqual(["q1", "q2"]);
  });
});

describe("the size order (B52)", () => {
  it("stands every row largest first, quiet ones included, and a row not measured yet last", () => {
    const rows = projectRows([
      project("small", { states: ["working"], disk: { total_bytes: GB } }),
      project("unmeasured", { states: ["working"], disk: null }),
      project("quiet-big", { disk: { total_bytes: 9 * GB } }),
      project("mid", { states: ["working"], disk: { total_bytes: 4 * GB } }),
    ]);
    const { active, quiet } = orderedRows(rows, true);
    expect(active.map((row) => row.project.id)).toEqual(["quiet-big", "mid", "small", "unmeasured"]);
    expect(quiet).toEqual([]);
  });
});

describe("a disk cell as the sweep left it (B53, B55)", () => {
  it("has no number before the first measurement", () => {
    expect(diskCell(undefined)).toEqual({ bytes: null, measuring: false, queued: false, failure: null });
  });

  it("keeps the last size while it is measured again or waits for the sweep", () => {
    expect(diskCell({ total_bytes: 5 * GB, unavailable_reason: null, measuring: true })).toMatchObject({ bytes: 5 * GB, measuring: true });
    expect(diskCell({ total_bytes: 5 * GB, unavailable_reason: null, measuring: false, queued: true })).toMatchObject({ bytes: 5 * GB, queued: true });
  });

  it("shows the measured part while a whole total is not known, and the reason a measurement failed beside the last size", () => {
    expect(diskCell({ total_bytes: null, confirmed_bytes: 2 * GB, unavailable_reason: null, measuring: false }).bytes).toBe(2 * GB);
    expect(diskCell({ total_bytes: 3 * GB, unavailable_reason: "timed_out", measuring: false })).toMatchObject({ bytes: 3 * GB, failure: "timed_out" });
  });
});

describe("the top line (B47)", () => {
  it("sums the measured projects and what a cleanup frees, with the device's free and total space", () => {
    const summary = diskSummary(projectRows([
      project("a", { disk: { total_bytes: 4 * GB, free_bytes: 200 * GB, volume_total_bytes: 500 * GB }, merged: [{ cache: GB, deps: 2 * GB }] }),
      project("b", { disk: { total_bytes: 6 * GB, free_bytes: 200 * GB, volume_total_bytes: 500 * GB } }),
      project("c", { disk: null }),
    ]));
    expect(summary).toEqual({ free: 200 * GB, total: 500 * GB, sum: 10 * GB, reclaimable: 3 * GB, low: false, measuring: false });
  });

  it("does not count a merged worktree an agent still works in toward what a cleanup frees", () => {
    expect(diskSummary(projectRows([project("a", { merged: [{ cache: GB, deps: GB, working: true }] })])).reclaimable).toBe(0);
  });

  it("warns below 10 GB free and not at it", () => {
    expect(diskSummary(projectRows([project("a", { disk: { free_bytes: 10 * GB - 1 } })])).low).toBe(true);
    expect(diskSummary(projectRows([project("a", { disk: { free_bytes: 10 * GB } })])).low).toBe(false);
  });

  it("has no sum before any project is measured, and says it is measuring while one is measured or waits", () => {
    const summary = diskSummary(projectRows([project("a", { disk: { measuring: true } }), project("b", { disk: null })]));
    expect(summary).toMatchObject({ free: null, total: null, sum: null, measuring: true });
    expect(diskSummary(projectRows([project("a", { disk: { queued: true } })])).measuring).toBe(true);
  });
});
