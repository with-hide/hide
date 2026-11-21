import { agentLogo, monogram } from "../agentLogos";
import { cn } from "../lib/utils";

/**
 * An agent's official mark at row size (the first-run tile draws the same logo
 * larger), on the plate that keeps a dark mark readable in the dark theme.
 * Decorative: the name always sits beside it. An agent with no bundled mark
 * draws its monogram, never an invented logo (design 10).
 */
export function AgentLogo({ agent, label, size = "default" }: { agent: string; label: string; size?: "default" | "sm" }) {
  const logo = agentLogo(agent);
  return (
    <span aria-hidden="true" className={cn("flex shrink-0 items-center justify-center overflow-hidden rounded-sm bg-(--logo-plate)", size === "sm" ? "size-(--size-icon)" : "size-(--size-icon-lg)")}>
      {logo ? (
        <img src={logo} alt="" className={cn("size-full object-contain", size === "default" && "p-xxs")} data-agent-logo={agent} />
      ) : (
        <span className="font-mono text-caption font-semibold text-muted-foreground" data-agent-monogram={agent}>
          {monogram(label)}
        </span>
      )}
    </span>
  );
}
