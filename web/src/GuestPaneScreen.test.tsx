// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { GuestPaneScreen } from "./GuestPaneScreen";
import { InterfaceLanguageBoundary } from "./i18n/client";
import { useShellStore } from "./store";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks; the terminal itself is not what this reads.
vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });
vi.mock("./terminals", async (load) => ({ ...(await load<typeof import("./terminals")>()), attachTerminal: () => () => undefined, focusTerminal: () => undefined }));
afterEach(() => { document.body.innerHTML = ""; });

const KEY = "ab".repeat(32);
const pane = (pane: string, folder: string, more: Record<string, string> = {}) => ({ id: `guest:macbook:pane:${pane}`, pane, since_unix_ms: Number(pane.at(-1)), folder, ...more });

// PRD device-connect-iroh B10, D-74 (operator review 2026-11-23): another machine's work opens as this computer's own pane does.
it("opens another machine's work with its project's tabs by name and a header of its status, name, project and machine, said once", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const saved = useShellStore.getState();
  try {
    await act(async () => useShellStore.setState({
      connection: "live",
      rest: {
        navigator: {
          devices: [],
          peers: [{ key: KEY, device_id: "macbook", name: "MacBook Pro", connected: true }],
          workspaces: [{ id: "hide", label: "hide", path: "/work/hide", device_id: "local", checkouts: [{ id: "hide-main", path: "/work/hide" }] }],
          guest_panes: [{ device_id: "macbook", key: KEY, panes: [
            pane("w1:p1", "/work/hide", { name: "배치 PR 처리", branch: "main", mark: "going" }),
            pane("w1:p2", "/work/hide", { branch: "m75-relay", mark: "quiet" }),
            pane("w1:p3", "/work/modakbul", { name: "강의 정산", mark: "quiet" }),
          ] }],
        },
        ui_state: { interface_language: "ko" },
      },
    } as never));
    const root = createRoot(document.body.appendChild(document.createElement("div")));
    const actions = createActions(() => true);
    await act(async () => root.render(<><InterfaceLanguageBoundary /><TooltipProvider><GuestPaneScreen paneId="guest:macbook:pane:w1:p1" actions={actions} /></TooltipProvider></>));

    // The tab strip: that machine's work in the same project, by the names its rows carry, and nothing of another project's.
    const tabs = [...document.querySelectorAll<HTMLElement>("[data-guest-tab]")];
    expect(tabs.map((tab) => tab.textContent)).toEqual(["배치 PR 처리", "터미널 · m75-relay"]);
    expect(document.querySelector("[data-guest-tab] [data-guest-mark]")).toBeNull();

    // The header: where the work stands, its name, its project, and once which machine started it.
    const header = document.querySelector<HTMLElement>("[data-guest-pane-header]")!;
    expect(header.querySelector("[data-guest-pane-status]")?.getAttribute("data-guest-pane-status")).toBe("going");
    expect(header.querySelector("[data-guest-pane-name]")?.textContent).toBe("배치 PR 처리");
    expect(header.querySelector("[data-guest-pane-project]")?.textContent).toBe("· hide");
    expect(header.querySelectorAll("[data-guest-mark='MacBook Pro']")).toHaveLength(1);
    expect(header.querySelector("[data-guest-pane-started]")?.textContent).toBe("MacBook Pro에서 시킨 일");
    expect(header.textContent?.split("MacBook Pro").length).toBe(2);
    // D-74's set is the pane's menu, unchanged: its button stays at the header's end.
    expect(header.querySelector("[data-guest-pane-more]")).not.toBeNull();
    await act(async () => root.unmount());
  } finally {
    useShellStore.setState(saved, true);
  }
});
