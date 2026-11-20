// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "../actions";
import type { WhatsNewState } from "../host";
import { useShellStore } from "../store";
import { TooltipProvider } from "./ui/tooltip";
import { WhatsNewHost } from "./whats-new";

// The once-per-launch decision of the window after an update (docs/UI_BEHAVIOR.md,
// What's new and Update history), against a stand-in host whose state arrives
// in the order the desktop host sends it: the kept copy first, the fetched file
// after. `releases.test.ts` owns what an offer holds; this owns when it opens.
vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });
// The shell subscribes to its host once per page, so one stand-in serves every test.
const host = vi.hoisted(() => {
  const host = { listener: null as ((state: unknown) => void) | null, save: (() => undefined) as (update: unknown) => void };
  (window as unknown as { hideHost: unknown }).hideHost = {
    kind: "electron",
    platform: "darwin",
    whatsNew: { onState: (listener: (state: unknown) => void) => { host.listener = listener; return () => undefined; }, refresh: () => undefined, save: (update: unknown) => host.save(update) },
  };
  return host;
});
const saved = useShellStore.getState();
let root: Root | null = null;
afterEach(async () => {
  await act(async () => root?.unmount());
  root = null;
  useShellStore.setState(saved, true);
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

const entry = (version: string, titles: string[]) => ({
  version,
  date: "2026-11-20",
  highlights: titles.map((title) => ({ drawing: "mission", chapter: null, text: { en: { title, line: `${title} line` } } })),
  changes: [],
});

/** Mounts the host part of the shell, as at a launch; the test pushes the host's states. */
async function mount() {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const save = vi.fn();
  host.save = save;
  useShellStore.setState({ rest: { ui_state: {} } as never });
  const state = (over: Partial<WhatsNewState>): WhatsNewState => ({ appVersion: "1.2.11", launchedAt: Date.now(), feed: null, fetchedAt: null, fetching: false, failed: false, record: { shown: "1.2.10", seen: "1.2.10", opened: [] }, ...over });
  // A launch starts with no file: the state an earlier test left is replaced first.
  host.listener?.(state({}));
  const actions = createActions(() => true);
  root = createRoot(document.body.appendChild(document.createElement("div")));
  await act(async () => root!.render(<TooltipProvider><WhatsNewHost actions={actions} /></TooltipProvider>));
  return { push: (over: Partial<WhatsNewState>) => act(async () => host.listener!(state(over))), save };
}

const windows = () => document.querySelectorAll("[data-whats-new]").length;

it("waits through a kept copy without the running version and opens once when the fetched file has it", async () => {
  const { push, save } = await mount();
  await push({ feed: { versions: [entry("1.2.10", ["ten"])] }, fetching: true });
  expect(windows()).toBe(0);
  await push({ feed: { versions: [entry("1.2.11", ["eleven"]), entry("1.2.10", ["ten"])] } });
  expect(windows()).toBe(1);
  expect(document.querySelector("[data-whats-new]")?.getAttribute("data-whats-new")).toBe("1.2.11");
  // Later states of the same launch (a history refresh, a save) open nothing more.
  await push({ feed: { versions: [entry("1.2.11", ["eleven"]), entry("1.2.10", ["ten"])] }, fetching: true });
  expect(windows()).toBe(1);
  expect(save).not.toHaveBeenCalled();
  await act(async () => document.querySelector<HTMLButtonElement>("[data-whats-new-ok]")!.click());
  expect(windows()).toBe(0);
  expect(save).toHaveBeenCalledExactlyOnceWith({ shown: "1.2.11" });
  // The host's answer to that save, before it lands in the record, opens nothing again.
  await push({ feed: { versions: [entry("1.2.11", ["eleven"]), entry("1.2.10", ["ten"])] } });
  expect(windows()).toBe(0);
});

it("opens nothing and records nothing when the file arrives more than 30 s after launch (B11)", async () => {
  const { push, save } = await mount();
  await push({ launchedAt: Date.now() - 31_000, feed: { versions: [entry("1.2.11", ["eleven"])] } });
  expect(windows()).toBe(0);
  expect(save).not.toHaveBeenCalled();
});

it("records a version with no main features as shown without opening anything (B6)", async () => {
  const { push, save } = await mount();
  await push({ feed: { versions: [entry("1.2.11", []), entry("1.2.10", ["ten"])] } });
  expect(windows()).toBe(0);
  expect(save).toHaveBeenCalledExactlyOnceWith({ shown: "1.2.11" });
});
