import { EventEmitter } from "node:events";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { initializeInterfaceI18n } from "../../../web/src/i18n/instance";
import { REGISTRY, type Command } from "../../../web/src/shortcuts";
import { BROWSER_CYCLE_END_CHANNEL, BROWSER_EVENT_CHANNEL } from "../channel";
import { BrowserViews, type ResolvedPage } from "./browser";
import { browserPartition } from "./browserSync";
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
      const subject = new BrowserViews({ event: vi.fn() } as unknown as HostLog, () => true, resolve, vi.fn(), vi.fn(), () => english);
      const workspace = "local\u0000/checkout";
      const requested = "file:///var/checkout/manual.html";
      const canonical = "file:///private/var/checkout/manual.html";
      const loadURL = vi.fn().mockResolvedValue(undefined);
      const page = {
        key: "manual", workspace, id: "manual", applied: 1,
        partition: browserPartition(workspace, requested, null), route: null,
        state: { url: requested, loading: true, failure: null }, report: null,
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

describe("native browser CDP inventory", () => {
  it("stops notifying a CDP owner that leaves while it is told a page became ineligible", () => {
    const subject = new BrowserViews({ event: vi.fn() } as unknown as HostLog, () => true, vi.fn(), vi.fn(), vi.fn(), () => english);
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

function candidate() {
  const send = vi.fn();
  const shellFocus = vi.fn();
  let windowFocused = true;
  const subject = new BrowserViews({ event: vi.fn() } as unknown as HostLog, () => true, vi.fn(), vi.fn(), vi.fn(), () => english);
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
