import { AgentMark, HideMark, ProfileBadge, type BadgeGround } from "./AgentMark";
import { useAgentLogosShown } from "./agentLogoMode";

// Kept apart from AgentMark.tsx, which the phone bundle also imports: the phone
// has no shell store, and this reads it (and the keyboard's hold) to decide.

/**
 * The mark a list row or an agent tab draws before its title, when the
 * operator's view mode shows logos or an ⌥ hold peeks at them (PRD
 * agent-logo-view-mode D-04, D-05); hidden, the row draws nothing here, and a
 * Hide AI-started row stands on its owl alone, as before, and a profile
 * agent's row on its profile badge alone (PRD agent-profiles B17). `hideAi`
 * is set for a Hide AI row, which is never in a project's tree (PRD
 * hide-ai-work-home); `profile` for an agent that runs with a profile other
 * than its built-in default.
 */
export function AgentRowMark({ kind, hideAi = null, profile = null }: { kind: string | null | undefined; hideAi?: BadgeGround | null; profile?: string | null }) {
  const shown = useAgentLogosShown();
  if (shown) return <AgentMark kind={kind} hideAi={hideAi} profile={profile} />;
  if (hideAi) return <HideMark className="size-(--size-agent-badge-compact)" />;
  return profile ? <ProfileBadge profile={profile} standalone /> : null;
}
