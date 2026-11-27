// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { Actions } from "./actions";
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
});

it("opens another computer's row on that computer's agent, by the pane its core knows it by (D-78)", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const saved = useShellStore.getState();
  const openComputerAgent = vi.fn();
  const actions = { openComputerAgent } as unknown as Actions;
  const root = createRoot(document.body.appendChild(document.createElement("ul")));
  try {
    await act(async () => {
      useShellStore.setState({
        connection: "live",
        pairing: NOTHING_UNDER_WAY,
        rest: {
          navigator: {
            devices: [],
            peers: [{ key: "ab12", device_id: "mini", name: "Mac mini", connected: true, opens: "active" }],
            peer_work: [{ device_id: "mini", key: "ab12", needs_you: 1, done: 0, rows: [{ id: "w1:p2", group: "needs_you", title: "Fix login", agent_kind: "claude", mark: "turn", status_code: "question" }] }],
          },
        },
      } as never);
      root.render(<TooltipProvider><PeerWorkRows group="needs_you" actions={actions} /></TooltipProvider>);
    });
    await act(async () => (document.querySelector('[data-peer-row="mini:w1:p2"]') as HTMLButtonElement).click());
    expect(openComputerAgent.mock.calls).toEqual([["mini", "w1:p2"]]);
  } finally {
    await act(async () => root.unmount());
    useShellStore.setState(saved, true);
  }
});
