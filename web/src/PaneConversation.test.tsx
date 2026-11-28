// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { PaneConversation } from "./PaneConversation";
import type { AgentRow, AgentSleep, SessionThreadSection } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { TooltipProvider } from "./components/ui/tooltip";
import { layOutConversation, watchResizes } from "./components/desk/thread-test-layout";
import { forgetScrolls } from "./conversationScroll";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });

// The conversation over a pane (the `대화` toggle) of an agent that sleeps in it: before, its box stayed and a send
// answered only that the session was asleep; now it reads as the session beside Desk does, dimmed with the sleep card
// in its middle and no box, and the card wakes the agent in its pane.

let saved: ReturnType<typeof useShellStore.getState>;
let mounted: ReturnType<typeof createRoot>[] = [];
let laidOut: () => Promise<void>;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  // The conversation draws only the rows its viewport reaches: give it one that holds them all.
  ({ laidOut } = watchResizes());
  layOutConversation();
  saved = useShellStore.getState();
  useUiStore.setState({ threadLease: null, paneConversation: { paneId: "w1:p2" } as never });
});

afterEach(async () => {
  for (const root of mounted) await act(async () => root.unmount());
  mounted = [];
  useShellStore.setState(saved, true);
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  forgetScrolls();
  document.body.innerHTML = "";
});

const section: SessionThreadSection = {
  generation: 1, state: "ready", pane_id: "w1:p2", machine: "mac", failure: null, pane: "same", connected: true, has_older: false, older: "idle", input: null, queue: null, choice: {},
  tail: { record: "r1", start: 0, updates: [{ sessionUpdate: "user_message_chunk", content: { type: "text", text: "그 e2e도 봐 줘" } }, { sessionUpdate: "agent_message_chunk", content: { type: "text", text: "상태를 기다리게 바꾸는 중이에요" } }], parts: {}, sessions: {} },
};
const row = (sleep: AgentSleep | null) => ({ id: "row", pane_id: "w1:p2", device_id: "mac", agent_kind: "codex", sleep, state: { working: false, asking: false } }) as unknown as AgentRow;

it("draws a sleeping agent's conversation dimmed with Wake agent in its middle and no box, and the box once it is awake", async () => {
  const events: { kind: string; payload: unknown }[] = [];
  const actions = createActions((event) => {
    events.push({ kind: event.kind, payload: event.payload });
    return true;
  });
  useShellStore.setState({ connection: "live", agents: [row({ state: "sleeping", since_unix_ms: 0, progress: "e2e 기다리는 방식 바꾸는 중" })], sessionThread: section, sessionThreadHistory: { generation: 1, pages: [] } });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  mounted.push(root);
  await act(async () => root.render(<TooltipProvider><PaneConversation paneId="w1:p2" actions={actions} /></TooltipProvider>));
  await laidOut();

  expect(container.textContent).toContain("상태를 기다리게 바꾸는 중이에요");
  expect(container.querySelector("[data-thread-asleep]")?.className).toContain("opacity-(--opacity-dimmed)");
  expect(container.querySelector("textarea")).toBeNull();
  expect(container.querySelector("[data-pane-sleep]")?.textContent).toContain("e2e 기다리는 방식 바꾸는 중");
  await act(async () => container.querySelector<HTMLButtonElement>('[data-agent-wake="w1:p2"]')!.click());
  expect(events.filter((event) => event.kind === "agent_wake")).toEqual([{ kind: "agent_wake", payload: { pane_id: "w1:p2", fresh: false } }]);

  await act(async () => useShellStore.setState({ agents: [row(null)] }));
  expect(container.querySelector("[data-pane-sleep]")).toBeNull();
  expect(container.querySelector("[data-thread-asleep]")).toBeNull();
  // Opening the conversation over the pane was its press: the box takes the keyboard once it stands.
  expect(document.activeElement).toBe(container.querySelector("textarea"));
});
