import { describe, expect, it } from "vitest";
import { agentProfilesView } from "./agentProfiles";
import type { AgentUsage, LimitCell } from "./generated/hided-ws";
import type { AgentProfileWire, MachineAgentProfiles } from "./snapshot";

const NOW = 1_800_000_000_000;

function wire(id: string, fields: Partial<AgentProfileWire> = {}): AgentProfileWire {
  return { id, kind: "claude-code", name: id, account: `/accounts/${id}`, program: null, env_keys: [], version: 1, install: "own", folder: `/accounts/${id}`, hook: null, skill: null, ...fields };
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
