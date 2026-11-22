// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { configureAccessCalls, receiveAccessResult, type AccessCall } from "../accessCall";
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
  return { machine: "local", limit: 16, pending: [], invites: [], connections: [], ...over };
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
  expect(q('[data-access-connection="b"] [data-access-mode-pill]')?.textContent).toBe("봇 공간");
  // A 전체 bot's owl wears a person in its corner; one in its own space none, and neither a ring.
  expect(q('[data-access-connection="a"] [data-bot-full]')).not.toBeNull();
  expect(q('[data-access-connection="b"] [data-bot-full]')).toBeNull();
  expect(q("[data-external-tools] .ring-primary")).toBeNull();
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

it("shows the empty list's line alone, with 봇 만들기 at the head of the list (B11)", async () => {
  const { q, unmount } = await mount(section({}), "ko");
  expect(q('[data-access-empty="true"]')?.textContent).toBe("연결된 봇이 없어요");
  expect(q("[data-access-create]")?.textContent).toBe("봇 만들기");
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

/** Answers Settings' `access_call`s as the core would, with `answer`, and records each call. */
function answering(answer: (call: AccessCall) => { ok: true; value: unknown } | { ok: false; code: string }) {
  const calls: AccessCall[] = [];
  configureAccessCalls((event) => {
    const { request_id, call } = event.payload as { request_id: string; call: AccessCall };
    calls.push(call);
    queueMicrotask(() => receiveAccessResult({ request_id, ...answer(call) }));
    return true;
  });
  return calls;
}

const MADE = { id: "inv-1", name: "hermes", mode: { kind: "all" }, code: "K7Q4-MZ2P-X3RD", expires_ms: NOW + 10 * 60_000 };
const settle = () => act(async () => { await Promise.resolve(); await Promise.resolve(); });

it("makes a bot from its name and mode and shows the sentence for its AI, which closes once the bot no longer waits", async () => {
  const calls = answering((call) => (call.call === "invite" && call.name === "taken" ? { ok: false, code: "access_name_taken" } : { ok: true, value: MADE }));
  const { q, press, unmount } = await mount(section({}), "ko");
  await press("[data-access-create]");
  const dialog = () => document.querySelector("[data-access-create-dialog]");
  const name = document.querySelector<HTMLInputElement>("[data-access-create-name]")!;
  const type = async (text: string) => act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(name, text);
    name.dispatchEvent(new Event("input", { bubbles: true }));
  });
  const submit = () => document.querySelector<HTMLButtonElement>("[data-access-create-submit]")!;
  // A name the core cannot take leaves the button off; one it refuses says why under the field.
  await type("a b");
  expect(submit().disabled).toBe(true);
  await type("taken");
  await act(async () => submit().click());
  await settle();
  expect(document.querySelector("[data-access-create-refusal]")?.textContent).toBe("같은 이름의 봇이 이미 있어요");
  await type("hermes");
  await act(async () => (dialog()!.querySelector('[data-access-option="all"]') as HTMLElement).click());
  await act(async () => submit().click());
  await settle();
  expect(calls).toEqual([
    { call: "invite", name: "taken", mode: { kind: "own" } },
    { call: "invite", name: "hermes", mode: { kind: "all" } },
  ]);
  expect(dialog()).toBeNull();
  const sentence = () => document.querySelector("[data-access-sentence-text]")?.textContent ?? null;
  expect(sentence()).toContain("Hide의 hermes 봇으로 일해 주세요.");
  expect(sentence()).toContain("`hide access join K7Q4-MZ2P-X3RD` 명령을 실행하고");
  // The answer came before the snapshot listing the bot: the sentence stays, and goes once the bot stops waiting.
  await act(async () => useShellStore.setState({ access: section({ invites: [{ id: "inv-1", name: "hermes", mode: { kind: "all" }, created_ms: NOW, expires_ms: MADE.expires_ms }] }) }));
  expect(sentence()).not.toBeNull();
  await act(async () => useShellStore.setState({ access: section({ connections: [tool({ id: "inv-1", name: "hermes" })] }) }));
  expect(sentence()).toBeNull();
  expect(q('[data-access-connection="inv-1"]')).not.toBeNull();
  await unmount();
});

it("lists the bots that wait for their AI with the time left, copies the live code's sentence, and gives a new code once it ran out", async () => {
  const calls = answering((call) => ({ ok: true, value: { ...MADE, id: "id" in call ? call.id : MADE.id } }));
  const { events, q, press, unmount } = await mount(section({
    invites: [
      { id: "live", name: "hermes", mode: { kind: "own" }, created_ms: NOW - 60_000, expires_ms: NOW + 9 * 60_000 },
      { id: "ended", name: "nightly", mode: { kind: "own" }, created_ms: NOW - 11 * 60_000, expires_ms: NOW - 60_000 },
    ],
  }), "ko");
  expect(q("[data-access-invite-count]")?.getAttribute("data-access-invite-count")).toBe("2");
  expect(q('[data-access-invite-line="live"]')?.textContent).toBe("9분 남음");
  expect(q('[data-access-invite-line="ended"]')?.textContent).toBe("코드 만료");
  expect(q('[data-access-invite-copy="ended"]')).toBeNull();
  await press('[data-access-invite-copy="live"]');
  await settle();
  await press('[data-access-invite-renew="ended"]');
  await settle();
  expect(calls).toEqual([{ call: "code", id: "live" }, { call: "renew", id: "ended" }]);
  expect(document.querySelector('[data-access-sentence="ended"]')).not.toBeNull();
  await press('[data-access-invite-cancel="live"]');
  expect(sent(events)).toEqual([["access_invite_cancel", { id: "live" }]]);
  await unmount();
});

it("lets only this machine's own window make a bot or see the bots waiting for their code", async () => {
  const { q, unmount } = await mount(section({
    invites: [{ id: "live", name: "hermes", mode: { kind: "own" }, created_ms: NOW, expires_ms: NOW + 600_000 }],
    connections: [tool({ id: "a" })],
  }), "ko", { pairing: null });
  expect(q("[data-access-create]")).toBeNull();
  expect(q("[data-access-invite]")).toBeNull();
  expect(q('[data-access-connection="a"]')).not.toBeNull();
  await unmount();
});

it("waits for the core's section before drawing either list", async () => {
  const { q, container, unmount } = await mount(null);
  expect(container.textContent).toBe("Connecting to hide…");
  expect(q("[data-access-count]")).toBeNull();
  await unmount();
});
