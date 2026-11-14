// Settings › External tools (PRD hide-external-control B2-B4, B11-B13, D-33,
// D-34): the requests waiting for the operator, each approved at View only or
// Act or refused, and the connected tools as rows like Mobile's paired
// phones, each with its two-level control, Disconnect, and how many agents
// and missions it started. Every value is the core's
// `access` section and every control sends one `access_*` event; a request
// reaches the operator here only, never as a turn, a badge or a notice (D-24).

import { EyeIcon, PlayIcon } from "lucide-react";
import { useState } from "react";
import type { Actions } from "../actions";
import { useMinuteClock } from "../components/elapsed";
import { Group, Note } from "../components/settings-rows";
import { Button } from "../components/ui/button";
import { RadioGroup, RadioGroupItem } from "../components/ui/radio-group";
import { ToggleGroup, ToggleGroupItem } from "../components/ui/toggle-group";
import { connectionLine, pendingLine, warnsOfWork } from "../externalTools";
import { useInterfaceTranslation } from "../i18n/client";
import { requireInterfaceLanguage } from "../i18n/locale";
import { coreMachineName } from "../screenMachine";
import type { AccessConnection, AccessLevel, AccessPending } from "../snapshot";
import { useShellStore } from "../store";

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
 * One connected tool (B11-B13): its name and line, which counts the agents and
 * missions it started, the two-level control and Disconnect, which acts at
 * once, and the warning when its work would become the operator's.
 */
function ConnectionRow({ connection, machine, now, actions }: { connection: AccessConnection; machine: string; now: number; actions: Actions }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  return (
    <div className="flex flex-wrap items-center gap-x-md gap-y-xs px-md py-sm" data-access-connection={connection.id} data-access-level={connection.level}>
      <span className="flex min-w-[min(100%,var(--size-settings-control-w))] flex-1 flex-col gap-xxs">
        <span className="break-words text-subhead font-medium text-foreground">{connection.name}</span>
        <span className="break-words text-body text-muted-foreground" data-access-line={connection.id}>
          {connectionLine(connection, machine, now, t, language).map((piece, index) => (
            <span key={index}>{index > 0 ? " · " : ""}<span className="whitespace-nowrap">{piece}</span></span>
          ))}
        </span>
        {warnsOfWork(connection) ? <span className="mt-xxs" data-access-warning={connection.id}><Note tone="warn">{t("externalTools.workWarning")}</Note></span> : null}
      </span>
      <span className="ml-auto flex flex-wrap items-center justify-end gap-sm">
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
  );
}
