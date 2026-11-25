import { describe, expect, it } from "vitest";
import { byMachine, guestPaneRefuses, guestPanes, guestPlaces, guestTabs, isGuestPaneId, peerRows } from "./guestWork";
import type { SnapshotRest, Workspace } from "./snapshot";

const KEY = "cd".repeat(32);
const TERMINAL = (branch: string) => `터미널 · ${branch}`;

function project(id: string, path: string): Workspace {
  return { id, label: id, path, device_id: "local", checkouts: [{ id: `${id}-main`, path }] } as unknown as Workspace;
}

function rest(panes: { pane: string; folder?: string; name?: string; branch?: string; mark?: string; agent?: string; since?: number }[]): SnapshotRest {
  return {
    navigator: {
      peers: [{ key: KEY, device_id: "macbook", name: "MacBook Air", connected: false }],
      guest_panes: [{ device_id: "macbook", key: KEY, panes: panes.map((pane) => ({ id: `guest:macbook:pane:${pane.pane}`, pane: pane.pane, since_unix_ms: pane.since ?? 1, folder: pane.folder, name: pane.name, branch: pane.branch, mark: pane.mark, agent: pane.agent })) }],
    },
  } as SnapshotRest;
}

describe("another machine's work on this computer (PRD device-connect-iroh B10, B68)", () => {
  it("names a pane by its folder and its machine's letters, whether or not that machine is connected", () => {
    const [pane] = guestPanes(rest([{ pane: "w1:p1", folder: "/work/shop-api" }]), [], TERMINAL);
    expect(pane).toMatchObject({ id: "guest:macbook:pane:w1:p1", machine: "MacBook Air", mark: "MA", label: "shop-api", folder: "/work/shop-api", projectId: null });
    // A pane whose node said no folder goes by its own id there.
    expect(guestPanes(rest([{ pane: "w1:p2" }]), [], TERMINAL)[0]).toMatchObject({ label: "w1:p2", folder: null });
  });

  it("names a pane by the name its machine told, its folder only when it told none (B10)", () => {
    const panes = guestPanes(rest([{ pane: "w1:p1", folder: "/work/shop-api", name: "로그인 고치기" }, { pane: "w1:p2", folder: "/work/shop-api" }]), [], TERMINAL);
    expect(panes.map((pane) => [pane.label, pane.folder])).toEqual([["로그인 고치기", "/work/shop-api"], ["shop-api", "/work/shop-api"]]);
  });

  it("names a plain terminal by its branch, and marks each row as its machine draws it (B10)", () => {
    const panes = guestPanes(rest([
      { pane: "w1:p1", folder: "/work/hide", name: "hide 배치 PR 처리", branch: "main", mark: "going" },
      { pane: "w1:p2", folder: "/work/hide", branch: "m75-relay", mark: "quiet" },
      // A mark a later build draws is none here; a blank branch is no branch.
      { pane: "w1:p3", folder: "/work/hide", branch: " ", mark: "sparkle" },
    ]), [project("hide", "/work/hide")], TERMINAL);
    expect(panes.map((pane) => [pane.label, pane.status, pane.project])).toEqual([
      ["hide 배치 PR 처리", "going", "hide"],
      ["터미널 · m75-relay", "quiet", "hide"],
      ["hide", null, "hide"],
    ]);
  });

  it("shows an earlier build's list, which tells no branch, mark or agent, by its names and folders without a mark or logo", () => {
    const panes = guestPanes(rest([{ pane: "w1:p1", folder: "/work/hide", name: "로그인 고치기" }, { pane: "w1:p2", folder: "/work/hide" }]), [], TERMINAL);
    expect(panes.map((pane) => [pane.label, pane.status, pane.agent])).toEqual([["로그인 고치기", null, null], ["hide", null, null]]);
  });

  it("carries the agent its machine says runs in a pane, for the row's logo, and none for a plain terminal (B10)", () => {
    const panes = guestPanes(rest([{ pane: "w1:p1", folder: "/work/hide", name: "로그인 고치기", agent: "codex" }, { pane: "w1:p2", folder: "/work/hide", branch: "main", agent: " " }]), [], TERMINAL);
    expect(panes.map((pane) => [pane.label, pane.agent])).toEqual([["로그인 고치기", "codex"], ["터미널 · main", null]]);
  });

  it("stands a pane under this computer's project that holds its folder, and under its machine's name otherwise", () => {
    const projects = [project("shop", "/work/shop-api"), project("shop-web", "/work/shop-api/web"), project("other", "/work/shop")];
    const panes = guestPanes(rest([{ pane: "w1:p1", folder: "/work/shop-api/web/src" }, { pane: "w1:p2", folder: "/work/shop-api" }, { pane: "w1:p3", folder: "/scratch/try", since: 0 }, { pane: "w1:p4", folder: "/work/shop-api-2" }]), projects, TERMINAL);
    const placed = Object.fromEntries(panes.map((pane) => [pane.label, pane.projectId]));
    // The longest checkout that holds the folder is its project; a folder beside one is in none.
    expect(placed).toEqual({ src: "shop-web", "shop-api": "shop", try: null, "shop-api-2": null });
    // One head per machine names it once, with its letters.
    expect(byMachine(panes.filter((pane) => pane.projectId === null))).toEqual([{ deviceId: "macbook", machine: "MacBook Air", mark: "MA", panes: [expect.objectContaining({ label: "try" }), expect.objectContaining({ label: "shop-api-2" })] }]);
    // A tab strip holds the same machine's panes of the same place.
    const loose = panes.find((pane) => pane.label === "try")!;
    expect(guestTabs(panes, loose).map((pane) => pane.label)).toEqual(["try", "shop-api-2"]);
  });

  it("keeps a project's panes inside it: under the checkout row that holds them, at its end when the project's row stands for that checkout, and hidden with a fold (operator review 2026-11-25)", () => {
    const modakbul = { id: "modakbul", label: "modakbul", path: "/work/modakbul", device_id: "local", checkouts: [{ id: "main", path: "/work/modakbul" }, { id: "login", path: "/work/modakbul/.worktrees/login" }] } as unknown as Workspace;
    const panes = guestPanes(rest([
      { pane: "w1:p1", folder: "/work/modakbul/docs", name: "강의 정산" },
      { pane: "w1:p2", folder: "/work/modakbul/.worktrees/login", name: "로그인 시험" },
      { pane: "w1:p3", folder: "/work/modakbul", name: "리마인드 확인" },
      { pane: "w1:p4", folder: "/scratch", name: "정리" },
    ]), [modakbul], TERMINAL);
    const names = (list: readonly { label: string }[] | undefined) => (list ?? []).map((pane) => pane.label);
    const none = new Set<string>();
    // Both checkouts drawn as rows: each pane under the deepest checkout that holds its folder, in the order they came.
    const open = guestPlaces(panes, "modakbul", { open: true, rows: new Set(["main", "login"]), folded: none });
    expect(names(open.byCheckout.get("main"))).toEqual(["강의 정산", "리마인드 확인"]);
    expect(names(open.byCheckout.get("login"))).toEqual(["로그인 시험"]);
    expect([open.atEnd, open.hidden]).toEqual([[], []]);
    // The worktree waits in the closed 다른 체크아웃 fold: its pane is hidden with it, not moved out.
    const otherFolded = guestPlaces(panes, "modakbul", { open: true, rows: new Set(["main"]), folded: new Set(["login"]) });
    expect(names(otherFolded.byCheckout.get("main"))).toEqual(["강의 정산", "리마인드 확인"]);
    expect([names(otherFolded.atEnd), names(otherFolded.hidden)]).toEqual([[], ["로그인 시험"]]);
    // The project's row stands for its main checkout, which draws no row: its panes stand at the project's end.
    const mainUnrowed = guestPlaces(panes, "modakbul", { open: true, rows: new Set(["login"]), folded: none });
    expect([names(mainUnrowed.atEnd), names(mainUnrowed.byCheckout.get("login"))]).toEqual([["강의 정산", "리마인드 확인"], ["로그인 시험"]]);
    // The project folded: every one of its panes is hidden with it.
    const folded = guestPlaces(panes, "modakbul", { open: false, rows: none, folded: none });
    expect([folded.byCheckout.size, folded.atEnd.length, names(folded.hidden)]).toEqual([0, 0, ["강의 정산", "로그인 시험", "리마인드 확인"]]);
    // A pane in no folder of the project is not the project's.
    expect([...open.byCheckout.values()].flat().concat(open.atEnd, folded.hidden).some((pane) => pane.label === "정리")).toBe(false);
  });

  it("does only the closed set to another machine's pane (D-74)", () => {
    for (const command of ["split_right", "split_down", "new_tab", "rename_tab", "start_agent"]) expect(guestPaneRefuses(command)).toBe(true);
    for (const command of ["close_pane", "close_tab", "search", "text_larger"]) expect(guestPaneRefuses(command)).toBe(false);
    expect(isGuestPaneId("guest:macbook:pane:w1:p1")).toBe(true);
    expect(isGuestPaneId("remote:macbook:pane:w1:p1")).toBe(false);
    expect(isGuestPaneId("w1:p1")).toBe(false);
  });
});

describe("a connected machine's own Needs You and Done (PRD device-connect-iroh D-50, B49)", () => {
  const told = (rows: { id: string; group: "needs_you" | "done" }[]): SnapshotRest =>
    ({
      navigator: {
        peers: [{ key: KEY, device_id: "windows", name: "Windows PC", connected: true }],
        peer_work: [{ device_id: "windows", key: KEY, needs_you: 1, done: 1, rows: rows.map((row) => ({ ...row, title: row.id, agent_kind: "claude", mark: "turn", status_code: "question" })) }],
      },
    }) as SnapshotRest;

  it("lists each row under its own state with its machine's name", () => {
    const state = told([{ id: "w1:p1", group: "needs_you" }, { id: "w1:p2", group: "done" }]);
    expect(peerRows(state, "needs_you").map(({ row, machine, deviceId }) => [row.id, machine, deviceId])).toEqual([["w1:p1", "Windows PC", "windows"]]);
    expect(peerRows(state, "done").map(({ row }) => row.id)).toEqual(["w1:p2"]);
  });

  it("lists nothing of a machine that told nothing", () => {
    expect(peerRows({ navigator: {} } as SnapshotRest, "needs_you")).toEqual([]);
    expect(peerRows(null, "done")).toEqual([]);
  });
});
