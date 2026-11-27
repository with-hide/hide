// @vitest-environment jsdom
// Another machine's view of its Hide (PRD device-connect-iroh D-67, B20,
// B63): the page reaches its host through the desktop's bridge only, holds
// no token, asks no `/health`, ignores what the host refuses, and opens its
// socket while its tile shows, resuming from the revision it drew.
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { BACKOFF_START_MS, nextBackoff } from "./connection";
import { browserBridge } from "./host";
import { HIDDEN_CLOSE, remoteBridge, type BridgeSocketEvent, type RemoteBridge } from "./remoteView";
import { useShellStore } from "./store";
import { connectShell } from "./ws";

/** The desktop's side of the bridge, which the test drives. */
class FakeBridge {
  opened = 0;
  closed = 0;
  readonly sent: string[] = [];
  private socketEvents: ((event: BridgeSocketEvent) => void) | null = null;
  private visible: ((visible: boolean) => void) | null = null;

  readonly bridge: RemoteBridge = {
    bridge: 1,
    socket: {
      events: (listener) => {
        this.socketEvents = listener;
      },
      open: () => {
        this.opened += 1;
        this.sent.length = 0;
      },
      send: (data) => {
        this.sent.push(String(data));
      },
      close: () => {
        this.closed += 1;
      },
    },
    clipboardWrite: async () => true,
    onMenuCommand: () => {},
    onVisible: (listener) => {
      this.visible = listener;
    },
  };

  emit(event: BridgeSocketEvent) {
    this.socketEvents?.(event);
  }

  frame(frame: unknown) {
    this.emit({ op: "message", data: JSON.stringify(frame) });
  }

  show(visible: boolean) {
    this.visible?.(visible);
  }

  handshake(): { token: string; have_revision: number; draws_pages?: boolean } {
    return JSON.parse(this.sent[0] ?? "{}") as { token: string; have_revision: number; draws_pages?: boolean };
  }

  kinds(): string[] {
    return this.sent.slice(1).map((message) => (JSON.parse(message) as { kind: string }).kind);
  }
}

const saved = useShellStore.getState();
let desktop: FakeBridge;
const fetched = vi.fn(async () => ({ ok: true }));

beforeEach(() => {
  vi.useFakeTimers();
  desktop = new FakeBridge();
  vi.stubGlobal("hideRemote", desktop.bridge);
  vi.stubGlobal("WebSocket", class {
    static readonly OPEN = 1;
    static readonly CLOSING = 2;
    static readonly CLOSED = 3;
    constructor() {
      throw new Error("another machine's view opens no socket of its own");
    }
  });
  fetched.mockClear();
  vi.stubGlobal("fetch", fetched);
  window.location.hash = "";
});

afterEach(() => {
  useShellStore.setState(saved, true);
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

it("a view of another machine opens through the bridge with no token and is drawn from the host", () => {
  const shell = connectShell({ onChunks: () => {}, onCopy: () => {} });
  expect(desktop.opened).toBe(1);
  desktop.emit({ op: "open" });
  expect(desktop.handshake().token).toBe("");
  desktop.frame({ type: "snapshot", payload: { revision: 4, rest: {} } });
  expect(useShellStore.getState().connection).toBe("live");
  // It is counted on the machine that shows it, never on the host.
  expect(desktop.kinds()).not.toContain("product_analytics_connected");

  // What the host refused stays a note in the page's own log.
  desktop.frame({ type: "refused", payload: { kind: "pairing_share", reason: "not_here" } });
  expect(useShellStore.getState().connection).toBe("live");
  expect(useShellStore.getState().diagnostics.at(-1)).toContain("pairing_share not_here");
  shell.close();
});

it("a dropped view opens again through the bridge and asks no health of a host it cannot reach", async () => {
  const shell = connectShell({ onChunks: () => {}, onCopy: () => {} });
  desktop.emit({ op: "open" });
  desktop.frame({ type: "snapshot", payload: { revision: 7, rest: {} } });

  desktop.emit({ op: "close", code: 1006, reason: "" });
  expect(useShellStore.getState().connection).toBe("reconnecting");
  await vi.advanceTimersByTimeAsync(nextBackoff(BACKOFF_START_MS));
  expect(desktop.opened).toBe(2);
  expect(fetched).not.toHaveBeenCalled();
  desktop.emit({ op: "open" });
  expect(desktop.handshake().have_revision).toBe(7);
  shell.close();
});

it("a hidden tile's view stays closed until the tile shows, then resumes at once", async () => {
  const shell = connectShell({ onChunks: () => {}, onCopy: () => {} });
  desktop.emit({ op: "open" });
  desktop.frame({ type: "snapshot", payload: { revision: 3, rest: {} } });

  desktop.show(false);
  desktop.emit({ op: "close", code: HIDDEN_CLOSE, reason: "hidden" });
  await vi.advanceTimersByTimeAsync(60_000);
  expect(desktop.opened).toBe(1);

  desktop.show(true);
  expect(desktop.opened).toBe(2);
  desktop.emit({ op: "open" });
  expect(desktop.handshake().have_revision).toBe(3);
  shell.close();
});

it("a connection never keeps the pairings an earlier one was shown", () => {
  vi.unstubAllGlobals();
  useShellStore.setState({ pairing: { offer: null, asks: [], join: null, requests: [] } });
  vi.stubGlobal("hideRemote", desktop.bridge);
  vi.stubGlobal("fetch", fetched);
  const shell = connectShell({ onChunks: () => {}, onCopy: () => {} });
  expect(useShellStore.getState().pairing).toBeNull();
  shell.close();
});

// docs/BROWSER_DISPLAYS.md, A page in another computer's screen: a desktop
// app that draws this machine's pages adds `browser` beside `bridge: 1`,
// which every page checks, so an older page and an older app each keep
// working with the other.
it("a view whose window draws its pages says so and finds them, and one from an older app still connects without", () => {
  expect(remoteBridge()).not.toBeNull();
  expect(browserBridge()).toBeNull();
  let shell = connectShell({ onChunks: () => {}, onCopy: () => {} });
  desktop.emit({ op: "open" });
  expect(desktop.handshake().draws_pages).toBeUndefined();
  shell.close();

  const browser = { sync: vi.fn(), endCycle: vi.fn(), capture: vi.fn(), command: vi.fn(), onEvent: vi.fn(() => () => {}) };
  vi.stubGlobal("hideRemote", { ...desktop.bridge, browser });
  expect(browserBridge()).toBe(browser);
  shell = connectShell({ onChunks: () => {}, onCopy: () => {} });
  desktop.emit({ op: "open" });
  expect(desktop.handshake().draws_pages).toBe(true);
  shell.close();
});
