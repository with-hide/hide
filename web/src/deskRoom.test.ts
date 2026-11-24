import { describe, expect, it } from "vitest";
import { activeTab, closePanel, closeSession, closeTab, DESK_ROOM_CLOSED, keepTab, namesView, openSession, openTab, panelPlace, roomWidths, tabKey, tabNames, toggleExplorer, type DeskTab } from "./deskRoom";
import type { DeskOutput } from "./snapshot";

// Expected answers are PRD desk-2-ui D-05, D-08, D-14, D-15, D-22-D-25 and
// B16-B26, B35-B40, and docs/UI_BEHAVIOR.md, Desk.

const output = (id: string, kind: DeskOutput["kind"] = "image"): DeskOutput => ({ id, kind, label: `${id}.png`, thumb: null, seen: false, pr_state: null, device_id: "local", checkout: "/repo", path: `/repo/${id}.png`, url: null });
const file = (path: string, root = "/repo"): DeskTab => ({ kind: "file", device_id: "local", root, path: `${root}/${path}` });
const mission: DeskTab = { kind: "mission", mission: "m-39", title: "1.3.0 원격 안정화" };
const pr: DeskTab = { kind: "output", output: { ...output("pr-2911", "pr"), label: "#2911", url: "https://github.com/o/r/pull/2911", path: null } };
const keys = (room: ReturnType<typeof openTab>) => room.panel?.tabs.map(tabKey) ?? [];

describe("the tab panel's open contract", () => {
  it("opens a file, a PR or a mission beside the session when one is open, and at the window's right edge when none is", () => {
    const beside = openSession(DESK_ROOM_CLOSED, "p1");
    for (const tab of [file("web/token.ts"), pr, mission]) {
      const opened = openTab(beside, tab);
      expect(panelPlace(opened)).toBe("beside_session");
      expect(opened.session).toBe("p1");
      expect(activeTab(opened.panel)).toEqual(tab);
    }
    for (const tab of [file("web/token.ts"), pr, mission]) {
      const opened = openTab(DESK_ROOM_CLOSED, tab);
      // No session is opened for it (B38, B39).
      expect(opened.session).toBeNull();
      expect(panelPlace(opened)).toBe("window_edge");
      expect(activeTab(opened.panel)).toEqual(tab);
    }
  });

  it("brings a tab already open to the front instead of adding it twice", () => {
    const room = openTab(openTab(openSession(DESK_ROOM_CLOSED, "p1"), pr), mission);
    const again = openTab(room, pr);
    expect(keys(again)).toEqual([tabKey(pr), tabKey(mission)]);
    expect(activeTab(again.panel)).toEqual(pr);
  });

  it("replaces the italic preview with the next single press in its place, and keeps a double-pressed file (B36)", () => {
    const one = openTab(openTab(openSession(DESK_ROOM_CLOSED, "p1"), mission), file("a.ts"), { preview: true });
    const two = openTab(one, file("b.ts"), { preview: true });
    expect(keys(two)).toEqual([tabKey(mission), tabKey(file("b.ts"))]);
    expect(two.panel?.preview).toBe(tabKey(file("b.ts")));
    const kept = keepTab(two, tabKey(file("b.ts")));
    expect(kept.panel?.preview).toBeNull();
    const three = openTab(kept, file("c.ts"), { preview: true });
    expect(keys(three)).toEqual([tabKey(mission), tabKey(file("b.ts")), tabKey(file("c.ts"))]);
    // A kept open of the preview's file keeps it.
    expect(openTab(three, file("c.ts")).panel?.preview).toBeNull();
  });
});

describe("Desk's room", () => {
  it("closes the tab panel when another session opens beside Desk, and keeps it for the same one (D-14, B25)", () => {
    const room = openTab(openSession(DESK_ROOM_CLOSED, "p1"), mission);
    expect(openSession(room, "p1").panel).not.toBeNull();
    expect(openSession(room, "p2")).toEqual({ session: "p2", panel: null, answer: 0 });
  });

  it("asks for the input box once per 답하기", () => {
    const once = openSession(DESK_ROOM_CLOSED, "p1", { answer: true });
    expect(openSession(once, "p1", { answer: true }).answer).toBe(2);
    expect(openSession(once, "p1").answer).toBe(1);
  });

  it("closes the tab panel alone on ✕, Escape or the dim, and both with the session panel's ✕ (B6, B21)", () => {
    const room = openTab(openSession(DESK_ROOM_CLOSED, "p1"), pr);
    expect(closePanel(room)).toEqual({ ...room, panel: null });
    expect(closeSession(room)).toEqual({ ...room, session: null, panel: null });
  });

  it("closes the panel with its last tab, the neighbour coming to the front", () => {
    const room = openTab(openTab(openTab(openSession(DESK_ROOM_CLOSED, "p1"), pr), mission), file("a.ts"));
    const shown = openTab(room, mission);
    const left = closeTab(shown, tabKey(mission));
    expect(activeTab(left.panel)).toEqual(file("a.ts"));
    expect(closeTab(closeTab(left, tabKey(pr)), tabKey(file("a.ts"))).panel).toBeNull();
  });

  it("opens and closes the Explorer column with 파일 ⇧⌘E, and only beside a session (B35)", () => {
    expect(toggleExplorer(DESK_ROOM_CLOSED)).toEqual(DESK_ROOM_CLOSED);
    const open = toggleExplorer(openSession(DESK_ROOM_CLOSED, "p1"));
    expect(open.panel?.explorer).toBe(true);
    expect(toggleExplorer(open).panel).toBeNull();
    const withTab = toggleExplorer(openTab(open, file("a.ts"), { preview: true }));
    expect([withTab.panel?.explorer, keys(withTab)]).toEqual([false, [tabKey(file("a.ts"))]]);
    // The Explorer column keeps the panel open after its last tab closes.
    expect(closeTab(open.panel ? openTab(open, pr) : open, tabKey(pr)).panel?.explorer).toBe(true);
  });

  it("names a file tab with its project only while another tab has the same file name (D-25, B40)", () => {
    const tabs = [file("web/index.ts"), file("src/index.ts", "/other"), file("web/token.ts"), mission];
    const names = tabNames(tabs, (tab) => (tab.root === "/repo" ? "herdr-ide" : "atorni-ax"));
    expect([...names.values()]).toEqual([
      { name: "index.ts", project: "herdr-ide" },
      { name: "index.ts", project: "atorni-ax" },
      { name: "token.ts", project: null },
      { name: "1.3.0 원격 안정화", project: null },
    ]);
  });

  it("gives up the view width first, covers Desk's place when both floors do not fit, and floats the session over a too-narrow chat (B24)", () => {
    const stored = { view_width: 700, session_width: 520 };
    expect(roomWidths(1600, stored, true)).toEqual({ session: 520, view: 700, panelCovers: false, sessionFloats: false });
    expect(roomWidths(1000, stored, true)).toEqual({ session: 520, view: 480, panelCovers: false, sessionFloats: false });
    expect(roomWidths(800, stored, true)).toMatchObject({ session: 520, view: 280, panelCovers: true, sessionFloats: true });
    expect(roomWidths(1400, { view_width: null, session_width: null }, false)).toMatchObject({ session: 0, view: 640 });
  });

  it("finds the View a thread's file names, as a path, from its folder, or with a Windows device's backslashes", () => {
    const posix = { ...output("notes"), path: "/repo/docs/notes.md" };
    expect([namesView(posix, "/repo/docs/notes.md"), namesView(posix, "/repo/./docs/x/../notes.md"), namesView(posix, "docs/notes.md"), namesView(posix, "./notes.md")]).toEqual([true, true, true, true]);
    expect([namesView(posix, "/other/docs/notes.md"), namesView(posix, "otes.md"), namesView(posix, "../notes.md"), namesView(posix, "docs\\notes.md")]).toEqual([false, false, false, false]);
    const windows = { ...output("notes"), path: "C:/repo/docs/notes.md" };
    expect([namesView(windows, "C:\\repo\\docs\\notes.md"), namesView(windows, "c:/repo/docs/notes.md"), namesView(windows, "docs\\notes.md")]).toEqual([true, true, true]);
  });
});
