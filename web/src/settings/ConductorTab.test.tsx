// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterAll, afterEach, expect, it, vi } from "vitest";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { DEFAULT_CHECKERS, DEFAULT_CONDUCTOR } from "../conductorSettings";
import { clientI18n } from "../i18n/translator";
import type { ConductorSettings } from "../snapshot";
import { useShellStore } from "../store";
import type { DispatchFn } from "../ws";
import { ConductorTab } from "./ConductorTab";

// Settings › Conductor's check defaults (PRD mission-map-git-criteria B36):
// the rows each mode uses and the one event an edit sends.

const browserLayout = vi.hoisted(() => {
  const canvas = HTMLCanvasElement.prototype.getContext;
  const scroll = Element.prototype.scrollIntoView;
  // The shell's modules reach xterm, which asks jsdom for a canvas it lacks; Radix scrolls a picked option into view.
  HTMLCanvasElement.prototype.getContext = () => null;
  Element.prototype.scrollIntoView = () => {};
  return { restore: () => {
    HTMLCanvasElement.prototype.getContext = canvas;
    Element.prototype.scrollIntoView = scroll;
  } };
});
afterAll(() => browserLayout.restore());
afterEach(() => {
  document.body.innerHTML = "";
});

async function mount(settings: ConductorSettings) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const saved = useShellStore.getState();
  const language = clientI18n.language;
  await act(async () => {
    await clientI18n.changeLanguage("en");
    useShellStore.setState({ connection: "offline", rest: { ui_state: { conductor_settings: settings } } } as never);
    root.render(<TooltipProvider><ConductorTab actions={actions} /></TooltipProvider>);
  });
  const group = (kind: string) => container.querySelector<HTMLElement>(`[data-conductor-check="${kind}"]`)!;
  return {
    events,
    group,
    fields: (kind: string) => [...group(kind).querySelectorAll<HTMLElement>("[data-check-field]")].map((field) => field.dataset.checkField),
    async pick(kind: string, field: string, option: string) {
      const control = group(kind).querySelector<HTMLElement>(`[data-check-field="${field}"]`)!;
      await act(async () => control.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true })));
      const item = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find((row) => row.textContent === option);
      if (!item) throw new Error(`no option ${option}`);
      await act(async () => item.click());
    },
    async unmount() {
      await act(async () => root.unmount());
      container.remove();
      useShellStore.setState(saved, true);
      await clientI18n.changeLanguage(language);
      vi.unstubAllGlobals();
    },
  };
}

it("shows who checks, the agent and the other-company preference for the Conductor's call, and the count and rule only for several", async () => {
  const tab = await mount(DEFAULT_CONDUCTOR);
  try {
    expect(tab.group("review").querySelector("h3")?.textContent).toBe("Review defaults");
    expect(tab.fields("review")).toEqual(["mode", "agent", "other_vendor"]);
    expect(tab.group("review").querySelector('[data-check-field="agent"]')?.textContent).toBe("Conductor chooses");
    // The note under the last group says when a change applies.
    expect(tab.group("verify").textContent).toContain("A change applies from the next mission.");
  } finally { await tab.unmount(); }
  const several = await mount({ ...DEFAULT_CONDUCTOR, checkers: { review: { ...DEFAULT_CHECKERS.review, mode: "none" }, verify: { ...DEFAULT_CHECKERS.verify, mode: "many", count: 3 } } });
  try {
    expect(several.fields("review")).toEqual(["mode"]);
    expect(several.fields("verify")).toEqual(["mode", "count", "rule", "agent", "other_vendor"]);
    expect(several.group("verify").querySelector('[data-check-field="count"]')?.textContent).toBe("3");
  } finally { await several.unmount(); }
});

it("sends both defaults whole when one field of one changes", async () => {
  const tab = await mount(DEFAULT_CONDUCTOR);
  try {
    await tab.pick("verify", "mode", "Several");
    await tab.pick("review", "agent", "Codex");
    await act(async () => tab.group("review").querySelector<HTMLElement>('[data-check-field="other_vendor"]')!.click());
    expect(tab.events.map((event) => event.payload)).toEqual([
      { checkers: { review: DEFAULT_CHECKERS.review, verify: { ...DEFAULT_CHECKERS.verify, mode: "many" } } },
      { checkers: { review: { ...DEFAULT_CHECKERS.review, agent: "codex" }, verify: DEFAULT_CHECKERS.verify } },
      { checkers: { review: { ...DEFAULT_CHECKERS.review, other_vendor: false }, verify: DEFAULT_CHECKERS.verify } },
    ]);
    expect(tab.events.every((event) => event.kind === "conductor_settings_set")).toBe(true);
  } finally { await tab.unmount(); }
});
