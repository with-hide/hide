// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { FirstRunDevices } from "./FirstRunDevices";
import { TooltipProvider } from "./components/ui/tooltip";
import { Dialog, DialogContent } from "./components/ui/dialog";
import type { HerdrProfile, HerdrProfiles } from "./snapshot";
import { useShellStore } from "./store";
import type { DispatchFn } from "./ws";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });

const profile = (id: string, patch: Partial<HerdrProfile> = {}): HerdrProfile => ({ id, label: id, target: `${id}@server.example`, session: "work", enabled: true, added_as: null, problem: null, device_id: null, connect_state: "saved", connect_refusal: null, ...patch });

async function mount(profiles: HerdrProfiles, devices: string[] | null = []) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const saved = useShellStore.getState();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const back = vi.fn();
  const next = vi.fn();
  await act(async () => {
    useShellStore.setState({ revision: 1, rest: { ui_state: { first_run_choices: { devices } }, status: { herdr_profiles: profiles } } } as never);
    root.render(<TooltipProvider><Dialog open><DialogContent aria-describedby={undefined}><FirstRunDevices actions={actions} primary={{ current: null }} onBack={back} onNext={next} /></DialogContent></Dialog></TooltipProvider>);
  });
  return { events, back, next, unmount: async () => { await act(async () => root.unmount()); container.remove(); useShellStore.setState(saved, true); } };
}
const button = (name: string) => document.querySelector<HTMLButtonElement>(`[data-first-run-${name}]`)!;

it("continues without connecting when the operator leaves every saved profile unchecked", async () => {
  const view = await mount({ state: "ready", error: null, truncated: false, profiles: [profile("one")] });
  try {
    const checkbox = document.querySelector<HTMLButtonElement>("#herdr-profile-one")!;
    await act(async () => checkbox.click());
    await act(async () => checkbox.click());
    expect(checkbox.getAttribute("data-state")).toBe("unchecked");
    await act(async () => button("device-connect").click());
    expect(view.next).toHaveBeenCalledOnce();
    expect(view.events.some((event) => event.kind === "herdr_profile_connect")).toBe(false);
  } finally {
    await view.unmount();
  }
});

it("requires a selection and explicit Connect, and never selects a disabled or already-added profile", async () => {
  const view = await mount({ state: "ready", error: null, truncated: false, profiles: [profile("one"), profile("off", { enabled: false, problem: "disabled" }), profile("old", { added_as: "Already here", device_id: "existing", connect_state: "added" })] });
  expect(view.events).toEqual([]);
  expect(document.querySelectorAll('[role="checkbox"][data-state="checked"]')).toHaveLength(0);
  expect(document.querySelector<HTMLButtonElement>('#herdr-profile-off')?.disabled).toBe(true);
  expect(document.querySelector<HTMLButtonElement>('#herdr-profile-old')?.disabled).toBe(true);
  await act(async () => document.querySelector<HTMLButtonElement>('#herdr-profile-one')!.click());
  expect(view.events.map((event) => event.kind)).toEqual(["first_run_draft"]);
  await act(async () => button("device-connect").click());
  expect(view.events.at(-1)).toEqual({ schema_version: 2, kind: "herdr_profile_connect", payload: { profile_id: "one", host_consent: true } });
  expect(button("device-connect").disabled).toBe(true);
  await act(async () => button("back").click());
  expect(view.back).toHaveBeenCalledOnce();
  expect(view.events.filter((event) => event.kind === "herdr_profile_connect")).toHaveLength(1);
  await view.unmount();
});

it("keeps loading and a failed read distinct from an empty result, and Retry only reads saved configuration", async () => {
  for (const state of ["loading", "failed"] as const) {
    const view = await mount({ state, error: state === "failed" ? "unavailable" : null, profiles: [], truncated: false });
    expect(view.next).not.toHaveBeenCalled();
    expect(document.querySelector(`[data-first-run-devices="${state}"]`)).not.toBeNull();
    if (state === "failed") {
      await act(async () => button("devices-retry").click());
      expect(view.events.map((event) => event.kind)).toEqual(["herdr_profiles_list"]);
    }
    await view.unmount();
  }
  const empty = await mount({ state: "ready", error: null, profiles: [], truncated: false });
  expect(empty.next).toHaveBeenCalledOnce();
  await empty.unmount();
});

it("keeps a successful device while Retry sends only the failed selected profile", async () => {
  const rows = [profile("ok", { device_id: "dev-ok", added_as: "OK", connect_state: "connected" }), profile("bad", { device_id: "dev-bad", added_as: "Bad", connect_state: "failed" })];
  const view = await mount({ state: "ready", error: null, profiles: rows, truncated: false }, ["ok", "bad"]);
  await act(async () => useShellStore.setState({ revision: 2, rest: { ui_state: { first_run_choices: { devices: ["ok", "bad"] } }, status: { herdr_profiles: { state: "ready", profiles: rows }, remote: [{ target_id: "dev-ok", state: "connected" }, { target_id: "dev-bad", state: "error" }] }, navigator: { devices: [{ id: "dev-ok", kit: { agents: [], components: [], busy: false } }] } } } as never));
  expect(document.querySelector('[data-first-run-device="ok"]')?.getAttribute("data-device-state")).toBe("connected");
  expect(document.querySelector('[data-first-run-device="bad"]')?.getAttribute("data-device-state")).toBe("failed");
  await act(async () => button("device-retry=\"bad\"").click());
  expect(view.events.filter((event) => event.kind === "herdr_profile_connect").map((event) => event.payload)).toEqual([{ profile_id: "bad", host_consent: true }]);
  await view.unmount();
});

it("follows a recovered live connection over an earlier profile failure while keeping current failures actionable", async () => {
  const row = profile("recovered", { device_id: "dev", added_as: "Recovered", connect_state: "failed" });
  const profiles: HerdrProfiles = { state: "ready", error: null, profiles: [row], truncated: false };
  const view = await mount(profiles, [row.id]);
  const update = async (remoteState: string, failedKit = false) => act(async () => useShellStore.setState((current) => ({
    revision: current.revision + 1,
    rest: {
      ui_state: { first_run_choices: { devices: [row.id] } },
      status: { herdr_profiles: profiles, remote: [{ target_id: "dev", state: remoteState }] },
      navigator: { devices: [{ id: "dev", kit: { agents: [], components: [{ id: "helper", state: failedKit ? "failed" : "installed" }], busy: false } }] },
    },
  }) as never));
  const shown = () => document.querySelector('[data-first-run-device="recovered"]')?.getAttribute("data-device-state");
  await update("connecting");
  expect(shown()).toBe("connecting");
  expect(button("device-connect").disabled).toBe(true);
  await update("connected");
  expect(shown()).toBe("connected");
  expect(document.querySelector('[data-first-run-device-retry="recovered"]')).toBeNull();
  expect(document.querySelector<HTMLButtonElement>("#herdr-profile-recovered")?.disabled).toBe(true);
  await update("error");
  expect(shown()).toBe("failed");
  await update("connected", true);
  expect(shown()).toBe("failed");
  expect(document.querySelector('[data-first-run-device-retry="recovered"]')).not.toBeNull();
  expect(view.events).toEqual([]);
  await view.unmount();
});

it("words Hide's own refusal to connect a profile from its code", async () => {
  const row = profile("full", { connect_state: "failed", connect_refusal: "device_limit" });
  const view = await mount({ state: "ready", error: null, profiles: [row], truncated: false }, [row.id]);
  const text = document.querySelector('[data-first-run-device="full"]')?.textContent ?? "";
  expect(text).toContain("Hide already keeps as many devices as it can. Remove one you no longer use, then connect.");
  await view.unmount();
});

// PRD wsl-device B1, B2, B29: this PC's default WSL distribution is the step's row, checked until the
// operator unchecks it; only Connect sends it, and a WSL 1 one shows the command that switches it instead.
const wslRow = (distro: string, patch: Partial<HerdrProfile> = {}) => profile(`wsl:${distro}`, { label: `WSL ${distro}`, target: distro, session: "default", wsl_distro: distro, ...patch });

it("checks this PC's default WSL distribution and sends it only when Connect is pressed (B1, B2)", async () => {
  const view = await mount({ state: "ready", error: null, truncated: false, profiles: [wslRow("Ubuntu"), profile("one")] }, null);
  try {
    const row = document.querySelector('[data-first-run-wsl="Ubuntu"]')!;
    expect(row.textContent).toContain("WSL Ubuntu");
    expect(row.textContent).toContain("This PC (WSL)");
    // A WSL row is no Herdr profile: no target or session line, and no "Saved in Herdr".
    expect(row.textContent).not.toContain("Session");
    expect(row.textContent).not.toContain("Saved in Herdr");
    const box = document.querySelector<HTMLButtonElement>("#herdr-profile-wsl\\:Ubuntu")!;
    expect(box.getAttribute("aria-label")).toBe("WSL Ubuntu · This PC (WSL)");
    expect(box.getAttribute("data-state")).toBe("checked");
    expect(document.querySelector("#herdr-profile-one")?.getAttribute("data-state")).toBe("unchecked");
    expect(document.querySelector("[data-first-run-devices]")?.closest("[role=dialog]")?.textContent).toContain("Connect your devices to Hide");
    // Showing the step sends nothing: nothing is installed before the press.
    expect(view.events).toEqual([]);
    await act(async () => button("device-connect").click());
    expect(view.events).toEqual([{ schema_version: 2, kind: "herdr_profile_connect", payload: { profile_id: "wsl:Ubuntu", host_consent: true } }]);
  } finally {
    await view.unmount();
  }
});

it("keeps the WSL row unchecked once the operator unchecked it, across a reload (B1)", async () => {
  const first = await mount({ state: "ready", error: null, truncated: false, profiles: [wslRow("Ubuntu")] }, null);
  await act(async () => document.querySelector<HTMLButtonElement>("#herdr-profile-wsl\\:Ubuntu")!.click());
  expect(first.events.at(-1)).toEqual({ schema_version: 2, kind: "first_run_draft", payload: { devices: [] } });
  await first.unmount();
  // The core keeps that choice as an empty draft, which is not "never chosen".
  const again = await mount({ state: "ready", error: null, truncated: false, profiles: [wslRow("Ubuntu")] }, []);
  expect(document.querySelector("#herdr-profile-wsl\\:Ubuntu")?.getAttribute("data-state")).toBe("unchecked");
  await act(async () => button("device-connect").click());
  expect(again.next).toHaveBeenCalledOnce();
  expect(again.events.some((event) => event.kind === "herdr_profile_connect")).toBe(false);
  await again.unmount();
});

it("shows a WSL 1 or unfinished distribution as not connectable, with the command that switches WSL 1 (B29)", async () => {
  const view = await mount({ state: "ready", error: null, truncated: false, profiles: [wslRow("Legacy", { problem: "wsl1" })] }, null);
  const box = document.querySelector<HTMLButtonElement>("#herdr-profile-wsl\\:Legacy")!;
  expect(box.disabled).toBe(true);
  expect(box.getAttribute("data-state")).toBe("unchecked");
  expect(document.querySelector('[data-first-run-wsl-command="Legacy"]')?.textContent).toBe("wsl --set-version Legacy 2");
  expect(document.querySelector('[data-first-run-wsl-problem="wsl1"]')?.textContent).toContain("Runs on WSL 1");
  await act(async () => button("device-connect").click());
  expect(view.events.some((event) => event.kind === "herdr_profile_connect")).toBe(false);
  expect(view.next).toHaveBeenCalledOnce();
  await view.unmount();
  const fresh = await mount({ state: "ready", error: null, truncated: false, profiles: [wslRow("Fresh", { problem: "setup_unfinished" })] }, null);
  expect(document.querySelector<HTMLButtonElement>("#herdr-profile-wsl\\:Fresh")?.disabled).toBe(true);
  expect(document.querySelector('[data-first-run-wsl-problem="setup_unfinished"]')?.textContent).toBe("Finish setting up the distribution first");
  await fresh.unmount();
});

it("keeps the saved-profile step as it was on a machine without WSL", async () => {
  const view = await mount({ state: "ready", error: null, truncated: false, profiles: [profile("one")] }, null);
  expect(document.querySelector("[data-first-run-wsl]")).toBeNull();
  expect(document.querySelector("#herdr-profile-one")?.getAttribute("data-state")).toBe("unchecked");
  expect(document.querySelector("[data-first-run-devices]")?.closest("[role=dialog]")?.textContent).toContain("Bring your Herdr devices into Hide");
  await view.unmount();
});
