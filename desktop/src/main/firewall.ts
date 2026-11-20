// The Windows firewall rules that let machines on the same network reach
// this PC's Hide directly (PRD device-connect-iroh D-38, B32). A per-user
// install cannot add them, so when the operator shares this PC or allows its
// first pairing, the app has `hide always-on firewall ask-allow` ask Windows
// once, through its permission prompt, to add them as an administrator.
// Declined, the connection still works through the relay, and the device's
// details offer the same ask again. Never asked at login, and on macOS and
// Linux there is nothing to ask.

import type { FirewallAnswer, FirewallState } from "../../../web/src/host";
import type { ChildResult } from "./spawn";

/** How long reading the rules may take. */
export const FIREWALL_CLI_TIMEOUT_MS = 30_000;
/**
 * How long the ask may take: the CLI gives Windows's prompt
 * `hide_platform::firewall::PROMPT_WITHIN` (10 minutes) and answers itself,
 * so this waits a minute past it for that answer.
 */
export const FIREWALL_PROMPT_TIMEOUT_MS = 11 * 60_000;

export type RunChild = (file: string, args: readonly string[], timeoutMs: number) => Promise<ChildResult>;

/** The `firewall` field of `hide always-on status --json`. */
export function parseState(result: ChildResult): FirewallState {
  const field = jsonField(result, "firewall");
  return field === "allowed" || field === "missing" || field === "no_port" || field === "not_used" ? field : "unknown";
}

/** The `answer` of `hide always-on firewall ask-allow`. */
export function parseAnswer(result: ChildResult): FirewallAnswer {
  const field = jsonField(result, "answer");
  return field === "allowed" || field === "declined" || field === "not_used" ? field : "failed";
}

function jsonField(result: ChildResult, name: string): unknown {
  if (result.code !== 0) return undefined;
  try {
    return (JSON.parse(result.stdout.trim()) as Record<string, unknown>)[name];
  } catch {
    return undefined;
  }
}

/** Whether this PC's rules are in place. */
export async function firewallState(platform: NodeJS.Platform, cli: string, run: RunChild): Promise<FirewallState> {
  if (platform !== "win32") return "not_used";
  return parseState(await run(cli, ["always-on", "status", "--json"], FIREWALL_CLI_TIMEOUT_MS));
}

/** Asks Windows once for the rules, and says what came of it. */
export async function allowFirewall(platform: NodeJS.Platform, cli: string, run: RunChild): Promise<FirewallAnswer> {
  if (platform !== "win32") return "not_used";
  return parseAnswer(await run(cli, ["always-on", "firewall", "ask-allow"], FIREWALL_PROMPT_TIMEOUT_MS));
}
