import { describe, expect, it } from "vitest";
import "../chapters";
import chapter from "../chapters/day";
import type { CoreEvent } from "../world";
import { DAY_WORLD, ISSUE_KEY, LOGIN_SESSION, type DayModel } from "./day";

const doneAt = (id: string, model: DayModel) => chapter.steps.find((step) => step.id === id)!.done!(model);
const apply = (model: DayModel, event: CoreEvent) => DAY_WORLD.apply(model, event)!;
const frames = (model: DayModel) => DAY_WORLD.snapshot(model, Date.now()).frames!;
const search = (query: string): CoreEvent => ({ kind: "session_work_search", payload: { query } });
const open = (session = LOGIN_SESSION): CoreEvent => ({ kind: "session_record_open", payload: { session } });
const restore: CoreEvent = { kind: "agent_start_in_checkout", payload: { checkout_path: "/work/hide-practice", provider: "codex", request_id: "r1", resume_session_id: LOGIN_SESSION.session_id } };

describe("the start-of-day flow's script (UI_BEHAVIOR, Practice window)", () => {
  it("lists every past session before a word is typed, and only the ones that hold a typed word after", () => {
    const initial = DAY_WORLD.initial();
    expect(frames(apply(initial, search(""))).sessionWorkSearch!.results).toHaveLength(3);
    const found = apply(initial, search("login"));
    expect(doneAt("search", found)).toBe(true);
    expect(frames(found).sessionWorkSearch!.results.map((row) => row.session.session_id)).toEqual([LOGIN_SESSION.session_id]);
    expect(frames(apply(initial, search("zzz"))).sessionWorkSearch!.results).toEqual([]);
  });

  it("does not count the first letters of a word as finding it", () => {
    expect(doneAt("search", apply(DAY_WORLD.initial(), search("log")))).toBe(false);
  });

  it("opens a session's record, and brings the session back once as a tab of its own", () => {
    const opened = apply(DAY_WORLD.initial(), open());
    expect(doneAt("record", opened)).toBe(true);
    expect(frames(opened).sessionRecord!.record!.session.session_id).toBe(LOGIN_SESSION.session_id);
    expect(doneAt("restore", opened)).toBe(false);
    const back = apply(opened, restore);
    expect(doneAt("restore", back)).toBe(true);
    expect(DAY_WORLD.panes(back)).toContain("restored");
    expect(DAY_WORLD.snapshot(back, Date.now()).rest.task_operation).toMatchObject({ phase: "ready", request_id: "r1", agent_phase: "started" });
    expect(DAY_WORLD.apply(back, restore)).toBeNull();
  });

  it("lists the backlog's issues on the project and opens the one asked for", () => {
    const rest = DAY_WORLD.snapshot(DAY_WORLD.initial(), Date.now()).rest;
    expect(rest.navigator!.workspaces![0]!.tasks!.tasks.map((task) => task.key)).toContain(ISSUE_KEY);
    const read = apply(DAY_WORLD.initial(), { kind: "issue_detail_request", payload: { workspace_id: "hide-practice", task_key: ISSUE_KEY } });
    expect(doneAt("issue", read)).toBe(true);
    expect(DAY_WORLD.snapshot(read, Date.now()).rest.issue_work!.detail).toMatchObject({ task_key: ISSUE_KEY, phase: "ready" });
  });
});
