// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { usePaneMenu, type TerminalMenuContext } from "./PaneRelations";
import type { TerminalContextLink } from "./terminalLinkProvider";
import type { PaneRow } from "./snapshot";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

afterEach(() => {
  document.body.innerHTML = "";
});

const row = (over: Partial<PaneRow> = {}) => ({ id: "w1:p2", children: null, lineage_path: [], ...over }) as unknown as PaneRow;

/** The menu a pane header opens, kept open while the pane's row and name change under it. */
async function openMenu(first: PaneRow, firstTitle: string) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  const actions = createActions(() => true);
  let open: (x: number, y: number) => void = () => {};
  let openTerminal: ReturnType<typeof usePaneMenu>["openTerminalAt"] = () => {};
  function Header({ pane, title }: { pane: PaneRow; title: string }) {
    const menu = usePaneMenu(pane, title, actions);
    open = menu.openAt;
    openTerminal = menu.openTerminalAt;
    return menu.menu;
  }
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const render = (pane: PaneRow, title: string) => act(async () => root.render(<Header pane={pane} title={title} />));
  await render(first, firstTitle);
  await act(async () => open(10, 10));
  const labels = () => Array.from(document.querySelectorAll('[role="menuitem"]')).map((item) => item.textContent);
  return {
    render, labels,
    header: () => act(async () => open(10, 10)),
    terminal: (context: TerminalMenuContext, link: Promise<TerminalContextLink | null>) => act(async () => openTerminal(10, 10, context, link)),
    unmount: () => act(async () => root.unmount()),
  };
}

it("keeps a dismissed or newer menu when a terminal link resolves late", async () => {
  const menu = await openMenu(row(), "Shell");
  const context: TerminalMenuContext = {
    selection: false, zoomed: false, paneCount: 1,
    chords: { copy: "", paste: "", find: "", splitRight: "", splitDown: "", zoom: "" },
  };
  for (const replace of [false, true]) {
    let resolve!: (link: TerminalContextLink) => void;
    const pending = new Promise<TerminalContextLink>((done) => { resolve = done; });
    await menu.terminal(context, pending);
    expect(menu.labels()).toContain("Paste");
    if (replace) await menu.header();
    else await act(async () => document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    await act(async () => resolve({ target: { kind: "url", url: "https://example.test/" }, found: null }));
    expect(menu.labels()).not.toContain("Copy link");
    if (replace) expect(menu.labels()).toContain("Close terminal");
    else expect(menu.labels()).toEqual([]);
  }
  await menu.unmount();
});

it("names the pane as its header does while the menu is open", async () => {
  const menu = await openMenu(row(), "Claude");
  expect(document.querySelector("[data-menu-title]")?.textContent).toBe("Claude");
  // The core's label for the session arrives after the menu opened.
  await menu.render(row(), "Agent two");
  expect(document.querySelector("[data-menu-title]")?.textContent).toBe("Agent two");
  expect(menu.labels()).toContain("Close terminal");
  await menu.unmount();
});

it("offers what the core now says about the pane while the menu is open", async () => {
  const menu = await openMenu(row({ fork: { available: false, reason: "This agent has not reported its conversation yet" } }), "Agent two");
  const fork = () => document.querySelector('[data-menu-group="fork"]');
  expect(fork()?.hasAttribute("data-disabled")).toBe(true);
  await menu.render(row({ fork: { available: true } }), "Agent two");
  expect(fork()?.hasAttribute("data-disabled")).toBe(false);
  await menu.unmount();
});
