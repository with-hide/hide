// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "../actions";
import { clientI18n } from "../i18n/translator";
import type { AgentRow } from "../snapshot";
import type { DispatchFn } from "../ws";
import { legacyAgentRow } from "../../test/legacyAgentRow";
import { PaneLetterButton } from "./letter-mark";
import { SidebarAgentRow, type AgentRowMenu } from "./sidebar-agent-row";
import { TooltipProvider } from "./ui/tooltip";

// The letter-waiting mark as docs/UI_BEHAVIOR.md (Agent panes) and
// docs/delivery.md (Delivering a held letter now) describe it: the core
// decides when it shows, the shell draws the pill and the envelope and sends
// one event per press.
vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });
afterEach(async () => {
  await clientI18n.changeLanguage("en");
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

async function render(node: React.ReactNode) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider>{node}</TooltipProvider>));
  return { container, rerender: (next: React.ReactNode) => act(async () => root.render(<TooltipProvider>{next}</TooltipProvider>)), unmount: () => act(async () => root.unmount()) };
}

it("draws the pane header pill with the count from two letters and sends one deliver event per press", async () => {
  await clientI18n.changeLanguage("ko");
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const pill = (letters: number) => <PaneLetterButton paneId="remote:mini:pane:w1:p2" letters={letters} onDeliver={actions.deliverLettersNow} />;
  const { container, rerender, unmount } = await render(pill(1));
  try {
    const button = () => container.querySelector<HTMLButtonElement>("[data-pane-letters]")!;
    expect(button().textContent).toBe("지금 전달");
    expect(button().getAttribute("aria-label")).toBe("대기 중인 편지 1통 지금 전달");
    await rerender(pill(3));
    expect(button().textContent).toBe("3지금 전달");
    expect(button().getAttribute("aria-label")).toBe("대기 중인 편지 3통 지금 전달");
    await act(async () => button().click());
    expect(events).toEqual([{ schema_version: 2, kind: "deliver_letters_now", payload: { pane_id: "remote:mini:pane:w1:p2" } }]);
  } finally {
    await unmount();
  }
});

it("draws letters held on an input no read finds as the same pill with the reason and no press", async () => {
  await clientI18n.changeLanguage("ko");
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const { container, unmount } = await render(<PaneLetterButton paneId="w1:p2" letters={2} unreadable onDeliver={actions.deliverLettersNow} />);
  try {
    const pill = container.querySelector<HTMLElement>("[data-pane-letters]")!;
    expect(pill.tagName).toBe("SPAN");
    expect(container.querySelector("button")).toBeNull();
    expect(pill.textContent).toBe("2칸이 좁음");
    expect(pill.getAttribute("aria-label")).toBe("편지 2통 대기 중: 칸이 좁거나 가려져 입력창을 읽을 수 없어요. 칸을 넓히면 전달돼요");
    await act(async () => pill.click());
    expect(events).toEqual([]);
  } finally {
    await unmount();
  }
});

it("marks a sidebar row with a quiet envelope and no number only while letters wait", async () => {
  await clientI18n.changeLanguage("en");
  const menu: AgentRowMenu = { items: () => [], onSelect: () => {} };
  const agent = (letters?: number): AgentRow => legacyAgentRow({
    id: "a", pane_id: "pane-a", identity_label: "Input return flow", agent_kind: "codex", mark: "quiet", group: "seen",
    status_code: "idle", changed_at_unix_ms: null, emphasized: false, unread: false, letters_waiting: letters,
  });
  const row = (letters?: number) => <ul><SidebarAgentRow agent={agent(letters)} device={null} place={null} depth={0} selected={false} onOpen={() => {}} onOpenPullRequest={() => {}} inset="0" menu={menu} /></ul>;
  const { container, rerender, unmount } = await render(row(2));
  try {
    const name = () => container.querySelector("[data-agent-open]")!.getAttribute("aria-label");
    const mark = container.querySelector('[data-pane="pane-a"] [data-agent-letters]');
    expect(mark).not.toBeNull();
    expect(mark!.closest("li")!.textContent).not.toContain("2");
    expect(name()).toContain("Letter waiting");
    await rerender(row(undefined));
    expect(container.querySelector("[data-agent-letters]")).toBeNull();
    expect(name()).not.toContain("Letter waiting");
  } finally {
    await unmount();
  }
});
