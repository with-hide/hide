import { emptyScope } from "../../test/legacyAgentScope";
import { describe, expect, it } from "vitest";
import type { Device, KitAgent, KitPiece } from "../snapshot";
import { agentHookWait, agentInstalled, agentLeftover, agentUnsupportedHere, systemName, checkFailedReason, docsUrl, agentMachines, agentProblems, agentStatus, installDocUrl, supportedAgents, SUPPORTED_AGENTS } from "./agentRows";

const piece = (state: KitPiece["state"], reason: string | null = null): KitPiece => ({ state, reason, location: null });
const agent = (id: string, patch: Partial<KitAgent> = {}): KitAgent => ({
  id,
  label: id,
  availability: "available",
  enabled: true,
  chosen: false,
  skill: piece("installed"),
  hook: null,
  doc_url: "https://example.test",
  ...patch,
});
const device = (patch: Partial<Device> & { agents?: KitAgent[]; unavailable?: string | null }): Device => {
  const { agents = [], unavailable = null, ...rest } = patch;
  return { agent_scope: emptyScope(),
    id: "studio",
    label: "Studio",
    kind: "remote",
    state: "ready",
    message: null,
    ssh_alias: "studio",
    agent_count: 0,
    test: null,
    kit: { unavailable, busy: false, components: [], agents, offers_reinstall: false, shares_account_with: null },
    ...rest,
  } as Device;
};

describe("the agents of a machine (B8, B9, D-06)", () => {
  it("lists the seven supported agents in their order and nothing a machine still reports from before", () => {
    const reported = ["cursor", "kiro", "gemini-cli", "omp", "claude-code", "amp", "pi", "codex"].map((id) => agent(id));
    expect(supportedAgents(reported).map((row) => row.id)).toEqual(["claude-code", "codex", "pi", "omp"]);
    expect(SUPPORTED_AGENTS).toEqual(["claude-code", "codex", "grok", "opencode", "pi", "omp", "antigravity"]);
  });

  it("calls an agent installed only when its program was found; an agent the operator switched on keeps its switch after its program goes (B9)", () => {
    // The program is there.
    expect(agentInstalled(agent("codex"))).toBe(true);
    // The program is not there and nothing is on: not installed.
    expect(agentInstalled(agent("codex", { availability: "not_installed", enabled: false }))).toBe(false);
    // Installed on a system Hide cannot attach to yet: listed with the installed ones, without a switch (B49).
    expect(agentInstalled(agent("codex", { availability: "unsupported_system", enabled: false }))).toBe(true);
    expect(agentUnsupportedHere(agent("codex", { availability: "unsupported_system" }))).toBe(true);
    // On only because Claude Code, Codex and agy are on by default (no recorded choice): not installed, and never Ready.
    const byDefault = agent("codex", { availability: "not_installed", enabled: true, chosen: false, sessions: 0 });
    expect(agentInstalled(byDefault)).toBe(false);
    expect(agentInstalled(agent("codex", { availability: "not_installed", enabled: true }))).toBe(false);
    expect(agentStatus(byDefault)).toEqual({ kind: "none" });
    // The operator switched it on and its program then went away: it stays listed so it can be switched off.
    const gone = agent("codex", { availability: "not_installed", enabled: true, chosen: true, sessions: 0 });
    expect(agentInstalled(gone)).toBe(true);
    expect(agentStatus(gone)).toEqual({ kind: "none" });
  });

  it("names a machine's system the way people say it, from Node's platform or a device helper's report (D-25)", () => {
    expect(systemName("win32")).toBe("Windows");
    expect(systemName("darwin")).toBe("macOS");
    expect(systemName("macos aarch64")).toBe("macOS");
    expect(systemName("linux x86_64")).toBe("Linux");
    expect(systemName(null)).toBeNull();
    expect(systemName("freebsd")).toBeNull();
  });

  it("splits each machine into the installed and the not installed, This Mac first, and says why a machine has no list", () => {
    const set = [agent("claude-code"), agent("codex", { enabled: false }), agent("pi", { availability: "not_installed", enabled: false })];
    const local = device({ id: "local", label: "mini", kind: "local", state: "local", agents: set });
    const gone = device({ id: "box", label: "Box", state: "unavailable", agents: set });
    const noKit = device({ id: "bare", label: "Bare", unavailable: "A daemon outside the package installs nothing" });
    const unread = device({ id: "new", label: "New" });
    const machines = agentMachines([local, gone, noKit, unread]);
    expect(machines.map((row) => row.device.id)).toEqual(["local", "box", "bare", "new"]);
    expect(machines[0]).toMatchObject({ blocked: null, unread: false });
    expect(machines[0]?.installed.map((row) => row.id)).toEqual(["claude-code", "codex"]);
    expect(machines[0]?.notInstalled.map((row) => row.id)).toEqual(["pi"]);
    expect(machines[1]?.blocked).toBe("unreachable");
    expect(machines[2]).toMatchObject({ blocked: "unavailable", unread: false });
    expect(machines[3]).toMatchObject({ blocked: null, unread: true });
  });

  it("reads as checking only while the core reads or before the first answer, and a machine that answered with no agent says so", () => {
    const failed = { id: "cli", state: "failed", reason: "another Hide was still changing this account's kit after 90 seconds", location: null } as never;
    const answered = (over: Partial<NonNullable<Device["kit"]>>): Device => {
      const base = device({ id: "box" });
      return { ...base, kit: { ...base.kit!, components: [failed], ...over } };
    };
    expect(agentMachines([answered({})])[0]).toMatchObject({ unread: false, unlisted: true });
    expect(agentMachines([answered({ checking: true })])[0]).toMatchObject({ unread: true, unlisted: false });
    expect(agentMachines([answered({ busy: true })])[0]).toMatchObject({ unread: true, unlisted: false });
    // Never answered: the tab's own read is on its way.
    expect(agentMachines([device({ id: "new" })])[0]).toMatchObject({ unread: true, unlisted: false });
    // A list is a list, and a kit that cannot run says why instead.
    expect(agentMachines([{ ...answered({}), kit: { ...answered({}).kit!, agents: [agent("claude-code")] } }])[0]).toMatchObject({ unread: false, unlisted: false });
    expect(agentMachines([answered({ unavailable: "A daemon outside the package installs nothing" })])[0]).toMatchObject({ blocked: "unavailable", unread: false, unlisted: false });
  });

  it("says why a machine shows no list in the Devices tab's own words: connecting is not unreachable, and Switched off points at Devices (B11)", () => {
    const set = [agent("claude-code")];
    const down = device({ id: "box", state: "unavailable", agents: set });
    const remote = (state: "not_connected" | "connecting" | "ready" | "failed"): never => [{ target_id: "box", state }] as never;
    expect(agentMachines([down], remote("connecting"))[0]?.blocked).toBe("connecting");
    expect(agentMachines([down], remote("not_connected"))[0]?.blocked).toBe("connecting");
    expect(agentMachines([down], remote("failed"))[0]?.blocked).toBe("unreachable");
    expect(agentMachines([down])[0]?.blocked).toBe("unreachable");
    expect(agentMachines([{ ...down, state: "disabled" } as Device], remote("failed"))[0]?.blocked).toBe("disabled");
    expect(agentMachines([{ ...down, state: "ready" } as Device], remote("failed"))[0]?.blocked).toBeNull();
  });

  it("opens each agent's installation guide and none for an agent Hide does not support", () => {
    for (const id of SUPPORTED_AGENTS) expect(installDocUrl(id)).toMatch(/^https:\/\//);
    expect(installDocUrl("kiro")).toBeNull();
    expect(installDocUrl("cursor")).toBeNull();
    expect(installDocUrl("gemini-cli")).toBeNull();
    expect(installDocUrl("omp")).toBe("https://omp.sh/docs/quickstart");
    expect(docsUrl("omp")).toBe("https://omp.sh/docs/skills");
    // The Docs link is this build's table, never a URL a machine reports (a device's helper supplies `doc_url`).
    for (const id of SUPPORTED_AGENTS) expect(docsUrl(id)).toMatch(/^https:\/\//);
    expect(docsUrl("kiro")).toBeNull();
  });

  it("words a failed check from a code and reads a code it does not know as the general line (B10, B68)", () => {
    expect(checkFailedReason("unreachable")).toBe("agents.checkReason.unreachable");
    expect(checkFailedReason("timed_out")).toBe("agents.checkReason.timed_out");
    expect(checkFailedReason("something_new")).toBe("agents.checkReason.failed");
  });
});

describe("what a row says (B13, B14, B16, B19, B20)", () => {
  it("names the pieces a Reinstall would repair, the failed ones first, only for an agent that is on (B13, B20)", () => {
    const row = agent("codex", { hook: piece("removed"), herdr: piece("failed", "herdr refused"), skill: piece("outdated") });
    expect(agentProblems(row).map(({ part }) => part)).toEqual(["herdr", "skill", "hook"]);
    expect(agentProblems(agent("codex", { hook: piece("absent", "no ~/.codex"), skill: piece("installed"), herdr: piece("off") }))).toEqual([]);
    expect(agentProblems(agent("codex", { enabled: false, skill: piece("removed") }))).toEqual([]);
    // A row from a helper whose build predates the Herdr piece has nothing there to fail.
    expect(agentProblems(agent("grok", { herdr: null }))).toEqual([]);
    // OpenCode's hook piece is Hide's plugin, Pi's and omp's Hide's extension, and each row says so.
    expect(agentProblems(agent("opencode", { hook: piece("outdated", "edited") })).map(({ part }) => part)).toEqual(["plugin"]);
    expect(agentProblems(agent("pi", { hook: piece("outdated", "edited") })).map(({ part }) => part)).toEqual(["extension"]);
    expect(agentProblems(agent("omp", { hook: piece("removed") })).map(({ part }) => part)).toEqual(["extension"]);
  });

  it("says why an agent's hook is not in while the agent is on and found, without a repair (opencode-plugin B14)", () => {
    const waiting = agentHookWait(agent("opencode", { hook: piece("absent", "OpenCode has not created ~/.config/opencode yet") }));
    expect(waiting).toEqual({ part: "plugin", piece: piece("absent", "OpenCode has not created ~/.config/opencode yet") });
    expect(agentProblems(agent("opencode", { hook: piece("absent", "OpenCode has not created ~/.config/opencode yet") }))).toEqual([]);
    expect(agentHookWait(agent("opencode", { enabled: false, hook: piece("absent", "no folder") }))).toBeNull();
    expect(agentHookWait(agent("opencode", { availability: "not_installed", hook: piece("absent", "not found") }))).toBeNull();
    expect(agentHookWait(agent("pi", { hook: piece("absent", "Pi has not created ~/.pi/agent yet") }))?.part).toBe("extension");
    expect(agentHookWait(agent("cursor", { hook: piece("absent") }))).toBeNull();
    expect(agentHookWait(agent("cursor", { hook: piece("installed") }))).toBeNull();
  });

  it("keeps a switched-off agent whose removal did not finish from reading Off (B14)", () => {
    expect(agentLeftover(agent("pi", { enabled: false, skill: piece("failed", "still there") }))?.reason).toBe("still there");
    expect(agentLeftover(agent("pi", { enabled: false }))).toBeNull();
    expect(agentLeftover(agent("pi", { enabled: true, skill: piece("failed") }))).toBeNull();
  });

  it("says Ready, how many sessions run, and nothing for the rest", () => {
    expect(agentStatus(agent("codex", { sessions: 0 }))).toEqual({ kind: "ready" });
    expect(agentStatus(agent("codex", { sessions: 3 }))).toEqual({ kind: "sessions", count: 3 });
    expect(agentStatus(agent("codex", { enabled: false, sessions: 3 }))).toEqual({ kind: "none" });
    expect(agentStatus(agent("grok", { partial: true, sessions: null }))).toEqual({ kind: "none" });
    expect(agentStatus(agent("grok", { partial: true, sessions: 2 }))).toEqual({ kind: "none" });
    expect(agentStatus(agent("codex", { sessions: null }))).toEqual({ kind: "none" });
  });
});
