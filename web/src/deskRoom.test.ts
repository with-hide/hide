import { describe, expect, it } from "vitest";
import { activeTab, MAX_DESK_FILE_TABS, closePanel, closeSession, closeTab, DESK_ROOM_CLOSED, followFrom, keepTab, namesView, newSessionBeside, openSession, outputNamed, openTab, panelPlace, roomWidths, tabKey, tabNames, toggleExplorer, type DeskRoomWindow, type DeskTab, type NewSessionFollow } from "./deskRoom";
import type { DeskItem, DeskJob, DeskOutput } from "./snapshot";

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

  it("closes the oldest file tab past the cap, never the one in front or the one beside it", () => {
    let room = openTab(openSession(DESK_ROOM_CLOSED, "p1"), file("first.md"));
    room = openTab(room, file("second.md"), { beside: true });
    room = openTab(room, mission);
    for (let n = 0; n < MAX_DESK_FILE_TABS - 1; n += 1) room = openTab(room, file(`more-${n}.md`));
    // 33 files: first.md was in front once, but more-30 is now; second.md is beside.
    room = openTab(room, file("first.md"));
    room = openTab(room, file("last.md"));
    const files = keys(room).filter((key) => key.startsWith("file:"));
    expect(files).toHaveLength(MAX_DESK_FILE_TABS);
    expect(files).toContain(tabKey(file("first.md")));
    expect(files).toContain(tabKey(file("second.md")));
    expect(files).not.toContain(tabKey(file("more-0.md")));
    expect(keys(room)).toContain(tabKey(mission));
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
    expect(openSession(room, "p2")).toEqual({ session: "p2", panel: null, closed: null, answer: 0 });
  });

  it("asks for the input box once per 답하기", () => {
    const once = openSession(DESK_ROOM_CLOSED, "p1", { answer: true });
    expect(openSession(once, "p1", { answer: true }).answer).toBe(2);
    expect(openSession(once, "p1").answer).toBe(1);
  });

  it("closes the tab panel alone on ✕, Escape or the dim, and both with the session panel's ✕ (B6, B21)", () => {
    const room = openTab(openSession(DESK_ROOM_CLOSED, "p1"), pr);
    expect(closePanel(room)).toEqual({ ...room, panel: null });
    expect(closeSession(room)).toEqual({ ...room, session: null, panel: null, closed: "p1" });
  });

  it("keeps the session just closed when ✕ is pressed with none open", () => {
    const closed = closeSession(openSession(DESK_ROOM_CLOSED, "p1"));
    expect(closeSession(closed)).toBe(closed);
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
    expect(roomWidths(1600, 0, stored, true)).toEqual({ session: 520, view: 700, viewMax: 1080, panelCovers: false, sessionFloats: false });
    expect(roomWidths(1000, 0, stored, true)).toEqual({ session: 520, view: 480, viewMax: 480, panelCovers: false, sessionFloats: false });
    expect(roomWidths(800, 0, stored, true)).toMatchObject({ session: 520, view: 280, panelCovers: true, sessionFloats: true });
  });

  it("opens the tab panel first over the sidebar up to the window's margin, and keeps a dragged width (Doop ③-다)", () => {
    const fresh = { view_width: null, session_width: null };
    // A 1440 window with a 340 sidebar: the panel's left edge lands at 140.
    expect(roomWidths(1100, 340, fresh, true)).toMatchObject({ session: 500, view: 800, viewMax: 940 });
    expect(roomWidths(1100, 340, fresh, false)).toMatchObject({ session: 0, view: 1300 });
    expect(roomWidths(1100, 340, { view_width: 520, session_width: null }, true)).toMatchObject({ view: 520 });
    expect(roomWidths(1100, 340, { view_width: 2000, session_width: null }, true)).toMatchObject({ view: 940 });
  });

  it("finds the View a thread's file names, as a path, from its folder, or with a Windows device's backslashes", () => {
    const posix = { ...output("notes"), path: "/repo/docs/notes.md" };
    expect([namesView(posix, "/repo/docs/notes.md"), namesView(posix, "/repo/./docs/x/../notes.md"), namesView(posix, "docs/notes.md"), namesView(posix, "./notes.md")]).toEqual([true, true, true, true]);
    expect([namesView(posix, "/other/docs/notes.md"), namesView(posix, "otes.md"), namesView(posix, "../notes.md"), namesView(posix, "docs\\notes.md")]).toEqual([false, false, false, false]);
    const windows = { ...output("notes"), path: "C:/repo/docs/notes.md" };
    expect([namesView(windows, "C:\\repo\\docs\\notes.md"), namesView(windows, "c:/repo/docs/notes.md"), namesView(windows, "docs\\notes.md")]).toEqual([true, true, true]);
  });
});

describe("the output a session's own line names (B27)", () => {
  const listed = { ...output("view-1"), device_id: "deepbox", checkout: "/home/me/repo", path: "/home/me/repo/shot.png" };
  const typed = (path: string): DeskOutput => ({ ...output(path), id: path, device_id: null, checkout: null, path });

  it("is the card's output, with its device and checkout, when the card lists the file", () => {
    expect(outputNamed([listed], typed("/home/me/repo/shot.png"), { device: "deepbox", checkout: "/home/me/repo", folder: "/home/me/repo" }, "mac")).toBe(listed);
  });

  it("takes the session's device and checkout for a file the card does not list, so it is read there and not on this machine", () => {
    const read = outputNamed([], typed("/home/me/repo/new.md"), { device: "deepbox", checkout: "/home/me/repo", folder: "/home/me/repo" }, "mac");
    expect(read).toMatchObject({ device_id: "deepbox", checkout: "/home/me/repo", path: "/home/me/repo/new.md" });
  });

  it("reads a relative spelling under the session's own folder, on this machine and on a device, a Windows one's backslashes as /", () => {
    expect(outputNamed([], typed("docs/report.pdf"), { device: "mac", checkout: "/work/repo", folder: "/work/repo/web" }, "mac")).toMatchObject({ device_id: null, path: "/work/repo/web/docs/report.pdf" });
    expect(outputNamed([], typed("../shot.png"), { device: "deepbox", checkout: "/home/me/repo", folder: "/home/me/repo/app" }, "mac")).toMatchObject({ device_id: "deepbox", checkout: "/home/me/repo", path: "/home/me/repo/shot.png" });
    expect(outputNamed([], typed(".\\out\\run.log"), { device: "winbox", checkout: "C:/src/repo", folder: "C:/src/repo" }, "mac")).toMatchObject({ path: "C:/src/repo/out/run.log" });
  });

  it("leaves a file of this machine's own session as typed", () => {
    const own = typed("/work/repo/a.md");
    expect(outputNamed([], own, { device: "mac", checkout: "/work/repo", folder: "/work/repo" }, "mac")).toBe(own);
    expect(outputNamed([], own, null, "mac")).toBe(own);
  });
});

// Expected answers are feedback 2's frame J (가-2, 나-1, 다) and the
// mission brief: a session a Desk turn starts opens beside Desk when the
// right side is empty, only the turn's first, and a handed letter opens nothing.
describe("a session a Desk turn starts (frame J)", () => {
  const card = (id: string, pane: string | null, state: DeskJob["state"] = "working"): DeskJob => ({
    id, agent_id: null, pane_id: pane, title: id, agent_kind: "codex", project: { workspace_id: "w", label: "w" }, location: { kind: "checkout", label: "w" },
    state, queue_position: null, now: null, plan: null, question: null, summary: null, pull_requests: [], outputs: [], joined: false,
    reason: null, failure: null, ask: null, mark: null, mark_read: false, machine: null, disconnected: false, changed_at_unix_ms: 0, read: false, mission: null, workers: [],
  });
  const user = (id: number): DeskItem => ({ kind: "user", id, text: "fix it", at_ms: 0, mentions: [] });
  const jobLine = (id: number, job: string): DeskItem => ({ kind: "job", id, job_id: job });
  const handed = (id: number, pane: string): DeskItem => ({ kind: "handed", id, agent_id: "a", pane_id: pane, name: "running", project: null, letter_kind: "request", line: "do it", letter_id: null, read: false, agent_kind: "claude", mark: null });
  /** Feeds the room each snapshot in turn, as the screen does, and returns every pane it opened. */
  const run = (snapshots: { items: DeskItem[]; cards: DeskJob[]; generation?: number }[], room: DeskRoomWindow = DESK_ROOM_CLOSED, entered: DeskItem[] = []) => {
    let follow: NewSessionFollow = followFrom(1, entered);
    let now = room;
    const opened: string[] = [];
    for (const { items, cards, generation = 1 } of snapshots) {
      const next = newSessionBeside(follow, generation, items, Object.fromEntries(cards.map((one) => [one.id, one])), now);
      follow = next.follow;
      if (next.open) {
        opened.push(next.open);
        now = openSession(now, next.open);
      }
    }
    return opened;
  };

  it("opens the turn's new session beside an empty side once its pane exists", () => {
    const items = [user(1), jobLine(2, "j1")];
    expect(run([{ items, cards: [card("j1", null, "starting")] }, { items, cards: [card("j1", "p1")] }])).toEqual(["p1"]);
  });

  it("leaves the session shown beside Desk where it is, and the new one a chip", () => {
    expect(run([{ items: [user(1), jobLine(2, "j1")], cards: [card("j1", "p1")] }], openSession(DESK_ROOM_CLOSED, "p0"))).toEqual([]);
    // A tab panel at the window's edge is a side that shows something too.
    expect(run([{ items: [user(1), jobLine(2, "j1")], cards: [card("j1", "p1")] }], openTab(DESK_ROOM_CLOSED, mission))).toEqual([]);
  });

  it("opens only the first of the sessions one turn starts, even when a later one gets its pane first", () => {
    const items = [user(1), jobLine(2, "j1"), jobLine(3, "j2")];
    expect(run([{ items, cards: [card("j1", null, "starting"), card("j2", "p2")] }, { items, cards: [card("j1", "p1"), card("j2", "p2")] }])).toEqual(["p1"]);
  });

  it("opens nothing for a letter handed to a session that already runs", () => {
    expect(run([{ items: [user(1), handed(2, "p9")], cards: [] }])).toEqual([]);
  });

  it("opens nothing that was there when the room was entered, and the next turn's session again", () => {
    const before = [user(1), jobLine(2, "j1")];
    expect(run([{ items: before, cards: [card("j1", "p1")] }], DESK_ROOM_CLOSED, before)).toEqual([]);
    const after = [...before, user(3), jobLine(4, "j2")];
    expect(run([{ items: before, cards: [card("j1", "p1")] }, { items: after, cards: [card("j1", "p1"), card("j2", "p2")] }], DESK_ROOM_CLOSED, before)).toEqual(["p2"]);
  });

  it("does not open the turn's first session again after the operator closes it, nor its second", () => {
    const items = [user(1), jobLine(2, "j1"), jobLine(3, "j2")];
    let follow = followFrom(1, []);
    const cards = Object.fromEntries([card("j1", "p1"), card("j2", "p2")].map((one) => [one.id, one]));
    const first = newSessionBeside(follow, 1, items, cards, DESK_ROOM_CLOSED);
    expect(first.open).toBe("p1");
    follow = first.follow;
    expect(newSessionBeside(follow, 1, items, cards, closeSession(openSession(DESK_ROOM_CLOSED, "p1"))).open).toBeNull();
  });

  it("passes over a start that failed: the turn's next session opens, and a start retried under the same job opens once it has its pane", () => {
    const items = [user(1), jobLine(2, "j1"), jobLine(3, "j2")];
    expect(run([{ items, cards: [card("j1", null, "failed"), card("j2", "p2")] }])).toEqual(["p2"]);
    // The candidate's Pi: its first `hide agent spawn` was refused (`invalid_agent_name`) and the retry kept the job.
    const retried = [user(1), jobLine(2, "j1")];
    expect(run([{ items: retried, cards: [card("j1", null, "failed")] }, { items: retried, cards: [card("j1", null, "starting")] }, { items: retried, cards: [card("j1", "p1")] }])).toEqual(["p1"]);
  });

  it("reads every line of a new conversation as new", () => {
    const old = [user(5), jobLine(6, "j1")];
    expect(run([{ items: [user(1), jobLine(2, "j9")], cards: [card("j9", "p9")], generation: 2 }], DESK_ROOM_CLOSED, old)).toEqual(["p9"]);
  });
});
