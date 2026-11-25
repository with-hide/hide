// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import type { AutomationCall } from "./automationCall";
import { AutomationForm } from "./AutomationForm";
import { TooltipProvider } from "./components/ui/tooltip";
import type { Automation, AutomationsSection } from "./snapshot";
import { useShellStore } from "./store";

// PRD automations B6, B7: the form keeps the core's reading of the schedule while only the name is typed.

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

const calls = vi.hoisted(() => [] as AutomationCall[]);
vi.mock("./automationCall", async (original) => ({
  ...(await original<typeof import("./automationCall")>()),
  // A stand-in core: a cron closer than five minutes is refused whatever the name, and so is a once date before 2026-11-25.
  callAutomations: async (call: AutomationCall) => {
    calls.push(call);
    if (call.call === "preview" && call.schedule.kind === "once" && call.schedule.date < "2026-11-25") return { ok: false, code: "schedule_once_past" };
    return call.call === "preview" && call.schedule.kind === "cron"
      ? { ok: false, code: "schedule_too_often" }
      : { ok: true, value: { reads_as: call.call === "preview" ? call.schedule : null, next_run_at_unix_ms: null, zone: null, own_folder: null } };
  },
}));

const automation = {
  id: "a1", name: "점검", task: "의존성 점검", schedule: { kind: "cron", expr: "*/4 * * * *" }, reads_as: { kind: "cron", expr: "*/4 * * * *" }, next_run_at_unix_ms: null,
  agent: { kind: "claude" }, folder: { path: "/srv/hide/automations/a1", default: true, worktree: false }, running: false, done: false, unseen: 0, records: [],
} as Automation;
const section: AutomationsSection = { items: [automation], unseen: 0 };

afterEach(() => {
  document.body.innerHTML = "";
  calls.length = 0;
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

/** Lets the preview's wait run out and its answer land. */
async function answer() {
  await act(async () => {
    vi.advanceTimersByTime(250);
    await Promise.resolve();
  });
  await act(async () => {
    await Promise.resolve();
  });
}

it("keeps the schedule's refusal, and [저장] off, while only the name is typed", async () => {
  vi.useFakeTimers();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  const actions = createActions(() => true);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const saved = useShellStore.getState();
  try {
    await act(async () => {
      useShellStore.setState({ connection: "live", rest: {} } as never);
      root.render(<TooltipProvider><AutomationForm section={section} automation={automation} actions={actions} /></TooltipProvider>);
    });
    await answer();
    const refusal = () => container.querySelector('[data-automation-refusal="schedule_too_often"]');
    expect(refusal()).not.toBeNull();

    const name = container.querySelector<HTMLInputElement>('[data-automation-field="name"]')!;
    const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
    await act(async () => {
      setValue.call(name, "점검 2");
      name.dispatchEvent(new Event("input", { bubbles: true }));
    });
    // Before the core reads the new name, the schedule's line still stands.
    expect(refusal()).not.toBeNull();
    expect(container.querySelector<HTMLButtonElement>("[data-automation-save]")?.disabled).toBe(true);

    await answer();
    expect(calls.filter((call) => call.call === "preview").at(-1)).toMatchObject({ name: "점검 2" });
    expect(refusal()).not.toBeNull();
  } finally {
    await act(async () => root.unmount());
    useShellStore.setState(saved, true);
  }
});

// mission-104 S-3: a done once automation's date has passed; an edit that keeps it is no refusal, so its name or task can be
// fixed, while a date the operator moves to the past is refused as any schedule is.
it("takes an edit of a done once automation that keeps its passed date, and refuses one moved to another past date", async () => {
  vi.useFakeTimers();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  const done = { ...automation, id: "o1", schedule: { kind: "once", date: "2026-11-20", time: "09:00" }, reads_as: { kind: "once", date: "2026-11-20", time: "09:00" }, done: true } as Automation;
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const saved = useShellStore.getState();
  try {
    await act(async () => {
      useShellStore.setState({ connection: "live", rest: {} } as never);
      root.render(<TooltipProvider><AutomationForm section={{ items: [done], unseen: 0 }} automation={done} actions={createActions(() => true)} /></TooltipProvider>);
    });
    await answer();
    const save = () => container.querySelector<HTMLButtonElement>("[data-automation-save]")!;
    expect(container.querySelector("[data-automation-refusal]")).toBeNull();
    expect(container.querySelector('[data-automation-preview="done"]')?.textContent).toBe("Fri 11-20 09:00 · once · Done");
    expect(save().disabled).toBe(false);

    const date = container.querySelector<HTMLInputElement>('[data-automation-field="date"]')!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(date, "2026-11-21");
      date.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await answer();
    expect(container.querySelector('[data-automation-refusal="schedule_once_past"]')).not.toBeNull();
    expect(save().disabled).toBe(true);
  } finally {
    await act(async () => root.unmount());
    useShellStore.setState(saved, true);
  }
});
