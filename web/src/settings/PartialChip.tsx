import { ExternalLinkIcon } from "lucide-react";
import { useId } from "react";
import { Badge } from "../components/ui/badge";
import { Popover, PopoverContent, PopoverTrigger } from "../components/ui/popover";
import { useInterfaceTranslation } from "../i18n/client";
import type { KitAgent } from "../snapshot";
import { docsUrl } from "./agentRows";

/**
 * The Basic chip of an agent outside the collaboration tier, on or off:
 * a button that opens the three groups of what works,
 * one line per feature with a mark and a word, never a colour alone. The list
 * is the kit's feature table, so it is what this build does with the agent.
 * Radix owns the keyboard: Enter or Space opens it, Escape closes it and
 * focus returns to the chip.
 */
const GROUPS = [
  { id: "basics", features: ["skill", "guidance", "start"] },
  { id: "sessions", features: ["titles", "sleep", "fork"] },
  { id: "collaboration", features: ["letters", "bell", "memory"] },
] as const;

export function PartialChip({ agent }: { agent: KitAgent }) {
  const { t } = useInterfaceTranslation();
  const headingId = useId();
  const features = agent.features ?? [];
  const docs = docsUrl(agent.id);
  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          type="button"
          aria-label={t("agents.partialAria", { agent: agent.label })}
          className="cursor-pointer rounded-xs outline-none focus-visible:ring-1 focus-visible:ring-ring"
          data-agent-partial={agent.id}
        >
          <Badge variant="outline">{t("agents.partial")}</Badge>
        </button>
      </PopoverTrigger>
      <PopoverContent align="end" className="max-h-(--radix-popover-content-available-height) overflow-y-auto" data-agent-partial-popover={agent.id}>
        <p className="font-semibold text-foreground">{t("agents.partialTitle", { agent: agent.label })}</p>
        {GROUPS.map((group) => (
          <section key={group.id} aria-labelledby={`${headingId}-${group.id}`} data-agent-feature-group={group.id} className="mt-sm">
            <h3 id={`${headingId}-${group.id}`} className="mb-xs text-caption font-semibold text-foreground">{t(`agents.group.${group.id}`)}</h3>
            <ul className="space-y-xs">
              {group.features.map((id) => features.find((feature) => feature.id === id)).filter((feature) => feature !== undefined).map((feature) => (
                <li key={feature.id} className="flex items-baseline gap-xs" data-agent-feature={`${feature.id}:${feature.supported ? "yes" : "no"}`}>
                  <span aria-hidden="true" className={`shrink-0 font-mono ${feature.supported ? "text-success" : "text-muted-foreground"}`}>
                    {feature.supported ? "✓" : "–"}
                  </span>
                  <span className="sr-only">{t(feature.supported ? "agents.feature.supported" : "agents.feature.unsupported")}: </span>
                  <span className={`min-w-0 break-words ${feature.supported ? "text-foreground" : "text-muted-foreground"}`}>{t(`agents.feature.${feature.id}`)}</span>
                </li>
              ))}
            </ul>
          </section>
        ))}
        {docs ? (
          <a
            href={docs}
            target="_blank"
            rel="noopener noreferrer"
            className="mt-sm inline-flex items-center gap-xs text-caption text-primary outline-none focus-visible:ring-1 focus-visible:ring-ring"
            aria-label={t("agents.docsAria", { agent: agent.label })}
          >
            {t("agents.docs")}
            <ExternalLinkIcon aria-hidden="true" className="size-(--size-icon-sm)" />
          </a>
        ) : null}
      </PopoverContent>
    </Popover>
  );
}
