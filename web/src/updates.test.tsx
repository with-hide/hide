// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { HelpMenu } from "./components/help-menu";
import { TooltipProvider } from "./components/ui/tooltip";
import type { UpdatePhase, UpdateStatus } from "./host";
import { AboutTab } from "./settings/AboutTab";
import { checkedAgo } from "./updates";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });
afterEach(() => { document.body.innerHTML = ""; });

const status = (phase: UpdatePhase, more: Partial<UpdateStatus> = {}): UpdateStatus => ({ phase, prerelease: false, installOnQuit: true, ...more });
const actions = createActions(() => true);
const updateRow = () => document.querySelector<HTMLElement>("[data-settings-update]");
const action = () => document.querySelector<HTMLButtonElement>("[data-update-action]");
const dot = () => document.querySelector("[data-help-update-dot]");

function Screen() {
  return (
    <TooltipProvider>
      <HelpMenu actions={actions} />
      <AboutTab actions={actions} />
    </TooltipProvider>
  );
}

// One test, in order: the page subscribes to the bridge once for its life (`updates.ts`).
it("shows the update only in the desktop app: the Help dot when ready, and every state in About", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.useFakeTimers({ toFake: ["Date", "setInterval", "clearInterval"] });
  vi.setSystemTime(10 * 60_000);
  const root = createRoot(document.body.appendChild(document.createElement("div")));
  try {
    // A browser tab or the phone: no bridge, no row, no dot (B24).
    await act(async () => root.render(<Screen />));
    expect(updateRow()).toBeNull();
    expect(dot()).toBeNull();
    await act(async () => root.unmount());

    let deliver: (next: UpdateStatus) => void = () => undefined;
    const bridge = { onStatus: vi.fn((listener: (next: UpdateStatus) => void) => { deliver = listener; listener(status({ kind: "idle", checkedAt: null })); return () => undefined; }), check: vi.fn(), restart: vi.fn(), setPrerelease: vi.fn() };
    window.hideHost = { kind: "electron", platform: "darwin", updates: bridge } as never;
    const app = createRoot(document.body.appendChild(document.createElement("div")));
    await act(async () => app.render(<Screen />));
    expect(bridge.onStatus).toHaveBeenCalledTimes(1);
    expect(updateRow()?.dataset.settingsUpdate).toBe("idle");
    expect(updateRow()?.textContent).toContain("Not checked yet");
    await act(async () => action()!.click());
    expect(bridge.check).toHaveBeenCalledTimes(1);

    const show = (next: UpdateStatus) => act(async () => deliver(next));
    await show(status({ kind: "idle", checkedAt: 0 }));
    expect(updateRow()?.textContent).toContain("10 min. ago");
    // The time it says moves with the clock while About is open.
    await act(async () => { vi.advanceTimersByTime(60 * 60_000); });
    expect(updateRow()?.textContent).toContain("1 hr. ago");

    await show(status({ kind: "checking" }));
    expect(action()).toBeNull();
    await show(status({ kind: "downloading", version: "0.5.0", percent: 42 }));
    expect(updateRow()?.textContent).toContain("0.5.0");
    expect(updateRow()?.textContent).toContain("42%");
    expect(dot()).toBeNull();

    // Ready: the dot, the trigger says so, and Restart is About's action (B15, B25).
    await show(status({ kind: "ready", version: "0.5.0" }));
    expect(dot()).not.toBeNull();
    expect(document.querySelector("[data-help-menu-trigger]")?.getAttribute("aria-label")).toBe("Help · Update ready");
    expect(action()?.dataset.updateAction).toBe("restart");
    const onQuit = updateRow()!.textContent;
    await act(async () => action()!.click());
    expect(bridge.restart).toHaveBeenCalledTimes(1);
    await show(status({ kind: "ready", version: "0.5.0" }, { installOnQuit: false }));
    expect(updateRow()!.textContent).not.toBe(onQuit);

    for (const stage of ["check", "download"] as const) {
      await show(status({ kind: "failed", stage }));
      expect(action()?.dataset.updateAction).toBe("retry");
      expect(dot()).toBeNull();
    }

    // Off: no action, and the preview switch cannot be turned (B23, B27).
    for (const reason of ["development", "location"] as const) {
      await show(status({ kind: "off", reason }));
      expect(action()).toBeNull();
      expect(document.querySelector<HTMLButtonElement>("[data-update-prerelease]")!.disabled).toBe(true);
    }

    await show(status({ kind: "idle", checkedAt: 0 }));
    const preview = document.querySelector<HTMLButtonElement>("[data-update-prerelease]")!;
    expect(preview.getAttribute("aria-checked")).toBe("false");
    await act(async () => preview.click());
    expect(bridge.setPrerelease).toHaveBeenCalledWith(true);
    await act(async () => app.unmount());
  } finally {
    vi.useRealTimers();
    delete window.hideHost;
  }
});

it("says how long ago a check finished in the page's language", () => {
  const minute = 60_000;
  expect(checkedAgo("en", 0, 30_000)).toBe("now");
  expect(checkedAgo("en", 0, 5 * minute)).toBe("5 min. ago");
  expect(checkedAgo("en", 0, 3 * 60 * minute)).toBe("3 hr. ago");
  expect(checkedAgo("en", 0, 2 * 24 * 60 * minute)).toBe("2 days ago");
});
