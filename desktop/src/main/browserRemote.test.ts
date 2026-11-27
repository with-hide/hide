// The pages another computer's screen places in this window
// (docs/BROWSER_DISPLAYS.md, A page in another computer's screen): kept apart
// from this window's, made only once their proxy is set, held to the
// screen's bounds and tile, and loaded only where that computer's core says.

import { EventEmitter } from "node:events";
import { afterEach, describe, expect, it, vi } from "vitest";
import { initializeInterfaceI18n } from "../../../web/src/i18n/instance";
import { BROWSER_SYNC_CHANNEL, REMOTE_BROWSER_EVENT_CHANNEL } from "../channel";
import { BrowserViews, type RemoteRoute, type RemoteSource } from "./browser";
import { placeIn, remoteFilePartition, remotePagePartition } from "./browserSync";
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
const FILE_PARTITION = remoteFilePartition(DEVICE, KEY, PAIRING);
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
    navigationHistory: { canGoBack: () => false, canGoForward: () => false, clear: vi.fn() }, focus: vi.fn(), stop: vi.fn(),
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
  const state = { ready: false, area: { bounds: TILE, zoom: 1 } as { bounds: typeof TILE; zoom: number } | null, storage: "live" as "live" | "clearing" | "full" | "waiting" | "unrecorded", partition: PARTITION as string | null };
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
    filePartition: () => (state.partition === null ? null : FILE_PARTITION),
    storage: () => state.storage,
    route,
    release: vi.fn(),
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
  // address the screen names for the display now: the display's own until
  // the core took the failure's report, then the failed link, as this
  // window's own Reload does.
  it("reload a page whose link failed at the address its screen names now", async () => {
    const { subject, remote, sync, display, views, route, settle } = screen();
    sync([display("d1")]);
    await settle();
    views[0]!.contents.emit("did-fail-load", {}, -105, "NAME_NOT_RESOLVED", "https://broken.example/", true);
    subject.remoteCommand(remote, { workspace, id: "d1" }, "reload");
    await settle();
    expect(route.mock.calls.at(-1)).toEqual([workspace, "d1", 3]);
    expect(views[0]!.contents.loadURL.mock.calls.at(-1)?.[0]).toBe("https://d1.example/");
    expect(views[0]!.contents.loadURL).toHaveBeenCalledTimes(2);
    views[0]!.contents.emit("did-fail-load", {}, -105, "NAME_NOT_RESOLVED", "https://broken.example/", true);
    sync([display("d1", true, RECT, "https://broken.example/")]);
    await settle();
    await vi.advanceTimersByTimeAsync(1_000);
    subject.remoteCommand(remote, { workspace, id: "d1" }, "reload");
    await settle();
    expect(views[0]!.contents.loadURL.mock.calls.at(-1)?.[0]).toBe("https://broken.example/");
  });

  it("tell an older Hide there to be updated rather than load its file, asking it first", async () => {
    const { sync, display, views, route, events, settle } = screen();
    route.mockResolvedValueOnce({ refused: "remote_file_unavailable" });
    sync([display("f1", true, RECT, "file:///Users/me/report.html")]);
    await settle();
    expect(route).toHaveBeenCalledOnce();
    expect(views[0]!.contents.loadURL).not.toHaveBeenCalled();
    expect(events().filter((event) => event.kind === "state").at(-1)?.state?.failure).toBe(english("native.browser.remoteUpdateThere"));
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
      filePartition: () => remoteFilePartition("pc", KEY, PAIRING),
      storage: () => "live",
      route: async (_workspace, id, load) => ({ url: `https://${id}.example/`, source_url: `https://${id}.example/`, load, own: true }),
      release: vi.fn(),
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

  // A clear of an earlier pairing's storage closes that pairing's pages
  // alone: the machine's pages on its new pairing stay.
  it("make no page while their storage's row could not be written, and say so", async () => {
    const { state, sync, display, views, events, settle } = screen();
    state.storage = "unrecorded";
    sync([display("d1")]);
    await settle();
    expect(views).toHaveLength(0);
    expect(events().filter((event) => event.kind === "state").at(-1)?.state?.failure).toBe(english("native.browser.remoteStorageUnrecorded"));
  });

  it("close for a storage clear only the pages on that partition", async () => {
    const { subject, state, sync, display, views, settle } = screen();
    const retained = ["d1", "d2"].map((id) => ({ workspace, id, area_id: "a1", url: `https://${id}.example/`, load: 3 }));
    sync([display("d1")], retained);
    await settle();
    state.partition = remotePagePartition(DEVICE, KEY, "22".repeat(16));
    sync([display("d2")], retained);
    await settle();
    expect(views).toHaveLength(2);
    subject.closeRemotePartition(PARTITION);
    expect(views[0]!.contents.close).toHaveBeenCalled();
    expect(views[1]!.contents.close).not.toHaveBeenCalled();
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

  // A display past the creation rate waits, and no hidden page closes for
  // it until it can be made.
  it("close no hidden page for a display that must wait for the creation rate", async () => {
    const { sync, display, views, settle } = screen();
    const ids = Array.from({ length: 9 }, (_, n) => `d${n + 1}`);
    const retained = ids.map((id) => ({ workspace, id, area_id: "a1", url: `https://${id}.example/`, load: 3 }));
    for (const id of ids.slice(0, 8)) {
      sync([display(id)], retained);
      await settle();
    }
    expect(views).toHaveLength(8);
    const closed = () => views.filter((view) => view.contents.close.mock.calls.length > 0).length;
    expect(closed()).toBe(4);
    sync([display("d9")], retained);
    await settle();
    expect(views).toHaveLength(8);
    expect(closed()).toBe(4);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(views).toHaveLength(9);
    expect(closed()).toBe(5);
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

// docs/BROWSER_DISPLAYS.md, Its checkout files: a file of that computer's
// checkout loads from the route that computer's daemon made for this
// screen, on its own loopback, which the screen's proxy carries there, in a
// storage apart from the screen's logins; it reaches its own route alone.
describe("a remote screen's file pages", () => {
  const FILE = "file:///Users/me/repo/report.html";
  const ROUTE = "http://127.0.0.1:5000/s3cret/report%2Ehtml";
  const routed = (url = ROUTE, own = true) => async (_workspace: string, _id: string, load: number): Promise<RemoteRoute> => ({ url, source_url: FILE, load, own, root: "/Users/me/repo" });

  it("load from that computer's route, in storage apart from the logins, and reach that route alone", async () => {
    for (const own of [true, false]) {
      const { sync, display, views, route, sessions, contentsOf, settle } = screen();
      // A device's file page of that computer is not that computer's own, and loads from its route all the same.
      route.mockImplementation(routed(ROUTE, own));
      sync([display("f1", true, RECT, FILE)]);
      await settle();
      expect(session.fromPartition).toHaveBeenCalledWith(FILE_PARTITION);
      expect(FILE_PARTITION).not.toBe(PARTITION);
      expect(views[0]!.contents.loadURL).toHaveBeenCalledExactlyOnceWith(ROUTE);
      const page = views[0]!.contents;
      const ask = (url: string, resourceType: string) => {
        native.fromId = () => contentsOf(page.id, ROUTE);
        let answer: { cancel: boolean } | undefined;
        sessions.get(FILE_PARTITION)!.beforeRequest({ url, resourceType, webContentsId: page.id }, (given) => { answer = given; });
        return answer?.cancel;
      };
      expect(ask(ROUTE, "mainFrame"), "own " + own).toBe(false);
      expect(ask("http://127.0.0.1:5000/s3cret/style.css", "stylesheet")).toBe(false);
      expect(ask("http://127.0.0.1:5173/api", "xhr")).toBe(true);
      expect(ask("http://localhost:5000/s3cret/style.css", "stylesheet")).toBe(true);
      expect(ask("https://cdn.example/app.js", "script")).toBe(true);
      expect(ask("http://192.168.0.4/", "image")).toBe(true);
      expect(ask("file:///Users/me/repo/.env", "xhr")).toBe(true);
    }
  });

  // TF1: whatever that computer answers, a file page loads only from its
  // own loopback over plain HTTP.
  it("load nothing from a route that is not that computer's loopback", async () => {
    for (const url of ["file:///etc/passwd", "http://files.example/s3cret/report.html", "http://localhost:5000/s3cret/report.html", "https://127.0.0.1:5000/s3cret/report.html", "http://127.0.0.1/s3cret/report.html", "http://u:p@127.0.0.1:5000/s3cret/report.html", "http://10.0.0.2:5000/s3cret/report.html"]) {
      const { sync, display, views, route, events, settle } = screen();
      route.mockImplementation(routed(url));
      sync([display("f1", true, RECT, FILE)]);
      await settle();
      expect(views[0]!.contents.loadURL, url).not.toHaveBeenCalled();
      expect(events().filter((event) => event.kind === "state").at(-1)?.state?.failure).toBe(english("native.browser.routeUnavailable"));
    }
  });

  // Its route ends with the screen's view (a hidden tile), so Reload asks for
  // a new one rather than reloading the ended one; a page that closes lets
  // its route go.
  it("ask for a new route on Reload, and let theirs go when they close", async () => {
    const { subject, remote, sync, display, views, route, settle } = screen();
    route.mockImplementation(routed());
    sync([display("f1", true, RECT, FILE)]);
    await settle();
    route.mockImplementation(routed("http://127.0.0.1:5001/n3w/report%2Ehtml"));
    subject.remoteCommand(remote, { workspace, id: "f1" }, "reload");
    await settle();
    expect(route).toHaveBeenCalledTimes(2);
    expect(views[0]!.contents.loadURL.mock.calls.at(-1)?.[0]).toBe("http://127.0.0.1:5001/n3w/report%2Ehtml");
    sync([]);
    await settle();
    expect(views[0]!.contents.close).toHaveBeenCalled();
    expect(remote.release).toHaveBeenCalledExactlyOnceWith(workspace, "f1", 3);
  });

  // TF3 holds whoever asks: a request of a file page's session that no page
  // of it owns (a worker's, an unknown contents') reaches nothing.
  it("refuse every request of their storage that no page of it owns", async () => {
    const { sync, display, route, sessions, settle } = screen();
    route.mockImplementation(routed());
    sync([display("f1", true, RECT, FILE)]);
    await settle();
    const ask = (url: string, webContentsId?: number) => {
      native.fromId = () => null;
      let answer: { cancel: boolean } | undefined;
      sessions.get(FILE_PARTITION)!.beforeRequest({ url, resourceType: "xhr", ...(webContentsId === undefined ? {} : { webContentsId }) }, (given) => { answer = given; });
      return answer?.cancel;
    };
    for (const url of ["https://public.example/collect", ROUTE, "http://127.0.0.1:5000/s3cret/style.css"]) {
      expect(ask(url), url).toBe(true);
      expect(ask(url, 99999), url).toBe(true);
    }
  });

  // That computer's file address keeps its own system's spelling: a Windows
  // checkout's page moved to a fragment or a query is reported under its
  // drive, whatever system this one runs.
  it("report a moved page under that computer's own file address", async () => {
    const { sync, display, views, route, events, settle } = screen();
    const windows = "file:///C:/repo/report.html";
    route.mockImplementation(async (_workspace, _id, load) => ({ url: ROUTE, source_url: windows, load, own: true, root: "C:/repo" }));
    sync([display("f1", true, RECT, windows)]);
    await settle();
    const page = views[0]!.contents;
    page.emit("did-navigate-in-page", {}, `${ROUTE}#section`, true);
    await settle();
    expect(events().filter((event) => event.kind === "state").at(-1)?.state).toMatchObject({ url: "file:///C:/repo/report.html#section" });
    page.emit("did-navigate-in-page", {}, "http://127.0.0.1:5000/s3cret/a%20b/100%25.html?v=2", true);
    await settle();
    expect(events().filter((event) => event.kind === "state").at(-1)?.state).toMatchObject({ url: "file:///C:/repo/a%20b/100%25.html?v=2" });
  });

  it("report a moved page of a Windows share under the share's own address", async () => {
    const { sync, display, views, route, events, settle } = screen();
    const share = "file:////server/share/repo/report.html";
    route.mockImplementation(async (_workspace, _id, load) => ({ url: ROUTE, source_url: share, load, own: true, root: "//server/share/repo" }));
    sync([display("f1", true, RECT, share)]);
    await settle();
    views[0]!.contents.emit("did-navigate-in-page", {}, `${ROUTE}#section`, true);
    await settle();
    expect(events().filter((event) => event.kind === "state").at(-1)?.state).toMatchObject({ url: "file:////server/share/repo/report.html#section" });
  });

  // A route answered after its page closed is let go by that answer, unless
  // a page made again for the same display at the same load took it.
  it("let go of a route answered after its page closed, and keep one a page made again holds", async () => {
    const { remote, sync, display, route, settle } = screen();
    let answer: (route: RemoteRoute) => void = () => undefined;
    route.mockImplementationOnce(() => new Promise((resolve) => { answer = resolve; }));
    sync([display("f1", true, RECT, FILE)]);
    await settle();
    sync([]);
    await settle();
    answer({ url: ROUTE, source_url: FILE, load: 3, own: true });
    await settle();
    expect(remote.release).toHaveBeenCalledExactlyOnceWith(workspace, "f1", 3);

    vi.mocked(remote.release).mockClear();
    route.mockImplementationOnce(() => new Promise((resolve) => { answer = resolve; }));
    sync([display("f2", true, RECT, FILE)]);
    await settle();
    sync([]);
    await settle();
    route.mockImplementation(routed());
    sync([display("f2", true, RECT, FILE)]);
    await settle();
    answer({ url: ROUTE, source_url: FILE, load: 3, own: true });
    await settle();
    expect(remote.release).not.toHaveBeenCalled();
  });

  // Reload asks for the address the screen names for the display now: a
  // page that moved to an anchor reloads there, whether the core's answer
  // came after its sync or before it, and a display that moved to another
  // file whose first route was refused reloads the new file.
  it("reload at the address their display holds now, not their first route's", async () => {
    const { subject, remote, sync, display, views, route, settle } = screen();
    let core = FILE;
    let port = 5000;
    route.mockImplementation(async (_workspace, _id, load) => ({ url: `http://127.0.0.1:${port}/s3cret/report%2Ehtml${new URL(core).hash}`, source_url: core, load, own: true, root: "/Users/me/repo" }));
    sync([display("f1", true, RECT, FILE)]);
    await settle();
    const page = views[0]!.contents;
    page.emit("did-navigate-in-page", {}, `${ROUTE}#results`, true);
    core = `${FILE}#results`;
    port = 5001;
    subject.remoteCommand(remote, { workspace, id: "f1" }, "reload");
    await settle();
    expect(page.loadURL.mock.calls.at(-1)?.[0]).toBe("http://127.0.0.1:5001/s3cret/report%2Ehtml#results");
    sync([display("f1", true, RECT, core)]);
    await settle();
    port = 5002;
    await vi.advanceTimersByTimeAsync(1_000);
    subject.remoteCommand(remote, { workspace, id: "f1" }, "reload");
    await settle();
    expect(page.loadURL.mock.calls.at(-1)?.[0]).toBe("http://127.0.0.1:5002/s3cret/report%2Ehtml#results");

    const other = "file:///Users/me/repo/other.html";
    route.mockResolvedValueOnce({ refused: "not_connected" });
    sync([{ ...display("f1", true, RECT, other), load: 4 }]);
    await settle();
    core = other;
    port = 5003;
    await vi.advanceTimersByTimeAsync(1_000);
    subject.remoteCommand(remote, { workspace, id: "f1" }, "reload");
    await settle();
    expect(route.mock.calls.at(-1)).toEqual([workspace, "f1", 4]);
    expect(page.loadURL.mock.calls.at(-1)?.[0]).toBe("http://127.0.0.1:5003/s3cret/report%2Ehtml");
  });

  // An earlier route ends with the screen's view: once a new route's load
  // commits, Back no longer reaches the ended one; whatever address the page
  // lands on that is not its route's (an old route, another origin, a path
  // outside the route), it reports the file it showed, never a web address.
  // A web page of the screen keeps its history through a new load.
  it("leave no ended route in their history and report a file address alone", async () => {
    const { subject, remote, sync, display, views, route, events, settle } = screen();
    route.mockImplementation(routed());
    sync([display("f1", true, RECT, FILE)]);
    await settle();
    const page = views[0]!.contents;
    page.loadURL.mockResolvedValue(undefined);
    route.mockImplementation(routed("http://127.0.0.1:5001/n3w/report%2Ehtml"));
    subject.remoteCommand(remote, { workspace, id: "f1" }, "reload");
    await settle();
    expect((page.navigationHistory as { clear: ReturnType<typeof vi.fn> }).clear).toHaveBeenCalledOnce();
    const reported = () => events().filter((event) => event.kind === "state").at(-1)?.state as { url: string } | undefined;
    page.emit("did-navigate", {}, ROUTE);
    await settle();
    expect(reported()?.url).toBe(FILE);
    page.emit("did-fail-load", {}, -20, "BLOCKED_BY_CLIENT", ROUTE, true);
    await settle();
    expect(reported()?.url).toBe(FILE);
    page.emit("did-navigate-in-page", {}, "http://127.0.0.1:5001/x", true);
    await settle();
    expect(reported()?.url).toBe(FILE);

    route.mockImplementation(async (_workspace, _id, load) => ({ url: "https://d1.example/", source_url: "https://d1.example/", load, own: true }));
    sync([display("d1")]);
    await settle();
    const web = views.at(-1)!.contents;
    web.loadURL.mockResolvedValue(undefined);
    sync([{ ...display("d1"), load: 4 }]);
    await settle();
    expect(web.loadURL).toHaveBeenCalledTimes(2);
    expect((web.navigationHistory as { clear: ReturnType<typeof vi.fn> }).clear).not.toHaveBeenCalled();
  });

  it("say so when the screen holds as many routes as it may", async () => {
    const { sync, display, route, events, settle } = screen();
    route.mockResolvedValueOnce({ refused: "browser_route_limit" });
    sync([display("f1", true, RECT, FILE)]);
    await settle();
    expect(events().filter((event) => event.kind === "state").at(-1)?.state?.failure).toBe(english("native.browser.remotePageLimit"));
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
