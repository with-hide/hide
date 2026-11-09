import { agentMarkOf, monogram } from "./agentLogos";
import hideMark from "./assets/hide-mark.png";
import { cn } from "./lib/utils";

// The agent's own mark: the logo of each supported agent (agentLogos.ts picks it
// from the adapter, the one rule every surface shares) and a neutral terminal
// mark for a tab of plain shells or an agent Hide does not know. The mark never
// carries the name alone; every caller also gives the full identity in its
// tooltip and label.

/** The panel a badge sits on, so its ring is that panel's colour (the sidebar's, or a Tools column's card). */
export type BadgeGround = "sidebar" | "card";

const GROUND: Record<BadgeGround, string> = { sidebar: "bg-sidebar", card: "bg-card" };

/**
 * Hide's own mark, the owl face at a small size: the mark a Hide AI-started
 * row draws beside its status mark in the place of the provider logo while
 * logos are hidden (PRD agent-mark-quiet D-04), and (as `HideAiBadge`) the
 * badge a shown logo wears. The full
 * square mark is used at every size (docs/BRAND.md); decorative, because the
 * words `Hide AI` stand beside it or in the row's name.
 */
export function HideMark({ className = "" }: { className?: string }) {
  return <img src={hideMark} alt="" aria-hidden="true" data-hide-mark="true" className={cn("shrink-0 rounded-xs object-contain", className)} />;
}

/** The 11 badge on a provider icon's lower-right corner: Hide AI started this agent (PRD hide-ai-origin-mark D-04). */
function HideAiBadge({ ground }: { ground: BadgeGround }) {
  return (
    <span
      aria-hidden="true"
      data-hide-ai-badge="true"
      className={cn("pointer-events-none absolute -right-(--size-hairline) -bottom-(--size-hairline) flex size-(--size-hide-badge) items-center justify-center rounded-xs", GROUND[ground])}
    >
      <HideMark className="size-(--size-hide-badge-mark)" />
    </span>
  );
}

/** The letter a profile badge carries: the profile name's first, upper-cased where its script has case. */
export function profileInitial(profile: string): string {
  return (Array.from(profile.trim())[0] ?? "").toLocaleUpperCase();
}

/**
 * The badge of an agent that runs with a profile other than its built-in
 * default (PRD agent-profiles B17): the profile name's first letter, on the
 * logo's lower-right corner where the Hide AI badge sits, or standing alone
 * in the logo's place while logos are hidden. Decorative: the caller's hover
 * and accessible name say the agent and the profile (`Claude Code · work`).
 */
export function ProfileBadge({ profile, standalone = false }: { profile: string; standalone?: boolean }) {
  return (
    <span
      aria-hidden="true"
      data-profile-badge={profile}
      className={cn(
        "pointer-events-none flex h-(--size-hide-badge) min-w-(--size-hide-badge) shrink-0 items-center justify-center rounded-xs bg-popover px-(--size-hairline) text-(length:--text-badge) leading-none font-semibold text-foreground ring-1 ring-border",
        !standalone && "absolute -right-(--size-hairline) -bottom-(--size-hairline)",
      )}
    >
      {profileInitial(profile)}
    </span>
  );
}

/**
 * The agent's mark. `hideAi` marks an agent Hide AI started: the provider's
 * icon wears the badge. `profile` names the profile it runs with when that is
 * not its built-in default, and the icon wears its badge instead; the two
 * share one corner, so a Hide AI agent keeps its owl and names its profile
 * in words only. A badge is part of the row's name through the caller's
 * accessible name, never carried here (D-09).
 */
export function AgentMark({ kind, className = "", hideAi = null, profile = null }: { kind: string | null | undefined; className?: string; hideAi?: BadgeGround | null; profile?: string | null }) {
  const badge = hideAi ? <HideAiBadge ground={hideAi} /> : profile ? <ProfileBadge profile={profile} /> : null;
  if (!badge) return <PlainMark kind={kind} className={className} />;
  return (
    <span data-agent-mark-badged="true" className="relative inline-flex shrink-0">
      <PlainMark kind={kind} className={className} />
      {badge}
    </span>
  );
}

function PlainMark({ kind, className }: { kind: string | null | undefined; className: string }) {
  const mark = agentMarkOf(kind);
  if (mark?.logo) {
    return (
      <img
        src={mark.logo}
        alt=""
        aria-hidden="true"
        data-agent-mark={mark.id}
        className={`h-[var(--size-agent-badge-compact)] w-[var(--size-agent-badge-compact)] shrink-0 rounded-xs object-contain ${mark.plated ? "bg-(--logo-plate) p-xxs" : ""} ${className}`}
      />
    );
  }
  if (mark) {
    return (
      <span
        aria-hidden="true"
        data-agent-mark={mark.id}
        className={`flex h-[var(--size-agent-badge-compact)] w-[var(--size-agent-badge-compact)] shrink-0 items-center justify-center rounded-xs bg-(--logo-plate) font-mono text-micro font-semibold text-muted-foreground ${className}`}
      >
        {monogram(mark.label)}
      </span>
    );
  }
  return (
    <span
      aria-hidden="true"
      data-agent-mark="neutral"
      className={`flex h-[var(--size-agent-badge-compact)] w-[var(--size-agent-badge-compact)] shrink-0 items-center justify-center rounded-xs bg-secondary font-mono text-micro text-subtle-foreground ${className}`}
    >
      {">_"}
    </span>
  );
}
