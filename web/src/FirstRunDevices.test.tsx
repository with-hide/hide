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

const profile = (id: string, patch: Partial<HerdrProfile> = {}): HerdrProfile => ({ id, label: id, target: `${id}@server.example`, session: "work", enabled: true, added_as: null, problem: null, device_id: null, connect_state: "saved", connect_error: null, ...patch });

async function mount(profiles: HerdrProfiles, devices: string[] = []) {
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
  const rows = [profile("ok", { device_id: "dev-ok", added_as: "OK", connect_state: "connected" }), profile("bad", { device_id: "dev-bad", added_as: "Bad", connect_state: "failed", connect_error: "SSH refused" })];
  const view = await mount({ state: "ready", error: null, profiles: rows, truncated: false }, ["ok", "bad"]);
  await act(async () => useShellStore.setState({ revision: 2, rest: { ui_state: { first_run_choices: { devices: ["ok", "bad"] } }, status: { herdr_profiles: { state: "ready", profiles: rows }, remote: [{ target_id: "dev-ok", state: "connected" }, { target_id: "dev-bad", state: "error" }] }, navigator: { devices: [{ id: "dev-ok", kit: { agents: [], components: [], busy: false } }] } } } as never));
  expect(document.querySelector('[data-first-run-device="ok"]')?.getAttribute("data-device-state")).toBe("connected");
  expect(document.querySelector('[data-first-run-device="bad"]')?.getAttribute("data-device-state")).toBe("failed");
  await act(async () => button("device-retry=\"bad\"").click());
  expect(view.events.filter((event) => event.kind === "herdr_profile_connect").map((event) => event.payload)).toEqual([{ profile_id: "bad", host_consent: true }]);
  await view.unmount();
});

it("follows a recovered live connection over an earlier profile failure while keeping current failures actionable", async () => {
  const row = profile("recovered", { device_id: "dev", added_as: "Recovered", connect_state: "failed", connect_error: "Earlier SSH failure" });
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
