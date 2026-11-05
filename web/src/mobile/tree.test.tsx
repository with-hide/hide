// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useInterfaceTranslation } from "../i18n/translator";
import { App } from "./App";
import { HostList } from "./HostList";
import { HOSTS_KEY } from "./hosts";
import type { AgentGroup, PhoneAgent, PhoneDevice } from "./protocol";
import { applyFrame, patch, usePhone } from "./store";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });
const initial = usePhone.getState();
afterEach(() => {
  usePhone.setState(initial, true);
  vi.unstubAllGlobals();
  localStorage.clear();
  history.replaceState(null, "", "/");
});

function agent(pane: string, over: Partial<PhoneAgent> = {}): PhoneAgent {
  return { device_id: "local", pane_id: pane, root_pane_id: pane, group: "working", mark: "going", wait: null, descendants_working: 0, agent_kind: "claude", title: `작업 ${pane}`, place: null, device_label: null, changed_at_unix_ms: null, line: null, status_code: "idle", demand: "none", emphasized: false, holds_notification: false, ...over };
}

const top = (row: PhoneAgent) => ({ kind: "agent" as const, ...row });
const working = (rows: PhoneAgent[], more: PhoneAgent[] = [], extra: Partial<AgentGroup> = {}): AgentGroup => ({ group: "working", count: rows.length + more.length, rows: rows.map(top), ...(more.length ? { more: more.map(top) } : {}), ...extra });
const kids = (n: number, from = 0) => Array.from({ length: n }, (_, at) => agent(`c${from + at}`));

async function mount(groups: AgentGroup[], devices: PhoneDevice[] = []) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  patch({ screen: "app", groups, devices });
  const container = document.createElement("div");
  const root = createRoot(container);
  await act(async () => root.render(<App />));
  return { container, unmount: () => act(async () => root.unmount()) };
}

const click = (container: HTMLElement, selector: string) => act(async () => container.querySelector<HTMLElement>(selector)!.click());
const panes = (container: HTMLElement, within: string) => [...container.querySelectorAll(`${within} [data-phone-agent]`)].map((row) => row.getAttribute("data-phone-agent"));

describe("a parent's children on the list", () => {
  it("stay folded under the parent until its toggle opens them, and only the toggle does", async () => {
    const parent = agent("p", { children: kids(2), descendants: { asking: 0, working: 2, done: 0 } });
    const { container, unmount } = await mount([working([parent])]);
    try {
      expect(container.querySelector('[data-phone-children="local|p"]')).toBeNull();
      await click(container, '[data-phone-children-toggle="local|p"]');
      expect(panes(container, '[data-phone-children="local|p"]')).toEqual(["local|c0", "local|c1"]);
      await click(container, '[data-phone-children-toggle="local|p"]');
      expect(container.querySelector('[data-phone-children="local|p"]')).toBeNull();
      // The parent's own row opens the parent, never its children.
      await click(container, '[data-phone-agent="local|p"]');
      expect(usePhone.getState().detail?.key).toEqual({ device_id: "local", pane_id: "p" });
      expect(container.querySelector('[data-phone-children="local|p"]')).toBeNull();
    } finally {
      await unmount();
    }
  });

  it("open at first when the core opened them, and keep what this phone chose through the next lists", async () => {
    const parent = agent("p", { children_open: true, children: kids(1), descendants: { asking: 1, working: 0, done: 0 } });
    const { container, unmount } = await mount([working([parent])]);
    try {
      expect(container.querySelector('[data-phone-children="local|p"]')).not.toBeNull();
      await click(container, '[data-phone-children-toggle="local|p"]');
      expect(container.querySelector('[data-phone-children="local|p"]')).toBeNull();
      const frame = { type: "agents" as const, devices: [], interface_language: null, color_theme: null };
      await act(async () => applyFrame({ ...frame, groups: [working([parent])] }));
      expect(container.querySelector('[data-phone-children="local|p"]')).toBeNull();
      // Once the parent leaves the list, the choice is not kept.
      await act(async () => applyFrame({ ...frame, groups: [working([agent("other")])] }));
      expect(usePhone.getState().childrenOpen).toEqual({});
    } finally {
      await unmount();
    }
  });

  it("show five siblings and open the rest in place", async () => {
    const parent = agent("p", { children_open: true, children: kids(5), more_children: kids(2, 5) });
    const { container, unmount } = await mount([working([parent])]);
    try {
      expect(panes(container, '[data-phone-children="local|p"]')).toHaveLength(5);
      await click(container, '[data-phone-fold="children:local|p"]');
      expect(panes(container, '[data-phone-children="local|p"]')).toHaveLength(7);
      expect(container.querySelector('[data-phone-fold="children:local|p"]')).toBeNull();
    } finally {
      await unmount();
    }
  });

  it("wear a pill for each state they are in, named for a screen reader", async () => {
    const parent = agent("p", { children: kids(3), descendants: { asking: 1, working: 1, done: 1 } });
    const { container, unmount } = await mount([working([parent])]);
    try {
      const pills = [...container.querySelectorAll('[data-phone-children-toggle="local|p"] [role="img"]')].map((pill) => pill.getAttribute("aria-label"));
      expect(pills).toEqual(["1 asking you", "1 working", "1 finished"]);
      expect(container.querySelector('[data-phone-children-toggle="local|p"]')!.textContent).toContain("3 children");
    } finally {
      await unmount();
    }
  });
});

describe("a group's fold", () => {
  it("counts every top row, lists the first few and opens the rest in place with its sleeping ones named", async () => {
    const group = working(kids(3), kids(2, 3), { more_asleep: 1 });
    const { container, unmount } = await mount([group]);
    try {
      expect(container.querySelector("[data-phone-group-count]")!.textContent).toBe("5");
      expect(panes(container, '[data-phone-group="working"]')).toHaveLength(3);
      const fold = container.querySelector('[data-phone-fold="group:working"]')!;
      expect(fold.textContent).toContain("Show 2 more");
      expect(fold.textContent).toContain("1 asleep included");
      await click(container, '[data-phone-fold="group:working"]');
      expect(panes(container, '[data-phone-group="working"]')).toHaveLength(5);
      expect(container.querySelector('[data-phone-fold="group:working"]')).toBeNull();
    } finally {
      await unmount();
    }
  });
});

describe("the device sheet", () => {
  function Sheet({ machines }: { machines: PhoneDevice[] }) {
    const { t } = useInterfaceTranslation();
    return <HostList t={t} machines={machines} activeRoom={"a".repeat(43)} active={{ online: true, lastSeen: null, needs: 3, working: 9 }} />;
  }

  it("draws the devices the Mac reaches under its row, the unconnected one dimmed, and says no pairing is needed", async () => {
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    const room = "a".repeat(43);
    localStorage.setItem(HOSTS_KEY, JSON.stringify([{ room, name: "MacBook Pro", noiseKey: "b".repeat(43), credential: "credential", protocol: 2, lastSeen: null }]));
    const machines: PhoneDevice[] = [
      { device_id: "local", label: null, local: true, connected: true, needs_you: 3, working: 9 },
      { device_id: "mini", label: "Mac mini", connected: true, needs_you: 1, working: 3 },
      { device_id: "old", label: "Old box", connected: false, needs_you: 0, working: 0 },
    ];
    const container = document.createElement("div");
    const root = createRoot(container);
    await act(async () => root.render(<Sheet machines={machines} />));
    try {
      const reached = container.querySelector('[data-phone-devices="true"]')!;
      expect(reached.textContent).toContain("Devices connected through this Mac");
      const mini = reached.querySelector('[data-phone-device-row="mini"]')!;
      expect(mini.textContent).toContain("Mac mini");
      expect(mini.textContent).toContain("Needs you 1 · Working 3");
      const old = reached.querySelector('[data-phone-device-row="old"]')!;
      expect(old.getAttribute("data-connected")).toBe("false");
      expect(old.className).toContain("opacity-(--opacity-dimmed)");
      expect(old.textContent).not.toContain("Needs you");
      expect(reached.textContent).toContain("Mac mini, Old box agents appear in the list with a chip");
      // The Mac's own row says it is the one being viewed.
      expect(container.textContent).toContain("Connected · Viewing now");
    } finally {
      await act(async () => root.unmount());
    }
  });
});
