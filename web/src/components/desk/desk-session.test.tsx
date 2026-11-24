// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { AgentSleep } from "../../snapshot";
import { SessionSleepingFoot } from "./desk-session";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });

// The foot that stands in a sleeping session's input box beside Desk (PRD desk-2-ui B43) says what the sleeping pane
// says (`SleepBody`): asleep, waking, or a wake that failed with its reason and both ways on.

let mounted: ReturnType<typeof createRoot>[] = [];

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
});

afterEach(async () => {
  for (const root of mounted) await act(async () => root.unmount());
  mounted = [];
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

async function foot(sleep: AgentSleep | null, state = "unknown") {
  const wake = vi.fn();
  const fresh = vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  mounted.push(root);
  await act(async () => root.render(<SessionSleepingFoot title="Agent two" sleep={sleep} state={state} onWake={sleep?.state === "waking" ? null : wake} onFresh={fresh} />));
  const button = (selector: string) => container.querySelector<HTMLButtonElement>(selector);
  return { container, wake, fresh, button };
}

it("offers Wake agent for an agent asleep in its pane", async () => {
  const { container, wake, button } = await foot({ state: "sleeping", since_unix_ms: 0 });
  expect(container.textContent).toContain("Agent two");
  expect(button("[data-desk-session-wake]")?.textContent).toBe("Wake agent");
  expect(button("[data-desk-session-wake-fresh]")).toBeNull();
  await act(async () => button("[data-desk-session-wake]")!.click());
  expect(wake).toHaveBeenCalledTimes(1);
});

it("says a wake is under way and offers nothing to press while it runs", async () => {
  const { container, button } = await foot({ state: "waking", since_unix_ms: Date.now() });
  expect(container.querySelector('[role="status"]')?.textContent).toBe("Resuming the conversation");
  expect(button("[data-desk-session-wake]")).toBeNull();
});

it("says a failed wake, why, and offers Retry and a new session as the pane does", async () => {
  const { container, wake, fresh, button } = await foot({ state: "failed", since_unix_ms: 0, reason: "The conversation file is gone" });
  expect(container.querySelector("[data-desk-session-sleeping]")?.getAttribute("data-desk-session-sleeping")).toBe("failed");
  expect(container.textContent).toContain("Couldn’t resume this conversation");
  expect(container.querySelector('[data-desk-session-wake-reason]')?.textContent).toBe("The conversation file is gone");
  expect(button("[data-desk-session-wake]")?.textContent).toBe("Retry");
  await act(async () => button("[data-desk-session-wake]")!.click());
  await act(async () => button("[data-desk-session-wake-fresh]")!.click());
  expect(wake).toHaveBeenCalledTimes(1);
  expect(fresh).toHaveBeenCalledTimes(1);
});

it("draws a session whose sleep closed its pane by its phase, with Wake agent when the core offers it", async () => {
  const { container, button } = await foot(null, "sleeping");
  expect(container.querySelector("[data-desk-session-sleeping]")?.getAttribute("data-desk-session-sleeping")).toBe("sleeping");
  expect(button("[data-desk-session-wake]")?.textContent).toBe("Wake agent");
});
