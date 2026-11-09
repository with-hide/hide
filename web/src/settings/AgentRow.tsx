import { ExternalLinkIcon } from "lucide-react";
import { useId, useState } from "react";
import type { Actions } from "../actions";
import { AgentLogo } from "../components/agent-logo";
import { Note, Row, Status } from "../components/settings-rows";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { Select, SelectContent, SelectItem, SelectSeparator, SelectTrigger, SelectValue } from "../components/ui/select";
import { Switch } from "../components/ui/switch";
import { coreErrorText } from "../coreError";
import { useErrorSince } from "../errorSince";
import { useShellStore } from "../store";
import { useInterfaceTranslation } from "../i18n/client";
import { kitWhyText } from "../kitWhy";
import { kitPartText } from "../settings";
import { agentPermission, type Device, type KitAgent, type KitPiece } from "../snapshot";
import { AgentUpdateGuide } from "../AgentUpdate";
import { agentHookWait, agentLeftover, agentProblems, agentStatus, agentTooOld, agentUnsupportedHere, docsUrl, installDocUrl, machineSystem, permissionOffer, permissionOptions, type AgentPart, type PermissionOffer } from "./agentRows";
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

type PermissionMode = "full" | "ask" | "custom";

/**
 * The agent's 권한 on this machine (the saved choice, what a choice sends, the
 * 직접 입력 draft and a refusal), shared by the select in the row's 권한 column
 * and the field under the row.
 */
function usePermission(device: Device, agent: KitAgent, actions: Actions) {
  const permission = useShellStore((s) => agentPermission(s.rest, device.id, agent.id));
  const saved = permission.mode === "custom" ? permission.text : "";
  const [draft, setDraft] = useState<string | null>(null);
  const [sentAt, setSentAt] = useState<number | null>(null);
  const error = useErrorSince(sentAt, ["agent_permission."]);
  const send = (next: { mode: "full" | "ask" } | { mode: "custom"; text: string }) => {
    setSentAt(Date.now());
    actions.setAgentPermission(device.id, agent.id, next);
  };
  const commit = () => {
    if (draft !== null && draft.trim() !== saved) send({ mode: "custom", text: draft });
    setDraft(null);
  };
  return { permission, saved, draft, setDraft, send, commit, error };
}

type Permission = ReturnType<typeof usePermission>;

/** The command a choice starts the agent with, shown under the choice in the open menu: the program and what it adds. */
function commandOf(offer: PermissionOffer, mode: "full" | "ask"): string {
  const options = permissionOptions(offer, { mode });
  return options ? `${offer.program} ${options}` : offer.program;
}

/**
 * The row's 권한 column: a select whose options each show, in the open menu,
 * the command Hide will run for them. An agent with no permission question
 * says so in the same column instead.
 */
function PermissionSelect({ device, agent, offer, state }: { device: Device; agent: KitAgent; offer: PermissionOffer; state: Permission }) {
  const { t } = useInterfaceTranslation();
  const key = `${device.id}:${agent.id}`;
  if (!offer.asks) {
    return (
      <span className="w-(--size-agent-permission-w) max-w-full text-body text-muted-foreground" data-agent-permission={`${key}:not-asked`}>
        {t("agents.permission.notAsked")}
      </span>
    );
  }
  const { permission, saved, send } = state;
  const command = (mode: "full" | "ask") => <code className="font-mono" data-agent-permission-command={`${key}:${mode}`}>{commandOf(offer, mode)}</code>;
  return (
    <Select
      value={permission.mode}
      onValueChange={(value) => {
        if (value === permission.mode) return;
        const mode = value as PermissionMode;
        send(mode === "custom" ? { mode, text: saved } : { mode });
      }}
    >
      <SelectTrigger
        size="sm"
        className="w-(--size-agent-permission-w)"
        aria-label={t("agents.permission.aria", { agent: agent.label })}
        data-agent-permission={`${key}:${permission.mode}`}
        data-agent-permission-select={key}
      >
        <SelectValue />
      </SelectTrigger>
      <SelectContent align="end" className="max-w-(--radix-select-content-available-width)">
        <SelectItem value="full" description={command("full")} data-agent-permission-option="full">{t("agents.permission.full")}</SelectItem>
        <SelectItem value="ask" description={command("ask")} data-agent-permission-option="ask">{t("agents.permission.ask")}</SelectItem>
        <SelectSeparator />
        <SelectItem value="custom" data-agent-permission-option="custom">{t("agents.permission.custom")}</SelectItem>
      </SelectContent>
    </Select>
  );
}

/**
 * Under the row while 직접 입력 is chosen: the program, then the options as a
 * field, saved on Enter or when it loses focus; Escape puts back what is saved.
 * A refused choice says so under it.
 */
function PermissionField({ device, agent, offer, state }: { device: Device; agent: KitAgent; offer: PermissionOffer; state: Permission }) {
  const { t } = useInterfaceTranslation();
  const key = `${device.id}:${agent.id}`;
  const { permission, saved, draft, setDraft, commit, error } = state;
  return (
    <>
      {offer.asks && permission.mode === "custom" ? (
        <span className="flex min-w-0 items-center gap-xs">
          <span className="shrink-0 font-mono text-body text-muted-foreground">{offer.program}</span>
          <Input
            mono
            className="h-(--size-control-sm)"
            value={draft ?? saved}
            placeholder={t("agents.permission.customPlaceholder")}
            spellCheck={false}
            autoCapitalize="off"
            autoCorrect="off"
            aria-label={t("agents.permission.customAria", { agent: agent.label })}
            aria-invalid={error ? true : undefined}
            onChange={(event) => setDraft(event.target.value)}
            onBlur={commit}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                commit();
              } else if (event.key === "Escape" && draft !== null) {
                event.preventDefault();
                event.stopPropagation();
                setDraft(null);
              }
            }}
            data-agent-permission-input={key}
          />
        </span>
      ) : null}
      {error ? (
        <Note tone="error" data-agent-permission-error={key} data-core-error={error.kind}>
          {coreErrorText(error, t, "agents.permission.notSaved")}
        </Note>
      ) : null}
    </>
  );
}

/**
 * One installed agent of one machine (D-08, D-09) on one line: its mark and
 * name, the 일부 기능 chip when Hide does only some things for it, how many
 * sessions run when any do, its 권한 and the switch; a healthy agent says
 * nothing more. A Docs link shows on hover or keyboard focus only (B12).
 * What needs the operator shows under the row, on this row alone, in plain
 * words with no piece named: a broken piece as `Hide 연결이 끊겼어요` with
 * Reinstall, whose hover and accessible description name the piece, its state
 * and its path (B13, B20); for a copy older than Hide's minimum, the versions
 * and the one command that updates it; why a hook waits; a removal that did
 * not finish.
 */
export function AgentRow({ device, agent, actions, onAct, readOnly = false }: { device: Device; agent: KitAgent; actions: Actions; onAct: () => void; readOnly?: boolean }) {
  const { t } = useInterfaceTranslation();
  const detailId = useId();
  const busy = device.kit?.busy === true;
  const status = agentStatus(agent);
  const problems = agentProblems(agent);
  const leftover = agentLeftover(agent);
  const leftoverWhy = leftover ? kitWhyText(leftover.why, t) : null;
  const waiting = agentHookWait(agent);
  const tooOld = agentTooOld(agent);
  const docs = docsUrl(agent.id);
  const worst = problems.some(({ piece }) => piece.state === "failed") ? "error" : "warn";
  const unsupported = agentUnsupportedHere(agent);
  // Hide starts this agent here, so its 권한 applies (none where it cannot attach).
  const offer = unsupported ? null : permissionOffer(agent.id);
  const permission = usePermission(device, agent, actions);
  const pieceText = (part: AgentPart, piece: KitPiece) => [t("agents.problem", { part: t(`agents.part.${part}`), state: kitPartText(piece, t) }), piece.location].filter(Boolean).join(" · ");
  const problemDetail = problems.map(({ part, piece }) => pieceText(part, piece)).join("; ");
  if (readOnly) return <Row className="opacity-(--opacity-dimmed)" label={<Name agent={agent} />} data-agent-row={`${device.id}:${agent.id}:read-only`} />;
  return (
    <Row
      className="group/agent"
      controlsEnd
      label={
        <span className="flex min-w-0 flex-wrap items-center gap-x-sm gap-y-xs">
          <Name agent={agent} />
          {agent.partial ? <PartialChip agent={agent} /> : null}
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
        problems.length > 0 || tooOld || waiting || leftover || (offer && (permission.permission.mode === "custom" || permission.error)) ? (
          <div className="flex flex-col gap-xs">
            {offer ? <PermissionField device={device} agent={agent} offer={offer} state={permission} /> : null}
            {problems.length > 0 ? (
              <div className="flex flex-wrap items-center gap-x-sm gap-y-xs" title={problemDetail} data-agent-problem={`${device.id}:${agent.id}`}>
                <Status tone={worst}>{t("agents.disconnected")}</Status>
                <span id={detailId} hidden>
                  {problemDetail}
                </span>
                <Button
                  variant="secondary"
                  size="sm"
                  disabled={busy}
                  aria-describedby={detailId}
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
              <div title={pieceText(waiting.part, waiting.piece)}>
                <Note data-agent-hook-wait={`${device.id}:${agent.id}`}>{kitWhyText(waiting.piece.why, t) ?? kitPartText(waiting.piece, t)}</Note>
              </div>
            ) : null}
            {leftover ? (
              <Note tone="error" data-agent-leftover={`${device.id}:${agent.id}`}>
                {leftoverWhy ? t("agents.leftover", { reason: leftoverWhy }) : t("agents.leftoverNoReason")}
              </Note>
            ) : null}
          </div>
        ) : null
      }
    >
      {status.kind === "sessions" ? (
        <span className="text-body text-muted-foreground" data-agent-status={`${device.id}:${agent.id}:sessions`}>
          {t("agents.status.sessions", { count: status.count })}
        </span>
      ) : null}
      {offer ? <PermissionSelect device={device} agent={agent} offer={offer} state={permission} /> : null}
      {/* Installed with nothing to attach to on this system: no switch (B49). */}
      {unsupported ? (
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
