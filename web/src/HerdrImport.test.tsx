// @vitest-environment jsdom
// PRD engine-bundle-4-herdr-removal B11, B15, B16, B18: the sidebar's line and Settings' import, drawn from the core's
// `status.herdr_import` and, per device, `status.herdr_import_devices`.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { HerdrImportGroup, HerdrImportLine } from "./HerdrImport";
import { TooltipProvider } from "./components/ui/tooltip";
import type { HerdrImport } from "./snapshot";
import { useShellStore } from "./store";
import type { DispatchFn } from "./ws";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

const found = (overrides: Partial<HerdrImport> = {}): HerdrImport => ({
  kind: "import",
  stage: "offered",
  workspaces: 2,
  tabs: 3,
  agents: 2,
  waiting: 0,
  herdr_running: true,
  failure: null,
  reason: null,
  refused: [],
  ...overrides,
});

const sent = (events: Parameters<DispatchFn>[0][]) => events.filter((event) => event.kind === "herdr_import").map((event) => event.payload);
const button = (selector: string) => document.querySelector(selector) as HTMLButtonElement | null;

let unmounts: (() => Promise<void>)[] = [];

async function mount(shown: HerdrImport | null, devices: HerdrImport[] = [], named: { id: string; label: string; kind: "local" | "remote" }[] = []) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const saved = useShellStore.getState();
  await act(async () => {
    useShellStore.setState({
      connection: "live",
      revision: 1,
      rest: { status: { herdr_import: shown, herdr_import_devices: devices }, navigator: { devices: named } },
    } as never);
    root.render(
      <TooltipProvider>
        <HerdrImportLine actions={actions} />
        <HerdrImportGroup actions={actions} />
      </TooltipProvider>,
    );
  });
  unmounts.push(async () => {
    await act(async () => root.unmount());
    container.remove();
    useShellStore.setState(saved, true);
  });
  return events;
}

afterEach(async () => {
  for (const unmount of unmounts) await unmount();
  unmounts = [];
  document.body.innerHTML = "";
});

it("imports another Herdr's save read only from the line and from Settings", async () => {
  const events = await mount(found({ kind: "import" }));
  expect(document.querySelector("[data-herdr-import-line=import]")?.textContent).toContain("Conversations open in Herdr open here as shells");
  await act(async () => button("[data-herdr-import-import]")!.click());
  await act(async () => button("[data-herdr-import-settings-import]")!.click());
  expect(sent(events)).toEqual([{ action: "import" }, { action: "import" }]);
});

it("shows a save it could not read with why and a retry, and nothing for no save", async () => {
  const events = await mount(found({ kind: "import", stage: "failed", failure: "unreadable", reason: "session.json is not a Herdr session", workspaces: 0, tabs: 0, agents: 0 }));
  const line = document.querySelector("[data-herdr-import-line=unreadable]");
  expect(line?.textContent).toContain("Couldn't read the Herdr session");
  expect(line?.textContent).toContain("session.json is not a Herdr session");
  expect(document.querySelector("[data-herdr-import-settings]")).toBeNull();
  await act(async () => button("[data-herdr-import-retry]")!.click());
  expect(sent(events)).toEqual([{ action: "look" }]);
});

it("lists the workspaces a move left out until they are read", async () => {
  const events = await mount(found({ kind: "result", stage: "moved", workspaces: 0, refused: [{ label: "api", reason: "its layout names a pane twice" }] }));
  expect(document.querySelector("[data-herdr-import-line=refused]")?.textContent).toContain("api: its layout names a pane twice");
  await act(async () => button("[data-herdr-import-done]")!.click());
  expect(sent(events)).toEqual([{ action: "continue" }]);
});

it("offers to resume the conversations an import left open once their Herdr closed", async () => {
  const events = await mount(found({ kind: "import", workspaces: 0, tabs: 0, agents: 0, waiting: 2, herdr_running: false }));
  expect(document.querySelector("[data-herdr-import-line=waiting]")?.textContent).toContain("2 conversations can resume here");
  expect(document.querySelector("[data-herdr-import-settings]")).toBeNull();
  await act(async () => button("[data-herdr-import-resume]")!.click());
  expect(sent(events)).toEqual([{ action: "import" }]);
});

it("draws nothing while there is no save", async () => {
  await mount(null);
  expect(document.querySelector("[data-herdr-import-line], [data-herdr-import-settings]")).toBeNull();
});

