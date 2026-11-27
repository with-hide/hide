// The pages another computer's screen places in this window
// (docs/BROWSER_DISPLAYS.md, A page in another computer's screen): kept apart
// from this window's, made only once their proxy is set, held to the
// screen's bounds and tile, and loaded only where that computer's core says.

import { EventEmitter } from "node:events";
import { afterEach, describe, expect, it, vi } from "vitest";
import { initializeInterfaceI18n } from "../../../web/src/i18n/instance";
import { BROWSER_SYNC_CHANNEL, REMOTE_BROWSER_EVENT_CHANNEL } from "../channel";
import { BrowserViews, type RemoteRoute, type RemoteSource } from "./browser";
import { placeIn, remotePagePartition } from "./browserSync";
import { session } from "electron";
import { ExternalFiles } from "./externalFiles";
import type { HostLog } from "./log";

const english = initializeInterfaceI18n("en").getFixedT(null, "translation");
const ipc = vi.hoisted(() => new Map<string, (event: unknown, value?: unknown) => void>());
const native = vi.hoisted(() => ({
  make: ((): unknown => { throw new Error("no native view"); }) as (options: unknown) => unknown,
  window: ((): unknown => { throw new Error("no popup window"); }) as (options: unknown) => unknown,
  fromId: ((): unknown => null) as (id: number) => unknown,
}));
vi.mock("electron", () => ({
  ipcMain: { on: (channel: string, listener: (event: unknown, value?: unknown) => void) => ipc.set(channel, listener), handle: vi.fn() },
  Menu: { getApplicationMenu: () => null },
  BrowserWindow: function BrowserWindow(options: unknown) { return native.window(options); },
  WebContentsView: function WebContentsView(options: unknown) { return native.make(options); },
  webContents: { fromId: (id: number) => native.fromId(id) },
  screen: { getDisplayMatching: () => ({ workArea: { x: 0, y: 0, width: 4000, height: 3000 } }) },
  session: {}, shell: {},
}));

const DEVICE = "mini";
const KEY = "ab".repeat(32);
const PAIRING = "11".repeat(16);
const PARTITION = remotePagePartition(DEVICE, KEY, PAIRING);
const workspace = "mini\u0000/web";
const TILE = { x: 100, y: 50, width: 800, height: 600 };
const RECT = { x: 400, y: 40, width: 600, height: 500 };

type Contents = EventEmitter & Record<string, unknown> & { id: number; loadURL: ReturnType<typeof vi.fn>; close: ReturnType<typeof vi.fn> };
type View = { contents: Contents; options: { webPreferences: Record<string, unknown> }; setBounds: ReturnType<typeof vi.fn>; setVisible: ReturnType<typeof vi.fn> };
type Session = { handlers: Map<string, (...args: unknown[]) => void>; beforeRequest: (details: Record<string, unknown>, callback: (answer: { cancel: boolean }) => void) => void };

afterEach(() => {
  vi.useRealTimers();
  ipc.clear();
});

/** A page manager in a window, with one remote screen whose proxy and route the test answers. */
function screen() {
  vi.useFakeTimers();
  let nextId = 100;
  const views: View[] = [];
  const contentsOf = (id: number, url = ""): Contents => Object.assign(new EventEmitter(), {
    id, isDestroyed: () => false, isFocused: () => false, getZoomFactor: () => 1, getURL: () => url, getTitle: () => "", close: vi.fn(),
    loadURL: vi.fn(() => new Promise(() => undefined)), setWindowOpenHandler: vi.fn(), setWebRTCIPHandlingPolicy: vi.fn(),
    setVisualZoomLevelLimits: vi.fn().mockResolvedValue(undefined), enableDeviceEmulation: vi.fn(), disableDeviceEmulation: vi.fn(),
    capturePage: vi.fn(async () => ({ isEmpty: () => false, toDataURL: () => "data:still" })), isCurrentlyAudible: () => false,
    navigationHistory: { canGoBack: () => false, canGoForward: () => false }, focus: vi.fn(), stop: vi.fn(),
  }) as unknown as Contents;
  native.make = (options) => {
    const view: View = { contents: contentsOf(++nextId), options: options as View["options"], setBounds: vi.fn(), setVisible: vi.fn() };
    views.push(view);
    return { webContents: view.contents, setBounds: view.setBounds, setVisible: view.setVisible };
  };
  const sessions = new Map<string, Session>();
  Object.assign(session, {
    fromPartition: vi.fn((partition: string) => {
      let made = sessions.get(partition);
      if (!made) {
        const handlers = new Map<string, (...args: unknown[]) => void>();
        const value: Session = { handlers, beforeRequest: () => undefined };
        sessions.set(partition, value);
        made = value;
      }
      const record = made;
      return {
        setPermissionRequestHandler: vi.fn(), setPermissionCheckHandler: vi.fn(),
        on: (name: string, handler: (...args: unknown[]) => void) => record.handlers.set(name, handler),
        webRequest: { onBeforeRequest: (_filter: unknown, listener: Session["beforeRequest"]) => { record.beforeRequest = listener; } },
      };
    }),
  });
  const log = { event: vi.fn() };
  const subject = new BrowserViews(log as unknown as HostLog, () => true, vi.fn(), vi.fn(), vi.fn(), () => english, new ExternalFiles(vi.fn()), { bound: 1e12, every: 1e12, now: () => Date.now() });
  const shellSend = vi.fn();
  const shell = Object.assign(new EventEmitter(), { isDestroyed: () => false, send: shellSend, focus: vi.fn(), getZoomFactor: () => 1 });
  const addChildView = vi.fn();
  const window = Object.assign(new EventEmitter(), {
    isFocused: () => true, isDestroyed: () => false, webContents: shell, getContentBounds: () => ({ x: 1000, y: 500, width: 1600, height: 1000 }), getBounds: () => ({ x: 1000, y: 470, width: 1600, height: 1030 }),
    contentView: { addChildView, removeChildView: vi.fn() },
  });
  subject.attach(window as never);
  const screenContents = Object.assign(new EventEmitter(), { id: 1, isDestroyed: () => false, isFocused: () => false, send: vi.fn(), focus: vi.fn() });
  const state = { ready: false, area: { bounds: TILE, zoom: 1 } as { bounds: typeof TILE; zoom: number } | null, storage: "live" as "live" | "clearing" | "full" | "waiting", partition: PARTITION as string | null };
  const addresses = new Map<string, string>();
  const route = vi.fn(async (_workspace: string, id: string, load: number): Promise<RemoteRoute> => {
    const url = addresses.get(id) ?? "";
    return { url, source_url: url, load, own: true };
  });
  const remote: RemoteSource = {
    device: DEVICE, key: KEY,
    contents: () => screenContents as never,
    area: () => state.area,
    focused: () => false,
    ready: () => state.ready,
    ensure: vi.fn(async () => { state.ready = true; return "ready"; }),
    partition: () => state.partition,
    storage: () => state.storage,
    route,
  };
  subject.addRemote(remote);
  const display = (id: string, visible = true, rect: typeof RECT | null = RECT, url = `https://${id}.example/`) => {
    addresses.set(id, url);
    return { id, area_id: "a1", url, load: 3, rect, visible };
  };
  const sync = (displays: ReturnType<typeof display>[], retained = displays.map(({ id, url }) => ({ workspace, id, area_id: "a1", url, load: 3 }))) =>
    subject.remoteSync(remote, { workspace, displays, retained });
  const events = () => screenContents.send.mock.calls.filter(([channel]) => channel === REMOTE_BROWSER_EVENT_CHANNEL).map(([, event]) => event as { kind: string; id: string; state?: { failure: string | null } });
  /** Lets the proxy's answer, the throttled sync and the reports land. */
  const settle = async () => { await vi.advanceTimersByTimeAsync(200); };
  return { subject, remote, state, views, sync, display, events, settle, log, route, sessions, shellSend, window, addChildView, contentsOf, screenContents };
}

describe("a remote screen's pages", () => {
  it("are made only once their proxy is set, in that machine's own storage, cut to the tile, with no dialogs, no spellcheck and no UDP of their own", async () => {
    const { sync, display, views, remote, settle } = screen();
    sync([display("d1")]);
    expect(remote.ensure).toHaveBeenCalledOnce();
    expect(views).toHaveLength(0);
    await settle();
    expect(views).toHaveLength(1);
    const [view] = views;
    expect(view!.options.webPreferences).toMatchObject({ disableDialogs: true, spellcheck: false, sandbox: true, contextIsolation: true, nodeIntegration: false });
    expect(session.fromPartition).toHaveBeenCalledWith(PARTITION);
    expect(view!.contents.setWebRTCIPHandlingPolicy).toHaveBeenCalledWith("disable_non_proxied_udp");
    expect(view!.setBounds).toHaveBeenLastCalledWith(placeIn(RECT, 1, TILE));
    expect(view!.contents.loadURL).toHaveBeenCalledExactlyOnceWith("https://d1.example/");
  });

  it("load nothing before that computer's core answers, and nothing on its refusal or a changed address", async () => {
    const { sync, display, views, route, events, settle } = screen();
    route.mockResolvedValueOnce({ refused: "remote_device_page_unavailable" });
    sync([display("d1")]);
    await settle();
    expect(views[0]!.contents.loadURL).not.toHaveBeenCalled();
    expect(events().filter((event) => event.kind === "state").at(-1)?.state?.failure).toBe(english("native.browser.remoteDevicePage"));
    route.mockResolvedValueOnce({ url: "https://elsewhere.example/", source_url: "https://elsewhere.example/", load: 4, own: true });
    sync([{ ...display("d1"), load: 4 }]);
    await settle();
    expect(views[0]!.contents.loadURL).not.toHaveBeenCalled();
    expect(events().filter((event) => event.kind === "state").at(-1)?.state?.failure).toBe(english("native.browser.routeUnavailable"));
  });

  // A slot that failed before its page was made has a Reload that works.
  it("apply the screen's last sync again on a failed slot's Reload", async () => {
    const { subject, remote, state, sync, display, views, settle } = screen();
    state.storage = "clearing";
    sync([display("d1")]);
    await settle();
    expect(views).toHaveLength(0);
    state.storage = "live";
    subject.remoteCommand(remote, { workspace, id: "d1" }, "reload");
    await settle();
    expect(views).toHaveLength(1);
  });

  // After a link in the page failed, Reload asks that computer's core for the
  // display's address again, which its answer matches, not the failed link.
  it("reload a page whose link failed at the address its route gave", async () => {
    const { subject, remote, sync, display, views, route, settle } = screen();
    sync([display("d1")]);
    await settle();
    views[0]!.contents.emit("did-fail-load", {}, -105, "NAME_NOT_RESOLVED", "https://broken.example/", true);
    subject.remoteCommand(remote, { workspace, id: "d1" }, "reload");
    await settle();
    expect(route.mock.calls.at(-1)).toEqual([workspace, "d1", 3]);
    expect(views[0]!.contents.loadURL.mock.calls.at(-1)?.[0]).toBe("https://d1.example/");
    expect(views[0]!.contents.loadURL).toHaveBeenCalledTimes(2);
  });

  it("refuse a file display of that computer before asking anything", async () => {
    const { sync, display, views, route, events, settle } = screen();
    sync([display("f1", true, RECT, "file:///Users/me/report.html")]);
    await settle();
    expect(route).not.toHaveBeenCalled();
    expect(views[0]!.contents.loadURL).not.toHaveBeenCalled();
    expect(events().filter((event) => event.kind === "state").at(-1)?.state?.failure).toBe(english("native.browser.remoteFile"));
  });

  it("stay apart from this window's: events go to the screen alone, and neither shell's sync closes the other's page of the same Workspace and id", async () => {
    const { sync, display, views, shellSend, events, settle } = screen();
    sync([display("d1")]);
    await settle();
    expect(events().some((event) => event.id === "d1")).toBe(true);
    expect(shellSend.mock.calls.filter(([, event]) => (event as { id?: string }).id === "d1")).toEqual([]);
    ipc.get(BROWSER_SYNC_CHANNEL)!({}, { workspace, displays: [], retained: [] });
    expect(views[0]!.contents.close).not.toHaveBeenCalled();
  });

  // Two screens and this window may show displays of the same Workspace and
  // id: a command, a still, the keyboard and an event of one reach its own
  // page alone.
  it("keep two screens' and this window's pages of the same id apart in commands, stills, focus and events", async () => {
    const { subject, remote, sync, display, views, shellSend, events, settle, screenContents } = screen();
    sync([display("d1")]);
    await settle();
    const otherContents = Object.assign(new EventEmitter(), { id: 2, isDestroyed: () => false, isFocused: () => false, send: vi.fn(), focus: vi.fn() });
    let otherFocused = false;
    const other: RemoteSource = {
      device: "pc", key: KEY,
      contents: () => otherContents as never,
      area: () => ({ bounds: TILE, zoom: 1 }),
      focused: () => otherFocused,
      ready: () => true,
      ensure: vi.fn(async () => "ready"),
      partition: () => remotePagePartition("pc", KEY, PAIRING),
      storage: () => "live",
      route: async (_workspace, id, load) => ({ url: `https://${id}.example/`, source_url: `https://${id}.example/`, load, own: true }),
    };
    subject.addRemote(other);
    subject.remoteSync(other, { workspace, displays: [display("d1")], retained: [{ workspace, id: "d1", area_id: "a1", url: "https://d1.example/", load: 3 }] });
    await settle();
    expect(views).toHaveLength(2);
    const [mine, theirs] = views as [View, View];
    const target = { workspace, id: "d1" };

    subject.remoteCommand(other, target, "stop");
    expect(theirs.contents.stop).toHaveBeenCalledOnce();
    expect(mine.contents.stop).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(1_000);
    await subject.remoteCapture(remote, target);
    expect(mine.contents.capturePage).toHaveBeenCalled();
    expect(theirs.contents.capturePage).not.toHaveBeenCalled();

    // The keyboard goes to a page only from inside its own screen.
    subject.remoteCommand(other, target, "focus");
    expect(theirs.contents.focus).not.toHaveBeenCalled();
    otherFocused = true;
    subject.remoteCommand(other, target, "focus");
    expect(theirs.contents.focus).toHaveBeenCalled();
    expect(mine.contents.focus).not.toHaveBeenCalled();

    // Each screen hears of its own page alone, and this window of neither.
    theirs.contents.emit("page-title-updated", {}, "Theirs");
    await settle();
    const heard = (contents: { send: ReturnType<typeof vi.fn> }) => contents.send.mock.calls.filter(([channel]) => channel === REMOTE_BROWSER_EVENT_CHANNEL).map(([, event]) => event as { state?: { title?: string } });
    expect(heard(otherContents).some((event) => event.state?.title === "Theirs")).toBe(true);
    expect(events().some((event) => (event as { state?: { title?: string } }).state?.title === "Theirs")).toBe(false);
    expect(shellSend.mock.calls.some(([, event]) => (event as { state?: { title?: string } }).state?.title === "Theirs")).toBe(false);
    expect(screenContents.send.mock.calls.length).toBeGreaterThan(0);

    // A sync of one closes nothing of the other.
    subject.remoteSync(other, { workspace, displays: [], retained: [] });
    await settle();
    expect(theirs.contents.close).toHaveBeenCalled();
    expect(mine.contents.close).not.toHaveBeenCalled();
  });

  it("are no CDP target", async () => {
    const { subject, sync, display, settle } = screen();
    sync([display("d1")]);
    await settle();
    expect(subject.cdpPages()).toEqual([]);
  });

  it("give a placement past four no page, and its slot says why", async () => {
    const { sync, display, views, events, settle } = screen();
    sync(["d1", "d2", "d3", "d4", "d5"].map((id) => display(id)));
    await settle();
    expect(views).toHaveLength(4);
    expect(events().find((event) => event.id === "d5" && event.kind === "state")?.state?.failure).toBe(english("native.browser.remotePageLimit"));
  });

  // The operator walks through five displays: the four first stay retained,
  // hidden, and the one on screen takes the place of the one shown longest
  // ago rather than being refused.
  it("close the hidden page shown longest ago for a placed display past four", async () => {
    const { sync, display, views, events, settle } = screen();
    const ids = ["d1", "d2", "d3", "d4", "d5"];
    const retained = ids.map((id) => ({ workspace, id, area_id: "a1", url: `https://${id}.example/`, load: 3 }));
    for (const id of ids) {
      sync([display(id)], retained);
      await settle();
    }
    expect(views).toHaveLength(5);
    expect(views.map((view) => view.contents.close.mock.calls.length > 0)).toEqual([true, false, false, false, false]);
    expect(events().some((event) => event.kind === "state" && event.state?.failure === english("native.browser.remotePageLimit"))).toBe(false);
  });

  // A display placed later in the same sync is not the one closed, though its
  // page was shown longest ago: the choice reads the sync's own placements.
  it("never close for a placement a page the same sync places", async () => {
    const { sync, display, views, settle } = screen();
    const ids = ["a", "b", "c", "d", "e"];
    const retained = ids.map((id) => ({ workspace, id, area_id: "a1", url: `https://${id}.example/`, load: 3 }));
    for (const id of ["a", "b", "c", "d"]) {
      sync([display(id)], retained);
      await settle();
    }
    sync([display("e"), display("a")], retained);
    await settle();
    expect(views).toHaveLength(5);
    expect(views.map((view) => view.contents.close.mock.calls.length > 0)).toEqual([false, true, false, false, false]);
  });

  it("make at most eight in ten seconds, and the ninth once the window has room", async () => {
    const { sync, display, views, settle } = screen();
    sync(["a", "b", "c", "d"].map((id) => display(id)));
    await settle();
    sync(["e", "f", "g", "h"].map((id) => display(id)));
    await settle();
    sync([display("i")]);
    await settle();
    expect(views).toHaveLength(8);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(views).toHaveLength(9);
  });

  it("apply only the latest of a burst of syncs", async () => {
    const { sync, display, views, settle } = screen();
    sync([display("d0")]);
    await settle();
    for (let n = 1; n <= 1000; n++) sync([display(`d${n}`)]);
    await settle();
    // The first of the burst lands at once; of the rest, only the latest.
    expect(views.map((view) => view.contents.loadURL.mock.calls[0]?.[0])).toEqual(["https://d0.example/", "https://d1.example/", "https://d1000.example/"]);
  });

  it("are made by no sync of a hidden screen, which is applied once it shows", async () => {
    const { subject, remote, state, sync, display, views, settle } = screen();
    state.area = null;
    sync([display("d1")]);
    await settle();
    expect(views).toHaveLength(0);
    state.area = { bounds: TILE, zoom: 1 };
    subject.remoteShown(remote);
    await settle();
    expect(views).toHaveLength(1);
  });

  it("wait for their storage's clear, and come back empty once it ends", async () => {
    const { subject, state, sync, display, views, events, settle } = screen();
    state.storage = "clearing";
    sync([display("d1")]);
    await settle();
    expect(views).toHaveLength(0);
    expect(events().find((event) => event.id === "d1")?.state?.failure).toBe(english("native.browser.remoteStorageClearing"));
    state.storage = "live";
    subject.remoteRetry(DEVICE, KEY);
    await settle();
    expect(views).toHaveLength(1);
  });

  it("draw no still of a page placed behind a still, and one at a time", async () => {
    const { subject, remote, sync, display, views, settle } = screen();
    sync([display("d1", false)]);
    await settle();
    expect(await subject.remoteCapture(remote, { workspace, id: "d1" })).toBeNull();
    sync([display("d1", true)]);
    await settle();
    const first = subject.remoteCapture(remote, { workspace, id: "d1" });
    expect(await subject.remoteCapture(remote, { workspace, id: "d1" })).toBeNull();
    expect(await first).toBe("data:still");
    expect(views[0]!.contents.capturePage).toHaveBeenCalledOnce();
  });

  it("download nothing, and say so once", async () => {
    const { sync, display, views, sessions, log, settle } = screen();
    sync([display("d1")]);
    await settle();
    const willDownload = sessions.get(PARTITION)!.handlers.get("will-download")!;
    const first = { preventDefault: vi.fn() };
    const second = { preventDefault: vi.fn() };
    willDownload(first, {}, views[0]!.contents);
    willDownload(second, {}, views[0]!.contents);
    expect(first.preventDefault).toHaveBeenCalled();
    expect(second.preventDefault).toHaveBeenCalled();
    expect(log.event.mock.calls.filter(([name]) => name === "browser.download_refused")).toEqual([["browser.download_refused", { reason: "remote_screen", display_id: "d1" }]]);
  });

  it("reach that computer's loopback and network only from a document there, and nothing before the proxy or of this computer's files", async () => {
    const { sync, display, views, sessions, state, contentsOf, settle } = screen();
    sync([display("d1")]);
    await settle();
    const page = views[0]!.contents;
    const ask = (url: string, resourceType: string, top: string | null, id = page.id) => {
      native.fromId = () => (top === null ? null : contentsOf(id, top));
      let answer: { cancel: boolean } | undefined;
      sessions.get(PARTITION)!.beforeRequest({ url, resourceType, webContentsId: id }, (given) => { answer = given; });
      return answer?.cancel;
    };
    expect(ask("https://cdn.example/app.js", "script", "https://d1.example/")).toBe(false);
    expect(ask("http://127.0.0.1:5173/api", "xhr", "https://d1.example/")).toBe(true);
    expect(ask("http://192.168.0.4/", "image", "https://d1.example/")).toBe(true);
    expect(ask("http://localhost:5173/", "mainFrame", null)).toBe(false);
    expect(ask("http://localhost:5173/app.js", "script", "http://localhost:5173/")).toBe(false);
    expect(ask("file:///etc/passwd", "mainFrame", null)).toBe(true);
    // A service worker's request has no document: a public page's.
    expect(ask("http://127.0.0.1:5173/", "other", null, 99999)).toBe(true);
    state.ready = false;
    expect(ask("https://cdn.example/app.js", "script", "https://d1.example/")).toBe(true);
  });

  it("keep a page that is not that computer's own off its loopback", async () => {
    const { sync, display, views, sessions, route, settle } = screen();
    route.mockImplementationOnce(async (_workspace, _id, load) => ({ url: "https://d1.example/", source_url: "https://d1.example/", load, own: false }));
    sync([display("d1")]);
    await settle();
    // The unspecified addresses reach the machine that dials them, as its loopback does.
    for (const url of ["http://localhost:3000/", "http://[::]:3000/", "http://[::ffff:0.0.0.0]:3000/", "http://0.0.0.0:3000/"]) {
      let answer: { cancel: boolean } | undefined;
      sessions.get(PARTITION)!.beforeRequest({ url, resourceType: "mainFrame", webContentsId: views[0]!.contents.id }, (given) => { answer = given; });
      expect(answer?.cancel, url).toBe(true);
    }
  });

  it("close with their screen, and keep their screen's place through a clear", async () => {
    const { subject, remote, sync, display, views, settle } = screen();
    sync([display("d1")]);
    await settle();
    subject.closeRemotePages(DEVICE, KEY);
    expect(views[0]!.contents.close).toHaveBeenCalled();
    subject.remoteRetry(DEVICE, KEY);
    await settle();
    expect(views).toHaveLength(2);
    subject.removeRemote(remote);
    expect(views[1]!.contents.close).toHaveBeenCalled();
    sync([display("d1")]);
    await settle();
    expect(views).toHaveLength(2);
  });
});

describe("a remote screen's popups", () => {
  function popups() {
    const harness = screen();
    const made: { options: Record<string, unknown>; contents: EventEmitter }[] = [];
    native.window = (options) => {
      const contents = Object.assign(new EventEmitter(), { id: 900 + made.length, setWebRTCIPHandlingPolicy: vi.fn(), setWindowOpenHandler: vi.fn(), on: EventEmitter.prototype.on });
      const window = Object.assign(new EventEmitter(), { webContents: contents, isVisible: () => true, isDestroyed: () => false, hide: vi.fn(), showInactive: vi.fn() });
      made.push({ options: options as Record<string, unknown>, contents });
      return window;
    };
    const open = async (focused = true) => {
      const page = harness.views[0]!.contents;
      page.isFocused = () => focused;
      const handler = (page.setWindowOpenHandler as ReturnType<typeof vi.fn>).mock.calls[0]![0] as (details: Record<string, unknown>) => { action: string; createWindow?: (options: unknown) => unknown };
      const answer = handler({ url: "https://sso.example/", disposition: "new-window", features: "width=500,height=600", referrer: { url: "" } });
      if (answer.action === "allow") answer.createWindow!({ width: 500, height: 600, webPreferences: {} });
      return answer.action;
    };
    const press = () => harness.views[0]!.contents.emit("input-event", {}, { type: "mouseDown" });
    return { ...harness, made, open, press };
  }

  it("open only right after a press, inside the tile, at most two, and none a second apart", async () => {
    const { sync, display, settle, open, press, made, log } = popups();
    sync([display("d1")]);
    await settle();
    expect(await open()).toBe("deny");
    press();
    expect(await open()).toBe("allow");
    const bounds = made[0]!.options as { x: number; y: number; width: number; height: number };
    const tile = { x: 1000 + TILE.x, y: 500 + TILE.y, width: TILE.width, height: TILE.height };
    expect(bounds.x).toBeGreaterThanOrEqual(tile.x);
    expect(bounds.y).toBeGreaterThanOrEqual(tile.y);
    expect(bounds.x + bounds.width).toBeLessThanOrEqual(tile.x + tile.width);
    expect(bounds.y + bounds.height).toBeLessThanOrEqual(tile.y + tile.height);
    press();
    expect(await open()).toBe("deny");
    await vi.advanceTimersByTimeAsync(1_100);
    press();
    expect(await open()).toBe("allow");
    await vi.advanceTimersByTimeAsync(1_100);
    press();
    expect(await open()).toBe("deny");
    expect(log.event.mock.calls.filter(([name]) => name === "browser.popup_refused").map(([, fields]) => (fields as { reason: string }).reason)).toEqual(["no_press", "no_press", "cap"]);
  });

  it("open none in a tile smaller than a popup, and none from a page without the keyboard", async () => {
    const { sync, display, settle, open, press, state, log } = popups();
    sync([display("d1")]);
    await settle();
    press();
    expect(await open(false)).toBe("deny");
    state.area = { bounds: { x: 0, y: 0, width: 300, height: 600 }, zoom: 1 };
    press();
    expect(await open()).toBe("deny");
    expect(log.event.mock.calls.filter(([name]) => name === "browser.popup_refused").map(([, fields]) => (fields as { reason: string }).reason)).toEqual(["no_press", "no_room"]);
  });

  it("are not moved by their page", async () => {
    const { sync, display, settle, open, press, made } = popups();
    sync([display("d1")]);
    await settle();
    press();
    await open();
    const event = { preventDefault: vi.fn() };
    made[0]!.contents.emit("content-bounds-updated", event, { x: 0, y: 0, width: 10, height: 10 });
    expect(event.preventDefault).toHaveBeenCalled();
  });
});
