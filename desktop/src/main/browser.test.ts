import { EventEmitter } from "node:events";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { initializeInterfaceI18n } from "../../../web/src/i18n/instance";
import { REGISTRY, type Command } from "../../../web/src/shortcuts";
import { BROWSER_COMMAND_CHANNEL, BROWSER_CYCLE_END_CHANNEL, BROWSER_EVENT_CHANNEL, BROWSER_SYNC_CHANNEL } from "../channel";
import { BrowserViews, type ResolvedPage } from "./browser";
import { browserPartition, MAX_LIVE_VIEWS, viewKey } from "./browserSync";
import { session } from "electron";
import { ExternalFiles, UNNAMED_GRANT_GRACE_MS, externalPartition } from "./externalFiles";
import type { HostLog } from "./log";

const english = initializeInterfaceI18n("en").getFixedT(null, "translation");
const ipc = vi.hoisted(() => new Map<string, (event: unknown, value?: unknown) => void>());
/** What `new WebContentsView()` returns, for a test that lets the host build its own pages. */
const nativeView = vi.hoisted(() => ({ make: (): unknown => { throw new Error("this test builds no native view"); } }));
vi.mock("electron", () => ({
  ipcMain: { on: (channel: string, listener: (event: unknown, value?: unknown) => void) => ipc.set(channel, listener), handle: vi.fn() },
  Menu: { getApplicationMenu: () => null },
  BrowserWindow: {}, WebContentsView: function WebContentsView() { return nativeView.make(); }, session: {}, shell: {},
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

// A page of one checkout that another checkout's Workspace shows (an agent's
// `hide browser open`, a terminal link) loads the files beside it from the
// checkout that holds it, which its route names, and nothing of the
// Workspace that shows it.
describe("a local file page's own files", () => {
  it("come from the checkout that holds the page, whichever Workspace shows it", async () => {
    vi.useFakeTimers();
    const holder = fs.mkdtempSync(path.join(os.tmpdir(), "hide-holder-"));
    const shower = fs.mkdtempSync(path.join(os.tmpdir(), "hide-shower-"));
    try {
      fs.writeFileSync(path.join(holder, "page.html"), '<link rel="stylesheet" href="style.css">');
      fs.writeFileSync(path.join(holder, "style.css"), "");
      fs.writeFileSync(path.join(shower, "secret.css"), "");
      const address = pathToFileURL(path.join(holder, "page.html")).href;
      const resolve = vi.fn(async (): Promise<ResolvedPage> => ({ url: address, source_url: address, load: 1, root: holder }));
      const subject = new BrowserViews({ event: vi.fn() } as unknown as HostLog, () => true, resolve, vi.fn(), vi.fn(), () => english, new ExternalFiles(vi.fn()));
      const workspace = `local\u0000${shower}`;
      const loadURL = vi.fn().mockResolvedValue(undefined);
      // A root the page had resolved before its route named one is dropped.
      const page = {
        key: "page", workspace, id: "page", applied: 1, partition: browserPartition(workspace, address, null), route: null, checkoutRoot: null,
        state: { url: address, loading: true, failure: null }, report: null, settled: new Set(), view: { webContents: { loadURL } },
      };
      (Reflect.get(subject, "pages") as Map<string, unknown>).set(page.key, page);
      Reflect.get(subject, "load").call(subject, page, address);
      await vi.advanceTimersByTimeAsync(0);
      expect(loadURL).toHaveBeenCalledExactlyOnceWith(address);
      const inside = (file: string) => Reflect.get(subject, "insideCheckout").call(subject, page, file) as boolean;
      expect(inside(path.join(holder, "style.css"))).toBe(true);
      expect(inside(path.join(shower, "secret.css"))).toBe(false);
    } finally {
      vi.clearAllTimers();
      vi.useRealTimers();
      fs.rmSync(holder, { recursive: true, force: true });
      fs.rmSync(shower, { recursive: true, force: true });
    }
  });
});

// A first page that stayed loading on Windows CI left no line saying which
// stage it stopped at (with-hide/hide-private#221): the route, the commit or the stop.
describe("native browser load stages", () => {
  /** A subject whose clock the test moves, whose routes and native loads each wait for the test to settle them. */
  function stages() {
    let now = 1_000;
    const log = { event: vi.fn() };
    const routes: { workspace: string; load: number; answer: (route: ResolvedPage) => void; refuse: (error: Error) => void }[] = [];
    const resolve = vi.fn((workspace: string, _id: string, load: number) => new Promise<ResolvedPage>((answer, refuse) => { routes.push({ workspace, load, answer, refuse }); }));
    const subject = new BrowserViews(log as unknown as HostLog, () => true, resolve, vi.fn(), vi.fn(), () => english, new ExternalFiles(vi.fn()), { bound: 600_000, every: 60_000, now: () => now });
    Reflect.set(subject, "node", "local");
    const url = "https://example.test/";
    let nextContents = 7;
    const page = (workspace: string) => {
      const loads: { url: string; finish: () => void; abort: () => void }[] = [];
      const contents = Object.assign(new EventEmitter(), {
        id: nextContents++,
        loadURL: vi.fn((target: string) => new Promise<void>((finish, fail) => { loads.push({ url: target, finish, abort: () => fail(Object.assign(new Error("aborted"), { errno: -3 })) }); })),
        setWindowOpenHandler: vi.fn(), getZoomFactor: () => 1, isDestroyed: () => false,
        setVisualZoomLevelLimits: vi.fn().mockResolvedValue(undefined), navigationHistory: { canGoBack: () => false, canGoForward: () => false },
      });
      const value = {
        key: `${workspace}/d1`, workspace, id: "d1", applied: 3, partition: browserPartition(workspace, url, "local"), route: null, loading: null,
        state: { url, loading: true, failure: null }, report: null, settled: new Set(), device: null, bounds: null, emulated: null, view: { webContents: contents }, contentsId: contents.id,
      };
      (Reflect.get(subject, "pages") as Map<string, unknown>).set(value.key, value);
      Reflect.get(subject, "watch").call(subject, value);
      /** The core asks for `load`, as a newer stamp from its snapshot does. */
      const ask = (load: number) => {
        value.applied = load;
        Reflect.get(subject, "load").call(subject, value, url);
      };
      return { contents, loads, ask };
    };
    const logged = () => log.event.mock.calls.filter(([name]) => String(name).startsWith("browser.load_") && name !== "browser.load_failed").map(([name, fields]) => [name, fields]);
    return { page, routes, logged, tick: (ms: number) => { now += ms; }, url };
  }

  it("logs each stage against its own load, so a reopen waiting for its route never takes the commit or stop of the load before it", async () => {
    const { page, routes, logged, tick, url } = stages();
    const { contents, loads, ask } = page("local\u0000/checkout");
    ask(3);
    tick(40);
    routes[0]!.answer({ url, source_url: url, load: 3 });
    await vi.waitFor(() => expect(loads).toHaveLength(1));
    // The reopen arrives while the first load is on its way, and the first
    // load commits and finishes while the reopen still waits for its route.
    tick(10);
    ask(4);
    contents.emit("did-navigate", {}, url);
    contents.emit("did-stop-loading");
    loads[0]!.finish();
    tick(100);
    routes[1]!.answer({ url, source_url: url, load: 4 });
    await vi.waitFor(() => expect(loads).toHaveLength(2));
    tick(200);
    contents.emit("did-navigate", {}, url);
    contents.emit("did-navigate", {}, `${url}next`);
    tick(60);
    loads[1]!.finish();
    await vi.waitFor(() => expect(logged()).toHaveLength(4));
    // A navigation the page made itself is not a load the core asked for.
    contents.emit("did-start-loading");
    contents.emit("did-navigate", {}, `${url}clicked`);
    contents.emit("did-stop-loading");
    expect(logged()).toEqual([
      ["browser.load_routed", { id: "d1", contents: 7, load: 3, ms: 40 }],
      ["browser.load_routed", { id: "d1", contents: 7, load: 4, ms: 100 }],
      ["browser.load_committed", { id: "d1", contents: 7, load: 4, ms: 300 }],
      ["browser.load_stopped", { id: "d1", contents: 7, load: 4, ms: 360, outcome: "loaded" }],
    ]);
  });

  it("tells apart two checkouts' displays of the same id and load by their native page", async () => {
    const { page, routes, logged, url } = stages();
    const first = page("local\u0000/one");
    const second = page("local\u0000/two");
    first.ask(3);
    second.ask(3);
    for (const route of routes) route.answer({ url, source_url: url, load: route.load });
    await vi.waitFor(() => expect(first.loads.length + second.loads.length).toBe(2));
    second.loads[0]!.abort();
    first.loads[0]!.finish();
    await vi.waitFor(() => expect(logged()).toHaveLength(4));
    expect(logged().filter(([name]) => name === "browser.load_stopped")).toEqual([
      ["browser.load_stopped", { id: "d1", contents: 8, load: 3, ms: 0, outcome: "aborted" }],
      ["browser.load_stopped", { id: "d1", contents: 7, load: 3, ms: 0, outcome: "loaded" }],
    ]);
  });

  it("ends a load whose route failed, so the page's next navigation logs no stage of it", async () => {
    const { page, routes, logged } = stages();
    const { contents, loads, ask } = page("local\u0000/checkout");
    ask(3);
    routes[0]!.refuse(new Error("view_unavailable"));
    await vi.waitFor(() => expect(contents.listenerCount("did-navigate")).toBeGreaterThan(0));
    await new Promise((settled) => setImmediate(settled));
    contents.emit("did-navigate", {}, "https://example.test/elsewhere");
    expect(loads).toHaveLength(0);
    expect(logged()).toEqual([]);
  });
});

describe("native browser CDP inventory", () => {
  it("stops notifying a CDP owner that leaves while it is told a page became ineligible", () => {
    const subject = new BrowserViews({ event: vi.fn() } as unknown as HostLog, () => true, vi.fn(), vi.fn(), vi.fn(), () => english, new ExternalFiles(vi.fn()));
    const told: unknown[] = [];
    // The gateway closes its session on the first retirement, and with it its observation.
    const stop = subject.observeCdp((retirement) => { told.push(retirement); stop(); });
    // A page the inventory reported, still alive but no longer eligible for CDP.
    const page = { key: "fixture\u0000manual", workspace: "fixture", id: "manual", view: { webContents: { id: 7, isDestroyed: () => false, getURL: () => "file:///manual.html" } }, contentsId: 7, state: { url: "file:///manual.html" } };
    (Reflect.get(subject, "pages") as Map<string, unknown>).set("manual", page);
    Reflect.set(subject, "cdpObserved", [{ key: page.key, area: "a1", contents: 7, url: "file:///manual.html", title: "" }]);
    expect(() => Reflect.get(subject, "publishCdp").call(subject)).not.toThrow();
    expect(told).toEqual([{ contentsId: 7, reason: "replaced" }]);
  });
});

describe("a phone's still of a page", () => {
  it("is the page's own capture whether or not it is in front, scaled down to its pixel cap, and else why there is none", async () => {
    vi.useFakeTimers();
    try {
      const subject = new BrowserViews({ event: vi.fn() } as unknown as HostLog, () => true, vi.fn(), vi.fn(), vi.fn(), () => english, new ExternalFiles(vi.fn()));
      const image = (width: number, height: number, empty = false) => {
        const resized = { toJPEG: () => Buffer.from("small jpeg") };
        return { isEmpty: () => empty, getSize: () => ({ width, height }), resize: vi.fn(() => resized), toJPEG: () => Buffer.from("jpeg") };
      };
      const pages = Reflect.get(subject, "pages") as Map<string, unknown>;
      const answers = new Map<string, () => Promise<unknown>>();
      for (const id of ["behind", "huge", "blank", "stuck"]) {
        // Behind another tab of its area: not visible, and taken all the same.
        pages.set(viewKey("ws", id), { key: viewKey("ws", id), id, visible: false, view: { webContents: { isDestroyed: () => false, capturePage: () => answers.get(id)!() } } });
      }
      answers.set("behind", async () => image(520, 720));
      const huge = image(4000, 3000);
      answers.set("huge", async () => huge);
      answers.set("blank", async () => image(0, 0, true));
      answers.set("stuck", () => new Promise(() => {}));
      Reflect.set(subject, "cdpAreas", new Map(["behind", "huge", "blank", "stuck", "unloaded"].map((id) => [viewKey("ws", id), "a1"])));
      const still = (display_id: string, area_id = "a1") => subject.still({ workspace: "ws", area_id, display_id });

      expect(await still("behind")).toEqual({ jpeg: new Uint8Array(Buffer.from("jpeg")) });
      expect(await still("huge")).toEqual({ jpeg: new Uint8Array(Buffer.from("small jpeg")) });
      // 1.5 million pixels of a 4:3 picture.
      expect(huge.resize).toHaveBeenCalledWith({ width: 1414, quality: "good" });
      expect(await still("blank")).toEqual({ failure: "not_drawn" });
      expect(await still("unloaded")).toEqual({ failure: "not_loaded" });
      // An area the core did not authorize for the display is not taken.
      expect(await still("behind", "a2")).toEqual({ failure: "display_missing" });
      // A capture that never answers frees its gateway request in 5 s.
      const stuck = still("stuck");
      await vi.advanceTimersByTimeAsync(5_000);
      expect(await stuck).toEqual({ failure: "capture_timeout" });
    } finally {
      vi.useRealTimers();
    }
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
    const contents = Object.assign(new EventEmitter(), { isFocused: () => focused, isDestroyed: () => false, setWindowOpenHandler: vi.fn(), focus: pageFocus, navigationHistory: { canGoBack: () => false, canGoForward: () => false } });
    const value = { key: id, workspace: "fixture", id, visible: true, view: { webContents: contents, setVisible: vi.fn() }, state: {}, report: null, route: null, applied: 1, partition: "fixture", shownAt: 0, settled: new Set() };
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
      /** A new document starts loading in the page, as a meta refresh or an agent's navigate does. */
      navigate: () => contents.emit("did-start-navigation", { isMainFrame: true, isSameDocument: false }),
      press: () => contents.emit("input-event", {}, { type: "mouseDown" }),
      stopLoading: () => contents.emit("did-stop-loading"),
    };
  };
  const forwarded = () => send.mock.calls.filter(([channel]) => channel === BROWSER_EVENT_CHANNEL).map(([, value]) => value as { kind: string; id: string; cycleId: number; key: string });
  return { page, forwarded, shellFocus, shellTakesFocus: () => shellContents.emit("focus"), shellInput: (type?: string, key?: string, control?: boolean, alt?: boolean) => input(shellContents, type, key, control, alt), registry: (rows: readonly Command[]) => subject.setRegistry(rows), blur: () => window.emit("blur"), windowReturn: () => window.emit("focus"), windowFocus: (focused: boolean) => { windowFocused = focused; } };
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
      const contents = Object.assign(new EventEmitter(), { id: ++contentsId, isFocused: () => false, isDestroyed: () => false, isCurrentlyAudible: () => false, close: vi.fn(), setWindowOpenHandler: vi.fn(), getURL: () => `https://${id}.example/`, getTitle: () => id, getZoomFactor: () => 1, enableDeviceEmulation: vi.fn(), disableDeviceEmulation: vi.fn() });
      const value = {
        key: `${workspace}\u0001${id}`, workspace, id, visible: false, placed: false, shownAt: 0, idleSince: 0, playing: false, downloads: 0, settled: new Set(),
        view: { webContents: contents, setVisible: vi.fn(), setBounds: vi.fn() }, contentsId: contents.id, state: { url: `https://${id}.example/`, loading: false, failure: null },
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

describe("a page that closes itself", () => {
  const workspace = "local\u0000/checkout";
  const rect = { x: 0, y: 0, width: 640, height: 480 };
  const placement = (id: string, visible: boolean) => ({ id, area_id: "a1", url: `https://${id}.example/`, load: 3, rect: visible ? rect : null, visible });
  const retained = [{ workspace, id: "closer", area_id: "a1" }, { workspace, id: "other", area_id: "a1" }];

  it("keeps placing the Workspace's other pages, and shows why on its slot without loading it again until reloaded", async () => {
    vi.useFakeTimers();
    try {
      // What Electron does: the page's `window.close()` destroys its contents
      // and the view then names none (reproduced on Electron 44.4.5).
      let nextId = 0;
      const views: { view: { webContents?: EventEmitter & { id: number }; setVisible: ReturnType<typeof vi.fn>; setBounds: ReturnType<typeof vi.fn> }; closeItself: () => void }[] = [];
      nativeView.make = () => {
        const contents = Object.assign(new EventEmitter(), {
          id: ++nextId, isDestroyed: () => false, isFocused: () => false, getZoomFactor: () => 1, getURL: () => "", getTitle: () => "",
          loadURL: vi.fn(() => new Promise(() => undefined)), setWindowOpenHandler: vi.fn(), setVisualZoomLevelLimits: vi.fn().mockResolvedValue(undefined),
          navigationHistory: { canGoBack: () => false, canGoForward: () => false },
        });
        const view: (typeof views)[number]["view"] = { webContents: contents, setVisible: vi.fn(), setBounds: vi.fn() };
        views.push({ view, closeItself: () => { view.webContents = undefined; contents.emit("destroyed"); } });
        return view;
      };
      Object.assign(session, { fromPartition: vi.fn(() => ({ setPermissionRequestHandler: vi.fn(), setPermissionCheckHandler: vi.fn(), on: vi.fn(), webRequest: { onBeforeRequest: vi.fn() } })) });
      const resolve = vi.fn(() => new Promise<ResolvedPage>(() => undefined));
      const subject = new BrowserViews({ event: vi.fn() } as unknown as HostLog, () => true, resolve, vi.fn(), vi.fn(), () => english, new ExternalFiles(vi.fn()));
      const send = vi.fn();
      const shellContents = Object.assign(new EventEmitter(), { isDestroyed: () => false, send, focus: vi.fn(), getZoomFactor: () => 1 });
      const removeChildView = vi.fn();
      const window = Object.assign(new EventEmitter(), { isFocused: () => true, isDestroyed: () => false, webContents: shellContents, contentView: { removeChildView, addChildView: vi.fn() } });
      subject.attach(window as never);
      const sync = (displays: ReturnType<typeof placement>[]) => Reflect.get(subject, "sync").call(subject, workspace, displays, retained);

      sync([placement("closer", true), placement("other", false)]);
      expect(resolve).toHaveBeenCalledExactlyOnceWith(workspace, "closer", 3);
      const closer = views[0]!;
      closer.closeItself();
      expect(removeChildView).toHaveBeenCalledWith(closer.view);
      // A fresh page stands on the same slot, shown, and nothing loads it.
      const fresh = views[1]!;
      expect(fresh.view.setBounds).toHaveBeenLastCalledWith(rect);
      expect(fresh.view.setVisible).toHaveBeenLastCalledWith(true);
      await vi.advanceTimersByTimeAsync(200);
      const states = send.mock.calls.map(([, event]) => event as { kind: string; id: string; state?: { loading: boolean; failure: string | null } }).filter((event) => event.kind === "state" && event.id === "closer");
      expect(states.at(-1)?.state).toMatchObject({ url: "https://closer.example/", loading: false, failure: english("native.browser.closedItself") });

      // The operator moves to the other page: it is built and shown, and the fresh one hides.
      expect(() => sync([placement("closer", false), placement("other", true)])).not.toThrow();
      const other = views[2]!;
      expect(other.view.setVisible).toHaveBeenLastCalledWith(true);
      expect(fresh.view.setVisible).toHaveBeenLastCalledWith(false);
      expect(resolve).toHaveBeenCalledTimes(2);

      // Back on it, the toolbar's reload loads its address again, once.
      sync([placement("closer", true), placement("other", false)]);
      expect(resolve).toHaveBeenCalledTimes(2);
      (ipc.get(BROWSER_COMMAND_CHANNEL) as unknown as (event: unknown, target: unknown, name: unknown) => void)({}, { workspace, id: "closer" }, "reload");
      expect(resolve).toHaveBeenLastCalledWith(workspace, "closer", 3);
      expect(resolve).toHaveBeenCalledTimes(3);
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("pages on no desk on screen (PRD lane-desk-pair-view B32)", () => {
  const workspace = "local\u0000/checkout";
  const rect = { x: 10, y: 20, width: 640, height: 480 };
  const row = (id: string, load: number, url = `https://${id}.example/`) => ({ workspace, id, area_id: "shelf", url, load });

  function host() {
    let nextId = 0;
    const views: { id: string; view: { webContents: EventEmitter & { id: number; getURL: () => string; close: ReturnType<typeof vi.fn>; enableDeviceEmulation: ReturnType<typeof vi.fn>; disableDeviceEmulation: ReturnType<typeof vi.fn> }; setVisible: ReturnType<typeof vi.fn>; setBounds: ReturnType<typeof vi.fn> } }[] = [];
    nativeView.make = () => {
      const contents = Object.assign(new EventEmitter(), {
        id: ++nextId, isDestroyed: () => false, isFocused: () => false, getZoomFactor: () => 1, getURL: () => "", getTitle: () => "", close: vi.fn(),
        enableDeviceEmulation: vi.fn(), disableDeviceEmulation: vi.fn(),
        loadURL: vi.fn(() => new Promise(() => undefined)), setWindowOpenHandler: vi.fn(), setVisualZoomLevelLimits: vi.fn().mockResolvedValue(undefined),
        navigationHistory: { canGoBack: () => false, canGoForward: () => false },
      });
      const view = { webContents: contents, setVisible: vi.fn(), setBounds: vi.fn() };
      views.push({ id: "", view });
      return view;
    };
    Object.assign(session, { fromPartition: vi.fn(() => ({ setPermissionRequestHandler: vi.fn(), setPermissionCheckHandler: vi.fn(), on: vi.fn(), webRequest: { onBeforeRequest: vi.fn() } })) });
    const resolve = vi.fn<(workspace: string, id: string, load: number) => Promise<ResolvedPage>>(() => new Promise<ResolvedPage>(() => undefined));
    const release = vi.fn();
    const log = { event: vi.fn() };
    let now = 1;
    const subject = new BrowserViews(log as unknown as HostLog, () => true, resolve, release, vi.fn(), () => english, new ExternalFiles(vi.fn()), { bound: 1_000_000, every: 1_000_000, now: () => now });
    const shellContents = Object.assign(new EventEmitter(), { isDestroyed: () => false, send: vi.fn(), focus: vi.fn(), getZoomFactor: () => 1 });
    const removeChildView = vi.fn();
    const window = Object.assign(new EventEmitter(), {
      isFocused: () => true, isDestroyed: () => false, webContents: shellContents, getContentBounds: () => ({ x: 0, y: 0, width: 1400, height: 820 }),
      contentView: { removeChildView, addChildView: vi.fn() },
    });
    subject.attach(window as never);
    const pages = Reflect.get(subject, "pages") as Map<string, { id: string; view: unknown; visible: boolean }>;
    // Through the channel the shell sends on, so the host reads what parseSync passes.
    const sync = (retained: ReturnType<typeof row>[], placed: { id: string; load: number; rect: typeof rect | null; device?: { width: number; mobile: boolean } }[] = [], inventoried = true) => {
      now += 1;
      ipc.get(BROWSER_SYNC_CHANNEL)!({}, {
        workspace: placed.length ? workspace : null,
        displays: placed.map((display) => ({ id: display.id, area_id: "shelf", url: `https://${display.id}.example/`, load: display.load, rect: display.rect, visible: display.rect !== null, ...(display.device ? { device: display.device } : {}) })),
        retained, ...(inventoried ? { inventoried: true } : {}),
      });
    };
    // What a shell sends first after it starts or reloads: no snapshot yet, so an empty list that is not the core's.
    const firstSync = () => sync([], [], false);
    const reloadShell = () => shellContents.emit("did-start-navigation", { isMainFrame: true, isSameDocument: false });
    const live = () => [...pages.values()].map((page) => page.id).sort();
    const pageOf = (id: string) => pages.get(`${workspace}\u0001${id}`);
    const viewOf = (id: string) => pageOf(id)?.view as (typeof views)[number]["view"] | undefined;
    return { subject, sync, firstSync, reloadShell, resolve, release, removeChildView, views, live, viewOf, pageOf, log };
  }

  it("loads a page the core asks for hidden, and the desk then shows that same page without loading it again", () => {
    const { sync, firstSync, resolve, views, viewOf, pageOf } = host();
    // The shell's first sync names nothing; its first inventory finds a page
    // opened before this window: nothing loads.
    firstSync();
    sync([row("earlier", 3)]);
    expect(views).toHaveLength(0);
    // An agent opens a page on no desk: it loads hidden, laid out at the
    // lane's width by emulation, since Chromium gives a page it never showed
    // no viewport of its own.
    sync([row("earlier", 3), row("agent", 4)]);
    expect(resolve).toHaveBeenCalledExactlyOnceWith(workspace, "agent", 4);
    const page = viewOf("agent")!;
    const laidOut = { width: 900, height: 820 };
    expect(page.setBounds).toHaveBeenLastCalledWith({ x: 0, y: 0, ...laidOut });
    expect(page.setVisible).not.toHaveBeenCalledWith(true);
    // Nothing is laid out before a document commits; each commit lays it out.
    expect(page.webContents.enableDeviceEmulation).not.toHaveBeenCalled();
    page.webContents.getURL = () => "https://agent.example/";
    page.webContents.emit("did-navigate", {}, "https://agent.example/");
    expect(page.webContents.enableDeviceEmulation).toHaveBeenLastCalledWith({ screenPosition: "desktop", screenSize: laidOut, viewPosition: { x: 0, y: 0 }, deviceScaleFactor: 0, viewSize: laidOut, scale: 1 });
    // The operator puts it on the desk: the same contents take the slot and
    // its own size, with no load.
    sync([row("earlier", 3), row("agent", 4)], [{ id: "agent", load: 4, rect }]);
    expect(views).toHaveLength(1);
    expect(viewOf("agent")).toBe(page);
    expect(resolve).toHaveBeenCalledOnce();
    expect(page.setBounds).toHaveBeenLastCalledWith(rect);
    expect(page.webContents.disableDeviceEmulation).toHaveBeenCalledOnce();
    expect(page.setVisible).toHaveBeenLastCalledWith(true);
    expect(pageOf("agent")?.visible).toBe(true);
    // Taken off the desk again, it stays loaded and hidden; a reopen loads it again in place.
    sync([row("earlier", 3), row("agent", 4)]);
    expect(page.setVisible).toHaveBeenLastCalledWith(false);
    sync([row("earlier", 3), row("agent", 5)]);
    expect(views).toHaveLength(1);
    expect(resolve).toHaveBeenLastCalledWith(workspace, "agent", 5);
    // The page opened before this window loads when the core asks again.
    sync([row("earlier", 6), row("agent", 5)]);
    expect(resolve).toHaveBeenLastCalledWith(workspace, "earlier", 6);
  });

  it("loads none of the pages it already had when a checkout comes back or the shell loads again", () => {
    const { sync, firstSync, reloadShell, resolve, log } = host();
    const offScreenLoads = () => log.event.mock.calls.filter(([kind]) => kind === "browser.loaded_off_screen").length;
    firstSync();
    sync([row("a", 3)]);
    sync([row("a", 3), row("b", 4)]);
    expect(resolve).toHaveBeenCalledExactlyOnceWith(workspace, "b", 4);
    // The device link drops: its checkout's pages leave the inventory, and
    // come back with the stamps they had.
    sync([]);
    sync([row("a", 3), row("b", 4)]);
    expect(resolve).toHaveBeenCalledOnce();
    // The shell loads again: its first sync names nothing, then the same inventory.
    reloadShell();
    firstSync();
    sync([row("a", 3), row("b", 4), row("c", 5)]);
    expect(resolve).toHaveBeenCalledOnce();
    expect(offScreenLoads()).toBe(1);
    // A page opened after that loads.
    sync([row("a", 3), row("b", 4), row("c", 5), row("d", 6)]);
    expect(resolve).toHaveBeenLastCalledWith(workspace, "d", 6);
    expect(offScreenLoads()).toBe(2);
  });

  it("keeps pages on no desk within the live-page cap and releases a closed View's page", () => {
    const { sync, firstSync, resolve, release, removeChildView, live, views, viewOf } = host();
    firstSync();
    // The operator looked at one page on a desk, then left it.
    const desk = row("desk", 1);
    sync([desk], [{ id: "desk", load: 1, rect }]);
    sync([desk]);
    const rows: ReturnType<typeof row>[] = [desk];
    for (let index = 1; index <= MAX_LIVE_VIEWS; index += 1) {
      rows.push(row(`p${index}`, index + 1));
      sync([...rows]);
    }
    // Twelve live at most: a page loaded off screen counts as just seen, so the
    // page shown longest ago goes and every page an agent just asked for stays.
    expect(live()).toHaveLength(MAX_LIVE_VIEWS);
    expect(live()).not.toContain("desk");
    expect(live()).toContain("p1");
    expect(live()).toContain(`p${MAX_LIVE_VIEWS}`);
    // An unloaded page is not loaded again on the next sync; only a new ask does that.
    const loads = resolve.mock.calls.length;
    sync([...rows]);
    expect(resolve).toHaveBeenCalledTimes(loads);
    // The core closes a View: its page ends and its route is let go.
    const closed = viewOf("p5")!;
    sync(rows.filter((entry) => entry.id !== "p5"));
    expect(live()).not.toContain("p5");
    expect(removeChildView).toHaveBeenCalledWith(closed);
    expect(closed.webContents.close).toHaveBeenCalledOnce();
    expect(release).toHaveBeenCalledWith(workspace, "p5", 6);
    // Every View closed: nothing is left alive.
    sync([]);
    expect(live()).toEqual([]);
    expect(views.every(({ view }) => view.webContents.close.mock.calls.length === 1)).toBe(true);
  });

  it("lays out a page shown at a device size and loaded again off screen only once its document commits", () => {
    const { sync, firstSync, views, viewOf } = host();
    firstSync();
    const phone = row("phone", 1);
    sync([phone], [{ id: "phone", load: 1, rect, device: { width: 390, mobile: true } }]);
    sync([phone]);
    const rows: ReturnType<typeof row>[] = [phone];
    for (let index = 1; index <= MAX_LIVE_VIEWS; index += 1) {
      rows.push(row(`p${index}`, index + 1));
      sync([...rows]);
    }
    expect(viewOf("phone")).toBeUndefined();
    // An agent opens it again: emulating it before a document commits would stall the main process.
    sync([row("phone", MAX_LIVE_VIEWS + 2), ...rows.slice(1)]);
    const page = viewOf("phone")!;
    expect(views.at(-1)?.view).toBe(page);
    expect(page.webContents.enableDeviceEmulation).not.toHaveBeenCalled();
    page.webContents.getURL = () => "https://phone.example/";
    page.webContents.emit("did-navigate", {}, "https://phone.example/");
    expect(page.webContents.enableDeviceEmulation).toHaveBeenCalledOnce();
    expect(page.webContents.enableDeviceEmulation.mock.calls[0]![0]).toMatchObject({ screenPosition: "mobile", viewSize: { width: 390 } });
  });

  it("does not load an evicted local file page again until the core asks", () => {
    const { sync, firstSync, resolve, live } = host();
    firstSync();
    // The core's inventory is empty, then an agent opens one file after another.
    sync([]);
    const rows = Array.from({ length: MAX_LIVE_VIEWS + 1 }, (_, index) => row(`f${index}`, index + 1, `file:///checkout/f${index}.html`));
    for (let index = 1; index <= rows.length; index += 1) sync(rows.slice(0, index));
    expect(live()).toHaveLength(MAX_LIVE_VIEWS);
    const loads = resolve.mock.calls.length;
    sync(rows);
    sync(rows);
    expect(resolve).toHaveBeenCalledTimes(loads);
  });
});

describe("a page's navigation and the keyboard", () => {
  beforeEach(() => ipc.clear());
  const turn = () => new Promise((done) => setImmediate(done));
  const pageFocusReports = (forwarded: () => { kind: string; id: string }[]) => forwarded().filter((event) => event.kind === "focus").map((event) => event.id);

  it("gives the keyboard back to the shell when a page's new document takes it", async () => {
    const { page, forwarded, shellFocus, shellTakesFocus } = candidate();
    const dashboard = page("dashboard");
    dashboard.focus(false);
    shellTakesFocus();
    dashboard.navigate();
    // Chromium focuses the committed document's view.
    dashboard.focus(true);
    dashboard.takeFocus();
    await turn();
    expect(shellFocus).toHaveBeenCalledOnce();
    expect(pageFocusReports(forwarded)).toEqual([]);
  });

  it("gives it back to the page that held it", async () => {
    const { page, forwarded, shellFocus } = candidate();
    const reading = page("reading"), dashboard = page("dashboard");
    reading.takeFocus();
    reading.focus(false);
    dashboard.focus(false);
    dashboard.navigate();
    dashboard.focus(true);
    dashboard.takeFocus();
    await turn();
    expect(reading.pageFocus).toHaveBeenCalledOnce();
    expect(shellFocus).not.toHaveBeenCalled();
    expect(pageFocusReports(forwarded)).toEqual(["reading"]);
  });

  it("keeps the keyboard in a page that held it, or that the operator pressed while it loaded", async () => {
    const { page, forwarded, shellFocus, shellTakesFocus } = candidate();
    const typing = page("typing");
    typing.takeFocus();
    typing.navigate();
    typing.takeFocus();
    const pressed = page("pressed");
    pressed.focus(false);
    shellTakesFocus();
    pressed.navigate();
    pressed.focus(true);
    pressed.takeFocus();
    pressed.press();
    await turn();
    expect(shellFocus).not.toHaveBeenCalled();
    expect(pageFocusReports(forwarded)).toEqual(["typing", "typing", "pressed"]);
  });

  it("gives the keyboard back to the shell when the operator took it there while the page that held it was loading", async () => {
    const { page, forwarded, shellFocus, shellTakesFocus } = candidate();
    const dashboard = page("dashboard");
    // The page holds the keyboard when its own document starts to load, so
    // the start shows nothing taken from anyone.
    dashboard.takeFocus();
    dashboard.navigate();
    // The operator moves the keyboard to the terminal before the document
    // commits; Chromium then focuses the view for the commit.
    dashboard.focus(false);
    shellTakesFocus();
    dashboard.focus(true);
    dashboard.takeFocus();
    await turn();
    expect(shellFocus).toHaveBeenCalledOnce();
    expect(pageFocusReports(forwarded)).toEqual(["dashboard"]);
    // The next load of the same page finds the keyboard in the shell, not in the page.
    dashboard.stopLoading();
    dashboard.focus(false);
    dashboard.navigate();
    dashboard.focus(true);
    dashboard.takeFocus();
    await turn();
    expect(shellFocus).toHaveBeenCalledTimes(2);
    expect(pageFocusReports(forwarded)).toEqual(["dashboard"]);
  });

  it("leaves a background window alone and treats focus after the load as the operator's", async () => {
    const { page, forwarded, shellFocus, shellTakesFocus, windowFocus } = candidate();
    const dashboard = page("dashboard");
    dashboard.focus(false);
    shellTakesFocus();
    windowFocus(false);
    dashboard.navigate();
    dashboard.focus(true);
    dashboard.takeFocus();
    await turn();
    expect(shellFocus).not.toHaveBeenCalled();
    expect(pageFocusReports(forwarded)).toEqual([]);
    windowFocus(true);
    dashboard.stopLoading();
    dashboard.takeFocus();
    await turn();
    expect(shellFocus).not.toHaveBeenCalled();
    expect(pageFocusReports(forwarded)).toEqual(["dashboard"]);
  });
});

describe("a file page's links (issue 2894)", () => {
  type OpenHandler = (details: { url: string; disposition: string; features: string; referrer: { url: string } }) => { action: string };

  /** A host with one file page loaded off screen, its route answered as hided answers it. */
  async function filePage(workspace: string, source: string, route: string) {
    let nextId = 0;
    const made: (EventEmitter & { setWindowOpenHandler: ReturnType<typeof vi.fn>; loadURL: ReturnType<typeof vi.fn> })[] = [];
    nativeView.make = () => {
      const contents = Object.assign(new EventEmitter(), {
        id: ++nextId, isDestroyed: () => false, isFocused: () => false, getZoomFactor: () => 1, getURL: () => "", getTitle: () => "", close: vi.fn(),
        enableDeviceEmulation: vi.fn(), disableDeviceEmulation: vi.fn(),
        loadURL: vi.fn(() => new Promise(() => undefined)), setWindowOpenHandler: vi.fn(), setVisualZoomLevelLimits: vi.fn().mockResolvedValue(undefined),
        navigationHistory: { canGoBack: () => false, canGoForward: () => false },
      });
      made.push(contents);
      return { webContents: contents, setVisible: vi.fn(), setBounds: vi.fn() };
    };
    Object.assign(session, { fromPartition: vi.fn(() => ({ setPermissionRequestHandler: vi.fn(), setPermissionCheckHandler: vi.fn(), on: vi.fn(), webRequest: { onBeforeRequest: vi.fn() } })) });
    const resolve = vi.fn((_workspace: string, _id: string, load: number) => Promise.resolve<ResolvedPage>({ url: route, source_url: source, load }));
    const log = { event: vi.fn() };
    const subject = new BrowserViews(log as unknown as HostLog, () => true, resolve, vi.fn(), vi.fn(), () => english, new ExternalFiles(vi.fn()));
    const send = vi.fn();
    const window = Object.assign(new EventEmitter(), {
      isFocused: () => true, isDestroyed: () => false, getContentBounds: () => ({ x: 0, y: 0, width: 1400, height: 820 }),
      webContents: Object.assign(new EventEmitter(), { isDestroyed: () => false, send, focus: vi.fn(), getZoomFactor: () => 1 }),
      contentView: { removeChildView: vi.fn(), addChildView: vi.fn() },
    });
    subject.attach(window as never);
    const sync = (retained: { workspace: string; id: string; area_id: string; url: string; load: number }[]) =>
      ipc.get(BROWSER_SYNC_CHANNEL)!({}, { workspace: null, displays: [], retained, inventoried: true });
    sync([]);
    sync([{ workspace, id: "preview", area_id: "shelf", url: source, load: 2 }]);
    // Ready once the host applied the route: it loads the route's address.
    await vi.waitFor(() => expect(made[0]?.loadURL).toHaveBeenCalledWith(route));
    const contents = made[0]!;
    const opened = () => send.mock.calls.filter(([channel, event]) => channel === BROWSER_EVENT_CHANNEL && (event as { kind: string }).kind === "open").map(([, event]) => event);
    const navigate = (url: string) => {
      const event = { preventDefault: vi.fn() };
      contents.emit("will-navigate", event, url);
      return event.preventDefault.mock.calls.length > 0;
    };
    const openWindow = (url: string, features = "") => (contents.setWindowOpenHandler.mock.calls[0]![0] as OpenHandler)({ url, disposition: features ? "new-window" : "foreground-tab", features, referrer: { url: "" } });
    return { opened, navigate, openWindow, log };
  }

  it("opens a local file page's web link, a click or a window.open, as a display beside it", async () => {
    const workspace = "local\u0000/checkout";
    const source = "file:///checkout/report.html";
    const { opened, navigate, openWindow, log } = await filePage(workspace, source, source);
    // A link to the operator's dev server, a loopback address, and a public one.
    expect(navigate("http://127.0.0.1:8765/")).toBe(true);
    expect(openWindow("https://example.com/").action).toBe("deny");
    expect(openWindow("https://example.com/sign-in", "width=400,height=500").action).toBe("deny");
    expect(opened()).toEqual([
      { kind: "open", workspace, id: "preview", url: "http://127.0.0.1:8765/" },
      { kind: "open", workspace, id: "preview", url: "https://example.com/" },
      { kind: "open", workspace, id: "preview", url: "https://example.com/sign-in" },
    ]);
    // Its own file links still move it in place.
    expect(navigate("file:///checkout/other.html")).toBe(false);
    expect(log.event).not.toHaveBeenCalledWith(expect.stringMatching(/refused$/), expect.objectContaining({ reason: "remote_file_boundary" }));
  });

  it("refuses a device file page's loopback link, a click or a window.open, and opens nothing", async () => {
    const workspace = "device-b\u0000/remote";
    const { opened, navigate, openWindow, log } = await filePage(workspace, "file:///remote/report.html", "http://127.0.0.1:53000/route-1/report.html");
    expect(navigate("http://localhost:3000/")).toBe(true);
    expect(openWindow("http://localhost:3000/").action).toBe("deny");
    expect(opened()).toEqual([]);
    expect(log.event).toHaveBeenCalledWith("browser.navigation_refused", { reason: "remote_file_boundary" });
    expect(log.event).toHaveBeenCalledWith("browser.window_open_refused", { reason: "remote_file_boundary" });
    // Its own route is its own page.
    expect(navigate("http://127.0.0.1:53000/route-1/other.html")).toBe(false);
  });
});
