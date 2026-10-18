// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { clientI18n } from "../i18n/translator";
import type { AgentRow } from "../snapshot";
import { legacyAgentRow } from "../../test/legacyAgentRow";
import { RowLineText } from "./row-line-text";
import { SidebarAgentRow, type AgentRowMenu } from "./sidebar-agent-row";
import { TooltipProvider } from "./ui/tooltip";

// A row's second line as docs/status-model.md (The mark) describes it: the
// lead word and the background tail keep their width, the sentence between
// them is the part a narrow row cuts (status-marks B27, B29), a waiting
// ring's line reaches assistive technology, and a sleeping row keeps its moon
// with no line (B31).
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
  return { container, unmount: () => act(async () => root.unmount()) };
}

it("keeps the lead and the tail whole and cuts only the sentence, with one separator between parts", async () => {
  const { container, unmount } = await render(<>
    <RowLineText data-case="all" parts={{ lead: "끝남", text: "배포 시작함", tail: "백그라운드 2" }} />
    <RowLineText data-case="lead-tail" parts={{ lead: "끝남", text: null, tail: "백그라운드 1" }} />
    <RowLineText data-case="text" parts={{ lead: null, text: "파서 고치는 중", tail: null }} />
  </>);
  try {
    const line = (name: string) => container.querySelector(`[data-case="${name}"]`)!;
    const cut = (name: string) => [...line(name).querySelectorAll("[data-line-part]")].map((part) => [part.getAttribute("data-line-part"), part.classList.contains("truncate")]);
    expect(line("all").textContent).toBe("끝남 · 배포 시작함 · 백그라운드 2");
    expect(cut("all")).toEqual([["lead", false], ["text", true], ["tail", false]]);
    expect(line("lead-tail").textContent).toBe("끝남 · 백그라운드 1");
    expect(cut("lead-tail")).toEqual([["lead", false], ["tail", false]]);
    // The separator sits inside the part after it, so a part that is absent leaves none behind.
    expect([...line("all").querySelectorAll("[data-line-part]")].map((part) => part.textContent)).toEqual(["끝남", " · 배포 시작함", " · 백그라운드 2"]);
    expect(line("text").textContent).toBe("파서 고치는 중");
    expect(cut("text")).toEqual([["text", true]]);
  } finally {
    await unmount();
  }
});

it("draws a waiting ring dashed and names it by its whole line, and draws a sleeping row's moon with no line, undimmed", async () => {
  await clientI18n.changeLanguage("ko");
  const menu: AgentRowMenu = { items: () => [], onSelect: () => {} };
  const waiting: AgentRow = legacyAgentRow({ id: "w", pane_id: "w", identity_label: "CI 확인", agent_kind: "claude", mark: "waiting", group: "working", status_code: "waiting", wait: "checks", activity: "stopped", changed_at_unix_ms: null, emphasized: false, unread: false });
  waiting.state = { ...waiting.state, line: { text: "", mode: "waiting", tone: { kind: "subtle", read: false }, pull_request: 58 } };
  const sleeping: AgentRow = legacyAgentRow({ id: "s", pane_id: "s", identity_label: "파서 리팩터링", agent_kind: "claude", mark: "asleep", group: "seen", status_code: "sleeping", activity: "stopped", detail: "파서를 나눴어요", changed_at_unix_ms: null, emphasized: false, unread: false });
  const row = (agent: AgentRow) => <SidebarAgentRow agent={agent} device={null} place={null} depth={0} selected={false} onOpen={() => {}} onOpenPullRequest={() => {}} inset="0" menu={menu} />;
  const { container, unmount } = await render(<ul>{row(waiting)}{row(sleeping)}</ul>);
  try {
    const item = (pane: string) => container.querySelector(`[data-pane="${pane}"]`)!;
    expect(item("w").querySelector("[data-agent-line]")!.textContent).toBe("대기 · CI · PR #58");
    // A waiting ring is dashed and says so (status-marks D-25, B36).
    expect(item("w").querySelector("[data-mark]")?.getAttribute("data-mark")).toBe("waiting");
    expect(item("w").querySelector('[data-mark] path')?.getAttribute("stroke-dasharray")).toBe("2.2 2");
    expect(item("w").querySelector("[data-agent-open]")!.getAttribute("aria-label")).toContain("진행 중 · 대기");
    expect(item("w").querySelector("[data-agent-open]")!.getAttribute("aria-label")).toContain("대기 · CI · PR #58");
    expect(item("s").querySelector('[data-mark="asleep"]')).not.toBeNull();
    expect(item("s").querySelector("[data-agent-line]")).toBeNull();
    expect(item("s").className).not.toContain("opacity-(--opacity-dimmed)");
    expect(item("s").querySelector("[data-agent-open]")!.getAttribute("aria-label")).toContain("재움 · 열면 깨어남");
  } finally {
    await unmount();
  }
});

it("draws a resting row's faint hollow ring, undimmed, and an unknown row with no shape, dimmed whole (sidebar-quiet-mark-indent B1, B2, B5)", async () => {
  await clientI18n.changeLanguage("ko");
  const menu: AgentRowMenu = { items: () => [], onSelect: () => {} };
  const quiet: AgentRow = legacyAgentRow({ id: "q", pane_id: "q", identity_label: "에이전트 포크 기능 기획안", agent_kind: "claude", mark: "quiet", group: "seen", status_code: "idle", activity: "stopped", changed_at_unix_ms: null, emphasized: false, unread: false });
  const unknown: AgentRow = legacyAgentRow({ id: "u", pane_id: "u", identity_label: "상태 확인 중", agent_kind: "claude", mark: "unknown", group: "seen", status_code: "unknown", activity: "unknown", changed_at_unix_ms: null, emphasized: false, unread: false });
  const row = (agent: AgentRow) => <SidebarAgentRow agent={agent} device={null} place={null} depth={0} selected={false} onOpen={() => {}} onOpenPullRequest={() => {}} inset="0" menu={menu} />;
  const { container, unmount } = await render(<ul>{row(quiet)}{row(unknown)}</ul>);
  try {
    const item = (pane: string) => container.querySelector(`[data-pane="${pane}"]`)!;
    // The ring is an outline inside the 12 px slot: a circle with no fill, thinner than the in-progress ring's 1.9.
    const ring = item("q").querySelector('[data-mark="quiet"] circle')!;
    expect([ring.getAttribute("fill"), ring.getAttribute("r"), ring.getAttribute("stroke-width")]).toEqual(["none", "3.4", "1.3"]);
    expect(item("q").className).not.toContain("opacity-(--opacity-dimmed)");
    expect(item("q").querySelector("[data-agent-open]")!.getAttribute("aria-label")).toContain("조용함");
    // The unknown row keeps the slot but draws nothing in it, and says why.
    expect(item("u").querySelector('[data-mark="unknown"]')!.childElementCount).toBe(0);
    expect(item("u").className).toContain("opacity-(--opacity-dimmed)");
    expect(item("u").querySelector("[data-agent-open]")!.getAttribute("aria-label")).toContain("상태 모름");
  } finally {
    await unmount();
  }
});
