// What the Agents tab shows for each machine (PRD settings-cleanup D-06 to
// D-13, B8 to B20, B67): the supported agents in the kit's order, which
// of them the machine has, and the one line each row owes the operator. Every
// value is the kit snapshot's; nothing here decides what an agent supports.

import { AGENT_ADAPTERS, agentAdapter } from "../agentAdapters";
import type { Device, KitAgent, KitPiece, RemoteStatus } from "../snapshot";
import { deviceState, kitPartNeedsReinstall } from "../settings";
import { hostBridge } from "../host";

/** The supported order and trusted vendor links come from this build's contract. */
export const SUPPORTED_AGENTS = AGENT_ADAPTERS.map((row) => row.id);

export function docsUrl(agentId: string): string | null {
  return agentAdapter(agentId)?.doc_url ?? null;
}

export function installDocUrl(agentId: string): string | null {
  return agentAdapter(agentId)?.install_url ?? null;
}

/**
 * The supported agents of one machine in the supported order. An older
 * helper that still lists agents Hide dropped shows none of them: the list is
 * the supported set, whatever a machine reports.
 */
export function supportedAgents(agents: readonly KitAgent[] | undefined): KitAgent[] {
  const byId = new Map((agents ?? []).map((agent) => [agent.id, agent]));
  return AGENT_ADAPTERS.flatMap((row) => {
    const agent = byId.get(row.id);
    return agent ? [{ ...agent, label: row.label }] : [];
  });
}

/**
 * Whether the machine has the agent: its program was found (D-07, B9). An
 * agent the operator switched on, whose program then went away, keeps its row
 * and its switch so it can be turned off. An agent that is on only because
 * Claude Code, Codex and agy are on by default is not kept: its program is not
 * there, so it is not installed (`chosen` is the record holding the
 * operator's own choice).
 */
export function agentInstalled(agent: KitAgent): boolean {
  return agent.availability !== "not_installed" || (agent.enabled && agent.chosen === true);
}

/**
 * An agent installed on the machine with nothing Hide can attach to on its
 * system (the kit's `unsupported_system`): it has no switch and no install
 * link, and says which system it is not on yet (PRD first-run-onboarding
 * D-25, B49).
 */
export function agentUnsupportedHere(agent: KitAgent): boolean {
  return agent.availability === "unsupported_system";
}

/** A system's name as people say it, from `process.platform` or a device helper's `macos aarch64`; null when unknown. */
export function systemName(platform: string | null | undefined): string | null {
  if (!platform) return null;
  if (/mac|darwin/i.test(platform)) return "macOS";
  if (/win/i.test(platform)) return "Windows";
  if (/linux/i.test(platform)) return "Linux";
  return null;
}

/** The system a machine runs: a device's as its helper reported it, this machine's as the window it is shown in runs on. */
export function machineSystem(device: Pick<Device, "kind" | "host"> | null | undefined): string | null {
  if (device?.host?.platform) return systemName(device.host.platform);
  if (device && device.kind === "remote") return null;
  const bridge = hostBridge();
  if (bridge) return systemName(bridge.platform);
  return typeof navigator === "undefined" ? null : systemName(navigator.platform || navigator.userAgent);
}

/** One machine of the switch at the top of the tab: This Mac first, then each device in the Devices order. */
export type AgentMachine = {
  device: Device;
  installed: KitAgent[];
  notInstalled: KitAgent[];
  /**
   * Why the machine shows no list (B11): its kit does not run, the device is
   * still connecting, was switched off in Devices, or cannot be reached. The
   * words are the Devices tab's own (`deviceState`), so the tabs cannot disagree.
   */
  blocked: "unreachable" | "connecting" | "disabled" | "unavailable" | null;
  /** The kit has not answered for this machine yet, or a read the core asked is under way. */
  unread: boolean;
  /**
   * The machine's kit answered and no read runs, but it listed no agent: a
   * pass that could not run (an account another Hide held, a retirement it
   * refused) says nothing about them, so the tab says so and offers Check
   * again rather than wait for a read nobody asked.
   */
  unlisted: boolean;
};

export function agentMachines(devices: readonly Device[], remote?: readonly RemoteStatus[]): AgentMachine[] {
  return devices.map((device) => {
    const kit = device.kit;
    const agents = supportedAgents(kit?.agents);
    // A device that is not ready answers nothing, so its last list is not a fact to act on (B11).
    const { state } = deviceState(device, remote?.find((row) => row.target_id === device.id));
    const gone = state === "connecting" || state === "disabled" ? state : state === "unavailable" ? "unreachable" : null;
    const silent = agents.length === 0 && !kit?.unavailable;
    const reading = kit?.busy === true || kit?.checking === true || (kit?.components.length ?? 0) === 0;
    return {
      device,
      installed: agents.filter(agentInstalled),
      notInstalled: agents.filter((agent) => !agentInstalled(agent)),
      blocked: gone ?? (kit?.unavailable ? "unavailable" : null),
      unread: silent && reading,
      unlisted: silent && !reading,
    };
  });
}

/**
 * What Check again says when the machine's last read failed (B10): the core
 * sends a code, never prose, and a code this build does not know reads as the
 * general line (the cause is in the diagnostic log, B68).
 */
export function checkFailedReason(code: string): "agents.checkReason.unreachable" | "agents.checkReason.timed_out" | "agents.checkReason.failed" {
  if (code === "unreachable") return "agents.checkReason.unreachable";
  if (code === "timed_out") return "agents.checkReason.timed_out";
  return "agents.checkReason.failed";
}

/** The pieces of one agent that a Reinstall would repair, the failed ones first (B13, B20). */
/** An agent's piece by name; the hook piece is named by what the contract says it is: OpenCode's is a plugin, Pi's and omp's an extension. */
export type AgentPart = "skill" | "hook" | "plugin" | "extension" | "herdr";

export type AgentProblem = { part: AgentPart; piece: KitPiece };

export function hookPart(agent: KitAgent): "hook" | "plugin" | "extension" {
  const kind = agentAdapter(agent.id)?.hook_kind;
  return kind === "plugin" || kind === "extension" ? kind : "hook";
}

export function agentProblems(agent: KitAgent): AgentProblem[] {
  if (!agent.enabled) return [];
  const pieces: AgentProblem[] = [{ part: "skill", piece: agent.skill }];
  if (agent.hook) pieces.push({ part: hookPart(agent), piece: agent.hook });
  if (agent.herdr) pieces.push({ part: "herdr", piece: agent.herdr });
  const broken = pieces.filter(({ piece }) => kitPartNeedsReinstall(piece));
  return [...broken.filter(({ piece }) => piece.state === "failed"), ...broken.filter(({ piece }) => piece.state !== "failed")];
}

/**
 * Why Hide could not put an agent's hook in, for an agent that is on and found (PRD opencode-plugin B14): the
 * agent has not made its configuration folder yet, or a file of that name is not Hide's. It is no repair, so it
 * is a quiet line without Reinstall, and an agent whose program is gone says that elsewhere.
 */
export function agentHookWait(agent: KitAgent): AgentProblem | null {
  if (!agent.enabled || agent.availability !== "available") return null;
  const hook = agent.hook;
  if (hook?.state !== "absent" || !hook.why) return null;
  return { part: hookPart(agent), piece: hook };
}

/**
 * A switched-off agent whose removal did not finish: something of Hide's is
 * still there, and its row says so instead of reading Off (B14).
 */
export function agentLeftover(agent: KitAgent): KitPiece | null {
  if (agent.enabled) return null;
  return [agent.skill, agent.hook, agent.herdr].find((piece): piece is KitPiece => piece?.state === "failed") ?? null;
}

/**
 * The status an agent that is on wears (B16, B19): how many of its sessions
 * run now, or Ready when it is set up and has none. A Basic agent, one that
 * is off and one whose sessions were not read yet wear none.
 */
export type AgentStatus = { kind: "none" } | { kind: "ready" } | { kind: "sessions"; count: number };

export function agentStatus(agent: KitAgent): AgentStatus {
  // An agent whose program is gone has no sessions and is never Ready (B9).
  if (!agent.enabled || agent.partial || agent.sessions == null || agent.availability !== "available") return { kind: "none" };
  if (agent.sessions === 0) return { kind: "ready" };
  return { kind: "sessions", count: agent.sessions };
}
