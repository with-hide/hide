// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { PeerWorkRows } from "./GuestWorkRows";
import { NOTHING_UNDER_WAY } from "./pairing";
import { useShellStore } from "./store";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

afterEach(() => {
  document.body.innerHTML = "";
  delete window.hideHost;
});

// The row hands the same show its machine's notice hands (agentNotices.test.tsx): both carry the root agent's pane id there.
it("opens another computer's row on that computer's screen with the agent's pane chosen (D-78)", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const saved = useShellStore.getState();
  const go = vi.fn();
  window.hideHost = { kind: "electron", platform: "darwin", remoteScreens: { show: vi.fn(), go, onState: () => () => {}, onWorkResult: () => () => {} } } as never;
  const events: unknown[] = [];
  const actions = createActions((event) => (events.push(event), true));
  const key = "ab".repeat(32);
  const root = createRoot(document.body.appendChild(document.createElement("ul")));
  try {
    await act(async () => {
      useShellStore.setState({
        connection: "live",
        pairing: NOTHING_UNDER_WAY,
        rest: {
          navigator: {
            devices: [{ id: "mini", kind: "remote", label: "Mac mini", state: "ready", peer_key: key, agent_scope: { raised: [] } }],
            peers: [{ key, device_id: "mini", name: "Mac mini", connected: true, opens: "active" }],
            peer_work: [{ device_id: "mini", key, needs_you: 1, done: 0, rows: [{ id: "w1:p2", group: "needs_you", title: "Fix login", agent_kind: "claude", mark: "turn", status_code: "question" }] }],
          },
        },
      } as never);
      root.render(<TooltipProvider><PeerWorkRows group="needs_you" actions={actions} /></TooltipProvider>);
    });
    await act(async () => (document.querySelector('[data-peer-row="mini:w1:p2"]') as HTMLButtonElement).click());
    expect(go.mock.calls).toEqual([[{ device: "mini", key, work: { op: "show", pane: "w1:p2" } }]]);
    expect(events).toEqual([]);
  } finally {
    await act(async () => root.unmount());
    useShellStore.setState(saved, true);
  }
});
