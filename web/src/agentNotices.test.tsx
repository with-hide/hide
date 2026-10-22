// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions, type Actions } from "./actions";
import { receiveAgentNotice, useAgentNotifications } from "./agentNotices";
import type { AgentScope } from "./agentScope";
import { clientI18n } from "./i18n/translator";
import { useShellStore } from "./store";
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
  expect(events.filter((event) => event.kind === "desktop_notices").map((event) => event.payload)).toEqual([{ wanted: true }]);

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
    ["Add a dark mode switch", "Question · 배포할 환경을 골라 주세요 · my-app"],
    ["Add a dark mode switch", "Approval · 계획 승인 대기 · my-app"],
    ["Add a dark mode switch", "Blocked · 로그인이 필요해요 · my-app"],
    ["Add a dark mode switch", "Done · 배포 완료 · my-app"],
    ["Add a dark mode switch", "Unfinished · 테스트 세 개 남음 · my-app"],
    ["Add a dark mode switch", "Done"],
  ]);
  await act(async () => { await clientI18n.changeLanguage("ko"); });
  await act(async () => receiveAgentNotice({ notices: [{ tag: "node|w1:p1", device_id: "node", pane_id: "w1:p1", title: "Add a dark mode switch", state: "needs_you", status_code: "question", detail: "Choose staging?", place: "" }], clear: [] }));
  expect(shown.at(-1)?.slice(1, 3)).toEqual(["Add a dark mode switch", "질문 · Choose staging?"]);
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
  expect(shown.slice(-2).map((notice) => notice[2])).toEqual(["질문 · 계획 검토 · 5단계 · my-app", "Question · Plan review · 5 steps · my-app"]);

  await act(async () => open?.("agent:node|w1:p1"));
  expect(events.some((event) => event.kind === "focus_pane" && event.payload.pane_id === "w1:p1")).toBe(true);

  // The switch off: the window stops asking, and a late frame shows nothing.
  await act(async () => {
    useShellStore.setState({ rest: { ui_state: { desktop_notifications: false }, navigator: { devices: [] } } } as never);
  });
  expect(events.filter((event) => event.kind === "desktop_notices").at(-1)?.payload).toEqual({ wanted: false });
  await act(async () => receiveAgentNotice({ notices: [{ tag: "node|w1:p2", device_id: "node", pane_id: "w1:p2", title: "x", state: "done", place: "" }], clear: [] }));
  expect(shown).toHaveLength(12);
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

// Operator decision 2026-10-18: include unread Done in the app icon under its own switch.
it.each([
  [false, false, [], 0, false],
  [true, false, ["Needs You · my-app"], 2, true],
  [false, true, ["Unread · my-app"], 1, true],
  [true, true, ["Needs You · my-app", "Unread · my-app"], 3, true],
] as const)("with Needs You=%s and Done=%s sends and counts only the chosen groups", async (needsYou, done, bodies, badge, wanted) => {
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
    expect(events.filter((event) => event.kind === "desktop_notices").map((event) => event.payload)).toEqual(wanted ? [{ wanted: true }] : []);
  } finally {
    await act(async () => root.unmount());
    useShellStore.setState(saved, true);
  }
});

it("counts enabled groups across devices, updates when Done is read, and keeps the legacy shared choice", async () => {
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
  expect(reported).toEqual([3, 4, 3, 12, 0]);
  await act(async () => root.unmount());
  useShellStore.setState(saved, true);
});
