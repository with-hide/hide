// Settings › 봇 (Bots; PRD hide-external-control B2-B4, B11-B13, D-33): the
// requests waiting for the operator, each approved for every workspace or for
// one or refused, and the connected bots as rows like Mobile's paired phones,
// each with its mode, Change mode, Disconnect, and how many agents and
// missions it started. Every value is the core's `access` section and every
// control sends one `access_*` event; a request reaches the operator here
// only, never as a turn, a badge or a notice (D-24).

import { ChevronDownIcon, FolderIcon } from "lucide-react";
import { useState } from "react";
import { useShallow } from "zustand/react/shallow";
import type { Actions } from "../actions";
import { botMode, botWorkspaceName, type BotMode } from "../bots";
import { BotFace, BotWorkspaceBadge } from "../components/bot-mark";
import { useMinuteClock } from "../components/elapsed";
import { Group, Note } from "../components/settings-rows";
import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuLabel, DropdownMenuRadioGroup, DropdownMenuRadioItem, DropdownMenuSeparator, DropdownMenuTrigger } from "../components/ui/dropdown-menu";
import { RadioGroup, RadioGroupItem } from "../components/ui/radio-group";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../components/ui/select";
import { connectionLine, pendingLine, warnsOfWork } from "../externalTools";
import { useInterfaceTranslation } from "../i18n/client";
import { requireInterfaceLanguage } from "../i18n/locale";
import { coreMachineName } from "../screenMachine";
import { catalogWorkspaces, listedWorkspaces, type AccessConnection, type AccessPending, type Workspace } from "../snapshot";
import { useShellStore } from "../store";

type ModeKind = BotMode["kind"];
const MODES: readonly ModeKind[] = ["all", "workspace"];

/** The value a mode reads as in a menu or a choice: `all`, or the workspace's id. */
const ALL_VALUE = "\u0000all";

export function ExternalToolsTab({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const access = useShellStore((s) => s.access);
  // Every bot runs on the core's machine (D-07), which a window on another machine names.
  const machine = useShellStore((s) => coreMachineName(s.rest?.navigator?.devices, t));
  // The catalog is a new array on every read; the shallow compare keeps one while its projects stay the same.
  const workspaces = useShellStore(useShallow((s) => listedWorkspaces(catalogWorkspaces(s.rest))));
  const now = useMinuteClock();
  if (!access) return <Note tone="pending">{t("externalTools.connecting")}</Note>;
  return (
    <div data-external-tools="true">
      {access.pending.length > 0 ? (
        <Group title={t("externalTools.pending.title")} caption={String(access.pending.length)} data-access-pending-count={String(access.pending.length)}>
          {access.pending.map((request) => <PendingRow key={request.id} request={request} machine={machine} workspaces={workspaces} now={now} actions={actions} />)}
        </Group>
      ) : null}
      <Group
        title={t("externalTools.connected.title")}
        caption={<span className="tabular-nums" data-access-count={`${access.connections.length}/${access.limit}`}>{t("externalTools.connected.count", { connected: access.connections.length, limit: access.limit })}</span>}
        data-access-connections={String(access.connections.length)}
      >
        {access.connections.length > 0
          ? access.connections.map((connection) => <ConnectionRow key={connection.id} connection={connection} machine={machine} workspaces={workspaces} now={now} actions={actions} />)
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
 * the two modes with one workspace chosen (the narrower), the workspace picked
 * from this core's projects, and the approve that names the choice beside
 * Refuse. With no project yet, only every workspace can be chosen.
 */
function PendingRow({ request, machine, workspaces, now, actions }: { request: AccessPending; machine: string; workspaces: readonly Workspace[]; now: number; actions: Actions }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const [kind, setKind] = useState<ModeKind>(workspaces.length > 0 ? "workspace" : "all");
  const [picked, setPicked] = useState<string | null>(null);
  // The pick follows the projects as they change: a project that left is no longer chosen.
  const workspace = workspaces.find((one) => one.id === picked) ?? workspaces[0] ?? null;
  const mode: BotMode | null = kind === "all" ? { kind: "all" } : workspace ? { kind: "workspace", workspace_id: workspace.id } : null;
  const line = pendingLine(request, machine, now, t, language);
  return (
    <div className="flex flex-col gap-md px-md py-md" data-access-pending={request.id}>
      <span className="flex min-w-0 items-center gap-md">
        <BotFace name={request.name} size="tile" />
        <span className="flex min-w-0 flex-col">
          <span className="break-words text-subhead font-medium text-foreground">{request.name}</span>
          <span className="min-w-0 break-words text-body text-muted-foreground">
            {line.machine} · <span className="break-all font-mono" data-access-program="true">{line.program}</span> · {line.requested}
          </span>
        </span>
      </span>
      <RadioGroup value={kind} onValueChange={(next) => setKind(next as ModeKind)} aria-label={t("externalTools.mode.label")} className="grid grid-cols-[repeat(auto-fit,minmax(min(100%,var(--size-settings-control-w)),1fr))] gap-sm" data-access-choice={kind}>
        {MODES.map((option) => {
          const unavailable = option === "workspace" && workspaces.length === 0;
          return (
            <label key={option} className="flex cursor-pointer items-start gap-sm rounded-md border border-border px-md py-sm has-data-[state=checked]:border-primary has-disabled:cursor-not-allowed has-disabled:opacity-(--opacity-disabled)">
              <RadioGroupItem value={option} disabled={unavailable} className="mt-xxs" data-access-option={option} />
              <span className="flex min-w-0 flex-col">
                <span className="text-subhead font-medium text-foreground">{t(`externalTools.mode.${option}`)}</span>
                <span className="break-words break-keep text-body text-muted-foreground">{t(unavailable ? "externalTools.mode.noWorkspace" : `externalTools.mode.${option}Description`)}</span>
              </span>
            </label>
          );
        })}
      </RadioGroup>
      {kind === "workspace" && workspace ? (
        <Select value={workspace.id} onValueChange={setPicked}>
          <SelectTrigger aria-label={t("externalTools.mode.pick")} className="w-full min-w-0" data-access-workspace={workspace.id}>
            <SelectValue>
              <span className="flex min-w-0 items-center gap-xs"><FolderIcon aria-hidden="true" className="size-(--size-icon) shrink-0 text-muted-foreground" /><span className="truncate">{workspace.label}</span></span>
            </SelectValue>
          </SelectTrigger>
          <SelectContent>
            {workspaces.map((one) => <SelectItem key={one.id} value={one.id}>{one.label}</SelectItem>)}
          </SelectContent>
        </Select>
      ) : null}
      <div className="flex flex-wrap items-center justify-end gap-sm">
        <Button variant="text" size="sm" onClick={() => actions.refuseAccess(request.id)} data-access-refuse={request.id}>
          {t("externalTools.refuse")}
        </Button>
        <Button variant="secondary" size="sm" disabled={mode === null} onClick={() => mode && actions.approveAccess(request.id, mode)} data-access-approve={kind}>
          {kind === "all" || !workspace ? t("externalTools.approve.all") : t("externalTools.approve.workspace", { workspace: workspace.label })}
        </Button>
      </div>
    </div>
  );
}

/**
 * One connected bot (B11-B13): its owl, its name with its mode, and its line,
 * which counts the agents and missions it started; Change mode, a menu of
 * every workspace or one project, and Disconnect, which acts at once; and the
 * warning when its work would become the operator's.
 */
function ConnectionRow({ connection, machine, workspaces, now, actions }: { connection: AccessConnection; machine: string; workspaces: readonly Workspace[]; now: number; actions: Actions }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const mode = botMode(connection);
  const workspace = botWorkspaceName(mode, workspaces);
  const current = mode.kind === "all" ? ALL_VALUE : mode.workspace_id;
  const change = (value: string) => {
    if (value === current) return;
    actions.setBotMode(connection.id, value === ALL_VALUE ? { kind: "all" } : { kind: "workspace", workspace_id: value });
  };
  return (
    <div className="flex flex-wrap items-center gap-x-md gap-y-xs px-md py-sm" data-access-connection={connection.id} data-access-mode={mode.kind}>
      <span className="relative flex shrink-0">
        <BotFace name={connection.name} size="tile" />
        {workspace === null ? null : <BotWorkspaceBadge className="ring-card" />}
      </span>
      <span className="flex min-w-[min(100%,var(--size-settings-control-w))] flex-1 flex-col gap-xxs">
        <span className="flex min-w-0 flex-wrap items-center gap-x-sm gap-y-xxs">
          <span className="break-words text-subhead font-medium text-foreground">{connection.name}</span>
          <Badge variant="secondary" className="min-w-0 max-w-full" data-access-mode-pill={mode.kind}>
            {workspace === null ? null : <FolderIcon aria-hidden="true" />}
            <span className="min-w-0 truncate">{workspace ?? t("externalTools.mode.all")}</span>
          </Badge>
        </span>
        <span className="break-words text-body text-muted-foreground" data-access-line={connection.id}>
          {connectionLine(connection, machine, now, t, language).map((piece, index) => (
            <span key={index}>{index > 0 ? " · " : ""}<span className="whitespace-nowrap">{piece}</span></span>
          ))}
        </span>
        {warnsOfWork(connection) ? <span className="mt-xxs" data-access-warning={connection.id}><Note tone="warn">{t("externalTools.workWarning")}</Note></span> : null}
      </span>
      <span className="ml-auto flex flex-wrap items-center justify-end gap-sm">
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="text" size="sm" data-access-change-mode={connection.id}>
              {t("externalTools.changeMode")}
              <ChevronDownIcon aria-hidden="true" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuRadioGroup value={current} onValueChange={change} aria-label={t("externalTools.mode.label")}>
              <DropdownMenuRadioItem value={ALL_VALUE} data-access-mode-option="all">{t("externalTools.mode.all")}</DropdownMenuRadioItem>
              {workspaces.length > 0 ? (
                <>
                  <DropdownMenuSeparator />
                  <DropdownMenuLabel>{t("externalTools.mode.workspace")}</DropdownMenuLabel>
                  {workspaces.map((one) => (
                    <DropdownMenuRadioItem key={one.id} value={one.id} data-access-mode-option={one.id}>
                      <span className="min-w-0 truncate">{one.label}</span>
                    </DropdownMenuRadioItem>
                  ))}
                </>
              ) : null}
            </DropdownMenuRadioGroup>
          </DropdownMenuContent>
        </DropdownMenu>
        <Button variant="text" size="sm" className="text-destructive" onClick={() => actions.revokeAccess(connection.id)} data-access-revoke={connection.id}>
          {t("externalTools.disconnect")}
        </Button>
      </span>
    </div>
  );
}
