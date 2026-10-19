// The Overview's 백로그 tab as data (PRD overview-device-tasks B26-B30, B39):
// which items stand in a project's backlog and in which order, what a pending
// add shows until its source lists it, what the search and the labels keep,
// what the input keeps of a title, and where an item goes. The expected
// answers are the PRD's Behaviors.

import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { english } from "./i18n/catalogs";
import { visibleBacklogProjects } from "./BacklogTab";
import { BACKLOG_FILTERS, BACKLOG_SHOWN, BACKLOG_TITLE_LIMIT, backlogIntent, backlogLabels, backlogOf, backlogTitle, destinationOf, filterRows, pendingItems, titleLeft, undoRetryable } from "./overviewBacklog";
import type { BoardProject } from "./projectBoard";
import type { BacklogFailure, BacklogItem, Task, TaskSource, Workspace } from "./snapshot";

function task(key: string, id: string, extra: Partial<Task> = {}): Task {
  return { key, source: key.startsWith("local:") ? "local" : "github", id, url: null, title: `Title ${id}`, open: true, ...extra } as Task;
}

function workspace(id: string, tasks: Task[], workedOn: string[] = []): Workspace {
  return {
    id,
    checkouts: workedOn.map((key, index) => ({ id: `${id}-c${index}`, task_key: key })),
    tasks: { source: { kind: "github", label: "GitHub", name: "acme/app", reading: false, failure: null, last_read_at_unix_ms: null }, tasks, overflow: false },
  } as unknown as Workspace;
}

function item(intent: string, phase: BacklogItem["phase"], at: number, extra: Partial<BacklogItem> = {}): BacklogItem {
  return { intent, workspace_id: "p", title: intent, phase, task_key: null, failure: null, retry: false, missed: null, at_unix_ms: at, ...extra };
}

describe("a project's backlog (B27, B28)", () => {
  it("keeps the open items no checkout works on, newest first, and counts the started ones apart", () => {
    const backlog = backlogOf(workspace("p", [
      task("github:acme/app#1", "#1", { created_at_unix_ms: 100 }),
      task("github:acme/app#2", "#2", { created_at_unix_ms: 300 }),
      task("github:acme/app#3", "#3", { created_at_unix_ms: 200 }),
      task("github:acme/app#4", "#4", { created_at_unix_ms: 400, open: false }),
    ], ["github:acme/app#2"]));
    expect(backlog.rows.map((row) => row.token)).toEqual(["#3", "#1"]);
    expect(backlog.started).toBe(1);
  });

  it("orders by when an item was made, falling back to its last change, and puts an undated one last", () => {
    const backlog = backlogOf(workspace("p", [
      task("local:/p#1", "L-1"),
      task("local:/p#2", "L-2", { updated_at_unix_ms: 50 }),
      task("local:/p#3", "L-3", { created_at_unix_ms: 10, updated_at_unix_ms: 999 }),
    ]));
    expect(backlog.rows.map((row) => [row.token, row.at])).toEqual([["L-2", 50], ["L-3", 10], ["L-1", null]]);
    expect(backlog.rows.every((row) => row.local)).toBe(true);
  });

  it("has no source and no rows while the project was never read", () => {
    expect(backlogOf({ checkouts: [], tasks: undefined } as unknown as Workspace)).toEqual({ source: null, rows: [], started: 0 });
  });
});

describe("what an add shows until its source lists it (B29, B30)", () => {
  const listed = workspace("p", [task("github:acme/app#9", "#9")]);

  it("keeps the waiting, the creating and the failed rows of the project, newest first", () => {
    const shown = pendingItems([item("old", "failed", 1), item("new", "creating", 3), item("mid", "waiting", 2), item("elsewhere", "waiting", 4, { workspace_id: "other" })], listed);
    expect(shown.map((one) => one.intent)).toEqual(["new", "mid", "old"]);
  });

  it("keeps a made item until the source lists it, so its row never blinks out", () => {
    expect(pendingItems([item("made", "created", 1, { task_key: "github:acme/app#10" })], listed).map((one) => one.intent)).toEqual(["made"]);
    expect(pendingItems([item("made", "created", 1, { task_key: "github:acme/app#9" })], listed)).toEqual([]);
  });

  it("shows nothing when the core holds no add", () => {
    expect(pendingItems(undefined, listed)).toEqual([]);
  });
});

describe("the search and the label chips (B27)", () => {
  const rows = backlogOf(workspace("p", [
    task("github:acme/app#1", "#1", { title: "Fix the login race", labels: [{ name: "bug", color: null }] }),
    task("github:acme/app#2", "#2", { title: "Write docs", labels: [{ name: "docs", color: null }] }),
    task("github:acme/app#30", "#30", { title: "Speed up the build", labels: [{ name: "bug", color: null }, { name: "perf", color: null }] }),
  ])).rows;

  it("finds by a word of the title or the token, ignoring case and outer spaces", () => {
    expect(filterRows(rows, "  LOGIN ", []).map((row) => row.token)).toEqual(["#1"]);
    expect(filterRows(rows, "#30", []).map((row) => row.token)).toEqual(["#30"]);
    expect(filterRows(rows, "", []).length).toBe(3);
  });

  it("keeps a row that carries any one picked label, together with the search", () => {
    expect(filterRows(rows, "", ["perf", "docs"]).map((row) => row.token).sort()).toEqual(["#2", "#30"]);
    expect(filterRows(rows, "build", ["bug"]).map((row) => row.token)).toEqual(["#30"]);
  });

  it("offers each label of the device's backlogs once, by name", () => {
    const other = workspace("q", [task("github:acme/other#5", "#5", { labels: [{ name: "bug", color: "#ff0000" }, { name: "a11y", color: null }] })]);
    const projects = [{ workspace: workspace("p", rows.map((row) => task(row.key, row.token!, { labels: [...row.labels] }))), agents: [], device: null }, { workspace: other, agents: [], device: null }] as BoardProject[];
    expect(backlogLabels(projects).map((label) => label.name)).toEqual(["a11y", "bug", "docs", "perf"]);
  });
});

describe("mixed source filters before the 50-row page", () => {
  const mixed = workspace("p", [
    ...Array.from({ length: 55 }, (_, index) => task(`github:acme/app#${index + 1}`, `#${index + 1}`, { updated_at_unix_ms: index, author: "mira", assignees: ["sam"] })),
    task("local:/p#1", "L-1", { open: false, author: null }),
    task("github:acme/app#80", "#80", { author: "lee", assignees: ["kim"], labels: [{ name: "bug", color: null }] }),
  ], ["github:acme/app#80"]);

  it("keeps closed memos and active GitHub items accessible without key collisions", () => {
    const rows = backlogOf(mixed, true).rows;
    expect(filterRows(rows, "", [], { ...BACKLOG_FILTERS, source: "local", status: "all" }).map((row) => row.token)).toEqual(["L-1"]);
    expect(filterRows(rows, "author:lee assignee:kim label:bug", [], { ...BACKLOG_FILTERS, status: "started" }).map((row) => row.token)).toEqual(["#80"]);
    expect(filterRows(rows, "", [], { ...BACKLOG_FILTERS, status: "closed" }).map((row) => row.token)).toEqual(["L-1"]);
  });

  it("filters across loaded pages before taking the first 50 results", () => {
    const rows = backlogOf(mixed, true).rows;
    const filtered = filterRows(rows, "#1", [], { ...BACKLOG_FILTERS, author: "mira", assignee: "sam" });
    expect(filtered.map((row) => row.token).sort()).toEqual(["#1", ...Array.from({ length: 10 }, (_, index) => `#${index + 10}`)].sort());
    expect(filterRows(rows, "", [], BACKLOG_FILTERS).slice(0, BACKLOG_SHOWN)).toHaveLength(50);
  });
});

describe("empty source states in all-project scope (B18, B25)", () => {
  const failed = workspace("failed", []);
  failed.tasks!.sources = [{ ...failed.tasks!.source!, failure: "GitHub is offline" }];
  const reading = workspace("reading", []);
  reading.tasks!.sources = [{ ...reading.tasks!.source!, reading: true }];
  const healthyEmpty = workspace("empty", []);
  const memo = workspace("memo", [task("local:/memo#1", "L-1")]);
  memo.tasks!.source = { ...memo.tasks!.source!, kind: "local", label: "Local", name: null };
  const pending = workspace("pending", []);
  const projects = [failed, reading, healthyEmpty, memo, pending];
  const making = [item("creating", "creating", 1, { workspace_id: "pending" })];

  it("keeps failed and loading sources beside another project's healthy memo, omitting healthy empty bands", () => {
    expect(visibleBacklogProjects(projects, "", [], BACKLOG_FILTERS, making).map((project) => project.id)).toEqual(["failed", "reading", "memo", "pending"]);
  });

  it("keeps GitHub failure and loading states through the GitHub filter, but not the memo filter", () => {
    expect(visibleBacklogProjects(projects, "", [], { ...BACKLOG_FILTERS, source: "github" }, undefined).map((project) => project.id)).toEqual(["failed", "reading"]);
    expect(visibleBacklogProjects(projects, "", [], { ...BACKLOG_FILTERS, source: "local" }, undefined).map((project) => project.id)).toEqual(["memo"]);
  });

  it("preserves source states and pending operations when item filters match nothing", () => {
    expect(visibleBacklogProjects(projects, "missing", ["unknown"], { ...BACKLOG_FILTERS, status: "closed" }, making).map((project) => project.id)).toEqual(["failed", "reading", "pending"]);
  });
});

describe("the title the input keeps (B29, B42)", () => {
  it("joins pasted lines into one line", () => {
    expect(backlogTitle("first line\r\n  second line\nthird")).toBe("first line second line third");
  });

  it("stops at 256 characters, a character outside the basic plane counting once", () => {
    const long = "가".repeat(BACKLOG_TITLE_LIMIT + 20);
    expect(Array.from(backlogTitle(long))).toHaveLength(256);
    const emoji = "🙂".repeat(300);
    expect(Array.from(backlogTitle(emoji))).toHaveLength(256);
    expect(titleLeft("🙂".repeat(250))).toBe(6);
    expect(titleLeft("")).toBe(256);
  });
});

describe("where an item goes (B26, B39)", () => {
  const source = (extra: Partial<TaskSource>): TaskSource => ({ kind: "github", label: "GitHub", name: "acme/app", reading: false, failure: null, last_read_at_unix_ms: null, ...extra });

  it("says public, private, or may be public for GitHub, and this device only for a memo or no source", () => {
    expect(destinationOf(source({ public: true }))).toBe("public");
    expect(destinationOf(source({ public: false }))).toBe("private");
    expect(destinationOf(source({}))).toBe("unknown");
    expect(destinationOf(source({ kind: "local", label: "Local", name: null }))).toBe("local");
    expect(destinationOf(null)).toBe("local");
  });
});

describe("an add's intent (D-34)", () => {
  it("is new for every add and safe to carry in a URL or a JSON key", () => {
    const intents = new Set(Array.from({ length: 200 }, () => backlogIntent()));
    expect(intents.size).toBe(200);
    for (const intent of intents) expect(intent).toMatch(/^[A-Za-z0-9_-]{18}$/);
  });
});

describe("a failed undo", () => {
  const failures: BacklogFailure[] = JSON.parse(fs.readFileSync(path.resolve(__dirname, "../../contracts/snapshot-wire-enums.json"), "utf8")).backlog_failure;

  it("offers 다시 시도 for every failure but a refusal (principle: only the policy refusal is final)", () => {
    expect(failures.filter((failure) => !undoRetryable(failure))).toEqual(["refused"]);
    expect(undoRetryable("unavailable")).toBe(true);
    expect(undoRetryable(null)).toBe(false);
  });

  it("names every failure the core writes", () => {
    expect(failures.filter((failure) => !(`backlog.failure.${failure}` in english))).toEqual([]);
  });
});
