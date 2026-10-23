import { describe, expect, it } from "vitest";
import { paneMenuItems, terminalMenuItems, type TerminalMenuContext } from "./PaneRelations";
import type { PaneRow } from "./snapshot";

const pane = { id: "w1:p1", children: null, lineage_path: [] } as unknown as PaneRow;
const chords = { copy: "⌘C", paste: "⌘V", find: "⌘F", splitRight: "⌘D", splitDown: "⌘⇧D", zoom: "⇧⌘↩" };
const context = (over: Partial<TerminalMenuContext>): TerminalMenuContext => ({ selection: false, zoomed: false, paneCount: 2, chords, ...over });

describe("a right-click in a terminal", () => {
  it("edits the text, then lays out the tab, then offers the header's pane actions", () => {
    const items = terminalMenuItems(pane, "reviewer", context({ selection: true }));
    expect(items.map((item) => item.id)).toEqual(["copy", "paste", "select_all", "find", "split_right", "split_down", "toggle_zoom", "copy_name", "copy_pane_id", "close_pane"]);
    expect(items.filter((item) => item.separated).map((item) => item.id)).toEqual(["split_right", "copy_name", "close_pane"]);
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
      link: { target: { kind: "path", path: "report.HTML", line: 12, column: null }, found: { real: "/project/report.HTML", kind: "file" }, inCheckout: true },
      reveal: { label: "explorer.revealFinder" },
    }));
    expect(items.slice(0, 5).map((item) => item.label)).toEqual(["Open in browser", "Open source", "Open in default application", "Reveal in Finder", "Copy path"]);
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
    const reason = "Herdr has not reported this agent's conversation yet";
    expect(forkItems({ available: false, reason: "no_conversation" })).toMatchObject([{ unavailable: reason }, { unavailable: reason }]);
  });

  it("offers neither on a shell and on an agent with no fork command", () => {
    expect(forkItems({ available: false, reason: null })).toEqual([]);
    expect(forkItems()).toEqual([]);
  });

  it("follows Sleep agent, apart from the copy items", () => {
    const items = paneMenuItems({ id: "w1:p1", children: null, lineage_path: [], fork: { available: true }, sleep_action: { available: true } } as unknown as PaneRow, "reviewer");
    expect(items.map((item) => item.id)).toEqual(["sleep_agent", "fork_agent", "fork_agent_side", "copy_name", "copy_pane_id", "close_pane"]);
    expect(items.filter((item) => item.separated).map((item) => item.id)).toEqual(["copy_name", "close_pane"]);
  });
});

// PRD pane-open-in-overview B3, B11: the header button's action, first among an agent pane's own items, never on a shell.
describe("the pane menu's Show in Overview item", () => {
  const row = { id: "w1:p1", children: null, lineage_path: [], sleep_action: { available: true } } as unknown as PaneRow;

  it("leads an agent pane's actions", () => {
    expect(paneMenuItems(row, "reviewer", undefined, undefined, true).map((item) => item.id)).toEqual(["open_in_overview", "sleep_agent", "copy_name", "copy_pane_id", "close_pane"]);
    expect(paneMenuItems(row, "reviewer", undefined, undefined, true)[0]).toMatchObject({ label: "Show in Overview", unavailable: null });
  });

  it("is absent on a shell, in the header menu and the terminal's", () => {
    const shell = { id: "w1:p2", children: null, lineage_path: [] } as unknown as PaneRow;
    expect(paneMenuItems(shell, "zsh").some((item) => item.id === "open_in_overview")).toBe(false);
    expect(terminalMenuItems(shell, "zsh", context({})).some((item) => item.id === "open_in_overview")).toBe(false);
    expect(terminalMenuItems(row, "reviewer", context({}), undefined, undefined, true).some((item) => item.id === "open_in_overview")).toBe(true);
  });
});

// PRD hide-plan B11: `계획` opens the session's plan, offered only while it has one.
describe("the pane menu's Plan item", () => {
  const row = { id: "w1:p1", children: null, lineage_path: [], sleep_action: { available: true } } as unknown as PaneRow;

  it("follows Show in Overview on a pane whose session has a plan, and is absent otherwise", () => {
    expect(paneMenuItems(row, "reviewer", undefined, undefined, true, true).map((item) => item.id)).toEqual(["open_in_overview", "open_plan", "sleep_agent", "copy_name", "copy_pane_id", "close_pane"]);
    expect(paneMenuItems(row, "reviewer", undefined, undefined, true, true)[1]).toMatchObject({ label: "Plan", unavailable: null });
    expect(paneMenuItems(row, "reviewer", undefined, undefined, true).some((item) => item.id === "open_plan")).toBe(false);
    expect(terminalMenuItems(row, "reviewer", context({}), undefined, undefined, true, true).some((item) => item.id === "open_plan")).toBe(true);
  });
});
