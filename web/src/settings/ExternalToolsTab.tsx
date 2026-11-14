// Settings › External tools (PRD hide-external-control B2-B4, B11-B13, B15,
// D-33, D-34): the requests waiting for the operator, each approved at View
// only or Act or refused, and the connected tools as rows like Mobile's
// paired phones, each with its two-level control, Disconnect, and the agents
// and missions it started when the row is opened. Every value is the core's
// `access` section and every control sends one `access_*` event; a request
// reaches the operator here only, never as a turn, a badge or a notice (D-24).

import { ChevronRightIcon, EyeIcon, PlayIcon, WaypointsIcon } from "lucide-react";
import { useMemo, useState } from "react";
import type { Actions } from "../actions";
import { AgentMark } from "../AgentMark";
import { markTone } from "../agentRow";
import { useMinuteClock } from "../components/elapsed";
import { Group, Note } from "../components/settings-rows";
import { StatusMark } from "../components/status-mark";
import { Button } from "../components/ui/button";
import { RadioGroup, RadioGroupItem } from "../components/ui/radio-group";
import { RowButton, RowOverlayButton } from "../components/ui/row-button";
import { ToggleGroup, ToggleGroupItem } from "../components/ui/toggle-group";
import { connectionLine, pendingLine, toolWork, warnsOfWork, type ToolWork } from "../externalTools";
import { useInterfaceTranslation } from "../i18n/client";
import { requireInterfaceLanguage } from "../i18n/locale";
import { cn } from "../lib/utils";
import { agentsByPane, allAgents } from "../paneAgents";
import { coreMachineName } from "../screenMachine";
import type { AccessConnection, AccessLevel, AccessPending } from "../snapshot";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";

const LEVELS: readonly AccessLevel[] = ["view", "act"];
const LEVEL_ICON = { view: EyeIcon, act: PlayIcon } as const;

export function ExternalToolsTab({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const access = useShellStore((s) => s.access);
  // Every tool runs on the core's machine (D-07), which a window on another machine names.
  const machine = useShellStore((s) => coreMachineName(s.rest?.navigator?.devices, t));
  const now = useMinuteClock();
  if (!access) return <Note tone="pending">{t("externalTools.connecting")}</Note>;
  return (
    <div data-external-tools="true">
      {access.pending.length > 0 ? (
        <Group title={t("externalTools.pending.title")} caption={String(access.pending.length)} data-access-pending-count={String(access.pending.length)}>
          {access.pending.map((request) => <PendingRow key={request.id} request={request} machine={machine} now={now} actions={actions} />)}
        </Group>
      ) : null}
      <Group
        title={t("externalTools.connected.title")}
        caption={<span className="tabular-nums" data-access-count={`${access.connections.length}/${access.limit}`}>{t("externalTools.connected.count", { connected: access.connections.length, limit: access.limit })}</span>}
        columns={<Legend />}
        data-access-connections={String(access.connections.length)}
      >
        {access.connections.length > 0
          ? access.connections.map((connection) => <ConnectionRow key={connection.id} connection={connection} machine={machine} now={now} actions={actions} />)
          : <Empty />}
      </Group>
    </div>
  );
}

/** What the two names mean, once above the list (B11, D-34). */
function Legend() {
  const { t } = useInterfaceTranslation();
  return (
    <div className="mb-sm flex flex-wrap gap-x-md gap-y-xxs text-caption text-muted-foreground" data-access-legend="true">
      {LEVELS.map((level) => {
        const Icon = LEVEL_ICON[level];
        return (
          <span key={level} className="inline-flex items-center gap-xs">
            <Icon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />
            <span className="font-medium text-subtle-foreground">{t(`externalTools.level.${level}`)}</span>
            <span>{t(`externalTools.legend.${level}`)}</span>
          </span>
        );
      })}
    </div>
  );
}

function Empty() {
  const { t } = useInterfaceTranslation();
  return (
    <div className="flex flex-col gap-xxs px-md py-sm" data-access-empty="true">
      <span className="text-subhead text-foreground">{t("externalTools.empty")}</span>
      <code className="break-all font-mono text-body text-muted-foreground">{t("externalTools.emptyCommand")}</code>
    </div>
  );
}

/** One request (B2, B3): name, machine and program, the two levels with View only chosen, and the approve that names the level beside Refuse. */
function PendingRow({ request, machine, now, actions }: { request: AccessPending; machine: string; now: number; actions: Actions }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const [level, setLevel] = useState<AccessLevel>("view");
  const line = pendingLine(request, machine, now, t, language);
  return (
    <div className="flex flex-col gap-sm px-md py-sm" data-access-pending={request.id}>
      <span className="flex min-w-0 flex-col gap-xxs">
        <span className="break-words text-subhead font-medium text-foreground">{request.name}</span>
        <span className="min-w-0 break-words text-body text-muted-foreground">
          {line.machine} · <span className="break-all font-mono" data-access-program="true">{line.program}</span> · {line.requested}
        </span>
      </span>
      <RadioGroup value={level} onValueChange={(next) => setLevel(next as AccessLevel)} aria-label={t("externalTools.level.label")} className="gap-xs" data-access-choice={level}>
        {LEVELS.map((option) => {
          const Icon = LEVEL_ICON[option];
          return (
            <label key={option} className="flex cursor-pointer items-start gap-sm">
              <RadioGroupItem value={option} className="mt-xxs" data-access-option={option} />
              <span className="flex min-w-0 flex-col">
                <span className="inline-flex items-center gap-xs text-subhead text-foreground">
                  <Icon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />
                  {t(`externalTools.level.${option}`)}
                </span>
                <span className="break-words break-keep text-body text-muted-foreground">{t(`externalTools.level.${option}Description`)}</span>
              </span>
            </label>
          );
        })}
      </RadioGroup>
      <div className="flex flex-wrap items-center gap-sm">
        <Button variant="secondary" size="sm" onClick={() => actions.approveAccess(request.id, level)} data-access-approve={level}>
          {t(`externalTools.approve.${level}`)}
        </Button>
        <Button variant="text" size="sm" onClick={() => actions.refuseAccess(request.id)} data-access-refuse={request.id}>
          {t("externalTools.refuse")}
        </Button>
      </div>
    </div>
  );
}

/**
 * One connected tool (B11-B13, B15): its name and line, the two-level control
 * and Disconnect, which acts at once; the warning when its work would become
 * the operator's; a press anywhere else opens its agents and missions.
 */
function ConnectionRow({ connection, machine, now, actions }: { connection: AccessConnection; machine: string; now: number; actions: Actions }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const [open, setOpen] = useState(false);
  return (
    <div data-access-connection={connection.id} data-access-level={connection.level}>
      <div className="relative">
        <RowOverlayButton aria-label={t("externalTools.open", { name: connection.name })} aria-expanded={open} onClick={() => setOpen(!open)} data-access-open={connection.id} />
        <div className="pointer-events-none relative flex flex-wrap items-center gap-x-md gap-y-xs px-md py-sm">
          <span className="flex min-w-[min(100%,var(--size-settings-control-w))] flex-1 items-start gap-sm">
            <ChevronRightIcon aria-hidden="true" className={cn("mt-xxs size-(--size-icon) shrink-0 text-muted-foreground transition-transform", open && "rotate-90")} />
            <span className="flex min-w-0 flex-col gap-xxs">
              <span className="break-words text-subhead font-medium text-foreground">{connection.name}</span>
              <span className="break-words text-body text-muted-foreground" data-access-line={connection.id}>
                {connectionLine(connection, machine, now, t, language).map((piece, index) => (
                  <span key={index}>{index > 0 ? " · " : ""}<span className="whitespace-nowrap">{piece}</span></span>
                ))}
              </span>
              {warnsOfWork(connection) ? <span className="mt-xxs" data-access-warning={connection.id}><Note tone="warn">{t("externalTools.workWarning")}</Note></span> : null}
            </span>
          </span>
          <span className="pointer-events-auto ml-auto flex flex-wrap items-center justify-end gap-sm">
            <ToggleGroup type="single" value={connection.level} onValueChange={(next) => next && next !== connection.level && actions.setAccessLevel(connection.id, next as AccessLevel)} aria-label={t("externalTools.level.label")}>
              {LEVELS.map((level) => {
                const Icon = LEVEL_ICON[level];
                return (
                  <ToggleGroupItem key={level} value={level} data-access-level-option={level}>
                    <Icon aria-hidden="true" />
                    {t(`externalTools.level.${level}`)}
                  </ToggleGroupItem>
                );
              })}
            </ToggleGroup>
            <Button variant="secondary" size="sm" className="text-destructive" onClick={() => actions.revokeAccess(connection.id)} data-access-revoke={connection.id}>
              {t("externalTools.disconnect")}
            </Button>
          </span>
        </div>
      </div>
      {open ? <WorkList connection={connection} actions={actions} /> : null}
    </div>
  );
}

/** The opened row (B15): each agent opens its pane, each mission its map, and Settings closes so it shows. */
function WorkList({ connection, actions }: { connection: AccessConnection; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const local = useShellStore((s) => s.agents);
  const remote = useShellStore((s) => s.rest?.status?.remote);
  const missions = useShellStore((s) => s.rest?.missions?.items);
  const work = useMemo(() => toolWork(connection, agentsByPane(allAgents(local, remote)), missions ?? []), [connection, local, remote, missions]);
  const go = (open: () => void) => {
    useUiStore.getState().closeOverlay("settings");
    open();
  };
  return (
    <div className="border-t border-border px-md py-xs pl-xl" data-access-work={connection.id}>
      {work.length === 0 ? <p className="py-xs text-body text-muted-foreground">{t("externalTools.noWork")}</p> : (
        <ul className="flex flex-col">
          {work.map((item) => (
            <li key={item.kind === "agent" ? item.agentId : item.id}>
              <WorkRow item={item} onOpen={() => go(() => item.kind === "agent" ? actions.openAgent(item.paneId) : actions.openMission(item.id))} />
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function WorkRow({ item, onOpen }: { item: ToolWork; onOpen: () => void }) {
  const { t } = useInterfaceTranslation();
  if (item.kind === "agent") {
    const row = item.row;
    return (
      <RowButton className="gap-sm text-body" onClick={onOpen} data-access-agent={item.paneId}>
        {row ? <StatusMark mark={row.mark} className={markTone(row)} /> : null}
        {row ? <AgentMark kind={row.agent_kind} /> : null}
        <span className={cn("min-w-0 flex-1 truncate", row ? "text-foreground" : "text-muted-foreground")}>
          {row ? row.identity_label : t("externalTools.agentGone", { id: item.agentId })}
        </span>
      </RowButton>
    );
  }
  return (
    <RowButton className="gap-sm text-body" onClick={onOpen} disabled={!item.mission} data-access-mission={item.id}>
      <WaypointsIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />
      <span className="sr-only">{t("externalTools.mission")}: </span>
      <span className="min-w-0 flex-1 truncate text-foreground">{item.mission?.title ?? item.id}</span>
    </RowButton>
  );
}
