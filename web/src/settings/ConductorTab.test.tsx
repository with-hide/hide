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
    useShellStore.setState({ connection: "live", rest: { ui_state: { conductor_settings: settings } } } as never);
    root.render(<TooltipProvider><ConductorTab actions={actions} /></TooltipProvider>);
  });
  const group = (kind: string) => container.querySelector<HTMLElement>(`[data-conductor-check="${kind}"]`)!;
  return {
    // Only the settings it sends; the page also asks for the start panel's model lists while it shows.
    get sent() { return events.filter((event) => event.kind === "conductor_settings_set").map((event) => event.payload); },
    /** The core's answer arriving: the value it kept, or a refusal. */
    async answer(rest: { checkers?: ConductorSettings["checkers"]; error?: string }) {
      await act(async () => useShellStore.setState((shell) => ({ rest: { ...shell.rest, ui_state: { conductor_settings: { ...settings, ...(rest.checkers ? { checkers: rest.checkers } : {}) } }, status: rest.error ? { last_error: { kind: rest.error, message: "", retryable: false, occurred_at: Date.now() + 1 } } : shell.rest?.status } }) as never));
    },
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

it("shows who checks, the agent and the other-company preference for the Conductor's call, the count and rule only for several, and a model and effort only for a chosen agent", async () => {
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
  const chosen = await mount({ ...DEFAULT_CONDUCTOR, checkers: { review: { ...DEFAULT_CHECKERS.review, mode: "one", agent: "codex", effort: "high" }, verify: DEFAULT_CHECKERS.verify } });
  try {
    expect(chosen.fields("review")).toEqual(["mode", "agent", "model", "effort", "other_vendor"]);
  } finally { await chosen.unmount(); }
});

it("sends both defaults whole, each edit built on the ones the core has not answered yet, so none is lost (B36)", async () => {
  const tab = await mount(DEFAULT_CONDUCTOR);
  const several = { ...DEFAULT_CHECKERS.verify, mode: "many" as const };
  const first = { review: DEFAULT_CHECKERS.review, verify: several };
  const second = { review: { ...DEFAULT_CHECKERS.review, agent: "codex" }, verify: several };
  const third = { review: { ...DEFAULT_CHECKERS.review, agent: "codex", other_vendor: false }, verify: several };
  try {
    // Three edits before any snapshot comes back.
    await tab.pick("verify", "mode", "Several");
    await tab.pick("review", "agent", "Codex");
    await act(async () => tab.group("review").querySelector<HTMLElement>('[data-check-field="other_vendor"]')!.click());
    expect(tab.sent).toEqual([{ checkers: first }, { checkers: second }, { checkers: third }]);
    // The first answer arriving late does not take back the later edits.
    await tab.answer({ checkers: first });
    expect(tab.group("review").querySelector('[data-check-field="agent"]')?.textContent).toBe("Codex");
    expect(tab.group("review").querySelector('[data-check-field="other_vendor"]')?.getAttribute("aria-checked")).toBe("false");
    expect(tab.group("verify").getAttribute("data-check-mode")).toBe("many");
    // A fourth edit still builds on all three.
    await tab.pick("verify", "count", "3");
    expect(tab.sent.at(-1)).toEqual({ checkers: { ...third, verify: { ...several, count: 3 } } });
    await tab.answer({ checkers: { ...third, verify: { ...several, count: 3 } } });
    expect(tab.group("verify").querySelector('[data-check-field="count"]')?.textContent).toBe("3");
  } finally { await tab.unmount(); }
});

it("goes back to what the core keeps when it refuses an edit", async () => {
  const tab = await mount(DEFAULT_CONDUCTOR);
  try {
    await tab.pick("review", "mode", "None");
    expect(tab.group("review").getAttribute("data-check-mode")).toBe("none");
    await tab.answer({ error: "conductor_settings.check_count_out_of_range" });
    expect(tab.group("review").getAttribute("data-check-mode")).toBe("conductor");
    // The next edit builds on what the core keeps.
    await tab.pick("verify", "mode", "Always one");
    expect(tab.sent.at(-1)).toEqual({ checkers: { review: DEFAULT_CHECKERS.review, verify: { ...DEFAULT_CHECKERS.verify, mode: "one" } } });
  } finally { await tab.unmount(); }
});
