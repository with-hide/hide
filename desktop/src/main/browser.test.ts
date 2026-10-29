import { EventEmitter } from "node:events";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { initializeInterfaceI18n } from "../../../web/src/i18n/instance";
import { REGISTRY, type Command } from "../../../web/src/shortcuts";
import { BROWSER_CYCLE_END_CHANNEL, BROWSER_EVENT_CHANNEL } from "../channel";
import { BrowserViews, type ResolvedPage } from "./browser";
import { browserPartition } from "./browserSync";
import { session } from "electron";
import { ExternalFiles, UNNAMED_GRANT_GRACE_MS, externalPartition } from "./externalFiles";
import type { HostLog } from "./log";

const english = initializeInterfaceI18n("en").getFixedT(null, "translation");
const ipc = vi.hoisted(() => new Map<string, (event: unknown, value?: unknown) => void>());
vi.mock("electron", () => ({
  ipcMain: { on: (channel: string, listener: (event: unknown, value?: unknown) => void) => ipc.set(channel, listener), handle: vi.fn() },
  Menu: { getApplicationMenu: () => null },
  BrowserWindow: {}, WebContentsView: {}, session: {}, shell: {},
}));

describe("native browser load generations", () => {
  it.each(["canonical", "superseded", "partition"] as const)("handles a delayed %s route after its loading report", async (scenario) => {
    vi.useFakeTimers();
    try {
      let answer!: (route: ResolvedPage) => void;
      const resolve = vi.fn(() => new Promise<ResolvedPage>((done) => { answer = done; }));
      const subject = new BrowserViews({ event: vi.fn() } as unknown as HostLog, () => true, resolve, vi.fn(), vi.fn(), () => english, new ExternalFiles(vi.fn()));
      const workspace = "local\u0000/checkout";
      const requested = "file:///var/checkout/manual.html";
      const canonical = "file:///private/var/checkout/manual.html";
      const loadURL = vi.fn().mockResolvedValue(undefined);
      const page = {
        key: "manual", workspace, id: "manual", applied: 1,
        partition: browserPartition(workspace, requested, null), route: null,
        state: { url: requested, loading: true, failure: null }, report: null, settled: new Set(),
        view: { webContents: { loadURL } },
      };
      (Reflect.get(subject, "pages") as Map<string, unknown>).set(page.key, page);
      Reflect.get(subject, "load").call(subject, page, requested);
      await vi.advanceTimersByTimeAsync(100);
      if (scenario === "superseded") page.applied = 2;
      const source = scenario === "partition" ? "https://example.test/" : canonical;
      answer({ url: source, source_url: source, load: 1 });
      await vi.advanceTimersByTimeAsync(0);
      if (scenario === "canonical") {
        expect(loadURL).toHaveBeenCalledExactlyOnceWith(canonical);
        expect(page.route).toEqual({ url: canonical, source_url: canonical, load: 1 });
      } else {
        expect(loadURL).not.toHaveBeenCalled();
        if (scenario === "partition") expect(page.state).toMatchObject({ loading: false, failure: expect.any(String) });
      }
    } finally {
      vi.clearAllTimers();
      vi.useRealTimers();
    }
  });
});

// A first page that stayed loading on Windows CI left no line saying which
// stage it stopped at (with-hide/hide-private#221): the route, the commit or the stop.
it("logs each stage of a load the core asked for with its display, load and time, and nothing for one the page began", async () => {
  let now = 1_000;
  let answer!: (route: ResolvedPage) => void;
  const log = { event: vi.fn() };
  const subject = new BrowserViews(log as unknown as HostLog, () => true, vi.fn(() => new Promise<ResolvedPage>((done) => { answer = done; })), vi.fn(), vi.fn(), () => english, new ExternalFiles(vi.fn()), { bound: 600_000, every: 60_000, now: () => now });
  const url = "https://example.test/";
  const contents = Object.assign(new EventEmitter(), {
    loadURL: vi.fn().mockResolvedValue(undefined), setWindowOpenHandler: vi.fn(), getZoomFactor: () => 1, isDestroyed: () => false,
    setVisualZoomLevelLimits: vi.fn().mockResolvedValue(undefined), navigationHistory: { canGoBack: () => false, canGoForward: () => false },
  });
  const page = {
    key: "d1", workspace: "local\u0000/checkout", id: "d1", applied: 3, partition: browserPartition("local\u0000/checkout", url, "local"), route: null, loading: null,
    state: { url, loading: true, failure: null }, report: null, settled: new Set(), device: null, bounds: null, emulated: null, view: { webContents: contents },
  };
  (Reflect.get(subject, "pages") as Map<string, unknown>).set(page.key, page);
  Reflect.set(subject, "node", "local");
  Reflect.get(subject, "watch").call(subject, page);
  const stages = () => log.event.mock.calls.filter(([name]) => String(name).startsWith("browser.load_")).map(([name, fields]) => [name, fields]);

  Reflect.get(subject, "load").call(subject, page, url);
  now += 40;
  answer({ url, source_url: url, load: 3 });
  await vi.waitFor(() => expect(contents.loadURL).toHaveBeenCalledOnce());
  now += 200;
  contents.emit("did-navigate", {}, url);
  contents.emit("did-navigate", {}, `${url}next`);
  now += 60;
  contents.emit("did-stop-loading");
  // A navigation the page made itself is not a load the core asked for.
  contents.emit("did-start-loading");
  contents.emit("did-navigate", {}, `${url}clicked`);
  contents.emit("did-stop-loading");
  expect(stages()).toEqual([
    ["browser.load_routed", { id: "d1", load: 3, ms: 40 }],
    ["browser.load_committed", { id: "d1", load: 3, ms: 240 }],
    ["browser.load_stopped", { id: "d1", load: 3, ms: 300 }],
  ]);
});

describe("native browser CDP inventory", () => {
  it("stops notifying a CDP owner that leaves while it is told a page became ineligible", () => {
    const subject = new BrowserViews({ event: vi.fn() } as unknown as HostLog, () => true, vi.fn(), vi.fn(), vi.fn(), () => english, new ExternalFiles(vi.fn()));
    const told: unknown[] = [];
    // The gateway closes its session on the first retirement, and with it its observation.
    const stop = subject.observeCdp((retirement) => { told.push(retirement); stop(); });
    // A page the inventory reported, still alive but no longer eligible for CDP.
    const page = { key: "fixture\u0000manual", workspace: "fixture", id: "manual", view: { webContents: { id: 7, isDestroyed: () => false, getURL: () => "file:///manual.html" } }, state: { url: "file:///manual.html" } };
    (Reflect.get(subject, "pages") as Map<string, unknown>).set("manual", page);
    Reflect.set(subject, "cdpObserved", [{ key: page.key, area: "a1", contents: 7, url: "file:///manual.html", title: "" }]);
    expect(() => Reflect.get(subject, "publishCdp").call(subject)).not.toThrow();
    expect(told).toEqual([{ contentsId: 7, reason: "replaced" }]);
  });
});

describe("outside file page sessions", () => {
  it("clears a released slot's storage and cache before handing it out again, and configures its session once (PRD outside-path-in-hide B4)", async () => {
    let now = 0;
    const externals = new ExternalFiles(vi.fn(), () => now);
    const subject = new BrowserViews({ event: vi.fn() } as unknown as HostLog, () => true, vi.fn(), vi.fn(), vi.fn(), () => english, externals);
    let cleared!: () => void;
    const pageSession = { clearStorageData: vi.fn(() => new Promise<void>((done) => { cleared = done; })), clearCache: vi.fn(() => Promise.resolve()) };
    Object.assign(session, { fromPartition: vi.fn(() => pageSession) });
    const first = externals.grant("/outside/a/index.html", "html")!;
    (Reflect.get(subject, "configuredSessions") as Set<string>).add(externalPartition(0));
    now += UNNAMED_GRANT_GRACE_MS;
    Reflect.get(subject, "keepGrants").call(subject, new Set());
    expect(pageSession.clearStorageData).toHaveBeenCalledOnce();
    expect(pageSession.clearCache).toHaveBeenCalledOnce();
    // Its handlers stay, so the next page in the slot adds none.
    expect((Reflect.get(subject, "configuredSessions") as Set<string>).has(externalPartition(0))).toBe(true);
    // Until the clear lands the slot is not handed out again.
    const second = externals.grant("/outside/b/index.html", "html")!;
    expect(externals.partition(second.id)).toBe(externalPartition(1));
    cleared();
    await new Promise((done) => setImmediate(done));
    expect(externals.partition(externals.grant("/outside/c/index.html", "html")!.id)).toBe(externalPartition(0));
    expect(externals.partition(first.id)).toBeNull();
  });
});

function candidate() {
  const send = vi.fn();
  const shellFocus = vi.fn();
  let windowFocused = true;
  const subject = new BrowserViews({ event: vi.fn() } as unknown as HostLog, () => true, vi.fn(), vi.fn(), vi.fn(), () => english, new ExternalFiles(vi.fn()));
  const shellContents = Object.assign(new EventEmitter(), { isDestroyed: () => false, send, focus: shellFocus });
  const window = Object.assign(new EventEmitter(), { isFocused: () => windowFocused, webContents: shellContents });
  subject.attach(window as never);
  const input = (contents: EventEmitter, type = "keyDown", key = "Tab", control = true, alt = false) => {
    const event = { preventDefault: vi.fn() };
    contents.emit("before-input-event", event, { type, key, code: key === "Control" ? "ControlLeft" : key, control, alt, meta: false, shift: false, isComposing: false });
    return event.preventDefault;
  };
  const page = (id: string) => {
    let focused = true;
    const pageFocus = vi.fn();
    const contents = Object.assign(new EventEmitter(), { isFocused: () => focused, setWindowOpenHandler: vi.fn(), focus: pageFocus });
    const value = { key: id, workspace: "fixture", id, visible: true, view: { webContents: contents, setVisible: vi.fn() }, state: {}, report: null, route: null, applied: 1, partition: "fixture", shownAt: 0 };
    const watch = Reflect.get(subject, "watch") as (page: unknown) => void;
    watch.call(subject, value);
    (Reflect.get(subject, "pages") as Map<string, unknown>).set(id, value);
    return {
      pageFocus,
      takeFocus: () => contents.emit("focus"),
      focus: (next: boolean) => { focused = next; },
      show: (visible: boolean) => Reflect.get(subject, "show").call(subject, value, visible),
      visibility: value.view.setVisible,
      input: (type?: string, key?: string, control?: boolean, alt?: boolean) => input(contents, type, key, control, alt),
    };
  };
  const forwarded = () => send.mock.calls.filter(([channel]) => channel === BROWSER_EVENT_CHANNEL).map(([, value]) => value as { kind: string; id: string; cycleId: number; key: string });
  return { page, forwarded, shellFocus, shellInput: (type?: string, key?: string, control?: boolean, alt?: boolean) => input(shellContents, type, key, control, alt), registry: (rows: readonly Command[]) => subject.setRegistry(rows), blur: () => window.emit("blur"), windowReturn: () => window.emit("focus"), windowFocus: (focused: boolean) => { windowFocused = focused; } };
}

describe("native held cycle delivery", () => {
  beforeEach(() => ipc.clear());
  it("freezing a held native page gives its remaining input to the same focused shell", () => {
    const { page, shellFocus, windowFocus } = candidate();
    const origin = page("origin"), other = page("other");
    other.focus(false);
    other.show(false);
    expect(shellFocus).not.toHaveBeenCalled();
    origin.input();
    expect(shellFocus).toHaveBeenCalledOnce();
    origin.show(false);
    expect(shellFocus).toHaveBeenCalledTimes(2);
    expect(shellFocus.mock.invocationCallOrder[1]).toBeGreaterThan(origin.visibility.mock.invocationCallOrder[0]!);
    origin.show(false);
    expect(shellFocus).toHaveBeenCalledTimes(2);
    origin.show(true);
    windowFocus(false);
    origin.show(false);
    expect(shellFocus).toHaveBeenCalledTimes(2);
  });
  it("a page hidden while it holds the keyboard hands it to the shell, whose layer over the page hid it", () => {
    const { page, shellFocus, windowFocus } = candidate();
    const quiet = page("quiet"), typing = page("typing");
    quiet.focus(false);
    quiet.show(false);
    expect(shellFocus).not.toHaveBeenCalled();
    typing.show(false);
    expect(shellFocus).toHaveBeenCalledOnce();
    expect(shellFocus.mock.invocationCallOrder[0]).toBeGreaterThan(typing.visibility.mock.invocationCallOrder[0]!);
    // A window in the background takes no keyboard at all.
    typing.show(true);
    windowFocus(false);
    typing.show(false);
    expect(shellFocus).toHaveBeenCalledOnce();
  });
  it("hands off a still-visible origin once and orders a quick shell release after the undelivered start", () => {
    const { page, shellFocus, shellInput, forwarded } = candidate();
    const origin = page("uncovered");
    expect(shellInput()).not.toHaveBeenCalled();
    expect(origin.input()).toHaveBeenCalledOnce();
    expect(shellFocus).toHaveBeenCalledOnce();
    expect(origin.visibility).not.toHaveBeenCalled();
    // No renderer has received the queued start yet. Native shell interception
    // still consumes the release and appends it on the same bridge, once.
    expect(shellInput("keyUp", "Control", false)).toHaveBeenCalledOnce();
    const queued = forwarded();
    expect(queued).toEqual([
      expect.objectContaining({ kind: "cycle-input", id: "uncovered", key: "Tab" }),
      expect.objectContaining({ kind: "cycle-input", id: "uncovered", key: "Control", cycleId: queued[0]!.cycleId }),
    ]);
    expect(shellInput("keyUp", "Control", false)).not.toHaveBeenCalled();
    expect(forwarded()).toHaveLength(2);
    expect(shellFocus).toHaveBeenCalledOnce();
  });
  it("routes rebound held repeats and Escape once without handing off again", () => {
    const { page, registry, shellInput, shellFocus, forwarded } = candidate();
    registry(REGISTRY.map((row) => row.id === "recent_area_tab" ? { ...row, electron: { code: "Tab", ctrl: true, alt: true } } : row));
    expect(page("origin").input("keyDown", "Tab", true, true)).toHaveBeenCalledOnce();
    expect(shellInput("keyDown", "Tab", true, true)).toHaveBeenCalledOnce();
    expect(shellInput("keyDown", "Escape", true, true)).toHaveBeenCalledOnce();
    expect(forwarded().map((row) => [row.id, row.key, row.cycleId])).toEqual([
      ["origin", "Tab", 1], ["origin", "Tab", 1], ["origin", "Escape", 1],
    ]);
    expect(shellFocus).toHaveBeenCalledOnce();
    expect(shellInput("keyUp", "Control", false, true)).not.toHaveBeenCalled();
  });
  it("never focuses a background window and cancels its held route on window blur", () => {
    const { page, shellFocus, shellInput, forwarded, windowFocus, blur } = candidate();
    windowFocus(false);
    page("origin").input();
    expect(shellFocus).not.toHaveBeenCalled();
    blur();
    expect(forwarded()).toEqual([
      expect.objectContaining({ kind: "cycle-input", cycleId: 1 }),
      expect.objectContaining({ kind: "window-focus", focused: false }),
      expect.objectContaining({ kind: "cycle-cancel", id: "origin", cycleId: 1 }),
    ]);
    expect(shellInput("keyUp", "Control", false)).not.toHaveBeenCalled();
  });
  it("reports window activation once per transition without borrowing native page focus", () => {
    const { page, forwarded, blur, windowReturn } = candidate();
    const native = page("native");
    native.takeFocus();
    blur();
    blur();
    windowReturn();
    windowReturn();
    expect(forwarded().filter((event) => event.kind === "window-focus")).toEqual([
      { kind: "window-focus", focused: false },
      { kind: "window-focus", focused: true },
    ]);
    expect(native.pageFocus).not.toHaveBeenCalled();
  });
  it("gives the keyboard back to a blur-cancelled origin once when the window returns", () => {
    const { page, blur, windowReturn } = candidate();
    const origin = page("origin"), hidden = page("hidden");
    origin.input();
    blur();
    windowReturn();
    expect(origin.pageFocus).toHaveBeenCalledOnce();
    windowReturn();
    expect(origin.pageFocus).toHaveBeenCalledOnce();
    // An origin hidden while the window was away stays unfocused until the shell shows it.
    hidden.input();
    blur();
    hidden.show(false);
    windowReturn();
    expect(hidden.pageFocus).not.toHaveBeenCalled();
    hidden.show(true);
    expect(hidden.pageFocus).toHaveBeenCalledOnce();
  });
  it("owes a blur-cancelled origin the keyboard until the shell shows it again, once", () => {
    const { page, blur, windowReturn } = candidate();
    const origin = page("origin");
    // The overlay covered the origin, so the window comes back before the
    // shell's sync has shown it again.
    origin.input();
    origin.show(false);
    blur();
    windowReturn();
    expect(origin.pageFocus).not.toHaveBeenCalled();
    origin.show(true);
    expect(origin.pageFocus).toHaveBeenCalledOnce();
    origin.show(false);
    origin.show(true);
    expect(origin.pageFocus).toHaveBeenCalledOnce();
  });
  it("pays the debt only after the window's own return, which Electron's focus restore precedes", () => {
    const { page, blur, windowReturn, windowFocus } = candidate();
    const origin = page("origin");
    // Key again but not back yet: the window's return restores the focus
    // Electron stored on blur, so a page paid before it would lose the keyboard.
    origin.input();
    origin.show(false);
    blur();
    origin.show(true);
    expect(origin.pageFocus).not.toHaveBeenCalled();
    windowReturn();
    expect(origin.pageFocus).toHaveBeenCalledOnce();
    // Back while the page is still covered, and shown before the window is key.
    const late = page("late");
    late.input();
    late.show(false);
    blur();
    windowFocus(false);
    windowReturn();
    late.show(true);
    expect(late.pageFocus).toHaveBeenCalledOnce();
  });
  it("pays the debt only to its page, and only once the window is back", () => {
    const { page, blur, windowReturn, windowFocus } = candidate();
    const origin = page("origin"), other = page("other");
    origin.input();
    origin.show(false);
    blur();
    windowFocus(false);
    origin.show(true);
    expect(origin.pageFocus).not.toHaveBeenCalled();
    windowFocus(true);
    other.show(false);
    other.show(true);
    expect(origin.pageFocus).not.toHaveBeenCalled();
    windowReturn();
    expect(origin.pageFocus).toHaveBeenCalledOnce();
    expect(other.pageFocus).not.toHaveBeenCalled();
  });
  it("forgets the debt when another page takes the keyboard or a new hold starts", () => {
    const { page, blur, windowReturn } = candidate();
    const origin = page("origin"), other = page("other");
    origin.input();
    origin.show(false);
    blur();
    other.takeFocus();
    origin.show(true);
    windowReturn();
    expect(origin.pageFocus).not.toHaveBeenCalled();
    origin.input();
    blur();
    origin.show(false);
    other.input();
    origin.show(true);
    windowReturn();
    expect(origin.pageFocus).not.toHaveBeenCalled();
  });
  it("release and Escape from another page reach the frozen origin once", () => {
    for (const key of ["Control", "Escape"]) {
      const { page, forwarded } = candidate();
      const origin = page("origin"), other = page("other");
      expect(origin.input()).toHaveBeenCalledOnce();
      const started = forwarded()[0]!;
      origin.focus(false);
      other.focus(true);
      expect(other.input(key === "Control" ? "keyUp" : "keyDown", key, key !== "Control")).toHaveBeenCalledOnce();
      expect(forwarded()).toEqual([started, expect.objectContaining({ kind: "cycle-input", id: "origin", cycleId: started.cycleId, key })]);
      expect(other.input("keyUp", "Control", false)).not.toHaveBeenCalled();
      expect(forwarded()).toHaveLength(2);
    }
  });
  it("a delayed old end cannot erase a new held cycle", () => {
    const { page, forwarded } = candidate();
    const origin = page("origin");
    origin.input();
    const oldId = forwarded()[0]!.cycleId;
    origin.input("keyUp", "Control", false);
    origin.input();
    const currentId = forwarded()[2]!.cycleId;
    expect(currentId).not.toBe(oldId);
    const end = ipc.get(BROWSER_CYCLE_END_CHANNEL)!;
    end({}, oldId);
    expect(origin.input("keyUp", "Control", false)).toHaveBeenCalledOnce();
    expect(forwarded()[3]).toMatchObject({ cycleId: currentId, key: "Control" });
    origin.input();
    const newestId = forwarded()[4]!.cycleId;
    end({}, newestId);
    end({}, newestId);
    expect(origin.input("keyUp", "Control", false)).not.toHaveBeenCalled();
    expect(forwarded()).toHaveLength(5);
  });
});

describe("unloading hidden pages (PRD browser-tab-idle-unload)", () => {
  const BOUND = 1000;
  const EVERY = 100;
  const workspace = "local\u0000/checkout";

  function views() {
    vi.useFakeTimers();
    let now = 0;
    const log = { event: vi.fn() };
    const subject = new BrowserViews(log as unknown as HostLog, () => true, vi.fn(), vi.fn(), vi.fn(), () => english, new ExternalFiles(vi.fn()), { bound: BOUND, every: EVERY, now: () => now });
    const send = vi.fn();
    const shellContents = Object.assign(new EventEmitter(), { isDestroyed: () => false, send, focus: vi.fn(), getZoomFactor: () => 1 });
    const window = Object.assign(new EventEmitter(), { isFocused: () => true, isDestroyed: () => false, webContents: shellContents, contentView: { removeChildView: vi.fn(), addChildView: vi.fn() } });
    subject.attach(window as never);
    const pages = Reflect.get(subject, "pages") as Map<string, Record<string, unknown>>;
    let contentsId = 0;
    const page = (id: string, extra: Record<string, unknown> = {}) => {
      const contents = Object.assign(new EventEmitter(), { id: ++contentsId, isFocused: () => false, isDestroyed: () => false, isCurrentlyAudible: () => false, close: vi.fn(), setWindowOpenHandler: vi.fn(), getURL: () => `https://${id}.example/`, getTitle: () => id });
      const value = {
        key: `${workspace}\u0001${id}`, workspace, id, visible: false, placed: false, shownAt: 0, idleSince: 0, playing: false, downloads: 0, settled: new Set(),
        view: { webContents: contents, setVisible: vi.fn(), setBounds: vi.fn() }, state: { url: `https://${id}.example/`, loading: false, failure: null },
        report: null, route: null, applied: 3, partition: "persist:hide-browser-web", fileRestricted: true, device: null, emulated: null, bounds: { x: 0, y: 0, width: 640, height: 480 }, ...extra,
      };
      Reflect.get(subject, "watch").call(subject, value);
      pages.set(value.key, value);
      (Reflect.get(subject, "cdpAreas") as Map<string, string>).set(value.key, "a1");
      return value;
    };
    const gone = () => send.mock.calls.filter(([channel, event]) => channel === BROWSER_EVENT_CHANNEL && (event as { kind: string }).kind === "gone").map(([, event]) => (event as { id: string }).id);
    const unloads = () => log.event.mock.calls.filter(([name]) => name === "browser.unloaded").map(([, fields]) => fields);
    return {
      subject, window, send, log, page, pages, gone, unloads,
      /** Moves the clock and lets the checks that fall due run. */
      pass: async (ms: number) => { for (let at = 0; at < ms; at += EVERY) { now += EVERY; await vi.advanceTimersByTimeAsync(EVERY); } },
    };
  }

  it("closes a hidden idle page past the bound and keeps every page something holds (B1, B3, B5)", async () => {
    try {
      const { subject, page, gone, unloads, pass } = views();
      page("idle");
      page("shown", { visible: true });
      // Drawn in its slot behind an overlay's still, or in a minimized window, it still counts as seen.
      page("placed", { placed: true });
      const playing = page("playing");
      (playing.view.webContents as EventEmitter).emit("media-started-playing");
      // A download its page started, through the session's own handler.
      const downloading = page("downloading");
      const handlers = new Map<string, (...args: unknown[]) => void>();
      Object.assign(session, { fromPartition: vi.fn(() => ({ setPermissionRequestHandler: vi.fn(), setPermissionCheckHandler: vi.fn(), on: (name: string, handler: (...args: unknown[]) => void) => handlers.set(name, handler), webRequest: { onBeforeRequest: vi.fn() } })) });
      Reflect.get(subject, "sessionFor").call(subject, "persist:hide-browser-web");
      const item = Object.assign(new EventEmitter(), { getMimeType: () => "application/zip" });
      handlers.get("will-download")!({ preventDefault: vi.fn() }, item, downloading.view.webContents);
      const driven = page("driven");
      Reflect.get(subject, "cdpLeases").set((driven.view.webContents as { id: number }).id, {});
      const opener = page("opener");
      Reflect.get(subject, "popups").set(99, { window: {}, owner: opener });
      await pass(BOUND - EVERY);
      expect(gone()).toEqual([]);
      await pass(EVERY);
      expect(gone()).toEqual(["idle"]);
      expect(unloads()).toEqual([{ id: "idle", reason: "idle" }]);
      // The download finishing ends its hold, and the bound counts from then (B4).
      item.emit("done");
      await pass(BOUND);
      expect(gone()).toEqual(["idle", "downloading"]);
    } finally {
      vi.useRealTimers();
    }
  });

  it("counts the bound again from when a hold ends (B4)", async () => {
    try {
      const { page, gone, pass } = views();
      const playing = page("playing");
      const contents = playing.view.webContents as EventEmitter;
      contents.emit("media-started-playing");
      await pass(BOUND * 3);
      contents.emit("media-paused");
      await pass(BOUND - EVERY);
      expect(gone()).toEqual([]);
      await pass(EVERY);
      expect(gone()).toEqual(["playing"]);
    } finally {
      vi.useRealTimers();
    }
  });

  it("stops checking when the window closes (B12)", async () => {
    try {
      const { window } = views();
      expect(vi.getTimerCount()).toBe(1);
      window.emit("closed");
      expect(vi.getTimerCount()).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it("loads an unloaded page again hidden for an agent, refuses while its load fails, and loads it again on the next connection (B10, B11)", async () => {
    try {
      const { subject, page, pages, send, log, pass } = views();
      page("mock");
      await pass(BOUND);
      expect(pages.size).toBe(0);
      const created: Record<string, unknown>[] = [];
      let revived!: ReturnType<typeof page>;
      Reflect.set(subject, "create", (_window: unknown, owner: string, display: Record<string, unknown>) => {
        created.push({ owner, ...display });
        revived = page(display.id as string, { state: { url: display.url, loading: true, failure: null } });
        return revived;
      });
      const load = vi.fn();
      Reflect.set(subject, "load", load);
      const update = (change: Record<string, unknown>) => Reflect.get(subject, "update").call(subject, revived, change);
      const scope = { workspace, area_id: "a1", display_id: "mock" };
      send.mockClear();
      const first = subject.cdpRevive(scope);
      expect(created).toEqual([{ owner: workspace, id: "mock", area_id: "a1", url: "https://mock.example/", load: 3, rect: null, visible: false }]);
      expect(revived.view.setVisible).not.toHaveBeenCalled();
      update({ loading: false, failure: "Failed to load (-105)" });
      expect(await first).toBe("failed");
      // The next connection loads the failed page again, in place.
      const second = subject.cdpRevive(scope);
      expect(load).toHaveBeenCalledExactlyOnceWith(revived, "https://mock.example/");
      update({ loading: true, failure: null });
      update({ loading: false });
      expect(await second).toBe("loaded");
      // Loaded, it is just seen: the next check keeps it.
      await pass(EVERY);
      expect(pages.has(revived.key as string)).toBe(true);
      // Loaded and settled, its record is gone: the next connection drives it as it is.
      expect(await subject.cdpRevive(scope)).toBe("none");
      // Nothing moved on the operator's screen.
      expect(send.mock.calls.map(([, event]) => (event as { kind: string }).kind).filter((kind) => kind !== "state")).toEqual([]);
      expect(log.event.mock.calls.filter(([name]) => name === "browser.reloaded_for_agent")).toEqual([["browser.reloaded_for_agent", { id: "mock" }], ["browser.reloaded_for_agent", { id: "mock" }]]);
      // A display not yet shown since the app started stays pending, so its connection keeps the 409 (D-16, B15);
      // one it never unloaded, or an area that does not hold it, has nothing to load either.
      (Reflect.get(subject, "cdpAreas") as Map<string, string>).set(`${workspace}\u0001pending`, "a1");
      expect(await subject.cdpRevive({ ...scope, display_id: "pending" })).toBe("none");
      expect(pages.has(`${workspace}\u0001pending`)).toBe(false);
      expect(await subject.cdpRevive({ ...scope, display_id: "other" })).toBe("none");
      expect(await subject.cdpRevive({ ...scope, area_id: "a2" })).toBe("none");
    } finally {
      vi.useRealTimers();
    }
  });

  it("holds a second connection until the page loads, and loads a page that failed after the wait again (B10, B11)", async () => {
    try {
      const { subject, page, pages, pass } = views();
      page("slow");
      await pass(BOUND);
      let revived!: ReturnType<typeof page>;
      Reflect.set(subject, "create", (_window: unknown, _owner: string, display: Record<string, unknown>) => {
        revived = page(display.id as string, { state: { url: display.url, loading: true, failure: null } });
        return revived;
      });
      const load = vi.fn();
      Reflect.set(subject, "load", load);
      const update = (change: Record<string, unknown>) => Reflect.get(subject, "update").call(subject, revived, change);
      const scope = { workspace, area_id: "a1", display_id: "slow" };
      let first: string | null = null;
      let second: string | null = null;
      void subject.cdpRevive(scope).then((outcome) => { first = outcome; });
      void subject.cdpRevive(scope).then((outcome) => { second = outcome; });
      await vi.advanceTimersByTimeAsync(0);
      expect(pages.size).toBe(1);
      expect([first, second]).toEqual([null, null]);
      // Past the wait both drive the page as it is, still loading.
      await vi.advanceTimersByTimeAsync(5_000);
      expect([first, second]).toEqual(["loaded", "loaded"]);
      // Its load fails later: the next connection loads it again.
      update({ loading: false, failure: "Failed to load (-105)" });
      const retry = subject.cdpRevive(scope);
      expect(load).toHaveBeenCalledExactlyOnceWith(revived, "https://slow.example/");
      update({ loading: true, failure: null });
      update({ loading: false });
      expect(await retry).toBe("loaded");
    } finally {
      vi.useRealTimers();
    }
  });
});
