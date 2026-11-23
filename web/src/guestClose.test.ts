// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createActions } from "./actions";
import { configureGuestCloseCalls, receiveGuestCloseResult } from "./guestCloseCall";
import { translate } from "./i18n/client";
import type { SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore, type Screen } from "./ui";
import { refuseCallsOf } from "./windowCall";

const KEY = "cd".repeat(32);
const SHOP = { id: "shop", label: "shop-api", path: "/work/shop-api", device_id: "mini", checkouts: [{ id: "shop-main", path: "/work/shop-api" }] };

/** The Mac mini's window, or the screen this Mac shows of it: the MacBook's work runs here, `panes` in their folders. */
function rest(panes: { pane: string; folder: string }[]): SnapshotRest {
  return {
    navigator: {
      devices: [{ id: "mini", kind: "local" }],
      focused_device_id: "mini",
      workspaces: [SHOP],
      peers: [{ key: KEY, device_id: "macbook", name: "MacBook Pro", connected: true }],
      guest_panes: [{ device_id: "macbook", key: KEY, panes: panes.map((pane, at) => ({ id: `guest:macbook:pane:${pane.pane}`, pane: pane.pane, since_unix_ms: at, folder: pane.folder })) }],
    },
  } as unknown as SnapshotRest;
}

const sent: { kind: string; payload: Record<string, unknown> }[] = [];
const savedShell = useShellStore.getState();
const savedUi = useUiStore.getState();

beforeEach(() => {
  sent.length = 0;
  configureGuestCloseCalls((event) => {
    sent.push(event);
    return true;
  });
});

afterEach(() => {
  configureGuestCloseCalls(null);
  useShellStore.setState(savedShell, true);
  useUiStore.setState(savedUi, true);
});

/** The operator confirmed closing `paneId` from its page; hided answers with `answer`, or the connection's refusal. */
async function close(state: SnapshotRest, paneId: string, answer: { ok: true } | { ok: false; code: string } | { refused: string }, screen: Screen = { kind: "guest", paneId }) {
  sent.length = 0;
  useShellStore.setState({ rest: state });
  useUiStore.setState({ screen, guestClose: paneId, notices: [] });
  const actions = createActions((event) => {
    sent.push(event);
    return true;
  });
  const closing = actions.closeGuestPane();
  const call = sent.find((event) => event.kind === "guest_pane_close");
  expect(call?.payload).toMatchObject({ call: { pane_id: paneId } });
  // The page waits for the answer: nothing moved yet.
  expect(useUiStore.getState().screen).toEqual(screen);
  if ("refused" in answer) refuseCallsOf("guest_pane_close", answer.refused);
  else receiveGuestCloseResult({ request_id: String(call?.payload.request_id), ...answer });
  await closing;
  return { screen: useUiStore.getState().screen, notices: useUiStore.getState().notices.map((notice: { text: string }) => notice.text) };
}

describe("closing another machine's work from its page (PRD device-connect-iroh B10, D-74)", () => {
  const two = rest([{ pane: "w1:p1", folder: "/work/shop-api" }, { pane: "w1:p2", folder: "/work/shop-api/web" }]);

  it("shows the next tab of that work once the close went, else the one before", async () => {
    expect(await close(two, "guest:macbook:pane:w1:p1", { ok: true })).toEqual({ screen: { kind: "guest", paneId: "guest:macbook:pane:w1:p2" }, notices: [] });
    expect((await close(two, "guest:macbook:pane:w1:p2", { ok: true })).screen).toEqual({ kind: "guest", paneId: "guest:macbook:pane:w1:p1" });
  });

  it("shows the project the last tab stood in, never the Overview", async () => {
    const result = await close(rest([{ pane: "w1:p1", folder: "/work/shop-api" }]), "guest:macbook:pane:w1:p1", { ok: true });
    expect(result.screen).toEqual({ kind: "workspace" });
    expect(sent.find((event) => event.kind === "focus_checkout")?.payload).toMatchObject({ workspace_id: "shop", checkout_id: "shop-main" });
  });

  it("shows this computer's Workspace when the work stood in no project of it", async () => {
    expect((await close(rest([{ pane: "w1:p1", folder: "/scratch" }]), "guest:macbook:pane:w1:p1", { ok: true })).screen).toEqual({ kind: "workspace" });
  });

  it("keeps the page and says why in one line when the close did not go", async () => {
    const here = { kind: "guest", paneId: "guest:macbook:pane:w1:p1" } as const;
    expect(await close(two, here.paneId, { ok: false, code: "pane_unavailable" })).toEqual({ screen: here, notices: [translate("pairing.work.closeFailed.gone")] });
    expect(await close(two, here.paneId, { ok: false, code: "socket_closed" })).toEqual({ screen: here, notices: [translate("pairing.work.closeFailed.offline")] });
    // An older build of that computer refuses the close from another computer's screen.
    expect(await close(two, here.paneId, { refused: "not_here" })).toEqual({ screen: here, notices: [translate("pairing.work.closeFailed.notHere")] });
  });

  it("leaves a page that shows something else where it is", async () => {
    expect((await close(two, "guest:macbook:pane:w1:p1", { ok: true }, { kind: "missions" })).screen).toEqual({ kind: "missions" });
  });
});

describe("the gear beside the rail (PRD device-connect-iroh B20)", () => {
  afterEach(() => {
    delete window.hideHost;
  });

  it("opens the Settings of the computer's screen in front, as ⌘, does, and this window's own otherwise", () => {
    let asked = 0;
    window.hideHost = { kind: "electron", platform: "darwin", remoteScreens: { settings: () => (asked += 1) } } as never;
    const actions = createActions(() => true);
    useUiStore.setState({ remoteScreen: "windows-pc", overlay: "none" });
    actions.openSettingsInFront();
    expect([asked, useUiStore.getState().overlay]).toEqual([1, "none"]);
    useUiStore.setState({ remoteScreen: null });
    actions.openSettingsInFront();
    expect([asked, useUiStore.getState().overlay]).toEqual([1, "settings"]);
  });
});
