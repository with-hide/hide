// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "../actions";
import { clientI18n } from "../i18n/translator";
import type { AccessConnection, AccessSection } from "../snapshot";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";
import type { DispatchFn } from "../ws";
import { ExternalToolsTab } from "./ExternalToolsTab";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

// A whole minute, so the shared clock read to the minute is this moment.
const NOW = Date.UTC(2026, 10, 14, 8, 0, 0);
const savedShell = useShellStore.getState();
const savedUi = useUiStore.getState();
const savedLanguage = clientI18n.language;

afterEach(async () => {
  useShellStore.setState(savedShell, true);
  useUiStore.setState(savedUi, true);
  await clientI18n.changeLanguage(savedLanguage);
  vi.unstubAllGlobals();
  vi.useRealTimers();
  document.body.innerHTML = "";
});

function tool(over: Partial<AccessConnection>): AccessConnection {
  return {
    id: "conn-1", name: "nightly-cron", level: "view", program: "/usr/local/bin/nightly", created_ms: NOW - 86_400_000,
    last_used_ms: NOW - 3 * 60_000, agent_count: 0, mission_count: 0, ...over,
  };
}

function section(over: Partial<AccessSection>): AccessSection {
  return { machine: "local", limit: 16, pending: [], connections: [], ...over };
}

async function mount(access: AccessSection | null, language: "en" | "ko" = "en", extra: Record<string, unknown> = {}) {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(NOW);
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  await clientI18n.changeLanguage(language);
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    useShellStore.setState({ connection: "live", access, ...extra } as never);
    useUiStore.getState().openSettings("externalTools");
    root.render(<ExternalToolsTab actions={actions} />);
  });
  const q = (selector: string) => container.querySelector(selector) as HTMLElement | null;
  const press = async (selector: string) => { await act(async () => { q(selector)!.click(); }); };
  return { events, q, press, container, unmount: () => act(async () => root.unmount()) };
}

const sent = (events: Parameters<DispatchFn>[0][]) => events.map((event) => [event.kind, event.payload]);

it("puts a request up with its machine and program, View only chosen, and approves it at the level picked (B2, B3)", async () => {
  const { events, q, press, container, unmount } = await mount(section({
    pending: [{ id: "req-1", name: "hermes-zutongg", program: "/opt/hermes/bin/hermes", requested_ms: NOW }],
  }), "ko");
  const row = q('[data-access-pending="req-1"]')!;
  expect(row.textContent).toContain("hermes-zutongg");
  expect(row.querySelector("[data-access-program]")?.textContent).toBe("/opt/hermes/bin/hermes");
  expect(row.textContent).toContain("보기만Hide에 보이는 것(상태, 에이전트 화면, 미션)을 볼 수만 있어요");
  expect(row.textContent).toContain("시키기보기에 더해 에이전트를 띄우고 끝내고 말을 넣고, 미션을 맡길 수 있어요");
  expect(q('[data-access-option="view"]')?.getAttribute("aria-checked")).toBe("true");
  expect(q("[data-access-approve]")?.textContent).toBe("보기만으로 승인");

  await press('[data-access-option="act"]');
  expect(q("[data-access-approve]")?.textContent).toBe("시키기로 승인");
  await press("[data-access-approve]");
  expect(sent(events)).toEqual([["access_approve", { id: "req-1", level: "act" }]]);
  // Nothing else on the page answers it, and a connected list stays empty until the core says otherwise.
  expect(container.querySelector('[data-access-empty="true"]')).not.toBeNull();
  await unmount();
});

it("approves at View only when nothing is picked, and refuses with the one button beside it (B3, B7)", async () => {
  const { events, press, unmount } = await mount(section({
    pending: [
      { id: "req-1", name: "one", program: null, requested_ms: NOW },
      { id: "req-2", name: "two", program: "/bin/two", requested_ms: NOW },
    ],
  }));
  await press('[data-access-pending="req-1"] [data-access-approve]');
  await press('[data-access-refuse="req-2"]');
  expect(sent(events)).toEqual([["access_approve", { id: "req-1", level: "view" }], ["access_refuse", { id: "req-2" }]]);
  await unmount();
});

it("names a request whose program the system did not say as an unknown program", async () => {
  const { q, unmount } = await mount(section({ pending: [{ id: "req-1", name: "one", program: null, requested_ms: NOW }] }));
  expect(q("[data-access-program]")?.textContent).toBe("Unknown program");
  await unmount();
});

it("lists connected tools under n / 16 with the legend, each line naming machine, last use and how much work it started, and opening nothing (B11)", async () => {
  const { q, container, unmount } = await mount(section({
    connections: [tool({ id: "a", name: "hermes-ctong", level: "act", agent_count: 2, mission_count: 1 }), tool({ id: "b" })],
  }), "ko");
  expect(q("[data-access-count]")?.textContent).toBe("2 / 16");
  expect(q("[data-access-legend]")?.textContent).toBe("보기만볼 수만시키기일까지 시킴");
  expect(q('[data-access-line="a"]')?.textContent).toMatch(/ · 3분 전 사용 · 에이전트 2 · 미션 1$/);
  expect(container.querySelectorAll("[data-access-connection]")).toHaveLength(2);
  // A row is a summary only: it opens no list of the tool's agents or missions.
  expect(q('[data-access-connection="a"] [aria-expanded]')).toBeNull();
  expect(q("[data-external-tools]")?.textContent).not.toContain("승인 대기");
  await unmount();
});

it("switches a tool's level with its two-part control and sends nothing for the level it already has (B12)", async () => {
  const { events, press, unmount } = await mount(section({ connections: [tool({ id: "a", level: "view" })] }));
  await press('[data-access-connection="a"] [data-access-level-option="view"]');
  expect(events).toEqual([]);
  await press('[data-access-connection="a"] [data-access-level-option="act"]');
  expect(sent(events)).toEqual([["access_level", { id: "a", level: "act" }]]);
  await unmount();
});

it("disconnects at once with no confirmation, and warns only on an Act tool that holds work (B13)", async () => {
  const { events, q, press, unmount } = await mount(section({
    connections: [
      tool({ id: "busy", level: "act", agent_count: 1 }),
      tool({ id: "idle", level: "act" }),
      tool({ id: "seeing", level: "view", agent_count: 1 }),
    ],
  }), "ko");
  expect(q('[data-access-warning="busy"]')?.textContent).toBe("끊거나 보기만으로 바꾸면 이 도구의 일은 내 것이 돼요");
  expect(q('[data-access-warning="idle"]')).toBeNull();
  // A View only tool's agents are already the operator's (D-14).
  expect(q('[data-access-warning="seeing"]')).toBeNull();
  await press('[data-access-revoke="busy"]');
  expect(sent(events)).toEqual([["access_revoke", { id: "busy" }]]);
  expect(document.querySelector('[role="alertdialog"]')).toBeNull();
  await unmount();
});

it("shows the empty list's line and the command that asks to connect (B11)", async () => {
  const { q, unmount } = await mount(section({}), "ko");
  expect(q('[data-access-empty="true"]')?.textContent).toBe("연결된 도구가 없어요hide access request --name <이름>");
  expect(q("[data-access-count]")?.textContent).toBe("0 / 16");
  await unmount();
});

it("waits for the core's section before drawing either list", async () => {
  const { q, container, unmount } = await mount(null);
  expect(container.textContent).toBe("Connecting to hide…");
  expect(q("[data-access-count]")).toBeNull();
  await unmount();
});
