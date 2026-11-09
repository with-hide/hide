// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterAll, afterEach, expect, it, vi } from "vitest";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { forgetCatalogs } from "../agentPicker";
import { DEFAULT_CONDUCTOR } from "../conductorSettings";
import { clientI18n } from "../i18n/translator";
import type { ConductorSettings } from "../snapshot";
import { useShellStore } from "../store";
import type { DispatchFn } from "../ws";
import { ConductorTab } from "./ConductorTab";

// Settings › Mission's verification: one strength and one toggle, each edit
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

async function mount(settings: ConductorSettings, rest: Record<string, unknown> = {}) {
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
    useShellStore.setState({ connection: "live", rest: { ...rest, ui_state: { conductor_settings: settings } } } as never);
    root.render(<TooltipProvider><ConductorTab actions={actions} /></TooltipProvider>);
  });
  const verification = () => container.querySelector<HTMLElement>("[data-conductor-verification]")!;
  return {
    // Only the settings it sends; the page also asks for the start panel's model lists while it shows.
    get sent() { return events.filter((event) => event.kind === "conductor_settings_set").map((event) => event.payload); },
    verification,
    strengths: () => [...verification().querySelectorAll<HTMLElement>("[data-conductor-strength-option]")],
    cleanup: () => container.querySelector<HTMLElement>('[data-conductor-cleanup] [role="switch"]')!,
    async unmount() {
      await act(async () => root.unmount());
      container.remove();
      useShellStore.setState(saved, true);
      await clientI18n.changeLanguage(language);
      vi.unstubAllGlobals();
    },
  };
}

it("offers one verification strength beside the candidates, and says what the chosen strength has the mission do", async () => {
  const tab = await mount(DEFAULT_CONDUCTOR);
  try {
    const group = tab.verification();
    expect(group.querySelector("h3")?.textContent).toBe("Verification");
    // One control: the strength's three choices, nothing else.
    expect(tab.strengths().map((option) => [option.textContent, option.getAttribute("data-state")])).toEqual([["Fast", "off"], ["Normal", "on"], ["Thorough", "off"]]);
    expect(group.querySelectorAll('[role="switch"], [role="combobox"], input')).toHaveLength(0);
    expect(group.querySelector("[data-conductor-strength-terms]")?.textContent).toContain("At most twice, only what changed");
    expect(group.textContent).toContain("A change applies from the next mission.");
  } finally { await tab.unmount(); }
  const thorough = await mount({ ...DEFAULT_CONDUCTOR, default_strength: "thorough" });
  try {
    expect(thorough.strengths().find((option) => option.getAttribute("data-state") === "on")?.textContent).toBe("Thorough");
    expect(thorough.verification().querySelector("[data-conductor-strength-terms]")?.textContent).toContain("At most three times");
  } finally { await thorough.unmount(); }
});

it("sends each edit as its own field, so a second edit before the first one's answer keeps it, and the strength already chosen sends nothing", async () => {
  const tab = await mount(DEFAULT_CONDUCTOR);
  try {
    await act(async () => tab.strengths().find((option) => option.textContent === "Normal")!.click());
    expect(tab.sent).toEqual([]);
    await act(async () => tab.strengths().find((option) => option.textContent === "Thorough")!.click());
    await act(async () => tab.cleanup().click());
    expect(tab.sent).toEqual([{ default_strength: "thorough" }, { clean_merged: false }]);
  } finally { await tab.unmount(); }
});

it("puts the merged-worktree cleanup switch right under the cap and the scope, on by default, with the line that says what it removes and keeps, and sends only its own field", async () => {
  const tab = await mount(DEFAULT_CONDUCTOR);
  try {
    const rows = [...document.querySelectorAll<HTMLElement>("[data-settings-conductor] > :first-child > * > *")];
    const cleanup = document.querySelector<HTMLElement>("[data-conductor-cleanup]")!;
    expect(rows.slice(-1)[0]).toBe(cleanup);
    expect(rows.length).toBe(3);
    expect(cleanup.textContent).toContain("Clean up a worktree once merged");
    expect(cleanup.textContent).toContain("Removes the worktree and build cache of a branch whose PR merged. One that still holds files you were changing, or is in use, stays.");
    const toggle = cleanup.querySelector<HTMLElement>('[role="switch"]')!;
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    await act(async () => toggle.click());
    expect(tab.sent).toEqual([{ clean_merged: false }]);
  } finally { await tab.unmount(); }
  const off = await mount({ ...DEFAULT_CONDUCTOR, clean_merged: false });
  try {
    expect(document.querySelector("[data-conductor-cleanup] [role=switch]")?.getAttribute("aria-checked")).toBe("false");
  } finally { await off.unmount(); }
});

it("leaves a purpose field focused on the Enter that confirms an input method's syllable, so the syllable is not committed twice, and the next Enter saves it", async () => {
  const tab = await mount({ ...DEFAULT_CONDUCTOR, candidates: [{ agent: "claude", purpose: "" }] });
  try {
    const field = document.querySelector<HTMLInputElement>("[data-candidate-purpose]")!;
    await act(async () => field.focus());
    // Chromium marks the confirming Enter as composing; Safari sends it after compositionend with only keyCode 229.
    await act(async () => field.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, isComposing: true })));
    await act(async () => field.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, keyCode: 229 })));
    expect(document.activeElement).toBe(field);
    await act(async () => field.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
    expect(document.activeElement).not.toBe(field);
  } finally { await tab.unmount(); }
});

/** Opens a select from the keyboard, as Radix takes it in jsdom, and reads each option as it is drawn. */
async function openMenu(trigger: HTMLElement): Promise<string[]> {
  trigger.focus();
  await act(async () => { trigger.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
  return [...document.querySelectorAll<HTMLElement>("[role=option]")].map((option) => option.textContent ?? "");
}

// PRD agent-profiles-phase2 B7, B14, B15: the agent cell is this machine's
// agent items, the model cell that item's own list, and a row whose profile
// is gone is dimmed and its item is not offered again.
it("lists this machine's agent items in a candidate's agent cell, offers the picked item's own models, and dims a row whose profile is gone", async () => {
  forgetCatalogs();
  const profile = (id: string, name: string) => ({ id, kind: "claude-code", name, account: `/accounts/${name}`, program: null, env_keys: [], version: 1, install: "own", startable: true });
  const rest = {
    navigator: { devices: [{ id: "mac", kind: "local", kit: { agents: [{ id: "claude-code", availability: "available" }], profiles: { state: "listed", version: 2, profiles: [profile("p-gate", "opengateway"), profile("p-work", "work")], defaults: {} } } }] },
    status: { background_ai: { provider: "claude", chosen: false, unavailable_reason: null, providers: [{
      id: "claude", label: "Claude Code", agent: "claude-code", state: "ready", headline: "", message: null, model: "", models: ["opus", "sonnet"], models_unavailable_reason: null,
      profile_models: [{ profile: "p-gate", version: 1, models: ["deepseek-v4", "glm-5"], model_labels: { "deepseek-v4": "DeepSeek V4" }, models_fixed: false, models_unavailable_reason: null }],
    }] } },
  };
  const tab = await mount({ ...DEFAULT_CONDUCTOR, candidates: [
    { agent: "claude", profile: "p-gate", purpose: "gateway" },
    { agent: "claude", purpose: "changes" },
    { agent: "claude", profile: "p-old", model: "opus", purpose: "old" },
  ] }, rest);
  try {
    const rows = [...document.querySelectorAll<HTMLElement>("[data-conductor-candidate]")];
    const agentCell = (row: HTMLElement) => row.querySelector<HTMLElement>("[data-candidate-item]")!;
    expect(rows.map((row) => agentCell(row).textContent)).toEqual(["Claude Code · opengateway", "Claude Code", "Claude Code · Missing profile"]);
    // The gone row admits no worker: dimmed, saying why in place of its model, and its item is in no list.
    expect(rows.map((row) => row.dataset.candidateGone ?? null)).toEqual([null, null, "true"]);
    expect(rows[2]!.querySelector("[data-candidate-missing]")?.textContent).toBe("Its profile was deleted");
    expect(rows[2]!.querySelector("[data-candidate-model]")).toBeNull();
    const items = await openMenu(agentCell(rows[0]!));
    expect(items.filter((item) => item.startsWith("Claude Code"))).toEqual(["Claude Code", "Claude Code · opengateway", "Claude Code · work"]);
    await act(async () => { document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); });
    // The opengateway row's model cell is that item's list, by the names its agent shows; the built-in row's is the agent's own.
    expect(await openMenu(rows[0]!.querySelector<HTMLElement>("[data-candidate-model]")!)).toEqual(["Default model", "DeepSeek V4", "glm-5"]);
    await act(async () => { document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); });
    expect(await openMenu(rows[1]!.querySelector<HTMLElement>("[data-candidate-model]")!)).toEqual(["Default model", "opus", "sonnet"]);
  } finally { await tab.unmount(); }
});

it("sends a picked item as its agent and profile, and the built-in item with no profile, as a row before profiles was kept", async () => {
  forgetCatalogs();
  const rest = { navigator: { devices: [{ id: "mac", kind: "local", kit: { agents: [], profiles: { state: "listed", version: 1, profiles: [{ id: "p-work", kind: "claude-code", name: "work", account: "/a", program: null, env_keys: [], version: 1, install: "own", startable: true }], defaults: {} } } }] } };
  const tab = await mount({ ...DEFAULT_CONDUCTOR, candidates: [{ agent: "claude", model: "opus", purpose: "changes" }] }, rest);
  try {
    const cell = document.querySelector<HTMLElement>("[data-candidate-item]")!;
    await openMenu(cell);
    await act(async () => [...document.querySelectorAll<HTMLElement>("[role=option]")].find((option) => option.textContent === "Claude Code · work")!.click());
    expect(tab.sent.at(-1)).toEqual({ candidates: [{ agent: "claude", profile: "p-work", purpose: "changes" }] });
  } finally { await tab.unmount(); }
});
