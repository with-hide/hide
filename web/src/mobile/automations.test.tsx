// @vitest-environment jsdom
// The phone's automations tab and an automation's page (docs/UI_BEHAVIOR.md,
// The phone app; PRD automations B34, B35): the list with its schedule, next
// run, unseen dot and buttons, the runs newest first, a press that reaches
// the computer once, the empty line, and what stays when the computer cannot
// be reached. No button makes, edits or removes an automation.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { importAppFirst } from "./appImport";
import type { PhoneAutomation, ServerFrame } from "./protocol";

vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

class FakeSocket {
  static CONNECTING = 0;
  static OPEN = 1;
  static instances: FakeSocket[] = [];
  readyState = 1;
  sent: Record<string, unknown>[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  constructor() {
    FakeSocket.instances.push(this);
  }
  send(data: string) {
    this.sent.push(JSON.parse(data) as Record<string, unknown>);
  }
  close() {}
}

const ZONE = "UTC";
// 2026-11-19 08:00 UTC, a Thursday.
const NOW = Date.UTC(2026, 10, 19, 8, 0);
const HOUR = 3_600_000;

function automation(id: string, over: Partial<PhoneAutomation> = {}): PhoneAutomation {
  return {
    id,
    name: `Automation ${id}`,
    schedule: { kind: "weekdays", time: "09:00" },
    next_run_at_unix_ms: NOW + HOUR,
    running: false,
    unseen: 0,
    records: [],
    ...over,
  };
}

/** A phone paired and connected to a fake hided; `automations` is the first frame's list, or none for a Hide before automations. */
async function connect(automations: PhoneAutomation[] | null) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("WebSocket", FakeSocket);
  vi.useFakeTimers({ toFake: ["Date"], now: NOW });
  FakeSocket.instances = [];
  vi.resetModules();
  const { App } = await import("./App");
  const connection = await import("./connection");
  const store = await import("./store");
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  store.patch({ screen: "pair", pair: { v: 1, endpoint: "https://mac.example.ts.net", code: "code" } });
  // The sockets this test's app opened, read by their own handles: one a former test's app opens later is not this one's.
  const opened: FakeSocket[] = [];
  const pair = () => {
    const before = FakeSocket.instances.length;
    connection.pairNow();
    opened.push(FakeSocket.instances[before]!);
    return opened.at(-1)!;
  };
  let socket = pair();
  const serve = (frame: ServerFrame) => act(async () => socket.onmessage?.({ data: JSON.stringify(frame) }));
  const open = async () => {
    await act(async () => socket.onopen?.());
    await serve({ type: "hello", mac_name: "Mac", phone_id: "phone-a", name: null, vapid_public_key: "key", notifications: "off" });
    await serve({ type: "agents", groups: [], devices: [], interface_language: "en", color_theme: null });
  };
  await open();
  const list = (items: PhoneAutomation[], asks?: string[]) => serve({ type: "automations", items, zone: ZONE, asks });
  if (automations) await list(automations);
  await act(async () => root.render(<App />));
  const q = (selector: string) => container.querySelector<HTMLElement>(selector);
  const click = (selector: string) => act(async () => q(selector)!.click());
  // Every press this phone sent, on any of its connections.
  const sent = () => opened.flatMap((one) => one.sent).filter((message) => message.type === "automation");
  const lose = () => act(async () => {
    socket.readyState = 3;
    socket.onclose?.();
  });
  /** The connection comes back, and hided sends the list as it is now. */
  const reconnect = async (items: PhoneAutomation[]) => {
    await act(async () => {
      socket = pair();
    });
    await open();
    await list(items);
  };
  return { container, serve, list, q, click, sent, lose, reconnect, unmount: () => act(async () => {
    root.unmount();
    // Its retry timer would open a socket in the next test.
    connection.stop();
  }).then(() => container.remove()) };
}

importAppFirst(FakeSocket);

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

it("shows no automations tab against a Hide that sends none, and the one empty line once it does", async () => {
  const { container, list, serve, click, unmount } = await connect(null);
  try {
    expect(container.querySelector('[data-phone-tab="automations"]')).toBeNull();
    await list([]);
    expect([...container.querySelectorAll("[data-phone-tab]")].map((tab) => tab.getAttribute("data-phone-tab"))).toEqual(["agents", "missions", "automations", "prs", "settings"]);
    await click('[data-phone-tab="automations"]');
    expect(container.querySelector('[data-phone-automations-state="empty"]')!.textContent).toBe("No automations yetYou can create one by asking an agent");
    expect(container.querySelector("[data-phone-automations-unreadable]")).toBeNull();
    // A list the computer could not read and set aside is one line above it, until the core drops it.
    await serve({ type: "automations", items: [], zone: ZONE, unreadable: true });
    expect(container.querySelector("[data-phone-automations-unreadable]")!.textContent).toBe("The automation list on the computer could not be read and was set aside · see the desktop");
  } finally {
    await unmount();
  }
});

it("lists each automation's schedule and next run in the computer's zone, its unseen dot, and only [다시 켜기] on one a person fixes (Doop 15)", async () => {
  const { container, click, unmount } = await connect([
    automation("on", { unseen: 2 }),
    automation("going", { running: true }),
    automation("off", { paused: { reason: "operator", at_unix_ms: NOW }, next_run_at_unix_ms: null }),
    automation("signed-out", { paused: { reason: "login_expired", at_unix_ms: NOW }, next_run_at_unix_ms: null }),
  ]);
  try {
    await click('[data-phone-tab="automations"]');
    const row = (id: string) => container.querySelector(`[data-phone-automation="${id}"]`)!;
    const presses = (id: string) => [...row(id).querySelectorAll("[data-phone-automation-press]")].map((button) => [button.getAttribute("data-phone-automation-press"), (button as HTMLButtonElement).disabled]);
    const next = (id: string) => row(id).querySelector("[data-phone-automation-next]")!;
    expect(next("on").textContent).toBe("Weekdays 09:00 · Today 09:00");
    expect(row("on").querySelector("[data-phone-automation-unseen]")).not.toBeNull();
    // A run going says so in the line, in the working colour (B19).
    expect(next("going").getAttribute("data-phone-automation-next")).toBe("running");
    expect(next("going").getAttribute("data-phone-automation-tone")).toBe("working");
    // The operator's own pause is a dimmed row; what a person fixes is the line in the warning colour with [다시 켜기] (B25).
    expect(next("off").getAttribute("data-phone-automation-paused")).toBe("operator");
    expect(next("signed-out").textContent).toBe("Weekdays 09:00 · Signed out · fix it, then");
    expect(next("signed-out").getAttribute("data-phone-automation-tone")).toBe("warning");
    // Running now and pausing are on the page; the list carries only the one press a person fixing it needs.
    expect([presses("on"), presses("going"), presses("off")]).toEqual([[], [], []]);
    expect(presses("signed-out")).toEqual([["resume", false]]);
    expect(row("signed-out").querySelector('[data-phone-automation-press="resume"]')!.textContent).toBe("Turn on again");
    // Nothing on the tab makes, edits or removes one (D-20).
    expect(container.querySelectorAll("[data-phone-automation-press]").length).toBe(1);
  } finally {
    await unmount();
  }
});

it("opens an automation's runs newest first with their marks, tags and results, and closes the page when it leaves", async () => {
  const records: PhoneAutomation["records"] = [
    { kind: "run", number: 4, trigger: "manual", started_at_unix_ms: NOW - HOUR, state: "running", attempt: 2, seen: false },
    { kind: "run", number: 3, trigger: "missed", started_at_unix_ms: NOW - 24 * HOUR, ended_at_unix_ms: NOW - 24 * HOUR + 12 * 60_000, state: "succeeded", attempt: 1, seen: false, spend: { tokens: 12_340, micro_usd: 420_000, unpriced: false } },
    { kind: "skipped", at_unix_ms: NOW - 48 * HOUR, last_unix_ms: NOW - 48 * HOUR, count: 1, reason: { kind: "previous_running" } },
    { kind: "run", number: 2, trigger: "scheduled", started_at_unix_ms: NOW - 72 * HOUR, ended_at_unix_ms: NOW - 71 * HOUR, state: "failed", attempt: 3, seen: true },
  ];
  const { container, list, click, unmount } = await connect([automation("a", { records, records_omitted: 30 })]);
  try {
    await click('[data-phone-tab="automations"]');
    await click('[data-phone-automation="a"] [data-slot="row-overlay-button"]');
    const lines = [...container.querySelectorAll("[data-phone-automation-record]")].map((line) => [
      line.getAttribute("data-phone-automation-record-tone"),
      line.querySelector("[data-phone-automation-tag]")?.getAttribute("data-phone-automation-tag") ?? null,
      line.querySelector("[data-phone-automation-result]")!.textContent,
    ]);
    expect(lines).toEqual([
      ["running", "manual", "Try 2"],
      ["ok", "missed", "12m"],
      ["skipped", null, "Skipped · the last run was still going"],
      ["failed", null, "Failed · 3 tries"],
    ]);
    // B39: a run the ledger read says what it spent, in the desktop's words.
    expect([...container.querySelectorAll("[data-phone-automation-spend]")].map((line) => line.textContent)).toEqual(["12.3K tokens"]);
    expect(container.querySelector("[data-phone-automation-older]")!.getAttribute("data-phone-automation-older")).toBe("30");
    await list([]);
    expect(container.querySelector("[data-phone-automation-page]")).toBeNull();
  } finally {
    await unmount();
  }
});

it("sends a press once under one request id, waits for hided's answer, and says why it was refused", async () => {
  const { container, serve, click, sent, unmount } = await connect([automation("a")]);
  try {
    await click('[data-phone-tab="automations"]');
    await click('[data-phone-automation="a"] [data-slot="row-overlay-button"]');
    await click('[data-phone-automation-page] [data-phone-automation-press="run"]');
    const [first] = sent();
    expect(first).toMatchObject({ type: "automation", action: "run", id: "a" });
    // While hided has not answered, no second press goes out.
    expect((container.querySelector('[data-phone-automation-page] [data-phone-automation-press="pause"]') as HTMLButtonElement).disabled).toBe(true);
    await serve({ type: "automation_result", request_id: first!.request_id as string, action: "run", id: "a", ok: false, reason: "running" });
    // The refusal is one line under the page's buttons.
    expect(container.querySelector("[data-phone-automation-notice]")!.textContent).toBe("A run is already going.");
    // A refused press sent nothing the core kept, so the next one is a new press.
    await click('[data-phone-automation-page] [data-phone-automation-press="run"]');
    const second = sent()[1]!;
    expect(second.request_id).not.toBe(first!.request_id);
    expect(container.querySelector("[data-phone-automation-notice]")).toBeNull();
    await serve({ type: "automation_result", request_id: second.request_id as string, action: "run", id: "a", ok: true, reason: null });
    expect(container.querySelector("[data-phone-automation-notice]")).toBeNull();
  } finally {
    await unmount();
  }
});

it("keeps the last list dimmed under the unreachable line with its buttons off, and says a press in flight did not arrive", async () => {
  const { container, click, lose, unmount } = await connect([automation("a")]);
  try {
    await click('[data-phone-tab="automations"]');
    await click('[data-phone-automation="a"] [data-slot="row-overlay-button"]');
    await click('[data-phone-automation-page] [data-phone-automation-press="pause"]');
    await lose();
    expect(container.querySelector("[data-phone-unreachable]")).not.toBeNull();
    expect([...container.querySelectorAll<HTMLButtonElement>("[data-phone-automation-press]")].every((button) => button.disabled)).toBe(true);
    expect(container.querySelector("[data-phone-automation-notice]")!.textContent).toBe("The computer can't be reached right now. Try again once connected.");
    await click('[data-phone-back="automations"]');
    expect(container.querySelector('[data-phone-automations="true"] [aria-busy="true"] [data-phone-automation="a"]')).not.toBeNull();
  } finally {
    await unmount();
  }
});

it("sends a press whose answer the lost connection took again under its id, and a press after the computer did it under a new one", async () => {
  const ran = (number: number): PhoneAutomation["records"][number] => ({ kind: "run", number, trigger: "manual", started_at_unix_ms: NOW - HOUR, ended_at_unix_ms: NOW - HOUR + 60_000, state: "succeeded", attempt: 1, seen: true });
  const before = [automation("a", { records: [ran(3)] })];
  const { click, sent, lose, reconnect, serve, unmount } = await connect(before);
  try {
    await click('[data-phone-tab="automations"]');
    await click('[data-phone-automation="a"] [data-slot="row-overlay-button"]');
    const run = '[data-phone-automation-page] [data-phone-automation-press="run"]';
    await click(run);
    await lose();
    // The computer had not taken it: the list comes back as it was, and the press again is the same press.
    await reconnect(before);
    await click(run);
    const [first, again] = sent();
    expect(again!.request_id).toBe(first!.request_id);
    await lose();
    // This time it ran: the list comes back with run 4, so the next press is a new one.
    await reconnect([automation("a", { records: [ran(4), ran(3)] })]);
    await click(run);
    const next = sent()[2]!;
    expect(next.request_id).not.toBe(first!.request_id);
    await serve({ type: "automation_result", request_id: next.request_id as string, action: "run", id: "a", ok: true, reason: null });
  } finally {
    await unmount();
  }
});

it("sends a new pause after one whose answer was lost took effect and was undone elsewhere", async () => {
  const on = [automation("a")];
  const { click, sent, lose, reconnect, list, unmount } = await connect(on);
  try {
    await click('[data-phone-tab="automations"]');
    await click('[data-phone-automation="a"] [data-slot="row-overlay-button"]');
    await click('[data-phone-automation-page] [data-phone-automation-press="pause"]');
    await lose();
    // The pause landed, then the desktop turned it on again: the list is back where the press saw it.
    await reconnect([automation("a", { paused: { reason: "operator", at_unix_ms: NOW }, next_run_at_unix_ms: null })]);
    await list(on);
    await click('[data-phone-automation-page] [data-phone-automation-press="pause"]');
    const [first, second] = sent();
    expect(second!.request_id).not.toBe(first!.request_id);
  } finally {
    await unmount();
  }
});

it("draws an automation that stopped for a person as one line of 내 차례 with [다시 켜기], the tab's own press, counted in the agents tab", async () => {
  const signedOut = automation("signed-out", { name: "Sort issues", paused: { reason: "login_expired", at_unix_ms: NOW }, next_run_at_unix_ms: null });
  const { container, list, click, sent, unmount } = await connect(null);
  try {
    await list([automation("on"), signedOut], ["signed-out"]);
    const line = container.querySelector('[data-phone-group="needs_you"] [data-phone-automation-ask="signed-out"]')!;
    // `<name>` over `자동화 <why> · 고치고` and [다시 켜기] (Doop 01).
    expect(line.textContent).toBe("Sort issuesAutomations Signed out · fix it, thenTurn on again");
    expect(container.querySelector('[data-phone-group="needs_you"] [data-phone-group-count]')!.textContent).toBe("1");
    expect(container.querySelector('[data-phone-tab="agents"]')!.getAttribute("data-phone-tab-count")).toBe("1");
    await click('[data-phone-automation-ask="signed-out"] [data-phone-automation-press="resume"]');
    expect(sent()).toEqual([expect.objectContaining({ action: "resume", id: "signed-out" })]);
    // The line opens the automation's page.
    await click('[data-phone-automation-ask="signed-out"] [data-slot="row-overlay-button"]');
    expect(container.querySelector('[data-phone-automation-page="signed-out"]')).not.toBeNull();
    // Turned on again, the core takes the ask away and the line goes.
    await list([automation("on"), { ...signedOut, paused: undefined }], []);
    await click('[data-phone-back="automations"]');
    await click('[data-phone-tab="agents"]');
    expect(container.querySelector("[data-phone-automation-ask]")).toBeNull();
    expect(container.querySelector('[data-phone-group="needs_you"]')).toBeNull();
  } finally {
    await unmount();
  }
});

// mission-104 S-5: a once automation reads as its date with the clock mark, a
// done one comes after the rest, and the one whose run ended is a line of
// 내 차례 counted in the agents tab until the core takes it away.
it("lists a once automation by its date, a done one last, and the one whose run ended as a counted line of 내 차례", async () => {
  const once = { kind: "once" as const, date: "2026-11-26", time: "09:00" };
  const waiting = automation("week", { name: "Check webhooks", schedule: once, next_run_at_unix_ms: NOW + 7 * 24 * HOUR });
  const ended = automation("migration", {
    name: "Check migration", schedule: { ...once, date: "2026-11-19", time: "07:56" }, next_run_at_unix_ms: null, done: true, look_run: 1,
    records: [{ kind: "run", number: 1, trigger: "scheduled", started_at_unix_ms: NOW - 4 * 60_000, ended_at_unix_ms: NOW, state: "succeeded", attempt: 1, seen: false }],
  });
  const { container, serve, click, sent, unmount } = await connect(null);
  try {
    await serve({ type: "automations", items: [ended, automation("daily"), waiting], zone: ZONE, looks: ["migration"] });
    const look = container.querySelector('[data-phone-group="needs_you"] [data-phone-automation-look="migration"]')!;
    expect(look.querySelector('[data-mark-automation="once"]')).not.toBeNull();
    const line = look.querySelector('[data-automation-look-line="ok"]')!;
    expect(line.textContent).toBe("Automation · once · Done · 4m");
    expect(line.querySelector("svg.text-success")).not.toBeNull();
    expect(container.querySelector('[data-phone-tab="agents"]')!.getAttribute("data-phone-tab-count")).toBe("1");
    // Opening the line looks at its run: the core is told once, with the run, and the automation's page opens.
    await click('[data-phone-automation-look="migration"] [data-slot="row-overlay-button"]');
    expect(sent()).toEqual([expect.objectContaining({ type: "automation", action: "look_seen", id: "migration", run: 1 })]);
    expect(container.querySelector('[data-phone-automation-page="migration"]')).not.toBeNull();
    await click('[data-phone-back="automations"]');

    const rows = [...container.querySelectorAll<HTMLElement>("[data-phone-automation]")];
    expect(rows.map((row) => row.getAttribute("data-phone-automation"))).toEqual(["daily", "week", "migration"]);
    expect(rows[2]!.getAttribute("data-phone-automation-state")).toBe("done");
    expect(container.querySelector('[data-phone-automation="week"] [data-phone-automation-once="true"]')!.textContent).toBe("Thu 11-26 09:00 · once");

    // Opened on the computer, the core takes the look away and the line and its count go.
    await serve({ type: "automations", items: [ended, automation("daily"), waiting], zone: ZONE, looks: [] });
    await click('[data-phone-tab="agents"]');
    expect(container.querySelector("[data-phone-automation-look]")).toBeNull();
    expect(container.querySelector('[data-phone-tab="agents"]')!.getAttribute("data-phone-tab-count")).toBeNull();
  } finally {
    await unmount();
  }
});
