import { ExternalLinkIcon } from "lucide-react";
import type { Actions } from "../actions";
import { AgentLogo } from "../components/agent-logo";
import { Note, Row, Status } from "../components/settings-rows";
import { Button } from "../components/ui/button";
import { Switch } from "../components/ui/switch";
import { useInterfaceTranslation } from "../i18n/client";
import { kitWhyText } from "../kitWhy";
import { kitPartText } from "../settings";
import type { Device, KitAgent } from "../snapshot";
import { AgentUpdateGuide } from "../AgentUpdate";
import { agentHookWait, agentLeftover, agentProblems, agentStatus, agentTooOld, agentUnsupportedHere, docsUrl, installDocUrl, machineSystem } from "./agentRows";
import { PartialChip } from "./PartialChip";

/** The link an agent row offers, as the row words it: a label and the arrow that says it leaves the app. */
function OutLink({ href, label, aria, className = "", ...data }: { href: string; label: string; aria: string; className?: string } & Record<`data-${string}`, string>) {
  return (
    <a
      href={href}
      target="_blank"
      rel="noopener noreferrer"
      aria-label={aria}
      className={`inline-flex items-center gap-xs text-body text-primary outline-none focus-visible:ring-1 focus-visible:ring-ring ${className}`}
      {...data}
    >
      {label}
      <ExternalLinkIcon aria-hidden="true" className="size-(--size-icon-sm)" />
    </a>
  );
}

function Name({ agent }: { agent: KitAgent }) {
  return (
    <span className="flex min-w-0 items-center gap-sm">
      <AgentLogo agent={agent.id} label={agent.label} />
      <span className="min-w-0 truncate font-semibold">{agent.label}</span>
    </span>
  );
}

/** "Not on Windows yet": an agent installed on a system Hide cannot attach to there yet (D-25). */
export function NotOnSystem({ system }: { system: string | null }) {
  const { t } = useInterfaceTranslation();
  return <>{system ? t("agents.notOnSystem", { system }) : t("agents.notOnThisSystem")}</>;
}

/**
 * An agent the machine does not have (B8): its mark, its name and the link to
 * the vendor's installation guide. Nothing to switch, so no switch.
 */
export function NotInstalledRow({ agent }: { agent: KitAgent }) {
  const { t } = useInterfaceTranslation();
  const href = installDocUrl(agent.id);
  return (
    <Row label={<Name agent={agent} />} data-agent-row={`${agent.id}:not-installed`}>
      {agent.partial ? <PartialChip agent={agent} /> : null}
      {href ? <OutLink href={href} label={t("agents.install")} aria={t("agents.installAria", { agent: agent.label })} data-agent-install={agent.id} /> : null}
    </Row>
  );
}

/**
 * One installed agent of one machine (D-08, D-09): its mark and name, the Basic
 * chip when Hide does only some things for it, the status of an agent that is
 * on, and the switch. A Docs link shows on hover or keyboard focus only (B12).
 * What needs the operator shows under the row, on this row alone: a part that
 * failed or was removed with its Reinstall (B13, B20), or, for a copy older than
 * Hide's minimum, the versions and the one command that updates it.
 */
export function AgentRow({ device, agent, actions, onAct }: { device: Device; agent: KitAgent; actions: Actions; onAct: () => void }) {
  const { t } = useInterfaceTranslation();
  const busy = device.kit?.busy === true;
  const status = agentStatus(agent);
  const problems = agentProblems(agent);
  const leftover = agentLeftover(agent);
  const leftoverWhy = leftover ? kitWhyText(leftover.why, t) : null;
  const waiting = agentHookWait(agent);
  const tooOld = agentTooOld(agent);
  const docs = docsUrl(agent.id);
  const worst = problems.some(({ piece }) => piece.state === "failed") ? "error" : "warn";
  return (
    <Row
      className="group/agent"
      label={
        <span className="flex min-w-0 flex-wrap items-center gap-x-sm gap-y-xs">
          <Name agent={agent} />
          {docs ? (
            <OutLink
              href={docs}
              label={t("agents.docs")}
              aria={t("agents.docsAria", { agent: agent.label })}
              className="opacity-0 transition-opacity focus-visible:opacity-100 group-focus-within/agent:opacity-100 group-hover/agent:opacity-100 [@media(hover:none)]:opacity-100"
              data-agent-docs={agent.id}
            />
          ) : null}
        </span>
      }
      data-agent-row={`${device.id}:${agent.id}:${agent.enabled ? "on" : "off"}`}
      detail={
        <>
          {problems.length > 0 ? (
            <div className="flex flex-wrap items-center gap-x-sm gap-y-xs" data-agent-problem={`${device.id}:${agent.id}`}>
              <Status tone={worst}>
                {problems.map(({ part, piece }) => t("agents.problem", { part: t(`agents.part.${part}`), state: kitPartText(piece, t) })).join("; ")}
              </Status>
              <Button
                variant="secondary"
                size="sm"
                disabled={busy}
                onClick={() => {
                  onAct();
                  actions.reinstallKit(device.id, [], [agent.id]);
                }}
                data-hook-reinstall={`${device.id}:${agent.id}`}
              >
                {busy ? t("settings.reinstalling") : t("settings.reinstall")}
              </Button>
            </div>
          ) : null}
          {tooOld?.update ? <AgentUpdateGuide agentId={agent.id} label={agent.label} update={tooOld.update} actions={actions} /> : null}
          {tooOld && !tooOld.update ? (
            <Note tone="warn" data-agent-update={agent.id}>
              {kitWhyText(tooOld.piece.why, t)}
            </Note>
          ) : null}
          {waiting ? (
            <Note data-agent-hook-wait={`${device.id}:${agent.id}`}>
              {t("agents.problem", { part: t(`agents.part.${waiting.part}`), state: kitWhyText(waiting.piece.why, t) ?? "" })}
            </Note>
          ) : null}
          {leftover ? (
            <Note tone="error" data-agent-leftover={`${device.id}:${agent.id}`}>
              {leftoverWhy ? t("agents.leftover", { reason: leftoverWhy }) : t("agents.leftoverNoReason")}
            </Note>
          ) : null}
        </>
      }
    >
      {agent.partial ? <PartialChip agent={agent} /> : null}
      {status.kind === "ready" ? (
        <Status tone="ok" data-agent-status={`${device.id}:${agent.id}:ready`}>
          {t("agents.status.ready")}
        </Status>
      ) : null}
      {status.kind === "sessions" ? (
        <Status tone="ok" data-agent-status={`${device.id}:${agent.id}:sessions`}>
          {t("agents.status.sessions", { count: status.count })}
        </Status>
      ) : null}
      {/* Installed with nothing to attach to on this system: no switch (B49). */}
      {agentUnsupportedHere(agent) ? (
        <Status tone="muted" data-agent-status={`${device.id}:${agent.id}:unsupported`}>
          <NotOnSystem system={machineSystem(device)} />
        </Status>
      ) : (
        <Switch
          checked={agent.enabled}
          disabled={busy}
          onCheckedChange={(checked) => {
            onAct();
            actions.setKitAgent(device.id, agent.id, checked);
          }}
          aria-label={agent.label}
          data-agent-switch={`${device.id}:${agent.id}:${agent.enabled ? "on" : "off"}`}
        />
      )}
    </Row>
  );
}
