// Read the generated Rust contract. No agent support table lives here.
import contract from "../../contracts/agent-adapters.json";

export const AGENT_ADAPTERS = contract;

export function agentAdapter(id: string) {
  const normalized = id.trim().toLowerCase();
  return AGENT_ADAPTERS.find((row) => row.id === normalized || row.aliases.some((alias) => alias === normalized));
}

/** The agent's product name for a tooltip, label or accessible name ("Claude Code", "Codex"): the adapter's label, or the engine kind as sent when Hide has no adapter for it. */
export function agentProductName(kind: string): string;
export function agentProductName(kind: string | null | undefined): string | null;
export function agentProductName(kind: string | null | undefined): string | null {
  return kind == null ? null : (agentAdapter(kind)?.label ?? kind);
}
