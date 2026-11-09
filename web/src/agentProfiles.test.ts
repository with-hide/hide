import { describe, expect, it } from "vitest";
import { agentProfilesView, startProfiles, signInStart } from "./agentProfiles";
import type { AgentUsage, LimitCell } from "./generated/hided-ws";
import type { AgentProfileWire, MachineAgentProfiles, SnapshotRest } from "./snapshot";
import type { StartTarget, StartTargets } from "./startTargets";

const NOW = 1_800_000_000_000;

function wire(id: string, fields: Partial<AgentProfileWire> = {}): AgentProfileWire {
  return { id, kind: "claude-code", name: id, account: `/accounts/${id}`, program: null, env_keys: [], version: 1, install: "own", folder: `/accounts/${id}`, hook: null, skill: null, startable: true, ...fields };
}

function weekly(used: number): LimitCell {
  return { state: "reading", freshness: "fresh", observed_at_ms: NOW, value: { windows: [{ kind: "weekly", scope: { kind: "all" }, outcome: { state: "open", used_percent: used, resets_at_s: NOW / 1000 + 3600 } }] } };
}

const machine = (profiles: AgentProfileWire[]): MachineAgentProfiles => ({ state: "listed", version: 1, profiles, defaults: {} });

describe("agentProfilesView", () => {
  it("shows each account's usage left once and a dash where another row holds the account (B23)", () => {
    const usage: AgentUsage = {
      agent: "claude",
      limits: weekly(60),
      consumption: { state: "unsupported" },
      profiles: [{ id: "work", limits: weekly(28.4) }, { id: "gateway", shared: true }, { id: "fresh", limits: { state: "loading" } }],
    };
    const view = agentProfilesView(machine([wire("work"), wire("gateway", { account: null, program: "claude-gateway" }), wire("fresh")]), "claude-code", "Claude Code", usage, NOW)!;
    expect(view.profiles.map((row) => [row.id, row.usageLeft])).toEqual([
      ["default", 40],
      ["work", 72],
      ["gateway", "shared"],
      ["fresh", null],
    ]);
  });

  it("reads each profile's sign-in and its program's folder warning from its machine (B10, B25)", () => {
    const view = agentProfilesView(machine([wire("new", { login: "signed_out" }), wire("odd", { login: "unknown" }), wire("gate", { wrapper_folder: true }), wire("old")]), "claude-code", "Claude Code")!;
    expect(view.profiles.map((row) => [row.id, row.login, row.wrapperFolder])).toEqual([
      ["default", null, false],
      ["new", "signed_out", false],
      ["odd", "unknown", false],
      ["gate", null, true],
      ["old", null, false],
    ]);
  });

  it("leaves a device's rows without usage, which is read on this machine alone", () => {
    const view = agentProfilesView(machine([wire("work")]), "claude-code", "Claude Code")!;
    expect(view.profiles.every((row) => row.usageLeft === null)).toBe(true);
  });
});

describe("signInStart", () => {
  const checkout = (deviceId: string, path: string, primary: boolean, disabled: StartTarget["disabled"] = null): StartTarget => ({
    key: `checkout:${deviceId}:${path}`, deviceId, label: path, disabled, kind: "checkout", checkoutPath: path, workspaceId: path, checkoutId: path, branch: "main", primary,
  });
  const targets = (defaultKey: string | null, ...items: StartTarget[]): StartTargets => ({
    groups: ["mac", "box"].map((deviceId) => ({ deviceId, deviceLabel: deviceId, connected: true, projects: [], items: items.filter((item) => item.deviceId === deviceId) })),
    frontDeviceId: "mac", defaultKey, currentKey: null,
  });

  it("starts the profile's agent where a start over Settings would, on the profile's own machine (B10)", () => {
    const all = targets("checkout:mac:/blog", checkout("mac", "/site", true), checkout("mac", "/blog", true), checkout("box", "/srv", false), checkout("box", "/api", true));
    expect(signInStart(all, "mac", "claude-code")).toEqual({ provider: "claude", checkoutPath: "/blog" });
    // Another machine's default is not this one's: its main checkout is.
    expect(signInStart(all, "box", "codex")).toEqual({ provider: "codex", checkoutPath: "/api" });
  });

  it("leaves it to the start panel where the machine has no checkout to start in", () => {
    expect(signInStart(targets(null, checkout("box", "/srv", true, "not_connected")), "box", "claude-code")).toBeNull();
    expect(signInStart(targets(null), "mac", "claude-code")).toBeNull();
  });
});

describe("startProfiles", () => {
  it("shows the core's final automatic selection without repeating its priority", () => {
    const rest = {
      navigator: { devices: [{ id: "mac", kit: { profiles: { ...machine([wire("p-side"), wire("p-new")]), defaults: { "claude-code": "p-side" } } } }] },
      status: { start_profiles: { project: { device_id: "mac", root: "/project", profiles: { "claude-code": "p-new" } } } },
    } as unknown as SnapshotRest;
    expect(startProfiles(rest, "mac", "project", "claude-code")?.picked).toBe("p-new");
  });
});
