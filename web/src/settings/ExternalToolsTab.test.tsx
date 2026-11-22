// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "../actions";
import { clientI18n } from "../i18n/translator";
import { NOTHING_UNDER_WAY } from "../pairing";
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
    id: "conn-1", name: "nightly-cron", mode: { kind: "all" }, program: "/usr/local/bin/nightly", created_ms: NOW - 86_400_000,
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
    useShellStore.setState({ connection: "live", access, pairing: NOTHING_UNDER_WAY, ...extra } as never);
    useUiStore.getState().openSettings("externalTools");
    root.render(<ExternalToolsTab actions={actions} />);
  });
  const q = (selector: string) => container.querySelector(selector) as HTMLElement | null;
  const press = async (selector: string) => { await act(async () => { q(selector)!.click(); }); };
  return { events, q, press, container, unmount: () => act(async () => root.unmount()) };
}

const sent = (events: Parameters<DispatchFn>[0][]) => events.map((event) => [event.kind, event.payload]);

it("puts a request up with the owl it will get, its own space chosen, and approves in the mode picked (B2, B3, B7)", async () => {
  const { events, q, press, unmount } = await mount(section({
    pending: [
      { id: "req-1", name: "hermes-zutongg", program: "/opt/hermes/bin/hermes", requested_ms: NOW },
      { id: "req-2", name: "orca", program: "/bin/orca", requested_ms: NOW },
      { id: "req-3", name: "two", program: "/bin/two", requested_ms: NOW },
    ],
  }));
  const row = q('[data-access-pending="req-1"]')!;
  expect(row.textContent).toContain("hermes-zutongg");
  expect(row.querySelector("[data-access-program]")?.textContent).toBe("/opt/hermes/bin/hermes");
  expect(row.querySelector("[data-bot-face]")).not.toBeNull();
  // The narrower mode is chosen until the operator picks the other, and the button names the choice.
  expect(row.querySelector("[data-access-choice]")?.getAttribute("data-access-choice")).toBe("own");
  expect(row.querySelector("[data-access-approve]")?.textContent).toBe("Approve in its own space");
  await press('[data-access-pending="req-1"] [data-access-approve]');
  await press('[data-access-pending="req-2"] [data-access-option="all"]');
  expect(q('[data-access-pending="req-2"] [data-access-approve]')?.textContent).toBe("Approve as Full");
  await press('[data-access-pending="req-2"] [data-access-approve]');
  await press('[data-access-refuse="req-3"]');
  expect(sent(events)).toEqual([
    ["access_approve", { id: "req-1", mode: { kind: "own" } }],
    ["access_approve", { id: "req-2", mode: { kind: "all" } }],
    ["access_refuse", { id: "req-3" }],
  ]);
  await unmount();
});

it("names a request whose program the system did not say as an unknown program", async () => {
  const { q, unmount } = await mount(section({ pending: [{ id: "req-1", name: "one", program: null, requested_ms: NOW }] }));
  expect(q("[data-access-program]")?.textContent).toBe("Unknown program");
  await unmount();
});

it("lists connected bots under n / 16 with their mode, each line naming machine, last use and how much work it started, and opening nothing (B11)", async () => {
  const { q, container, unmount } = await mount(section({
    connections: [tool({ id: "a", name: "hermes-ctong", agent_count: 2, mission_count: 1 }), tool({ id: "b", mode: { kind: "own" } })],
  }), "ko");
  expect(q("[data-access-count]")?.textContent).toBe("2 / 16");
  expect(q('[data-access-connection="a"] [data-access-mode-pill]')?.textContent).toBe("전체");
  expect(q('[data-access-connection="b"] [data-access-mode-pill]')?.textContent).toBe("자기 공간");
  // A 전체 bot's owl wears the lime ring; one in its own space none.
  expect(q('[data-access-connection="a"] [data-bot-face]')?.className).toContain("ring-primary");
  expect(q('[data-access-connection="b"] [data-bot-face]')?.className).not.toContain("ring-primary");
  expect(q('[data-access-line="a"]')?.textContent).toMatch(/ · 3분 전 사용 · 에이전트 2 · 미션 1$/);
  expect(container.querySelectorAll("[data-access-connection]")).toHaveLength(2);
  // A row is a summary only: it opens no list of the tool's agents or missions.
  expect(q('[data-access-connection="a"] [aria-expanded]:not([data-access-change-mode])')).toBeNull();
  expect(q("[data-external-tools]")?.textContent).not.toContain("승인 대기");
  await unmount();
});

it("disconnects at once with no confirmation, and warns only on an own-mode bot that holds work (B13)", async () => {
  const { events, q, press, unmount } = await mount(section({
    connections: [
      tool({ id: "busy", mode: { kind: "own" }, agent_count: 1 }),
      tool({ id: "idle", mode: { kind: "own" } }),
      tool({ id: "whole", agent_count: 1, mission_count: 1 }),
    ],
  }), "ko");
  expect(q('[data-access-warning="busy"]')?.textContent).toBe("끊으면 이 봇이 하던 일은 내 것이 돼요");
  expect(q('[data-access-warning="idle"]')).toBeNull();
  // An all-mode bot's work is the operator's already, so nothing moves on a disconnect (mission-87).
  expect(q('[data-access-warning="whole"]')).toBeNull();
  await press('[data-access-revoke="busy"]');
  expect(sent(events)).toEqual([["access_revoke", { id: "busy" }]]);
  expect(document.querySelector('[role="alertdialog"]')).toBeNull();
  await unmount();
});

it("shows the empty list's line and the command that asks to connect (B11)", async () => {
  const { q, unmount } = await mount(section({}), "ko");
  expect(q('[data-access-empty="true"]')?.textContent).toBe("연결된 봇이 없어요hide access request --name <이름>");
  expect(q("[data-access-count]")?.textContent).toBe("0 / 16");
  await unmount();
});

it("changes a bot's mode from its menu and sends nothing for the mode it already has (B12)", async () => {
  const { events, q, unmount } = await mount(section({ connections: [tool({ id: "a", mode: { kind: "own" } })] }));
  const open = async () => {
    const trigger = q('[data-access-change-mode="a"]')!;
    await act(async () => { trigger.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true, button: 0 })); });
  };
  await open();
  await act(async () => { (document.querySelector('[data-access-mode-option="own"]') as HTMLElement).click(); });
  await open();
  await act(async () => { (document.querySelector('[data-access-mode-option="all"]') as HTMLElement).click(); });
  expect(sent(events)).toEqual([["access_mode", { id: "a", mode: { kind: "all" } }]]);
  await unmount();
});

it("waits for the core's section before drawing either list", async () => {
  const { q, container, unmount } = await mount(null);
  expect(container.textContent).toBe("Connecting to hide…");
  expect(q("[data-access-count]")).toBeNull();
  await unmount();
});
