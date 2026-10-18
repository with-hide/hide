import { expect, it } from "vitest";
import type { MissedWork, SessionRecord, SessionWorkSummary, WorkRequest, WorkTask } from "./snapshot";
import { checklist, openMissedWork, outputPrState, recordTitle, taskState } from "./sessionWork";

const missed = (id: string, at: number, status: MissedWork["status"] = "open"): MissedWork => ({ id, kind: "problem", text: id, status, at_unix_ms: at, turn_offset: 1, issue_repository: null, issue_number: null, creating: false, backlog_key: null });
const work = (id: string, cwd: string, items: MissedWork[]): SessionWorkSummary => ({
  session: { device_id: "local", agent: "claude", session_id: id }, title: "Current goal", fallback_title: "Claude", native_title: null, manual_title: null,
  cwd, ended_at_unix_ms: null, open_missed: items.filter((item) => item.status === "open").length, subtree_open_missed: 0, missed: items,
});

it("collects every open missed item, of live and ended sessions alike, newest first", () => {
  const rows = [work("live", "/repo", [missed("a", 1), missed("handled", 9, "resolved")]), work("ended", "/repo/sub", [missed("b", 3)]), work("elsewhere", "/other", [missed("c", 5)])];
  expect(openMissedWork(rows).map(({ item }) => item.id)).toEqual(["c", "b", "a"]);
  expect(openMissedWork([work("none", "/repo", [missed("done", 1, "dismissed")])])).toEqual([]);
});

it("titles a record by its manual name, else the model's goal or the native name", () => {
  const row = work("s", "/repo", []);
  expect(recordTitle(row, true)).toBe("Current goal");
  expect(recordTitle(row, false)).toBe("Claude");
  expect(recordTitle({ ...row, manual_title: "Mine" }, true)).toBe("Mine");
});

const SESSION = { device_id: "local", agent: "claude", session_id: "root" };
const task = (id: string, chapter: string, at: number, done = false, end: WorkTask["end"] = "done"): WorkTask => ({ id, chapter_id: chapter, name: `task ${id}`, at_unix_ms: at, last_at_unix_ms: at, end, done, unjudged: false });
const request = (offset: number, at: number, task_ids: string[], judged = true): WorkRequest => ({ offset, at_unix_ms: at, text: `request ${offset}`, author: "operator", task_ids, chapter_id: null, judged });
const record = (patch: Partial<SessionRecord>): SessionRecord => ({
  session: SESSION, title: "Goal", fallback_title: "Claude", native_title: null, manual_title: null, state: "idle", last_end: null, cwd: null, path: null, started_at_unix_ms: 0, ended_at_unix_ms: null,
  chapters: [], tasks: [], requests: [], missed: [], outputs: [], children: [], ...patch,
});

it("lists tasks first-asked first, whatever chapters they fall in, and counts the finished ones", () => {
  const list = checklist(record({
    tasks: [task("late", "b", 30, false, "unfinished"), task("early", "a", 10, true)],
    chapters: [{ id: "b", goal: "Second", at_unix_ms: 20 }, { id: "a", goal: "First", at_unix_ms: 0 }],
  }));
  expect(list.tasks.map((item) => item.task.id)).toEqual(["early", "late"]);
  expect([list.done, list.total]).toEqual([1, 2]);
});

const child = (id: string, started: number | null): SessionRecord => record({ session: { ...SESSION, session_id: id }, title: id, started_at_unix_ms: started });

it("hangs each child under the task whose time it started in, a request's tasks counting as their first, and the rest at the end", () => {
  const list = checklist(record({
    tasks: [task("task-1", "c", 10), task("task-1.2", "c", 10), task("task-4", "c", 40)],
    children: [child("late", 45), child("early", 5), child("in-split", 20), child("unknown", null), child("at-start", 10), child("at-next", 40)],
  }));
  expect(list.tasks.map((item) => item.children.map((one) => one.session.session_id))).toEqual([["at-start", "in-split"], [], ["at-next", "late"]]);
  expect(list.delegated.map((one) => one.session.session_id)).toEqual(["early", "unknown"]);
});

it("hangs a child under the task its spawn turn worked on, over the newer task its start time falls in (PRD task-continuation B7)", () => {
  const list = checklist(record({
    tasks: [task("task-1", "c", 10), task("task-1.2", "c", 10), task("task-4", "c", 40)],
    children: [{ ...child("spawned-in-letter-turn", 50), spawn_task: "task-1" }, { ...child("by-time", 45), spawn_task: null }, { ...child("gone-task", 5), spawn_task: "task-9" }],
  }));
  expect(list.tasks.map((item) => item.children.map((one) => one.session.session_id))).toEqual([["spawned-in-letter-turn"], [], ["by-time"]]);
  expect(list.delegated.map((one) => one.session.session_id)).toEqual(["gone-task"]);
});

it("reads an unjudged task that is not done as unjudged, while PR facts and done still win (PRD task-continuation D-09)", () => {
  const unjudged = { ...task("t1", "c", 10, false, "unfinished"), unjudged: true };
  expect(taskState(unjudged, [])).toBe("unjudged");
  expect(taskState({ ...unjudged, done: true }, [])).toBe("done");
  const open = { kind: "pr" as const, repository: "a/b", number: 1, title: "", url: "", state: "open", stale: false, last_read_at_unix_ms: null, created_at_unix_ms: 0, task_id: "t1", source: SESSION, source_name: "" };
  expect(taskState(unjudged, [open])).toBe("review");
});

it("lists every child at the end of a record with no task", () => {
  const list = checklist(record({ children: [child("b", 20), child("a", 10)] }));
  expect(list.delegated.map((one) => one.session.session_id)).toEqual(["b", "a"]);
});

it("keeps only open missed work, named by the task its turn's request carries, the descendants' last", () => {
  const own = (id: string, turn_offset: number, status: MissedWork["status"] = "open"): MissedWork => ({ ...missed(id, turn_offset, status), turn_offset });
  const below = record({ session: { ...SESSION, session_id: "child" }, title: "Child", missed: [missed("from-child", 2), missed("child-handled", 3, "backlog")] });
  const list = checklist(record({
    tasks: [task("t1", "c", 10), task("t2", "c", 20)],
    requests: [request(1, 10, ["t1"]), request(2, 20, ["t2"]), request(3, 30, [])],
    missed: [own("under-t2", 2), own("no-task-turn", 3), own("dismissed", 2, "dismissed"), own("resolved", 1, "resolved")],
  }), [below]);
  expect(list.missed.map(({ missed: one, task: name, named }) => [one.id, name, named])).toEqual([["under-t2", "task t2", false], ["no-task-turn", null, false], ["from-child", null, true]]);
});

it("gives each task the request that asked for it and shows the newest unjudged request only while working", () => {
  const requests = [request(1, 10, ["t1"]), request(2, 20, []), request(3, 30, [], false)];
  const base = { tasks: [task("t1", "c", 10)], requests };
  const working = checklist(record({ ...base, state: "working" }));
  expect(working.tasks[0]!.request?.offset).toBe(1);
  expect(working.latest?.offset).toBe(3);
  expect(checklist(record({ ...base, state: "idle" })).latest).toBeNull();
});

it("maps a recorded PR state to the PR mark's state", () => {
  expect(["ci_failed", "review", "merged", "closed"].map(outputPrState)).toEqual(["failed", "pending", "merged", "closed"]);
});

it("hangs a split request's pull requests and children once, from its first task, and gives its tasks one state", () => {
  const pr = (number: number, task_id: string, state: string) => ({ kind: "pr" as const, repository: "acme/app", number, title: `#${number}`, url: `u${number}`, state, stale: false, last_read_at_unix_ms: null, created_at_unix_ms: 1, task_id, source: SESSION, source_name: "root" });
  const list = checklist(record({
    tasks: [task("task-3", "c", 10, false, "waiting"), task("task-3.2", "c", 10, false, "waiting"), task("task-3.3", "c", 10, false, "waiting"), task("task-9", "c", 90)],
    requests: [request(3, 10, ["task-3", "task-3.2", "task-3.3"]), request(5, 50, ["task-3", "task-3.2", "task-3.3"]), request(9, 90, ["task-9"])],
    outputs: [pr(7, "task-3", "open")],
    children: [child("helper", 50)],
  }));
  const items = list.tasks;
  expect(items.map((item) => item.task.id)).toEqual(["task-3", "task-3.2", "task-3.3", "task-9"]);
  expect(items.map((item) => item.request?.offset)).toEqual([3, 3, 3, 9]);
  expect(items.map((item) => item.prs.map((one) => one.number))).toEqual([[7], [], [], []]);
  expect(items.map((item) => item.children.length)).toEqual([1, 0, 0, 0]);
  // The open pull request on the group's first task is the whole group's.
  expect(items.map((item) => item.state)).toEqual(["review", "review", "review", "unfinished"]);
});
