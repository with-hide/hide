import { expect, it } from "vitest";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { installWindowScreen, windowScreen } from "./windowScreen";

it("reads the screen the window shows: this machine's own, or another computer's in its tile", () => {
  useUiStore.setState({ remoteScreen: null });
  expect(windowScreen()).toEqual({ screen: "own" });
  useUiStore.setState({ remoteScreen: "mini" });
  expect(windowScreen()).toEqual({ screen: "computer" });
  useUiStore.setState({ remoteScreen: null });
});

// PRD device-connect-iroh D-55: this machine's work that runs on the paired machine whose screen shows here, shown on
// its page there, is named by this core's pane id, so the core reads it as looked at while it shows.
it("names this machine's pane a paired screen shows, and none for another machine's work or another screen", () => {
  const KEY = "k".repeat(64);
  useShellStore.setState({
    rest: {
      navigator: {
        devices: [{ id: "mac", kind: "local" }, { id: "mini-device", kind: "remote", peer_key: KEY }],
        peers: [{ key: KEY, device_id: "mini", name: "mini", opens: "active", connected: true }],
      },
      status: { remote: [{ target_id: "mini-device", state: "connected", session: { agents: [{ pane_id: "remote:mini-device:pane:w1:p3", device_id: "mini-device", device_pane_id: "w1:p3" }], workspaces: [] } }] },
    },
  } as never);
  useUiStore.setState({ remoteScreen: "mini", remoteShowing: { mini: { device: "mac", pane: "w1:p3" } } });
  expect(windowScreen()).toEqual({ screen: "computer", shown_there: "remote:mini-device:pane:w1:p3" });
  useUiStore.setState({ remoteShowing: { mini: { device: "windows-pc", pane: "w1:p3" } } });
  expect(windowScreen()).toEqual({ screen: "computer" });
  useUiStore.setState({ remoteShowing: { mini: { device: "mac", pane: "w9:p9" } } });
  expect(windowScreen()).toEqual({ screen: "computer" });
  useUiStore.setState({ remoteScreen: "other", remoteShowing: { mini: { device: "mac", pane: "w1:p3" } } });
  expect(windowScreen()).toEqual({ screen: "computer" });
  useUiStore.setState({ remoteScreen: null, remoteShowing: {} });
  useShellStore.setState({ rest: null } as never);
});

// The daemon holds nothing of a connection it has not met, so each new live connection is told once.
it("tells at once and again each time the connection is live again", () => {
  useShellStore.setState({ connection: "live" } as never);
  let told = 0;
  let changed = 0;
  const remove = installWindowScreen(() => (told += 1), () => (changed += 1));
  useShellStore.setState({ connection: "connecting" } as never);
  useShellStore.setState({ connection: "live" } as never);
  useShellStore.setState({ connection: "live", rest: null } as never);
  remove();
  useShellStore.setState({ connection: "connecting" } as never);
  useShellStore.setState({ connection: "live" } as never);
  expect(told).toBe(2);
  expect(changed).toBe(0);
});

// Which of this machine's panes a paired screen shows is read from the snapshot, so a snapshot while one shows may change it.
it("asks again on a snapshot only while another computer's screen shows", () => {
  useShellStore.setState({ connection: "live", rest: null } as never);
  let changed = 0;
  const remove = installWindowScreen(() => {}, () => (changed += 1));
  useShellStore.setState({ rest: {} } as never);
  useUiStore.setState({ remoteScreen: "mini" });
  useShellStore.setState({ rest: {} } as never);
  remove();
  useUiStore.setState({ remoteScreen: null });
  useShellStore.setState({ rest: null } as never);
  expect(changed).toBe(1);
});
