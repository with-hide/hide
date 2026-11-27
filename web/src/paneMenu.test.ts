import { describe, expect, it } from "vitest";
import { paneMenuItems, refuseNarrowSplits, terminalMenuItems, type TerminalMenuContext } from "./PaneRelations";
import { PANE_FLOOR } from "./paneFloor";
import type { PaneRow } from "./snapshot";

const pane = { id: "w1:p1", children: null, lineage_path: [] } as unknown as PaneRow;
const chords = { copy: "⌘C", paste: "⌘V", find: "⌘F", splitRight: "⌘D", splitDown: "⌘⇧D", zoom: "⇧⌘↩" };
const context = (over: Partial<TerminalMenuContext>): TerminalMenuContext => ({ selection: false, zoomed: false, paneCount: 2, chords, ...over });

describe("a right-click in a terminal", () => {
  it("edits the text, then lays out the tab, then offers the header's pane actions", () => {
    const items = terminalMenuItems(pane, "reviewer", context({ selection: true }));
    expect(items.map((item) => item.id)).toEqual(["copy", "paste", "select_all", "find", "split_right", "split_down", "toggle_zoom", "copy_reference", "close_pane"]);
    expect(items.filter((item) => item.separated).map((item) => item.id)).toEqual(["split_right", "copy_reference", "close_pane"]);
    expect(items.find((item) => item.id === "split_down")?.shortcut).toBe("⌘⇧D");
  });

  it("offers Copy only over a selection", () => {
    expect(terminalMenuItems(pane, "reviewer", context({})).some((item) => item.id === "copy")).toBe(false);
  });

  it("offers URL opening and copying before the ordinary terminal actions", () => {
    const items = terminalMenuItems(pane, "reviewer", context({ link: { target: { kind: "url", url: "https://example.test/docs" }, found: null } }));
    expect(items.slice(0, 4).map((item) => item.label)).toEqual(["Open in Hide", "Open in default browser", "Copy link", "Paste"]);
    expect(items[3]?.separated).toBe(true);
  });

  it("lets an HTML link open its rendered page, its source, its default app or its file manager", () => {
    const items = terminalMenuItems(pane, "reviewer", context({
      link: { target: { kind: "path", path: "report.HTML", line: 12, column: null }, found: { real: "/project/report.HTML", kind: "file" }, inCheckout: true, sourceInHide: true },
      reveal: { label: "explorer.revealFinder" },
    }));
    expect(items.slice(0, 5).map((item) => item.label)).toEqual(["Open in browser", "Open source", "Open in default application", "Reveal in Finder", "Copy path"]);
  });

  it("offers an outside HTML file's page and source in Hide where this window shows outside files (PRD outside-path-in-hide B9)", () => {
    const link = { target: { kind: "path" as const, path: "/tmp/preview.html", line: null, column: null }, found: { real: "/tmp/preview.html", kind: "file" as const }, inCheckout: false };
    const shown = terminalMenuItems(pane, "reviewer", context({ link: { ...link, sourceInHide: true }, reveal: { label: "explorer.revealFinder" } }));
    expect(shown.slice(0, 5).map((item) => item.label)).toEqual(["Open in browser", "Open source", "Open in default application", "Reveal in Finder", "Copy path"]);
    const elsewhere = terminalMenuItems(pane, "reviewer", context({ link: { ...link, sourceInHide: false }, reveal: { label: "explorer.revealFinder" } }));
    expect(elsewhere.some((item) => item.id === "link_source")).toBe(false);
  });

  it("treats a directory named .html as a folder and names the host's actual file manager", () => {
    const items = terminalMenuItems(pane, "reviewer", context({
      link: { target: { kind: "path", path: "./pages.html", line: null, column: null }, found: { real: "/project/pages.html", kind: "directory" } },
      reveal: { label: "explorer.revealFileExplorer" },
    }));
    expect(items.slice(0, 4).map((item) => item.label)).toEqual(["Open", "Open in default application", "Reveal in File Explorer", "Copy path"]);
    expect(items.some((item) => item.id === "link_source")).toBe(false);
  });

  it("zooms a pane that has siblings, unzooms a zoomed one, and says why a lone pane cannot zoom", () => {
    const zoom = (over: Partial<TerminalMenuContext>) => terminalMenuItems(pane, "reviewer", context(over)).find((item) => item.id === "toggle_zoom");
    expect(zoom({ paneCount: 2 })).toMatchObject({ label: "Zoom pane", unavailable: null });
    expect(zoom({ paneCount: 3, zoomed: true })).toMatchObject({ label: "Unzoom pane", unavailable: null });
    expect(zoom({ paneCount: 1 })).toMatchObject({ label: "Zoom pane", unavailable: "This pane is the only one in its tab" });
  });
});

// PRD fork-operator-root B2, B3, D-03: both placements share one availability.
describe("the pane menu's Fork agent items", () => {
  const forkItems = (fork?: PaneRow["fork"], sleep_action?: PaneRow["sleep_action"]) =>
    paneMenuItems({ id: "w1:p1", children: null, lineage_path: [], fork, sleep_action } as unknown as PaneRow, "reviewer").filter((item) => item.id === "fork_agent" || item.id === "fork_agent_side");

  it("offers both placements on a pane the core can fork", () => {
    expect(forkItems({ available: true })).toMatchObject([
      { id: "fork_agent", label: "Fork agent", unavailable: null },
      { id: "fork_agent_side", label: "Fork agent to the side", unavailable: null },
    ]);
  });

  it("draws both disabled with the core's reason on an agent that cannot fork yet", () => {
    const reason = "This agent has not reported its conversation yet";
    expect(forkItems({ available: false, reason: "no_conversation" })).toMatchObject([{ unavailable: reason }, { unavailable: reason }]);
  });

  it("offers neither on a shell and on an agent with no fork command", () => {
    expect(forkItems({ available: false, reason: null })).toEqual([]);
    expect(forkItems()).toEqual([]);
  });

  it("follows Sleep agent, apart from the copy items", () => {
    const items = paneMenuItems({ id: "w1:p1", children: null, lineage_path: [], fork: { available: true }, sleep_action: { available: true } } as unknown as PaneRow, "reviewer");
    expect(items.map((item) => item.id)).toEqual(["sleep_agent", "fork_agent", "fork_agent_side", "copy_reference", "close_pane"]);
    expect(items.filter((item) => item.separated).map((item) => item.id)).toEqual(["copy_reference", "close_pane"]);
  });
});

// docs/UI_BEHAVIOR.md, The pane floor: a split right that would leave a half
// under the floor stays listed, disabled with its reason; Split down is the height's.
describe("the terminal menu of a pane too narrow to split right", () => {
  const agent = { id: "w1:p1", children: null, lineage_path: [], fork: { available: true } } as unknown as PaneRow;
  const offered = (cols: number | null) => refuseNarrowSplits(terminalMenuItems(agent, "reviewer", context({})), cols).filter((item) => ["split_right", "split_down", "fork_agent_side"].includes(item.id));
  const reason = "Too narrow to split: an agent's input box would not fit beside it";

  it("disables Split right and Fork agent to the side with the reason, and leaves Split down", () => {
    expect(offered(PANE_FLOOR.split_right_columns - 1)).toMatchObject([
      { id: "split_right", unavailable: reason },
      { id: "split_down", unavailable: null },
      { id: "fork_agent_side", unavailable: reason },
    ]);
  });

  it("offers them on a pane wide enough, and on a terminal not drawn yet", () => {
    for (const cols of [PANE_FLOOR.split_right_columns, null]) {
      expect(offered(cols).map((item) => item.unavailable)).toEqual([null, null, null]);
    }
  });

  it("keeps the reason an item already has", () => {
    const busy = refuseNarrowSplits([{ id: "split_right", label: "Split right", unavailable: "busy" }], 2);
    expect(busy[0]?.unavailable).toBe("busy");
  });
});

// PRD pane-open-in-overview B3, B11: the header button's action, first among an agent pane's own items, never on a shell.
describe("the pane menu's Show in Overview item", () => {
  const row = { id: "w1:p1", children: null, lineage_path: [], sleep_action: { available: true } } as unknown as PaneRow;

  it("leads an agent pane's actions", () => {
    expect(paneMenuItems(row, "reviewer", undefined, undefined, true).map((item) => item.id)).toEqual(["open_in_overview", "sleep_agent", "copy_reference", "close_pane"]);
    expect(paneMenuItems(row, "reviewer", undefined, undefined, true)[0]).toMatchObject({ label: "Show in Overview", unavailable: null });
  });

  it("is absent on a shell, in the header menu and the terminal's", () => {
    const shell = { id: "w1:p2", children: null, lineage_path: [] } as unknown as PaneRow;
    expect(paneMenuItems(shell, "zsh").some((item) => item.id === "open_in_overview")).toBe(false);
    expect(terminalMenuItems(shell, "zsh", context({})).some((item) => item.id === "open_in_overview")).toBe(false);
    expect(terminalMenuItems(row, "reviewer", context({}), undefined, undefined, true).some((item) => item.id === "open_in_overview")).toBe(true);
  });
});

// PRD hide-plan B11: `작업 단계` opens the session's plan, offered only while it has one.
describe("the pane menu's Plan item", () => {
  const row = { id: "w1:p1", children: null, lineage_path: [], sleep_action: { available: true } } as unknown as PaneRow;

  it("follows Show in Overview on a pane whose session has a plan, and is absent otherwise", () => {
    expect(paneMenuItems(row, "reviewer", undefined, undefined, true, true).map((item) => item.id)).toEqual(["open_in_overview", "open_plan", "sleep_agent", "copy_reference", "close_pane"]);
    expect(paneMenuItems(row, "reviewer", undefined, undefined, true, true)[1]).toMatchObject({ label: "Task steps", unavailable: null });
    expect(paneMenuItems(row, "reviewer", undefined, undefined, true).some((item) => item.id === "open_plan")).toBe(false);
    expect(terminalMenuItems(row, "reviewer", context({}), undefined, undefined, true, true).some((item) => item.id === "open_plan")).toBe(true);
  });
});

describe("what the pane head moved into its menus (PRD lane-desk-pair-view B19, B20, B21)", () => {
  const row = { id: "w1:p1", children: null, lineage_path: [] } as unknown as PaneRow;

  it("leads an agent pane's menu with @ and pair view, apart from the rest", () => {
    const items = paneMenuItems(row, "reviewer", undefined, undefined, false, false, { mention: true, pair: "off" });
    expect(items.map((item) => item.id)).toEqual(["mention", "pair_view", "copy_reference", "close_pane"]);
    expect(items[1]).toMatchObject({ label: "Focus mode", unavailable: null });
    expect(items[2]?.separated).toBe(true);
  });

  it("names the way back on the pane in pair view, and offers focus mode on a shell too (D-53, D-55)", () => {
    expect(paneMenuItems(row, "reviewer", undefined, undefined, false, false, { mention: true, pair: "on" })[1]).toMatchObject({ label: "Back to the usual view", unavailable: null });
    const shell = paneMenuItems(row, "zsh", undefined, undefined, false, false, { mention: false, pair: "off" });
    expect(shell.map((item) => item.id)).toEqual(["pair_view", "copy_reference", "close_pane"]);
    expect(shell[0]).toMatchObject({ label: "Focus mode", unavailable: null });
  });

  it("draws pair view with a terminal's layout items, and @ with the pane's own", () => {
    const items = terminalMenuItems(row, "reviewer", context({}), undefined, undefined, false, false, { mention: true, pair: "off" });
    expect(items.map((item) => item.id)).toEqual(["paste", "select_all", "find", "split_right", "split_down", "toggle_zoom", "pair_view", "mention", "copy_reference", "close_pane"]);
    expect(items.filter((item) => item.separated).map((item) => item.id)).toEqual(["split_right", "mention", "copy_reference", "close_pane"]);
  });
});
