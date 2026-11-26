// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions, type Actions } from "./actions";
import { notificationBadgeCount, receiveAgentNotice, useAgentNotifications } from "./agentNotices";
import type { AgentScope } from "./agentScope";
import { clientI18n } from "./i18n/translator";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import type { DispatchFn } from "./ws";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

afterEach(() => {
  document.body.innerHTML = "";
  delete window.hideHost;
});

function Probe({ actions }: { actions: Actions }) {
  useAgentNotifications(actions);
  return null;
}

it("reports host activity on connection, transitions, and reconnect, and releases the listener", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  let current = true;
  let listener: ((inUse: boolean) => void) | undefined;
  window.hideHost = {
    kind: "electron", platform: "linux",
    reportBadge: () => undefined, onNotificationOpen: () => () => undefined,
    onComputerUse: (next: (inUse: boolean) => void) => {
      listener = next;
      next(current);
      return () => { listener = undefined; };
    },
  } as never;
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const saved = useShellStore.getState();
  const root = createRoot(document.body.appendChild(document.createElement("div")));
  try {
    await act(async () => {
      useShellStore.setState({ connection: "live", rest: null });
      root.render(<Probe actions={actions} />);
    });
    current = false;
    await act(async () => listener?.(current));
    await act(async () => useShellStore.setState({ connection: "connecting" } as never));
    expect(listener).toBeUndefined();
    await act(async () => useShellStore.setState({ connection: "live" }));
    expect(events.filter((event) => event.kind === "desktop_activity").map((event) => event.payload)).toEqual([
      { in_use: true }, { in_use: false }, { in_use: false },
    ]);
  } finally {
    await act(async () => root.unmount());
    expect(listener).toBeUndefined();
    useShellStore.setState(saved, true);
  }
});

it("asks for agent notices while the switch is on, words each one, withdraws a seen agent's, and opens a clicked one's pane (B31, B34, B35)", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const shown: unknown[][] = [];
  const withdrawn: string[][] = [];
  let open: ((id: string) => void) | null = null;
  window.hideHost = {
    kind: "electron",
    platform: "darwin",
    notify: (...args: unknown[]) => shown.push(args),
    withdrawNotifications: (ids: string[]) => withdrawn.push(ids),
    reportBadge: () => undefined,
    onNotificationOpen: (listener: (id: string) => void) => {
      open = listener;
      return () => (open = null);
    },
  } as never;
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => {
    events.push(event);
    return true;
  });
  const saved = useShellStore.getState();
  const root = createRoot(document.body.appendChild(document.createElement("div")));
  await act(async () => {
    useShellStore.setState({ connection: "live", rest: { ui_state: { desktop_notifications: true }, navigator: { devices: [] } } } as never);
    root.render(<Probe actions={actions} />);
  });
  expect(events.filter((event) => event.kind === "desktop_notices").map((event) => event.payload)).toEqual([{ wanted: true, needs_you: true }]);

  await act(async () => {
    receiveAgentNotice({ notices: [{ tag: "node|w1:p1", device_id: "node", pane_id: "w1:p1", title: "Add a dark mode switch", state: "needs_you", place: "my-app" }], clear: [] });
    receiveAgentNotice({ notices: [], clear: ["node|w1:p1"] });
  });
  expect(shown).toEqual([["agent:node|w1:p1", "Add a dark mode switch", "Needs You · my-app", { unlessFocused: true }]]);
  expect(withdrawn).toEqual([["agent:node|w1:p1"]]);

  // The notification leads with the row's own status word, with or without
  // detail (status-marks B23); a frame with no status code says the section.
  await act(async () => {
    for (const [status_code, detail] of [["question", "배포할 환경을 골라 주세요"], ["approval", "계획 승인 대기"], ["error", "로그인이 필요해요"], ["done", "배포 완료"], ["unfinished", "테스트 세 개 남음"]] as const) {
      receiveAgentNotice({ notices: [{ tag: "node|w1:p1", device_id: "node", pane_id: "w1:p1", title: "Add a dark mode switch", state: status_code === "done" || status_code === "unfinished" ? "done" : "needs_you", status_code, detail, place: "my-app" }], clear: [] });
    }
    receiveAgentNotice({ notices: [{ tag: "node|w1:p1", device_id: "node", pane_id: "w1:p1", title: "Add a dark mode switch", state: "done", status_code: "done", detail: null, place: "" }], clear: [] });
  });
  expect(shown.slice(1).map((notice) => notice.slice(1, 3))).toEqual([
    ["Add a dark mode switch", "To check · 배포할 환경을 골라 주세요 · my-app"],
    ["Add a dark mode switch", "Approval · 계획 승인 대기 · my-app"],
    ["Add a dark mode switch", "Blocked · 로그인이 필요해요 · my-app"],
    ["Add a dark mode switch", "Done · 배포 완료 · my-app"],
    ["Add a dark mode switch", "Unfinished · 테스트 세 개 남음 · my-app"],
    ["Add a dark mode switch", "Done"],
  ]);
  await act(async () => { await clientI18n.changeLanguage("ko"); });
  await act(async () => receiveAgentNotice({ notices: [{ tag: "node|w1:p1", device_id: "node", pane_id: "w1:p1", title: "Add a dark mode switch", state: "needs_you", status_code: "question", detail: "Choose staging?", place: "" }], clear: [] }));
  expect(shown.at(-1)?.slice(1, 3)).toEqual(["Add a dark mode switch", "확인할 것 · Choose staging?"]);
  await act(async () => {
    receiveAgentNotice({ notices: [{ tag: "node|w1:p1", device_id: "node", pane_id: "w1:p1", title: "Add a dark mode switch", state: "done", status_code: "done", detail: null, place: "my-app" }], clear: [] });
    receiveAgentNotice({ notices: [{ tag: "node|w1:p1", device_id: "node", pane_id: "w1:p1", title: "Add a dark mode switch", state: "needs_you", status_code: "approval", detail: null, place: "my-app" }], clear: [] });
  });
  expect(shown.slice(-2).map((notice) => notice[2])).toEqual(["끝남 · my-app", "승인 · my-app"]);
  // A question the core asks for the agent comes as its subject, worded in the window's language (hide-plan B14).
  const review = { tag: "node|w1:p1", device_id: "node", pane_id: "w1:p1", title: "Add a dark mode switch", state: "needs_you", status_code: "question", detail: null, subject: { cause: "plan_review", steps: 5 }, place: "my-app" } as const;
  await act(async () => receiveAgentNotice({ notices: [review], clear: [] }));
  await act(async () => { await clientI18n.changeLanguage("en"); });
  await act(async () => receiveAgentNotice({ notices: [review], clear: [] }));
  expect(shown.slice(-2).map((notice) => notice[2])).toEqual(["확인할 것 · 작업 단계 검토 · 5단계 · my-app", "To check · Task steps review · 5 steps · my-app"]);

  // A View to look at says the eye's word and the View's name (status-model.md, A View to look at).
  await act(async () => receiveAgentNotice({ notices: [{ tag: "node|w1:p1", device_id: "node", pane_id: "w1:p1", title: "Add a dark mode switch", state: "needs_you", status_code: "look", detail: "report.html", place: "my-app" }], clear: [] }));
  expect(shown.at(-1)?.[2]).toBe("To look at · report.html · my-app");

  await act(async () => open?.("agent:node|w1:p1"));
  expect(events.some((event) => event.kind === "focus_pane" && event.payload.pane_id === "w1:p1")).toBe(true);
  expect(events.some((event) => event.kind === "pair_view")).toBe(false);

  // A row whose call is a View: its notice names the View, so a click opens it in focus mode as the row's chip does,
  // in one event and with no pane focus of its own (operator decision 2026-11-23).
  const look = { group: "done", count: 1, view_id: "v3", label: "report.html", workspace_id: null, checkout_id: null };
  await act(async () => useShellStore.setState({ agents: [{ pane_id: "w1:p1", mark: "look", look, state: { asking: false } }] } as never));
  const beforeLook = events.length;
  await act(async () => open?.("agent:node|w1:p1"));
  expect(events.slice(beforeLook)).toEqual([{ schema_version: 2, kind: "pair_view", payload: { pane_id: "w1:p1", display_id: "v3", bring_forward: true } }]);
  await act(async () => useShellStore.setState({ agents: [] }));

  // A notice another machine handed over is worded like this machine's own, and a click selects that machine's tile (PRD device-connect-iroh D-50, D-60).
  await act(async () => {
    useShellStore.setState({ rest: { ui_state: { desktop_notifications: true }, navigator: { devices: [], peers: [{ key: "ab12", device_id: "device-mini", name: "Mac mini" }] } } } as never);
  });
  await act(async () => receiveAgentNotice({ notices: [{ tag: "peer:ab12|node|w1:p1", peer: "ab12", device_id: "node", pane_id: "w1:p1", title: "Fix login", state: "needs_you", status_code: "question", detail: "Which branch?", place: "shop-api" }], clear: [] }));
  expect(shown.at(-1)).toEqual(["agent:peer:ab12|node|w1:p1", "Fix login", "To check · Which branch? · shop-api", { unlessFocused: true }]);
  const before = events.length;
  await act(async () => open?.("agent:peer:ab12|node|w1:p1"));
  expect(events.slice(before)).toEqual([{ schema_version: 2, kind: "focus_device", payload: { device_id: "device-mini" } }]);
  await act(async () => receiveAgentNotice({ notices: [], clear: ["peer:ab12|node|w1:p1"] }));
  expect(withdrawn.at(-1)).toEqual(["agent:peer:ab12|node|w1:p1"]);

  // A phone paired with a machine this computer may open shows even over a focused window, named by that machine, and a click selects its tile (PRD device-connect-iroh D-70).
  await act(async () => receiveAgentNotice({ notices: [], clear: [], phones: [{ tag: "peer:ab12|phone|Pixel", peer: "ab12", phone: "Pixel" }] }));
  expect(shown.at(-1)).toEqual(["agent:peer:ab12|phone|Pixel", "New phone connected", "Mac mini · Pixel", { unlessFocused: false }]);
  const beforePhone = events.length;
  await act(async () => open?.("agent:peer:ab12|phone|Pixel"));
  expect(events.slice(beforePhone)).toEqual([{ schema_version: 2, kind: "focus_device", payload: { device_id: "device-mini" } }]);

  // The switch off: the window stops asking, and a late frame shows nothing.
  await act(async () => {
    useShellStore.setState({ rest: { ui_state: { desktop_notifications: false }, navigator: { devices: [] } } } as never);
  });
  expect(events.filter((event) => event.kind === "desktop_notices").at(-1)?.payload).toEqual({ wanted: false, needs_you: false });
  await act(async () => receiveAgentNotice({ notices: [{ tag: "node|w1:p2", device_id: "node", pane_id: "w1:p2", title: "x", state: "done", place: "" }], clear: [] }));
  expect(shown).toHaveLength(15);
  // A new phone still shows with both switches off: it is how the operator sees a pairing they did not make (D-70, B61).
  await act(async () => receiveAgentNotice({ notices: [], clear: [], phones: [{ tag: "peer:ab12|phone|Pixel 2", peer: "ab12", phone: "Pixel 2" }] }));
  expect(shown.at(-1)).toEqual(["agent:peer:ab12|phone|Pixel 2", "New phone connected", "Pixel 2", { unlessFocused: false }]);
  expect(shown).toHaveLength(16);
  await act(async () => root.unmount());
  useShellStore.setState(saved, true);
});

/** The core's raised groups; `more` rows still contribute when the list folds. */
function device(id: string, shown: number, more: number, done = 1, doneMore = 0) {
  const refs = (n: number, at: number) => Array.from({ length: n }, (_, i) => ({ pane_id: `${id}:p${at + i}`, occurrence: 0 }));
  const raised: AgentScope["raised"] = [{ group: "done", shown: refs(done, 90), more: refs(doneMore, 90 + done) }];
  if (shown + more > 0) raised.unshift({ group: "needs_you", shown: refs(shown, 0), more: refs(more, shown) });
  return { id, agent_scope: { raised } };
}

// Operator decision 2026-11-23: the app icon counts 내 차례 alone, never unread
// Done, whatever the Done switch says (replacing 2026-10-18's Done in the icon).
it.each([
  [false, false, [], 0, false],
  [true, false, ["Needs You · my-app"], 2, true],
  [false, true, ["Unread · my-app"], 0, true],
  [true, true, ["Needs You · my-app", "Unread · my-app"], 2, true],
] as const)("with Needs You=%s and Done=%s sends only the chosen groups and counts 내 차례 alone", async (needsYou, done, bodies, badge, wanted) => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const shown: string[] = [];
  const reported: number[] = [];
  window.hideHost = {
    kind: "electron", platform: "linux",
    notify: (_id: string, _title: string, body: string) => shown.push(body),
    reportBadge: (count: number) => reported.push(count),
    withdrawNotifications: () => undefined,
    onNotificationOpen: () => () => undefined,
  } as never;
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const saved = useShellStore.getState();
  const root = createRoot(document.body.appendChild(document.createElement("div")));
  try {
    await act(async () => {
      useShellStore.setState({ connection: "live", rest: { ui_state: { desktop_notifications: needsYou, desktop_notifications_done: done }, navigator: { devices: [device("node", 2, 0)] } } } as never);
      root.render(<Probe actions={actions} />);
    });
    await act(async () => receiveAgentNotice({ notices: [
      { tag: "node|w1:p1", device_id: "node", pane_id: "w1:p1", title: "One", state: "needs_you", place: "my-app" },
      { tag: "node|w1:p2", device_id: "node", pane_id: "w1:p2", title: "Two", state: "done", place: "my-app" },
    ], clear: [] }));
    expect(shown).toEqual(bodies);
    expect(reported).toEqual([badge]);
    expect(events.filter((event) => event.kind === "desktop_notices").map((event) => event.payload)).toEqual(wanted ? [{ wanted: true, needs_you: needsYou }] : []);
  } finally {
    await act(async () => root.unmount());
    useShellStore.setState(saved, true);
  }
});

it("counts 내 차례 across devices, moves with it alone, and counts nothing while Needs You is off", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const reported: number[] = [];
  window.hideHost = {
    kind: "electron",
    platform: "win32",
    reportBadge: (count: number) => reported.push(count),
    onNotificationOpen: () => () => undefined,
  } as never;
  const actions = createActions(() => true);
  const saved = useShellStore.getState();
  const root = createRoot(document.body.appendChild(document.createElement("div")));
  const rest = (on: boolean, needsYou: number, done = 1) => ({ ui_state: { desktop_notifications: on }, navigator: { devices: [device("node", needsYou, 0, done)] } });
  await act(async () => {
    useShellStore.setState({ connection: "connecting", rest: rest(true, 2) } as never);
    root.render(<Probe actions={actions} />);
  });
  expect(reported).toEqual([]);
  await act(async () => useShellStore.setState({ connection: "live" } as never));
  // A new snapshot with the same count sends no extra badge update.
  await act(async () => useShellStore.setState({ rest: rest(true, 2) } as never));
  await act(async () => useShellStore.setState({ rest: rest(true, 3) } as never));
  await act(async () => useShellStore.setState({ rest: rest(true, 3, 0) } as never));
  await act(async () => useShellStore.setState({ rest: {
    ui_state: { desktop_notifications: true, desktop_notifications_done: true },
    navigator: { devices: [device("node", 5, 2, 2, 1), device("mini", 1, 0, 0), device("box", 0, 0)] },
  } } as never));
  await act(async () => useShellStore.setState({ rest: null }));
  await act(async () => useShellStore.setState({ rest: rest(false, 3) } as never));
  // Done read or unread never moves it; with Needs You off it counts nothing.
  expect(reported).toEqual([2, 3, 8, 0]);
  await act(async () => root.unmount());
  useShellStore.setState(saved, true);
});

// Operator decision 2026-11-23: the icon says the number 내 차례 says, so a
// mission or an automation that is its own Needs You item, a once automation's
// ended run among them (mission-104 S-4), counts with the
// agents, under the Needs You switch only.
it("counts a device's own mission and automation asks with its agents under Needs You", () => {
  const rest = (needsYou: boolean) => ({
    ui_state: { desktop_notifications: needsYou, desktop_notifications_done: false },
    navigator: { devices: [
      { id: "node", agent_scope: { ...device("node", 1, 0, 0).agent_scope, mission_asks: ["mission-1", "mission-2"], automation_asks: ["auto-1"], automation_looks: ["once-1"] } },
      device("mini", 1, 0, 0),
    ] },
  }) as never;
  expect(notificationBadgeCount(rest(true))).toBe(6);
  expect(notificationBadgeCount(rest(false))).toBe(0);
});

// mission-104 S-4: a once automation's ended run is one desktop notification
// when its line appears, none for a line already there when the page went
// live, taken back when the line goes, and a click opens that run.
it("notifies a once automation's ended run once, takes it back when it is opened, and opens the run when clicked", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const shown: unknown[][] = [];
  const withdrawn: string[][] = [];
  let open: ((id: string) => void) | null = null;
  window.hideHost = {
    kind: "electron",
    platform: "darwin",
    notify: (...args: unknown[]) => shown.push(args),
    withdrawNotifications: (ids: string[]) => withdrawn.push(ids),
    reportBadge: () => undefined,
    onNotificationOpen: (listener: (id: string) => void) => {
      open = listener;
      return () => (open = null);
    },
  } as never;
  const actions = createActions(() => true);
  const saved = useShellStore.getState();
  const root = createRoot(document.body.appendChild(document.createElement("div")));
  const ended = (id: string, name: string) => ({
    id, name, task: "t", schedule: { kind: "once", date: "2026-11-25", time: "18:56" }, reads_as: { kind: "once", date: "2026-11-25", time: "18:56" }, next_run_at_unix_ms: null,
    agent: { kind: "claude" }, folder: { path: "/a", default: true, worktree: false }, running: false, done: true, look_run: 1, unseen: 1,
    records: [{ kind: "run", number: 1, trigger: "scheduled", started_at_unix_ms: 0, ended_at_unix_ms: 4 * 60_000, state: "succeeded", attempt: 1, opened: false, resumable: true, conversation_missing: false, seen: false, answer: "All tables migrated" }],
  });
  const automations = { items: [ended("old", "Earlier check"), ended("db", "Check migration")], zone: "UTC" };
  const rest = (looks: string[]) => ({ ui_state: { desktop_notifications: true }, navigator: { devices: [{ id: "node", agent_scope: { raised: [], automation_looks: looks } }] } });
  try {
    await act(async () => {
      useShellStore.setState({ connection: "live", rest: rest(["old"]), automations } as never);
      root.render(<Probe actions={actions} />);
    });
    // The line there when the page went live is hided's to have told; a new one is told once, words and all.
    await act(async () => useShellStore.setState({ rest: rest(["old", "db"]) } as never));
    await act(async () => useShellStore.setState({ rest: rest(["old", "db"]) } as never));
    expect(shown).toEqual([["agent:automation-look:db|1", "Check migration", "Automation · once · Done · 4m · All tables migrated", { unlessFocused: true }]]);
    await act(async () => useShellStore.setState({ rest: rest(["old"]) } as never));
    expect(withdrawn).toEqual([["agent:automation-look:db|1"]]);
    await act(async () => open?.("agent:automation-look:db|1"));
    expect(useUiStore.getState().screen).toEqual({ kind: "automations" });
    expect(useUiStore.getState().automations).toMatchObject({ selected: "db", run: 1 });
  } finally {
    await act(async () => root.unmount());
    useShellStore.setState(saved, true);
  }
});
