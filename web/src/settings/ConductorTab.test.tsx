// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterAll, afterEach, expect, it, vi } from "vitest";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { DEFAULT_CONDUCTOR } from "../conductorSettings";
import { clientI18n } from "../i18n/translator";
import type { ConductorSettings } from "../snapshot";
import { useShellStore } from "../store";
import type { DispatchFn } from "../ws";
import { ConductorTab } from "./ConductorTab";

// Settings › Conductor's verification: one strength and one toggle, each edit
// sending only its own field.

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
  const verification = () => container.querySelector<HTMLElement>("[data-conductor-verification]")!;
  return {
    // Only the settings it sends; the page also asks for the start panel's model lists while it shows.
    get sent() { return events.filter((event) => event.kind === "conductor_settings_set").map((event) => event.payload); },
    verification,
    strengths: () => [...verification().querySelectorAll<HTMLElement>("[data-conductor-strength-option]")],
    async unmount() {
      await act(async () => root.unmount());
      container.remove();
      useShellStore.setState(saved, true);
      await clientI18n.changeLanguage(language);
      vi.unstubAllGlobals();
    },
  };
}

it("offers one verification strength and the other-company toggle beside the candidates, and says what the chosen strength has the Conductor do", async () => {
  const tab = await mount(DEFAULT_CONDUCTOR);
  try {
    const group = tab.verification();
    expect(group.querySelector("h3")?.textContent).toBe("Verification");
    // Two controls: the strength's three choices and the switch, nothing else.
    expect(tab.strengths().map((option) => [option.textContent, option.getAttribute("data-state")])).toEqual([["Fast", "off"], ["Normal", "on"], ["Thorough", "off"]]);
    expect([...group.querySelectorAll('[role="switch"]')].map((toggle) => [toggle.getAttribute("aria-label"), toggle.getAttribute("aria-checked")])).toEqual([["Checks use a model from another company than the worker's", "true"]]);
    expect(group.querySelectorAll('[role="combobox"], input')).toHaveLength(0);
    expect(group.querySelector("[data-conductor-strength-terms]")?.textContent).toContain("At most twice, only what changed");
    expect(group.textContent).toContain("A change applies from the next mission.");
  } finally { await tab.unmount(); }
  const thorough = await mount({ ...DEFAULT_CONDUCTOR, default_strength: "thorough", other_vendor: false });
  try {
    expect(thorough.strengths().find((option) => option.getAttribute("data-state") === "on")?.textContent).toBe("Thorough");
    expect(thorough.verification().querySelector("[data-conductor-strength-terms]")?.textContent).toContain("At most three times");
    expect(thorough.verification().querySelector('[role="switch"]')?.getAttribute("aria-checked")).toBe("false");
  } finally { await thorough.unmount(); }
});

it("sends each edit as its own field, so a second edit before the first one's answer keeps it, and the strength already chosen sends nothing", async () => {
  const tab = await mount(DEFAULT_CONDUCTOR);
  try {
    await act(async () => tab.strengths().find((option) => option.textContent === "Normal")!.click());
    expect(tab.sent).toEqual([]);
    await act(async () => tab.strengths().find((option) => option.textContent === "Thorough")!.click());
    await act(async () => tab.verification().querySelector<HTMLElement>('[role="switch"]')!.click());
    expect(tab.sent).toEqual([{ default_strength: "thorough" }, { other_vendor: false }]);
  } finally { await tab.unmount(); }
});
