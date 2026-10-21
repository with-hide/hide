// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { legacyAgentRow } from "../test/legacyAgentRow";
import { AgentPrMark } from "./components/pr-mark";
import { AgentRecordTitle } from "./components/agent-record-title";
import { TooltipProvider } from "./components/ui/tooltip";
import { clientI18n } from "./i18n/translator";
import type { SessionPull } from "./prMark";
import type { AgentPullRequest, PrState } from "./snapshot";
import { useUiStore } from "./ui";

// The shell imports xterm, whose canvas API is absent from jsdom.
vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });

const savedUi = useUiStore.getState();
const savedLanguage = clientI18n.language;
afterEach(async () => {
  useUiStore.setState(savedUi, true);
  await clientI18n.changeLanguage(savedLanguage);
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

it.each([
  ["en", "checking", "Checking connection"],
  ["ko", "unconfirmed", "연결 미확인"],
] as const)("names provisional history in %s and keeps its PR accessible", async (language, connection, words) => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  await clientI18n.changeLanguage(language);
  const pull: AgentPullRequest = { number: 231, title: "연결된 결과물", url: "https://example.invalid/pull/231", badge: "open", checks: "pending", head_branch: "feature/old", closing_issues: [], live: false, duty: false, created: false, settled_at_unix_ms: null };
  const agent = { ...legacyAgentRow({ pane_id: "resumed", identity_label: "긴 한국어 제목과 mixed text 기록" }), record_connection: connection, record_summary: "저장된 짧은 요약" };
  const opened: AgentPullRequest[] = [];
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () => root.render(<TooltipProvider><AgentRecordTitle agent={agent} /><AgentPrMark agent={agent} family={[{pull,state:"pending",owners:[agent]}]} onOpen={p => opened.push(p)} /></TooltipProvider>));
    // An unconfirmed title is only dimmed; its words ride the hint and accessible name.
    expect(container.querySelector("[data-agent-title]")?.getAttribute("data-record-connection")).toBe(connection);
    expect(container.textContent).not.toContain(words);
    expect(container.querySelector("[data-agent-title]")?.parentElement?.getAttribute("title")).toContain(words);
    expect(container.querySelector("[data-agent-title]")?.textContent).toBe(agent.identity_label);
    const button = container.querySelector("button")!;
    expect(button.getAttribute("aria-label")).toContain(words);
    expect(button.dataset.recordProvisional).toBe("true");
    await act(async () => button.click());
    expect(opened.map(p => p.url)).toEqual([pull.url]);
    await act(async () => root.render(<AgentRecordTitle agent={{ ...agent, record_connection: "confirmed" }} />));
    expect(container.querySelector("[data-record-connection]")).toBeNull();
  } finally {
    await act(async () => root.unmount());
  }
});

// B11/B12: same-shaped PR icons must name each item's own state, rather than
// leaving the family's worst state as the only one a screen reader can read.
it.each([
  ["en", ["231 CI failed", "232 CI running or review pending", "233 Mergeable"]],
  ["ko", ["231 CI 실패", "232 CI 도는 중이거나 리뷰 대기", "233 머지 가능"]],
] as const)("reads each family PR state in %s and selects that PR", async (language, words) => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  await clientI18n.changeLanguage(language);
  const agent = legacyAgentRow({ pane_id: "parent", identity_label: "부모" });
  const states: PrState[] = ["failed", "pending", "mergeable"];
  const family: SessionPull[] = states.map((state, index) => ({
    state,
    pull: { number: 231 + index, title: `검증 ${index}`, url: `https://example.invalid/pull/${231 + index}`, badge: "open", checks: "pending", head_branch: "feat/menu", closing_issues: [], live: true, duty: false, created: true, settled_at_unix_ms: null },
    owners: [legacyAgentRow({ pane_id: `child-${index}`, identity_label: `소유 세션 ${index}` })],
  }));
  const opened: AgentPullRequest[] = [];
  let rowOpens = 0;
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () => root.render(<TooltipProvider><div onClick={() => rowOpens++}><AgentPrMark agent={agent} family={family} onOpen={(pull) => opened.push(pull)} /></div></TooltipProvider>));
    const trigger = container.querySelector("button")!;
    await act(async () => trigger.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
    const items = Array.from(document.querySelectorAll<HTMLElement>('[role="menuitem"]'));
    expect(items).toHaveLength(3);
    expect(items.map((item) => item.querySelector('[role="img"]')?.getAttribute("aria-label"))).toEqual(words.map((word) => "#" + word));
    expect(items.map((item) => item.getAttribute("aria-label"))).toEqual(words.map((word, index) => `#${word} · 검증 ${index} · 소유 세션 ${index}`));
    expect(items.map((item) => item.textContent)).toEqual(["#" + "231검증 0소유 세션 0", "#" + "232검증 1소유 세션 1", "#" + "233검증 2소유 세션 2"]);
    await act(async () => items[1]!.click());
    expect(opened.map((pull) => pull.url)).toEqual(["https://example.invalid/pull/232"]);
    expect(rowOpens).toBe(0);
    expect(document.querySelector('[role="menu"]')).toBeNull();
  } finally {
    await act(async () => root.unmount());
  }
});
