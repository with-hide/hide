import { ExternalLinkIcon } from "lucide-react";
import type { Actions } from "./actions";
import { useInterfaceTranslation } from "./i18n/client";
import { installDocUrl } from "./settings/agentRows";
import { FixCommand } from "./settings/FixCommand";
import type { AgentUpdate } from "./snapshot";

// What updates an agent whose copy is older than Hide's minimum, the same in
// Settings > Agents, the first-run agent step and a pane's badge
// (docs/agent-hooks.md, Updating an agent that is too old). The kit chose the
// command from where the copy is installed; with no command Hide could not
// tell how it was installed, and the vendor's install page stands in for a
// guessed one. A command is never translated.

/** The one command that updates the copy, with Copy, or the agent's install page when Hide cannot tell how it was installed. */
export function AgentUpdateCommand({ agentId, label, update, actions }: { agentId: string; label: string; update: AgentUpdate; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  if (update.command) {
    return <FixCommand command={update.command} what="agent update command" actions={actions} data-agent-update-command={agentId} />;
  }
  const href = installDocUrl(agentId);
  if (!href) return null;
  return (
    <a
      href={href}
      target="_blank"
      rel="noopener noreferrer"
      aria-label={t("agents.update.howAria", { agent: label })}
      className="inline-flex items-center gap-xs text-body text-primary outline-none focus-visible:ring-1 focus-visible:ring-ring"
      data-agent-update-docs={agentId}
    >
      {t("agents.update.how")}
      <ExternalLinkIcon aria-hidden="true" className="size-(--size-icon-sm)" />
    </a>
  );
}

/** The versions, the command and the copy's path, as one agent row's lines say them. */
export function AgentUpdateGuide({ agentId, label, update, actions }: { agentId: string; label: string; update: AgentUpdate; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="flex min-w-0 flex-col items-start gap-xs" data-agent-update={agentId}>
      <span className="break-keep text-body text-warning">{t("agents.update.versions", { installed: update.installed, minimum: update.minimum })}</span>
      <AgentUpdateCommand agentId={agentId} label={label} update={update} actions={actions} />
      <span className="min-w-0 break-all font-mono text-caption text-muted-foreground">{update.path}</span>
    </div>
  );
}
