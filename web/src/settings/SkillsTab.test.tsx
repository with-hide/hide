// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { clientI18n } from "../i18n/translator";
import type { SnapshotRest, StoreReport } from "../snapshot";
import { useShellStore } from "../store";
import { SkillsTab } from "./SkillsTab";

// Settings › 스킬·MCP before and after a machine's first store report: three
// quiet rows while it is read, the empty state once it holds nothing, and
// the table once it holds an item (PRD skill-mcp-manager, layout C).
afterEach(() => {
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

const agents = [{ id: "claude-code", label: "Claude Code", availability: "available", enabled: true, chosen: false, skill: { state: "installed", why: null, location: null }, hook: null, doc_url: "https://example.test" }];

async function render(store: StoreReport | undefined) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  await clientI18n.changeLanguage("en");
  const rest = { status: { remote: [] }, navigator: { devices: [{ id: "mac", kind: "local", kit: { agents, components: [], store } }] } } as unknown as SnapshotRest;
  useShellStore.setState({ rest, connection: "live" });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><SkillsTab actions={createActions(() => {})} /></TooltipProvider>));
  return { container, unmount: () => act(async () => root.unmount()) };
}

const report = (items: StoreReport["items"]): StoreReport => ({ columns: { skill: [{ id: "claude", agents: ["claude-code"] }] }, items, unmanaged: [], trash: [], previews: [], diffs: [] }) as StoreReport;

it("reads as three quiet rows until the machine's first report, then as empty when it holds nothing", async () => {
  const reading = await render(undefined);
  expect(reading.container.querySelector("[data-store-loading]")).not.toBeNull();
  expect(reading.container.querySelector("[data-store-empty]")).toBeNull();
  await reading.unmount();

  const empty = await render(report([]));
  expect(empty.container.querySelector("[data-store-loading]")).toBeNull();
  expect(empty.container.querySelector('[data-store-empty="mac"]')?.textContent).toContain("Nothing on");
  await empty.unmount();
});

it("draws a held item as a row of the kind's table that unfolds below itself", async () => {
  const item = { id: "skill:archify", kind: "skill", name: "archify", source: { kind: "folder", path: "/skills/archify" }, version: { kind: "folder" }, on: true, cells: [{ column: "claude", state: "on" }] };
  const { container, unmount } = await render(report([item] as StoreReport["items"]));
  try {
    expect(container.querySelector('[data-store-cell="skill:archify:claude"]')?.getAttribute("data-store-cell-state")).toBe("on");
    expect(container.querySelector('[data-store-detail="skill:archify"]')).toBeNull();
    await act(async () => container.querySelector<HTMLElement>('[data-store-row="skill:archify"] td')!.click());
    expect(container.querySelector('[data-store-detail="skill:archify"]')?.textContent).toContain("/skills/archify");
  } finally {
    await unmount();
  }
});
