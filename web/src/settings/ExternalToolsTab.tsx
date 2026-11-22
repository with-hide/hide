// Settings › 봇 (Bots; PRD hide-external-control B2-B4, B11-B13, D-33): the
// requests waiting for the operator, each approved into the bot's own space or
// for all of Hide, or refused, and the connected bots as rows like Mobile's
// paired phones, each with its mode, Change mode, Disconnect, and how many
// agents and missions it started. Every value is the core's `access` section
// and every control sends one `access_*` event; a request reaches the operator
// here only, never as a turn, a badge or a notice (D-24).

import { ChevronDownIcon } from "lucide-react";
import { useState } from "react";
import type { Actions } from "../actions";
import { BOT_MODES, type BotModeKind } from "../bots";
import { BotFace } from "../components/bot-mark";
import { useMinuteClock } from "../components/elapsed";
import { Group, Note } from "../components/settings-rows";
import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuRadioGroup, DropdownMenuRadioItem, DropdownMenuTrigger } from "../components/ui/dropdown-menu";
import { RadioGroup, RadioGroupItem } from "../components/ui/radio-group";
import { connectionLine, pendingLine, warnsOfWork } from "../externalTools";
import { useInterfaceTranslation } from "../i18n/client";
import { requireInterfaceLanguage } from "../i18n/locale";
import { ownWindow } from "../pairing";
import { coreMachineName } from "../screenMachine";
import type { AccessConnection, AccessPending } from "../snapshot";
import { useShellStore } from "../store";

export function ExternalToolsTab({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const access = useShellStore((s) => s.access);
  // Every bot runs on the core's machine (D-07), which a window on another machine names.
  const machine = useShellStore((s) => coreMachineName(s.rest?.navigator?.devices, t));
  const now = useMinuteClock();
  // Only this machine's own window lets a bot in or changes its mode (PRD
  // device-connect-iroh B52); another machine's view lists the bots.
  const own = useShellStore((s) => ownWindow(s.pairing));
  if (!access) return <Note tone="pending">{t("externalTools.connecting")}</Note>;
  return (
    <div data-external-tools="true">
      {own && access.pending.length > 0 ? (
        <Group title={t("externalTools.pending.title")} caption={String(access.pending.length)} data-access-pending-count={String(access.pending.length)}>
          {access.pending.map((request) => <PendingRow key={request.id} request={request} machine={machine} now={now} actions={actions} />)}
        </Group>
      ) : null}
      <Group
        title={t("externalTools.connected.title")}
        caption={<span className="tabular-nums" data-access-count={`${access.connections.length}/${access.limit}`}>{t("externalTools.connected.count", { connected: access.connections.length, limit: access.limit })}</span>}
        data-access-connections={String(access.connections.length)}
      >
        {access.connections.length > 0
          ? access.connections.map((connection) => <ConnectionRow key={connection.id} connection={connection} machine={machine} now={now} actions={own ? actions : null} />)
          : <Empty />}
      </Group>
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

/**
 * One request (B2, B3): the owl it will get, its name, machine and program,
 * the two modes with its own space chosen (the narrower), and the approve
 * that names the choice beside Refuse.
 */
function PendingRow({ request, machine, now, actions }: { request: AccessPending; machine: string; now: number; actions: Actions }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const [mode, setMode] = useState<BotModeKind>("own");
  const line = pendingLine(request, machine, now, t, language);
  return (
    <div className="flex flex-col gap-md px-md py-md" data-access-pending={request.id}>
      <span className="flex min-w-0 items-center gap-md">
        <BotFace name={request.name} size="tile" mode={mode} />
        <span className="flex min-w-0 flex-col">
          <span className="break-words text-subhead font-medium text-foreground">{request.name}</span>
          <span className="min-w-0 break-words text-body text-muted-foreground">
            {line.machine} · <span className="break-all font-mono" data-access-program="true">{line.program}</span> · {line.requested}
          </span>
        </span>
      </span>
      <RadioGroup value={mode} onValueChange={(next) => setMode(next as BotModeKind)} aria-label={t("externalTools.mode.label")} className="grid grid-cols-[repeat(auto-fit,minmax(min(100%,var(--size-settings-control-w)),1fr))] gap-sm" data-access-choice={mode}>
        {BOT_MODES.map((option) => (
          <label key={option} className="flex cursor-pointer items-start gap-sm rounded-md border border-border px-md py-sm has-data-[state=checked]:border-primary">
            <RadioGroupItem value={option} className="mt-xxs" data-access-option={option} />
            <span className="flex min-w-0 flex-col">
              <span className="text-subhead font-medium text-foreground">{t(`externalTools.mode.${option}`)}</span>
              <span className="break-words break-keep text-body text-muted-foreground">{t(`externalTools.mode.${option}Description`)}</span>
            </span>
          </label>
        ))}
      </RadioGroup>
      <div className="flex flex-wrap items-center justify-end gap-sm">
        <Button variant="text" size="sm" onClick={() => actions.refuseAccess(request.id)} data-access-refuse={request.id}>
          {t("externalTools.refuse")}
        </Button>
        <Button variant="secondary" size="sm" onClick={() => actions.approveAccess(request.id, mode)} data-access-approve={mode}>
          {t(`externalTools.approve.${mode}`)}
        </Button>
      </div>
    </div>
  );
}

/**
 * One connected bot (B11-B13): its owl, its name with its mode, and its line,
 * which counts the agents and missions it started; Change mode, a menu of the
 * two modes, and Disconnect, which acts at once; and the warning when its work
 * would become the operator's.
 */
function ConnectionRow({ connection, machine, now, actions }: { connection: AccessConnection; machine: string; now: number; actions: Actions | null }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const mode = connection.mode.kind;
  return (
    <div className="flex flex-wrap items-center gap-x-md gap-y-xs px-md py-sm" data-access-connection={connection.id} data-access-mode={mode}>
      <BotFace name={connection.name} size="tile" mode={mode} />
      <span className="flex min-w-[min(100%,var(--size-settings-control-w))] flex-1 flex-col gap-xxs">
        <span className="flex min-w-0 flex-wrap items-center gap-x-sm gap-y-xxs">
          <span className="break-words text-subhead font-medium text-foreground">{connection.name}</span>
          <Badge variant="secondary" data-access-mode-pill={mode}>{t(`externalTools.mode.${mode}`)}</Badge>
        </span>
        <span className="break-words text-body text-muted-foreground" data-access-line={connection.id}>
          {connectionLine(connection, machine, now, t, language).map((piece, index) => (
            <span key={index}>{index > 0 ? " · " : ""}<span className="whitespace-nowrap">{piece}</span></span>
          ))}
        </span>
        {warnsOfWork(connection) ? <span className="mt-xxs" data-access-warning={connection.id}><Note tone="warn">{t("externalTools.workWarning")}</Note></span> : null}
      </span>
      {actions ? <span className="ml-auto flex flex-wrap items-center justify-end gap-sm">
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="text" size="sm" data-access-change-mode={connection.id}>
              {t("externalTools.changeMode")}
              <ChevronDownIcon aria-hidden="true" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuRadioGroup value={mode} onValueChange={(next) => next !== mode && actions.setAccessMode(connection.id, next as BotModeKind)} aria-label={t("externalTools.mode.label")}>
              {BOT_MODES.map((option) => (
                <DropdownMenuRadioItem key={option} value={option} data-access-mode-option={option}>{t(`externalTools.mode.${option}`)}</DropdownMenuRadioItem>
              ))}
            </DropdownMenuRadioGroup>
          </DropdownMenuContent>
        </DropdownMenu>
        <Button variant="text" size="sm" className="text-destructive" onClick={() => actions.revokeAccess(connection.id)} data-access-revoke={connection.id}>
          {t("externalTools.disconnect")}
        </Button>
      </span> : null}
    </div>
  );
}
