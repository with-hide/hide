// @vitest-environment jsdom
// PRD hide-external-control B44: while the rail is hidden the top row's menu
// lists the tools at Act apart under the devices, with their letters and grey
// number, and picking one shows its cell as the rail's tile does.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { HiddenRailMenu } from "./components/sidebar-header";
import { TooltipProvider } from "./components/ui/tooltip";
import { initializeInterfaceI18n } from "./i18n/instance";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });

afterEach(() => {
  vi.unstubAllGlobals();
  document.body.replaceChildren();
});

it("lists the tools apart under the devices, and picking one selects its cell", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  initializeInterfaceI18n("en");
  const picked: string[] = [];
  const menu = {
    devices: [{ id: "local", label: "This Mac", remote: false, connected: true }],
    frontId: "local",
    tools: [{ connection: "c0ffee00c0ffee00", name: "nightly", turn: 2 }, { connection: "beef0000beef0000", name: "weekly", turn: 0 }],
    toolFront: null,
    onSelectTool: (connection: string) => picked.push(connection),
    onSelect: () => undefined,
    onAddDevice: () => undefined,
    onShowRail: () => undefined,
  };
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><HiddenRailMenu title={{ name: "This Mac", note: null }} menu={menu} /></TooltipProvider>));
  // Radix opens the menu on Enter at its trigger, as a keyboard does.
  await act(async () => { container.querySelector("[data-sidebar-device-menu]")?.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
  const items = [...document.body.querySelectorAll<HTMLElement>("[data-device-menu-item], [data-device-menu-tool]")];
  expect(items.map((item) => item.dataset.deviceMenuItem ?? `tool:${item.dataset.deviceMenuTool}`)).toEqual(["local", "tool:c0ffee00c0ffee00", "tool:beef0000beef0000"]);
  expect(items.slice(1).map((item) => item.textContent)).toEqual(["NnightlyTool's turn 2", "WweeklyExternal tool"]);
  await act(async () => { items[1]!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
  expect(picked).toEqual(["c0ffee00c0ffee00"]);
  await act(async () => root.unmount());
});
