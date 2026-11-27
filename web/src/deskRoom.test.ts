import { describe, expect, it } from "vitest";
import { closeSession, rightSide, DESK_ROOM_CLOSED, deskOpener, deskShown, followFrom, followMoved, hideDesk, leaveRoom, namesView, newSessionBeside, openSession, outputNamed, panelPlace, roomWidths, showDesk, takeAnswer, toggleExpanded, toggleExplorer, type DeskRoomWindow, type NewSessionFollow } from "./deskRoom";
import type { DeskItem, DeskJob, DeskMoved, DeskOutput } from "./snapshot";

// Expected answers are PRD desk-2-ui D-05, D-15, B16-B26 and PRD
// desk-own-desk D-02, D-03, D-05, D-14, D-16, B6, B9, B13, B16, B18,
// B36, B37, B40, and docs/UI_BEHAVIOR.md, Desk. Desk's desk's tabs are the
// core's; the window keeps only whether it shows.

const output = (id: string, kind: DeskOutput["kind"] = "image"): DeskOutput => ({ id, kind, label: `${id}.png`, thumb: null, seen: false, pr_state: null, device_id: "local", checkout: "/repo", path: `/repo/${id}.png`, url: null });

describe("Desk's room", () => {
  it("shows Desk's desk beside the session when one is open, and at the window's right edge when none is (B16)", () => {
    expect(panelPlace(showDesk(openSession(DESK_ROOM_CLOSED, "p1")))).toBe("beside_session");
    expect(panelPlace(showDesk(DESK_ROOM_CLOSED))).toBe("window_edge");
  });

  it("keeps the desk as it was when another session opens beside Desk, and closes that session's Explorer column (B13, B36)", () => {
    const room = toggleExplorer(showDesk(openSession(DESK_ROOM_CLOSED, "p1")), 2);
    expect(openSession(room, "p1")).toEqual(room);
    expect(openSession(room, "p2")).toEqual({ ...room, session: "p2", explorer: false });
  });

  // Mission-109's walk on main: a 답하기 on a card whose session was not open gave no keyboard (regression of pull request 3076).
  it("names the session a 답하기 was pressed for until that session's box takes it, and lets it go on any other open", () => {
    const pressed = openSession(DESK_ROOM_CLOSED, "p1", { answer: true });
    expect(pressed).toMatchObject({ session: "p1", answer: "p1" });
    expect(takeAnswer(pressed, "p2")).toBe(pressed);
    expect(takeAnswer(pressed, "p1").answer).toBeNull();
    // Pressed again while open, it asks again.
    expect(openSession(takeAnswer(pressed, "p1"), "p1", { answer: true }).answer).toBe("p1");
    // Another session opened, the same one opened for another reason, or the session closed: nothing is left to take.
    expect(openSession(pressed, "p2").answer).toBeNull();
    expect(openSession(pressed, "p1").answer).toBeNull();
    expect(closeSession(pressed).answer).toBeNull();
    expect(leaveRoom(pressed).answer).toBeNull();
    // A session that moved to another pane takes its press with it.
    expect(followMoved(pressed, [{ pane_id: "p1", live_pane_id: "p9" } satisfies DeskMoved])).toMatchObject({ session: "p9", answer: "p9" });
  });

  it("hides the desk alone on ✕, Escape or the dim, and keeps it with the session panel's ✕ (B13, B18)", () => {
    const room = toggleExpanded(showDesk(openSession(DESK_ROOM_CLOSED, "p1")));
    expect(hideDesk(room)).toEqual({ ...room, desk: false, expanded: false });
    expect(closeSession(room)).toEqual({ ...room, session: null });
    expect(hideDesk(DESK_ROOM_CLOSED)).toBe(DESK_ROOM_CLOSED);
  });

  it("leaves the room as it is when ✕ is pressed with no session open", () => {
    const closed = closeSession(openSession(DESK_ROOM_CLOSED, "p1"));
    expect(closeSession(closed)).toBe(closed);
  });

  // Operator request 2026-11-27 (mission-109): ⌃⇥ away from Desk and back finds the session still beside Desk.
  it("keeps the session beside Desk when the room is left, folding only the desk, its ⤢ and its Explorer", () => {
    const room = toggleExplorer(toggleExpanded(showDesk(openSession(DESK_ROOM_CLOSED, "p1", { answer: true }))), 2);
    expect(leaveRoom(room)).toEqual({ ...DESK_ROOM_CLOSED, session: "p1" });
    const bare = openSession(DESK_ROOM_CLOSED, "p1");
    expect(leaveRoom(bare)).toBe(bare);
  });

  it("draws the desk only while it shows and holds a tab or the Explorer's column, so its last tab closing closes it (B6)", () => {
    const shown = showDesk(DESK_ROOM_CLOSED);
    expect([deskShown(shown, 2), deskShown(shown, 0), deskShown(DESK_ROOM_CLOSED, 2)]).toEqual([true, false, false]);
    const explorer = toggleExplorer(openSession(DESK_ROOM_CLOSED, "p1"), 0);
    expect(deskShown(explorer, 0)).toBe(true);
  });

  it("opens and closes the Explorer column with 파일 ⇧⌘E, and only beside a session (B37)", () => {
    expect(toggleExplorer(DESK_ROOM_CLOSED, 0)).toBe(DESK_ROOM_CLOSED);
    const open = toggleExplorer(openSession(DESK_ROOM_CLOSED, "p1"), 0);
    expect([open.desk, open.explorer]).toEqual([true, true]);
    // With no tab, closing the column hides the desk; with tabs, the desk stays.
    expect(toggleExplorer(open, 0).desk).toBe(false);
    expect([toggleExplorer(open, 3).desk, toggleExplorer(open, 3).explorer]).toEqual([true, false]);
  });

  it("covers Desk's chat with ⤢ and goes back with it again (B40)", () => {
    const room = showDesk(DESK_ROOM_CLOSED);
    expect(toggleExpanded(room).expanded).toBe(true);
    expect(toggleExpanded(toggleExpanded(room))).toEqual(room);
  });

  it("gives up the view width first, covers Desk's place when both floors do not fit, and floats the session over a too-narrow chat (B24)", () => {
    const stored = { view_width: 700, session_width: 520 };
    expect(roomWidths(1600, 0, stored, true)).toEqual({ session: 520, view: 700, viewMax: 1080, panelCovers: false, sessionFloats: false });
    expect(roomWidths(1000, 0, stored, true)).toEqual({ session: 520, view: 480, viewMax: 480, panelCovers: false, sessionFloats: false });
    expect(roomWidths(800, 0, stored, true)).toMatchObject({ session: 520, view: 280, panelCovers: true, sessionFloats: true });
  });

  it("opens Desk's desk first over the sidebar up to the window's margin, and keeps a dragged width (Doop ③-다)", () => {
    const fresh = { view_width: null, session_width: null };
    // A 1440 window with a 340 sidebar: the panel's left edge lands at 140.
    expect(roomWidths(1100, 340, fresh, true)).toMatchObject({ session: 500, view: 800, viewMax: 940 });
    expect(roomWidths(1100, 340, fresh, false)).toMatchObject({ session: 0, view: 1300 });
    expect(roomWidths(1100, 340, { view_width: 520, session_width: null }, true)).toMatchObject({ view: 520 });
    expect(roomWidths(1100, 340, { view_width: 2000, session_width: null }, true)).toMatchObject({ view: 940 });
  });

  it("finds the View a thread's file names by its absolute path, `.` and `..` read, or with a Windows device's backslashes", () => {
    const posix = { ...output("notes"), path: "/repo/docs/notes.md" };
    expect([namesView(posix, "/repo/docs/notes.md"), namesView(posix, "/repo/./docs/x/../notes.md")]).toEqual([true, true]);
    expect([namesView(posix, "/other/docs/notes.md"), namesView(posix, "docs\\notes.md")]).toEqual([false, false]);
    const windows = { ...output("notes"), path: "C:/repo/docs/notes.md" };
    expect([namesView(windows, "C:\\repo\\docs\\notes.md"), namesView(windows, "c:/repo/docs/notes.md")]).toEqual([true, true]);
  });

  it("never names a View by a relative spelling, which a file of the same name in another folder would also end with", () => {
    const posix = { ...output("notes"), path: "/repo/docs/notes.md" };
    expect([namesView(posix, "docs/notes.md"), namesView(posix, "./notes.md"), namesView(posix, "notes.md")]).toEqual([false, false, false]);
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

  it("is the View its call's answer named, never another View of the same name in another folder, for a page and a file alike", () => {
    // As the core lists a session's Views (`desk/jobs.rs`): two folders each hold an index.html and a notes.md.
    const page = (id: string, url: string): DeskOutput => ({ ...output(id, "page"), label: id, path: null, device_id: "mac", checkout: "/work/repo", url });
    const file = (id: string, path: string): DeskOutput => ({ ...output(id, "doc"), path, device_id: "mac", checkout: "/work/repo" });
    const pageA = page("v1", "file:///work/repo/a/index.html");
    const pageB = page("v2", "file:///work/repo/b/index.html");
    const notesA = file("v3", "/work/repo/a/notes.md");
    const notesB = file("v4", "/work/repo/b/notes.md");
    const listed = [pageA, pageB, notesA, notesB];
    // As a thread's view line carries it (`threadView.ts`): the call's argument as typed.
    const typedPage = (target: string): DeskOutput => ({ ...output(target, "page"), label: target, device_id: null, checkout: null, path: null, url: target });
    const place = { device: "mac", checkout: "/work/repo", folder: "/work/repo/a" };
    expect(outputNamed(listed, typedPage("index.html"), place, "mac", "v2")).toBe(pageB);
    expect(outputNamed(listed, typedPage("github.com/with-hide/hide/pull/2529/files"), place, "mac", "v1")).toBe(pageA);
    expect(outputNamed(listed, typed("notes.md"), place, "mac", "v4")).toBe(notesB);
  });

  it("leaves a page line whose call named no View, or a View the card no longer lists, as the words it typed", () => {
    const pageA = { ...output("v1", "page"), path: null, url: "file:///work/repo/a/index.html" };
    const typedPage = (target: string): DeskOutput => ({ ...output(target, "page"), label: target, device_id: null, checkout: null, path: null, url: target });
    const place = { device: "mac", checkout: "/work/repo", folder: "/work/repo/a" };
    const refused = typedPage("index.html");
    expect(outputNamed([pageA], refused, place, "mac", null)).toBe(refused);
    const closed = typedPage("github.com/with-hide/hide/pull/2529/files");
    expect(outputNamed([pageA], closed, place, "mac", "v9")).toBe(closed);
  });

  it("leaves a line whose call named no View as its words when they are relative, though the agent's folder holds a View of that name after a cd", () => {
    // The pane's folder is /work/repo; the agent ran `cd b` and then `hide file open notes.md` and `hide browser open index.html`, which were refused.
    const notesA = { ...output("v3", "doc"), path: "/work/repo/notes.md", device_id: "mac", checkout: "/work/repo" };
    const pageA = { ...output("v1", "page"), path: null, url: "file:///work/repo/index.html", device_id: "mac", checkout: "/work/repo" };
    const typedPage = (target: string): DeskOutput => ({ ...output(target, "page"), label: target, device_id: null, checkout: null, path: null, url: target });
    const place = { device: "mac", checkout: "/work/repo", folder: "/work/repo" };
    expect(outputNamed([notesA, pageA], typed("notes.md"), place, "mac", null)).not.toBe(notesA);
    const page = typedPage("index.html");
    expect(outputNamed([notesA, pageA], page, place, "mac", null)).toBe(page);
    // Words that name one View alone still find it: an absolute path, an address with its scheme.
    expect(outputNamed([notesA, pageA], typed("/work/repo/notes.md"), place, "mac", null)).toBe(notesA);
    expect(outputNamed([notesA, pageA], typedPage("file:///work/repo/index.html"), place, "mac", null)).toBe(pageA);
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
    // Desk's desk at the window's edge is a side that shows something too.
    expect(run([{ items: [user(1), jobLine(2, "j1")], cards: [card("j1", "p1")] }], showDesk(DESK_ROOM_CLOSED))).toEqual([]);
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

// The review's S1: once a gone session runs again in another pane, the room itself is that pane, so every action of the panel reaches it.
describe("the room after a gone session runs again", () => {
  it("moves the open session to the live pane, and keeps the desk", () => {
    const moved = [{ pane_id: "w1:p9", live_pane_id: "w1:p20" }];
    const room: DeskRoomWindow = { ...DESK_ROOM_CLOSED, session: "w1:p9", desk: true, explorer: true, answer: "w1:p9" };
    expect(followMoved(room, moved)).toEqual({ ...room, session: "w1:p20", answer: "w1:p20" });
    // Another pane, or no pair, leaves the very same room.
    const other: DeskRoomWindow = { ...DESK_ROOM_CLOSED, session: "w1:p3", desk: true };
    expect(followMoved(other, moved)).toBe(other);
    expect(followMoved(room, [])).toBe(room);
  });
});

describe("rightSide", () => {
  it("is the session beside Desk, else 한눈에 unless folded, else nothing", () => {
    expect(rightSide(openSession(DESK_ROOM_CLOSED, "p1"), true)).toBe("session");
    expect(rightSide(DESK_ROOM_CLOSED, false)).toBe("glance");
    expect(rightSide(closeSession(openSession(DESK_ROOM_CLOSED, "p1")), false)).toBe("glance");
    expect(rightSide(DESK_ROOM_CLOSED, true)).toBeNull();
  });
});

describe("deskOpener", () => {
  it("counts the desk's tabs while it is folded and holds some, and stands nowhere while it shows or is empty", () => {
    const beside = openSession(DESK_ROOM_CLOSED, "p1");
    expect(deskOpener(DESK_ROOM_CLOSED, 3)).toBe(3);
    expect(deskOpener(beside, 1)).toBe(1);
    expect(deskOpener(showDesk(beside), 2)).toBeNull();
    expect(deskOpener(DESK_ROOM_CLOSED, 0)).toBeNull();
    // A desk shown only for the Explorer's column holds no tab: nothing to open.
    expect(deskOpener(toggleExplorer(beside, 0), 0)).toBeNull();
  });
});
