// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { legacyAgentRow } from "../../test/legacyAgentRow";
import type { Subtree } from "../close";
import type { AgentRow } from "../snapshot";
import { useShellStore } from "../store";
import { SubtreeList } from "./subtree-list";
import { TooltipProvider } from "./ui/tooltip";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });

afterEach(() => {
  window.location.hash = "";
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

// The MacBook's window after the core moved to the Mac mini: the core's row
// keeps the name it was registered under, and the MacBook dialed in under its
// host name, which the core hands out as its agents' device label.
it("names a descendant on this window's own machine This Mac, not by the host name the core knows it by", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("navigator", { platform: "MacIntel", userAgent: "" });
  window.location.hash = "#token=t&node=mbp";
  const agent = (pane: string, device: string, label: string | null) => legacyAgentRow({
    id: `agent:${pane}`, pane_id: pane, identity_label: "아주 긴 한국어 mixed title", record_connection: "unconfirmed", record_summary: "저장된 요약", agent_kind: "claude", mark: "going", group: "working",
    status_code: "working", activity: "working", device_id: device, ...(label ? { device_label: label } : {}),
  }) as AgentRow;
  const subtree: Subtree = {
    ids: ["child"],
    rows: [
      { agent: agent("parent", "mini-node", null), depth: 0, target: true, state: "working" },
      { agent: agent("child", "mbp", "MacBook-Pro.local"), depth: 1, target: false, state: "working" },
    ],
    counts: { working: 1, waiting: 0, unread: 0, unknown: 0 },
    unknown: false,
    targetUnknown: false,
  } as Subtree;
  const saved = useShellStore.getState();
  useShellStore.setState({ rest: { navigator: { devices: [
    { id: "mini-node", kind: "local", label: "Mac mini", machine_name: "Mac-mini.local" },
    { id: "mbp", kind: "remote", label: "MacBook-Pro.local", dials_in: true },
  ] } } as never });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () => root.render(<TooltipProvider><SubtreeList subtree={subtree} targetDevice="mini-node" /></TooltipProvider>));
    const child = container.querySelector('[data-subtree-row="child"]')?.getAttribute("aria-label") ?? "";
    expect(child).toContain("This Mac");
    expect(child).toContain("Connection unconfirmed");
    expect(child).toContain("저장된 요약");
    expect(container.querySelectorAll('[data-record-connection="unconfirmed"]')).toHaveLength(2);
    expect(child).not.toContain("MacBook-Pro.local");
  } finally {
    await act(async () => root.unmount());
    useShellStore.setState(saved, true);
  }
});

// The operator's case: 33 children whose agents ended and one still working.
it("folds the finished children under one line that opens, keeping the working one in the tree", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const agent = (pane: string, status: "working" | "idle") => legacyAgentRow({ id: `agent:${pane}`, pane_id: pane, identity_label: pane, agent_kind: "claude", mark: status === "working" ? "going" : "quiet", status_code: status, activity: status }) as AgentRow;
  const children = Array.from({ length: 33 }, (_, index) => ({ agent: agent(`c${index}`, index === 7 ? "working" : "idle"), depth: 1, target: false, state: index === 7 ? "working" : "quiet" }));
  const subtree = {
    ids: children.map((row) => row.agent.pane_id),
    rows: [{ agent: agent("parent", "idle"), depth: 0, target: true, state: "quiet" }, ...children],
    counts: { working: 1, waiting: 0, unread: 0, unknown: 0 },
    unknown: false,
    targetUnknown: false,
  } as Subtree;
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const rows = () => [...container.querySelectorAll("[data-subtree-row]")].map((row) => row.getAttribute("data-subtree-row"));
  try {
    await act(async () => root.render(<TooltipProvider><SubtreeList subtree={subtree} targetDevice={undefined} /></TooltipProvider>));
    expect(rows()).toEqual(["parent", "c7"]);
    const fold = container.querySelector<HTMLButtonElement>("[data-subtree-fold]")!;
    expect(fold.textContent).toBe("32 finished children");
    expect(fold.getAttribute("aria-expanded")).toBe("false");
    await act(async () => fold.click());
    expect(fold.getAttribute("aria-expanded")).toBe("true");
    expect(rows()).toHaveLength(34);
    expect(rows().slice(0, 3)).toEqual(["parent", "c7", "c0"]);
  } finally {
    await act(async () => root.unmount());
  }
});
