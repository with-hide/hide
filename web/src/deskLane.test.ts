import { describe, expect, it } from "vitest";
import { activeTab, afterKeep, backFromThread, closeLane, namesView, closeTab, DESK_LANE_CLOSED, openMission, openThread, outputTab, showTab, toggleLane, toggleOutputs, toggleThreads, viewTabs } from "./deskLane";
import type { DeskKeptTab, DeskOutput } from "./snapshot";

// Expected answers are PRD session-docs-and-assistant B28-B35 and the
// project lane's rules UI_BEHAVIOR.md (The right lane) gives Desk's lane.

const output = (id: string): DeskOutput => ({ id, kind: "image", label: `${id}.png`, thumb: null, seen: false, pr_state: null, device_id: "local", checkout: "/repo", path: `/repo/${id}.png`, url: null });
const NONE: DeskKeptTab[] = [];

describe("Desk's lane", () => {
  it("opens the column a rail button names alone from a closed lane, and beside the other once it is up", () => {
    const threads = toggleThreads(DESK_LANE_CLOSED);
    expect([threads.up, threads.tool, threads.view]).toEqual([true, { kind: "threads" }, false]);
    const both = toggleOutputs(threads);
    expect([both.tool, both.view]).toEqual([{ kind: "threads" }, true]);
    // Pressed while it shows, a button closes its column alone; the last one takes the lane down.
    const view = toggleThreads(both);
    expect([view.up, view.tool, view.view]).toEqual([true, null, true]);
    expect(toggleOutputs(view).up).toBe(false);
  });

  it("brings back the columns ⌘E took down, and the Threads list when it had none", () => {
    const both = toggleOutputs(openThread(DESK_LANE_CLOSED, "job-a"));
    const down = closeLane(both);
    expect(down.up).toBe(false);
    expect(toggleLane(down)).toEqual({ ...both, up: true });
    expect(toggleLane(DESK_LANE_CLOSED).tool).toEqual({ kind: "threads" });
  });

  it("replaces the preview with the next output pressed and leaves a kept tab where it is", () => {
    const kept = [outputTab("job-a", output("kept"))];
    const first = showTab(DESK_LANE_CLOSED, kept, outputTab("job-a", output("one")));
    const second = showTab(first, kept, outputTab("job-b", output("two")));
    expect(viewTabs(kept, second).map(({ tab, preview }) => [tab.kind === "output" && tab.output.id, preview])).toEqual([["kept", false], ["two", true]]);
    const back = showTab(second, kept, kept[0]!);
    expect(activeTab(kept, back)).toBe(kept[0]);
    expect(back.preview).toBe(second.preview);
  });

  it("stops calling a tab the preview once the core keeps it", () => {
    const tab = outputTab("job-a", output("one"));
    const shown = showTab(DESK_LANE_CLOSED, NONE, tab);
    const kept = afterKeep(shown, tab);
    expect(kept.preview).toBeNull();
    expect(viewTabs([tab], kept)).toEqual([{ tab, preview: false }]);
  });

  it("closes the 보기 column with its last tab and the lane with its last column", () => {
    const tab = outputTab("job-a", output("one"));
    const shown = showTab(DESK_LANE_CLOSED, NONE, tab);
    expect(closeTab(shown, NONE, tab).up).toBe(false);
    const beside = showTab(openThread(DESK_LANE_CLOSED, "job-a"), NONE, tab);
    const left = closeTab(beside, NONE, tab);
    expect([left.up, left.view, left.tool]).toEqual([true, false, { kind: "thread", job: "job-a", conductor: null }]);
  });

  it("opens a mission's map beside its Conductor's thread, a node's thread in its place, and the path back to the Conductor", () => {
    const mission = openMission(DESK_LANE_CLOSED, NONE, { id: "mission-1", title: "Desk 1차" }, "conductor");
    expect(activeTab(NONE, mission)).toEqual({ kind: "mission", mission: "mission-1", title: "Desk 1차" });
    expect(mission.tool).toEqual({ kind: "thread", job: "conductor", conductor: null });
    const worker = openThread(mission, "worker", { conductor: "conductor" });
    expect(activeTab(NONE, worker)).toEqual(activeTab(NONE, mission));
    expect(backFromThread(worker).tool).toEqual({ kind: "thread", job: "conductor", conductor: null });
    expect(backFromThread(openThread(DESK_LANE_CLOSED, "job-a")).tool).toEqual({ kind: "threads" });
  });

  it("asks for the thread's input box once per 답하기 and never for another thread", () => {
    const answered = openThread(DESK_LANE_CLOSED, "job-a", { answer: true });
    expect(answered.answer).toEqual({ job: "job-a", seq: 1 });
    expect(openThread(answered, "job-a", { answer: true }).answer).toEqual({ job: "job-a", seq: 2 });
    expect(openThread(answered, "job-b").answer).toBeNull();
  });

  it("finds the View a thread's file names, as a path, from its folder, or with a Windows device's backslashes", () => {
    const posix = { ...output("notes"), path: "/repo/docs/notes.md" };
    expect([namesView(posix, "/repo/docs/notes.md"), namesView(posix, "/repo/./docs/x/../notes.md"), namesView(posix, "docs/notes.md"), namesView(posix, "./notes.md")]).toEqual([true, true, true, true]);
    expect([namesView(posix, "/other/docs/notes.md"), namesView(posix, "otes.md"), namesView(posix, "../notes.md"), namesView(posix, "docs\\notes.md")]).toEqual([false, false, false, false]);
    const windows = { ...output("notes"), path: "C:/repo/docs/notes.md" };
    expect([namesView(windows, "C:\\repo\\docs\\notes.md"), namesView(windows, "c:/repo/docs/notes.md"), namesView(windows, "docs\\notes.md")]).toEqual([true, true, true]);
  });
});
