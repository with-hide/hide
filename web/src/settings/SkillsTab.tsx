import type { TFunction } from "i18next";
import { CheckIcon, ChevronDownIcon, ChevronRightIcon, FileIcon, FolderIcon, GitBranchIcon, GlobeIcon, HandIcon, PackageIcon, PlusIcon } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { AgentMark } from "../AgentMark";
import type { Actions } from "../actions";
import { useMinuteClock } from "../components/elapsed";
import { Note } from "../components/settings-rows";
import { StatusMark } from "../components/status-mark";
import { RowOverlayButton } from "../components/ui/row-button";
import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "../components/ui/dropdown-menu";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../components/ui/select";
import { Switch } from "../components/ui/switch";
import { Toast } from "../components/ui/toast";
import { Hint } from "../components/ui/tooltip";
import { useInterfaceTranslation } from "../i18n/client";
import { requireInterfaceLanguage } from "../i18n/locale";
import { kitUnavailableText } from "../kitWhy";
import { cn } from "../lib/utils";
import { lastSeen } from "../mobileSettings";
import { machineName, windowDeviceId, windowFirst } from "../screenMachine";
import type { Device, StoreAsk, StoreCell, StoreFoundCopy, StoreItem, StoreItemKind, StoreReport, StoreUnmanaged } from "../snapshot";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";
import { agentMachines, type AgentMachine } from "./agentRows";
import { AddDialog, AdoptDialog, AskDialog, FilesDialog, KeysDialog, UpdateDialog, refusalText, type AdoptTarget } from "./StoreDialogs";
import {
  agentShortName,
  allSwitch,
  cellOf,
  cellOn,
  columnName,
  columnWants,
  failureKey,
  homePath,
  itemsOf,
  keyNames,
  openableSource,
  pathParts,
  rowLine,
  serverLine,
  shownAgents,
  shownColumns,
  sourceLine,
  trashFor,
  updatable,
  versionText,
  type ShownColumn,
} from "./skillsMcp";
import { useAgentsDemand } from "./useAgentsDemand";
import { useStoreRequest } from "./useStoreRequest";

/** What a row's press needs from the page around it. */
type Page = {
  device: Device;
  report: StoreReport;
  /** This Mac, where a folder can be opened. */
  local: boolean;
  readOnly: boolean;
  /** The machine is not connected: its last list shows dimmed. */
  away: boolean;
  actions: Actions;
  newSession: ReadonlySet<string>;
  openFiles: (item: StoreItem) => void;
  openKeys: (item: StoreItem) => void;
  /** Shows what a take-over would bring in, which only its Apply sends (B20). */
  openAdopt: (target: AdoptTarget) => void;
  openUpdate: (items: StoreItem[]) => void;
  /** Deletes the item; the page keeps the request, since the row goes before its answer lands. */
  remove: (item: StoreItem) => void;
  /** The item whose delete is on its way. */
  removing: string | null;
};

/**
 * Settings › 스킬·MCP (PRD skill-mcp-manager; decisions of 2026-11-19 and the
 * layout of 2026-11-20, mockup card C): one machine's skills and MCP servers
 * at a time, one table per kind whose rows unfold in place. Everything shown
 * is the machine's last store report; every press is one store request the
 * core runs on that machine.
 */
export function SkillsTab({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const remote = useShellStore((s) => s.rest?.status?.remote);
  const own = useShellStore((s) => windowDeviceId(s.rest?.navigator?.devices));
  const asks = useShellStore((s) => s.rest?.status?.store_asks);
  const askRequested = useUiStore((s) => s.storeAskRequested);
  const [selected, setSelected] = useState<string | null>(null);
  useAgentsDemand(actions, true);
  const machines = agentMachines(windowFirst(devices), remote);
  // 확인하기 on an agent's ask opens the machine it asks about.
  const askDevice = asks?.find((ask) => ask.id === askRequested)?.device_id ?? null;
  const machine = machines.find((row) => row.device.id === (askDevice ?? selected ?? own)) ?? machines[0];
  if (!machine) return <Note>{t("store.reading")}</Note>;
  return (
    <MachineStore
      key={machine.device.id}
      machine={machine}
      machines={machines}
      own={own}
      onMachine={(id) => {
        setSelected(id);
        useUiStore.getState().clearStoreAsk();
      }}
      devices={devices}
      actions={actions}
    />
  );
}

function MachineStore({ machine, machines, own, onMachine, devices, actions }: { machine: AgentMachine; machines: AgentMachine[]; own: string; onMachine: (id: string) => void; devices: Device[] | undefined; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const { device } = machine;
  const name = machineName(devices, device.id, t);
  const report = device.kit?.store ?? null;
  const shown = useMemo(() => shownAgents(device.kit?.agents), [device.kit?.agents]);
  const newSession = useMemo(() => new Set((device.kit?.store_new_session ?? []).map((row) => `${row.item}\u0000${row.column}`)), [device.kit?.store_new_session]);
  const away = machine.blocked === "unreachable" || machine.blocked === "connecting" || machine.blocked === "disabled";
  const keysGone = report?.refusal === "keys_secret_gone";
  const readOnly = away || keysGone || machine.blocked === "unavailable" || report === null;
  const asks = useShellStore((s) => s.rest?.status?.store_asks)?.filter((ask) => ask.device_id === device.id) ?? [];
  const askRequested = useUiStore((s) => s.storeAskRequested);
  const [adding, setAdding] = useState(false);
  const [files, setFiles] = useState<StoreItem | null>(null);
  const [keys, setKeys] = useState<StoreItem | null>(null);
  const [adopting, setAdopting] = useState<AdoptTarget | null>(null);
  const [updates, setUpdates] = useState<string[] | null>(null);
  const [removed, setRemoved] = useState<StoreItem | null>(null);
  const [undone, setUndone] = useState(false);
  const removal = useStoreRequest(device.id, actions);
  const check = useStoreRequest(device.id, actions);
  const page: Page = {
    device,
    report: report ?? EMPTY_REPORT,
    local: device.id === own,
    readOnly,
    away,
    actions,
    newSession,
    openFiles: setFiles,
    openKeys: setKeys,
    openAdopt: setAdopting,
    openUpdate: (items) => setUpdates(items.map((item) => item.id)),
    remove: (item) => {
      setRemoved(item);
      setUndone(false);
      removal.send([{ op: "delete", item: item.id }]);
    },
    removing: removal.pending ? (removed?.id ?? null) : null,
  };
  const skills = report ? itemsOf(report, "skill") : [];
  const mcps = report ? itemsOf(report, "mcp") : [];
  const empty = report !== null && skills.length + mcps.length + report.unmanaged.length === 0;
  const newer = report ? updatable(report) : [];
  const ask = asks.find((row) => row.id === askRequested) ?? null;
  // The head (approved visual, 2026-11-20): one line under the page title, with 업데이트 n and the primary Add at its right.
  const head = (
    <div className="mb-xl flex flex-wrap items-center gap-sm" data-store-head="true">
      <p className="min-w-0 flex-1 text-body text-muted-foreground">{t("store.subtitle", { device: name })}</p>
      {newer.length > 0 && !readOnly ? (
        <UpdatesChip
          count={newer.length}
          checkedAt={report?.checked_at_unix_ms ?? null}
          onOpen={() => {
            // Opening the walk also checks again now (decision 2026-11-19).
            check.send([{ op: "check_updates" }]);
            page.openUpdate(newer);
          }}
        />
      ) : null}
      {machines.length > 1 ? (
        <Select value={device.id} onValueChange={onMachine}>
          <SelectTrigger size="sm" variant="ghost" aria-label={t("store.machine")} data-store-machines="true">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {machines.map((row) => (
              <SelectItem key={row.device.id} value={row.device.id} data-store-machine-choice={row.device.id}>
                {machineName(devices, row.device.id, t)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      ) : null}
      {device.id !== own && !readOnly && !empty ? <CopyFromHere device={device} own={own} actions={actions} /> : null}
      {empty ? null : (
        <Button size="sm" disabled={readOnly} onClick={() => setAdding(true)} data-store-add="true">
          <PlusIcon aria-hidden="true" />
          {t("store.add")}
        </Button>
      )}
    </div>
  );
  return (
    <div className="@container/store" data-store-machine={device.id}>
      {head}
      {asks.length > 0 ? (
        <div className="mb-lg flex flex-col gap-xs" data-store-asks="true">
          {asks.map((row) => (
            <div key={row.id} className="flex items-center gap-sm">
              <StatusMark mark="turn" />
              <span className="min-w-0 flex-1 truncate text-body text-foreground" data-store-ask-line={row.id}>{askTitle(row, t)}</span>
              <Button size="sm" variant="secondary" onClick={() => actions.openStoreAsk(row.id)} data-store-ask-open={row.id}>
                {t("agents.human.confirm")}
              </Button>
            </div>
          ))}
        </div>
      ) : null}
      <MachineState machine={machine} name={name} report={report} actions={actions} />
      {report === null && !away && machine.blocked !== "unavailable" ? <Loading /> : null}
      {empty ? (
        <div className="flex flex-col items-center gap-md py-xxl text-center" data-store-empty={device.id}>
          <p className="text-subhead text-muted-foreground">{t("store.empty", { device: name })}</p>
          {!readOnly ? (
            <div className="flex gap-sm">
              {device.id !== own ? <CopyFromHere device={device} own={own} actions={actions} /> : null}
              <Button size="sm" onClick={() => setAdding(true)} data-store-empty-add="true">
                <PlusIcon aria-hidden="true" />
                {t("store.add")}
              </Button>
            </div>
          ) : null}
        </div>
      ) : null}
      {report && !empty ? (
        <div data-store-lists={readOnly ? "read-only" : "live"}>
          <Kind kind="skill" items={skills} shown={shown} page={page} />
          <Kind kind="mcp" items={mcps} shown={shown} page={page} />
          {skills.length + mcps.length > 0 ? <Legend /> : null}
          {report.unmanaged.length > 0 ? <Unmanaged rows={report.unmanaged} report={report} shown={shown} page={page} /> : null}
        </div>
      ) : null}
      {/* The undo list stands apart from the lists: deleting the last item leaves it (B18). */}
      {report && report.trash.length > 0 ? <Trash report={report} page={page} /> : null}
      {removed && removal.refusal ? <Note tone="error" data-store-delete-refused={removed.id}>{refusalText(removal.refusal, t)}</Note> : null}
      {removed && removal.done && !undone && report ? <UndoToast key={removal.sent} item={removed} report={report} page={page} onDone={() => setUndone(true)} /> : null}
      {adding && report ? <AddDialog device={device} name={name} report={report} shown={shown} local={device.id === own} actions={actions} onClose={() => setAdding(false)} /> : null}
      {files ? <FilesDialog item={files} onClose={() => setFiles(null)} /> : null}
      {adopting && report ? <AdoptDialog device={device} target={adopting} report={report} shown={shown} actions={actions} onClose={() => setAdopting(null)} /> : null}
      {keys && report ? <KeysDialog device={device} item={keys} report={report} actions={actions} onClose={() => setKeys(null)} /> : null}
      {updates && report ? <UpdateDialog device={device} queue={updates} report={report} actions={actions} onClose={() => setUpdates(null)} /> : null}
      {ask && report ? <AskDialog ask={ask} report={report} shown={shown} actions={actions} onClose={() => useUiStore.getState().clearStoreAsk()} /> : null}
    </div>
  );
}

const EMPTY_REPORT: StoreReport = { columns: {}, items: [], unmanaged: [], trash: [], previews: [], diffs: [] };

function askTitle(ask: StoreAsk, t: TFunction<"translation">): string {
  switch (ask.what) {
    case "install":
      return t("store.ask.install", { names: ask.names.join(", ") });
    case "update":
      return t("store.ask.update", { name: ask.item.split(":").slice(1).join(":") });
    case "add_mcp":
      return t("store.ask.addMcp", { name: ask.server.name });
    case "set_mcp":
      return t("store.ask.setMcp", { name: ask.server.name });
  }
}

/** docs/agent-hooks.md, When the MCP keys' secret is lost: the operator's start-over by hand. */
const KEYS_LOST_URL = "https://github.com/with-hide/hide/blob/main/docs/agent-hooks.md#when-the-mcp-keys-secret-is-lost";

/** Why the machine's list cannot change now, in one line, with the one press that helps (scene 6, decisions: keys.secret). */
function MachineState({ machine, name, report, actions }: { machine: AgentMachine; name: string; report: StoreReport | null; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const { device } = machine;
  if (machine.blocked === "unreachable" || machine.blocked === "connecting" || machine.blocked === "disabled") {
    return (
      <div className="mb-lg flex flex-wrap items-center gap-sm">
        <span className="min-w-0 flex-1">
          <Note data-store-away={device.id} data-store-blocked={machine.blocked}>{t("store.disconnected", { device: name })}</Note>
        </span>
        {machine.blocked === "unreachable" ? (
          <Button variant="secondary" size="sm" onClick={() => actions.retryDevice(device.id)} data-store-reconnect={device.id}>
            {t("store.reconnect")}
          </Button>
        ) : null}
      </div>
    );
  }
  if (machine.blocked === "unavailable") {
    return <Note data-store-unavailable={device.id}>{device.kit?.unavailable ? kitUnavailableText(device.kit.unavailable, t) : t("store.unavailable", { device: name })}</Note>;
  }
  if (report?.refusal === "keys_secret_gone") {
    return (
      <div className="mb-lg">
        <Note tone="error" data-store-refusal="keys_secret_gone">
          {t("store.keysGone", { device: name })}{" "}
          <Button variant="link" size="sm" className="h-auto p-none" data-store-keys-help onClick={() => actions.openLink(KEYS_LOST_URL, true)}>{t("store.keysGoneHelp")}</Button>
        </Note>
      </div>
    );
  }
  if (report?.refusal === "unreadable") return <div className="mb-lg"><Note tone="error" data-store-refusal="unreadable">{t("store.unreadable", { device: name })}</Note></div>;
  return null;
}

/** The first read of a machine: three quiet rows where its list will stand. */
/** The first read of a machine: three quiet rows where its list will stand. */
function Loading() {
  const { t } = useInterfaceTranslation();
  return (
    <section className="mb-xl" data-store-loading="true">
      <div className="pb-sm"><SectionLabel title={t("store.section.skill")} caption={t("store.reading")} /></div>
      {[0, 1, 2].map((row) => (
        <div key={row} className="flex items-center gap-md border-t border-border py-sm" aria-hidden="true">
          <span className="size-(--size-control-lg) animate-pulse rounded-md bg-muted" />
          <span className="h-(--size-badge-height) w-1/4 animate-pulse rounded-xs bg-muted" />
          <span className="flex-1" />
          <span className="h-(--size-checkbox) w-(--size-control) animate-pulse rounded-xl bg-muted" />
        </div>
      ))}
    </section>
  );
}

function UpdatesChip({ count, checkedAt, onOpen }: { count: number; checkedAt: number | null; onOpen: () => void }) {
  const { t, i18n } = useInterfaceTranslation();
  const now = useMinuteClock();
  const words = t("store.updates", { count });
  const when = checkedAt ? t("store.updates.checked", { when: lastSeen(checkedAt, now, false, t, requireInterfaceLanguage(i18n.language)) }) : t("store.updates.never");
  return (
    <Hint label={`${words} · ${when}`}>
      <Button variant="secondary" size="sm" onClick={onOpen} data-store-updates={count}>
        <UpdateDot />
        {words}
      </Button>
    </Hint>
  );
}

/** The small dot after a version a check found newer (B36). */
function UpdateDot({ className }: { className?: string }) {
  return <span aria-hidden="true" className={cn("inline-block size-(--size-status-mark) shrink-0 rounded-full bg-primary", className)} />;
}

/** 이 Mac에서 복사: this Mac's list onto the device, with the keys typed again there (B27). */
function CopyFromHere({ device, own, actions }: { device: Device; own: string; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const [sent, setSent] = useState<string | null>(null);
  const answer = useShellStore((s) => (sent ? s.rest?.status?.store_answers?.find((row) => row.request_id === sent) : undefined));
  const pending = sent !== null && (!answer || answer.state === "pending");
  return (
    <Button
      variant="secondary"
      size="sm"
      disabled={pending}
      aria-busy={pending}
      onClick={() => {
        const id = `store-copy-${Date.now().toString(36)}`;
        setSent(id);
        actions.copyStore(device.id, id);
      }}
      data-store-copy={device.id}
    >
      {t("store.copyFrom", { device: machineName(devices, own, t) })}
    </Button>
  );
}

/** A section's title and count, with the page's controls at its right when it is the first. */

/** A section's small title and count, at the head of its table's first column. */
function SectionLabel({ title, count, caption }: { title: string; count?: number; caption?: string }) {
  return (
    <span className="flex min-w-0 items-baseline gap-sm text-micro font-semibold uppercase tracking-wider text-muted-foreground">
      <span className="shrink-0">{title}</span>
      {count !== undefined ? <span className="font-medium">{count}</span> : null}
      {caption ? <span className="min-w-0 truncate font-normal normal-case tracking-normal">{caption}</span> : null}
    </span>
  );
}

/**
 * One kind's table (the approved visual of 2026-11-20): the section's title
 * heads the name column and each agent's logo heads its cell column; a skill
 * row has a cell per skill folder (Claude, 공용, agy), an MCP row one per
 * agent. Cells show the state only, the row's one switch turns the item on
 * or off everywhere, and a press on the row unfolds it below. The switch
 * column has one width in both tables, so they end at the same edge.
 */
function Kind({ kind, items, shown, page }: { kind: StoreItemKind; items: StoreItem[]; shown: ReadonlySet<string>; page: Page }) {
  const { t } = useInterfaceTranslation();
  const columns = shownColumns(page.report, kind, shown);
  if (items.length === 0) return null;
  const title = t(kind === "skill" ? "store.section.skill" : "store.section.mcp");
  // Every head's logos stand on one line; the shared head's word hangs under its logos, in room this padding keeps.
  const head = cn("align-bottom font-normal", columns.some((column) => column.shared) ? "pb-xl" : "pb-sm");
  return (
    <section className="mb-xl" data-store-section={kind}>
      {/* A machine that is away shows its last list, dimmed and with no controls (scene 6). */}
      <div className={cn("overflow-x-auto", page.away && "opacity-(--opacity-disabled)")} inert={page.readOnly || undefined}>
        <table className="w-full min-w-(--size-settings-control-w) table-fixed border-collapse text-body" data-store-table={kind}>
          <colgroup>
            <col />
            {columns.map((column) => <col key={column.id} className={column.shared ? "w-(--size-store-shared-column)" : "w-(--size-store-column)"} />)}
            <col className="w-(--size-store-switch-column)" />
          </colgroup>
          <thead>
            <tr>
              <th className={cn(head, "text-left")}><SectionLabel title={title} count={items.length} /></th>
              {columns.map((column) => (
                <th key={column.id} className={cn(head, "text-center")} data-store-column={column.id}>
                  <ColumnHead column={column} columns={columns} />
                </th>
              ))}
              <th className={head}><span className="sr-only">{t("store.column.on")}</span></th>
            </tr>
          </thead>
          <tbody>
            {items.map((item) => <ItemRow key={item.id} item={item} columns={columns} page={page} />)}
          </tbody>
        </table>
      </div>
    </section>
  );
}

/**
 * A column's head: its agent's logo, named in its hint; the shared skill
 * folder's is the logos of the installed agents that read it, overlapped,
 * with 공용 under them, and a hint that names them and which of them read
 * another column's folder as well (Grok and omp read Claude's).
 */
function ColumnHead({ column, columns }: { column: ShownColumn; columns: readonly ShownColumn[] }) {
  const { t } = useInterfaceTranslation();
  const name = columnName(column, t("store.column.shared"));
  if (!column.shared) {
    return (
      <Hint label={name}>
        <span className="inline-flex align-bottom outline-none focus-visible:ring-1 focus-visible:ring-ring" tabIndex={0} data-store-agent-head={column.agent ?? column.id}>
          <AgentMark kind={column.agent} />
          <span className="sr-only">{name}</span>
        </span>
      </Hint>
    );
  }
  const readers = column.readers.map(agentShortName);
  const also = columns
    .filter((other) => other.id !== column.id)
    .map((other) => ({ other, agents: other.readers.filter((agent) => agent !== other.agent && column.readers.includes(agent)) }))
    .filter(({ agents }) => agents.length > 0);
  const hint = [t("store.column.sharedHint", { agents: readers.join(" · ") }), ...also.map(({ other, agents }) => t("store.column.sharedAlso", { agents: agents.map(agentShortName).join(" · "), column: columnName(other, name) }))].join(" ");
  return (
    <Hint label={hint}>
      <span className="relative inline-flex align-bottom outline-none focus-visible:ring-1 focus-visible:ring-ring" tabIndex={0} data-store-shared-hint={column.id}>
        <span className="flex" aria-hidden="true">
          {column.readers.map((agent, index) => (
            <span key={agent} className={cn("inline-flex rounded-xs bg-dialog ring-1 ring-dialog", index > 0 && "-ml-xxs")}>
              <AgentMark kind={agent} />
            </span>
          ))}
        </span>
        <span className="absolute left-1/2 top-full mt-xxs -translate-x-1/2 whitespace-nowrap text-micro text-muted-foreground">{name}</span>
      </span>
    </Hint>
  );
}

/** The GitHub mark (Primer Octicons `mark-github`, MIT), the tile of a GitHub skill whose owner's picture is not kept. */
function GithubMark({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 16 16" aria-hidden="true" className={className}>
      <path fill="currentColor" d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8Z" />
    </svg>
  );
}

/**
 * An item's icon tile: a GitHub skill's owner's picture as the store keeps
 * it (never loaded from the web here), else the mark of where it comes from:
 * GitHub, another git host, a folder, a package or a remote server.
 */
function ItemTile({ item, avatars }: { item: StoreItem; avatars: Record<string, string> | undefined }) {
  const picture = item.owner ? avatars?.[item.owner] : undefined;
  if (picture) return <img src={picture} alt="" className="size-(--size-control-lg) shrink-0 rounded-md border border-border object-cover" data-store-avatar={item.owner ?? ""} />;
  const glyph = "size-(--size-icon)";
  const mark = item.owner ? <GithubMark className={glyph} />
    : item.server ? (item.server.reach.kind === "remote" ? <GlobeIcon className={glyph} /> : <PackageIcon className={glyph} />)
    : item.source.kind === "git" ? <GitBranchIcon className={glyph} />
    : item.source.kind === "folder" ? <FolderIcon className={glyph} />
    : <FileIcon className={glyph} />;
  return (
    <span aria-hidden="true" className="flex size-(--size-control-lg) shrink-0 items-center justify-center rounded-md border border-border bg-secondary text-subtle-foreground" data-store-tile={item.owner ? "github" : item.source.kind}>
      {mark}
    </span>
  );
}

/** The short version as a small pill, with the dot after it when a check found a newer one. */
function Version({ item }: { item: StoreItem }) {
  const { t } = useInterfaceTranslation();
  const text = versionText(item.version, t);
  if (!text && !item.update) return null;
  return (
    <span className="inline-flex min-w-0 shrink-[3] items-center gap-xs" data-store-version={item.id}>
      {text ? <span className="truncate rounded-xs bg-secondary px-xs font-mono text-micro text-muted-foreground">{text}</span> : null}
      {item.update ? (
        <span className="inline-flex shrink-0" title={t("store.version.update")} data-store-update-dot={item.id}>
          <UpdateDot />
          <span className="sr-only">{t("store.version.update")}</span>
        </span>
      ) : null}
    </span>
  );
}

/** A row: its tile, name, version and line, one state cell per column and its one switch; a press anywhere but the switch unfolds it. */
function ItemRow({ item, columns, page }: { item: StoreItem; columns: ShownColumn[]; page: Page }) {
  const { t } = useInterfaceTranslation();
  const [open, setOpen] = useState(false);
  const request = useStoreRequest(page.device.id, page.actions);
  const all = allSwitch(item, columns);
  const toggle = () => setOpen((now) => !now);
  return (
    <>
      <tr className="cursor-pointer border-t border-border hover:bg-accent" onClick={toggle} data-store-row={item.id} data-store-open={open ? "true" : "false"}>
        <td className="relative py-sm pr-md">
          <RowOverlayButton
            hover={false}
            aria-label={t("store.expand", { name: item.name })}
            aria-expanded={open}
            onClick={(event) => {
              event.stopPropagation();
              toggle();
            }}
            data-store-row-toggle={item.id}
          />
          <span className="pointer-events-none relative flex min-w-0 items-center gap-md">
            <ItemTile item={item} avatars={page.report.avatars ?? undefined} />
            <span className="flex min-w-0 flex-col">
              <span className="flex min-w-0 items-center gap-sm">
                <span className="min-w-0 truncate text-subhead font-semibold text-foreground">{item.name}</span>
                <Version item={item} />
                {item.folder_missing ? <Badge variant="outline" className="shrink-0 border-warning text-warning">{t("store.detail.folderMissing")}</Badge> : null}
              </span>
              <span className="truncate text-caption text-muted-foreground" data-store-line={item.id}>{rowLine(item)}</span>
            </span>
          </span>
        </td>
        {columns.map((column) => (
          <td key={column.id} className="py-sm text-center">
            <CellMark item={item} column={column} cell={cellOf(item, column)} page={page} />
          </td>
        ))}
        <td className="py-sm text-right" onClick={(event) => event.stopPropagation()}>
          <Switch checked={all.checked} disabled={request.pending} aria-busy={request.pending} onCheckedChange={() => request.send([all.change])} aria-label={t("store.switch.all", { name: item.name })} data-store-all={item.id} />
        </td>
      </tr>
      {request.refusal ? (
        <tr>
          <td colSpan={columns.length + 2} className="pb-sm pl-(--size-store-detail-inset)"><Note tone="error">{refusalText(request.refusal, t)}</Note></td>
        </tr>
      ) : null}
      {open ? (
        <tr>
          <td colSpan={columns.length + 2} className="p-none">
            <RowDetail item={item} columns={columns} page={page} />
          </td>
        </tr>
      ) : null}
    </>
  );
}

/** The four marks a cell can show, drawn once for the cells and the legend. */
function OnMark({ dim = false }: { dim?: boolean }) {
  return (
    <span aria-hidden="true" className={cn("inline-flex size-(--size-icon-lg) items-center justify-center rounded-full bg-primary/15 text-primary", dim && "opacity-(--opacity-dimmed)")}>
      <CheckIcon className="size-(--size-icon-sm)" strokeWidth={3} />
    </span>
  );
}

function OffMark({ dim = false }: { dim?: boolean }) {
  return <span aria-hidden="true" className={cn("inline-block size-(--size-agent-mark) rounded-full border border-muted-foreground opacity-(--opacity-read-status)", dim && "opacity-(--opacity-disabled)")} />;
}

function UnsupportedMark() {
  return <span aria-hidden="true" className="text-muted-foreground opacity-(--opacity-dimmed)">—</span>;
}

function FailedMark() {
  return <span aria-hidden="true" className="inline-flex size-(--size-icon-lg) items-center justify-center rounded-full bg-destructive/15 text-caption font-bold text-destructive">!</span>;
}

function TheirsMark() {
  return (
    <span aria-hidden="true" className="inline-flex size-(--size-icon-lg) items-center justify-center rounded-full bg-warning/15 text-warning">
      <HandIcon className="size-(--size-icon-sm)" />
    </span>
  );
}

/**
 * A cell's state, which it only shows (decision 2026-11-20): ✓ applied, ○
 * off, — not supported (its hint says by whom), ! failed (the row unfolds
 * with why and Retry), and the hand for a copy of the same name Hide did not
 * write. A press on a cell is the row's.
 */
function CellMark({ item, column, cell, page }: { item: StoreItem; column: ShownColumn; cell: StoreCell; page: Page }) {
  const { t } = useInterfaceTranslation();
  const name = columnName(column, t("store.column.shared"));
  const fresh = page.newSession.has(`${item.id}\u0000${column.id}`);
  const [mark, title] = (() => {
    switch (cell.state) {
      case "unsupported":
        return [<UnsupportedMark key="mark" />, t("store.cell.unsupported", { agent: name })];
      case "failed":
        return [<FailedMark key="mark" />, `${name} · ${t(failureKey(cell.failure, item.kind))}`];
      case "hand_written":
        return [<TheirsMark key="mark" />, t("store.cell.handWritten")];
      case "agent_off":
        return [<OffMark key="mark" dim />, t("store.cell.agentOff")];
      case "waiting":
        return [<OnMark key="mark" dim />, t("store.cell.waiting")];
      case "on":
        return [<OnMark key="mark" />, fresh ? t("store.cell.newSession") : t("store.cell.on")];
      case "off":
        return [<OffMark key="mark" />, t("store.cell.off")];
    }
  })();
  return (
    <span className="inline-flex align-middle" title={title} data-store-cell={`${item.id}:${column.id}`} data-store-cell-state={cell.state} data-store-new-session={fresh ? "true" : undefined}>
      {mark}
      <span className="sr-only">{title}</span>
    </span>
  );
}

/** What the marks mean, once under the tables. */
function Legend() {
  const { t } = useInterfaceTranslation();
  const entry = "inline-flex items-center gap-xs";
  return (
    <div className="-mt-md mb-xl flex flex-wrap items-center gap-x-lg gap-y-xs text-caption text-muted-foreground" data-store-legend="true">
      <span className={entry}><OnMark />{t("store.legend.on")}</span>
      <span className={entry}><OffMark />{t("store.legend.off")}</span>
      <span className={entry}><UnsupportedMark />{t("store.legend.unsupported")}</span>
      <span className={entry}><FailedMark />{t("store.legend.failed")}</span>
      <span>{t("store.legend.logos")}</span>
    </div>
  );
}

/** A path as one line: home as `~`, the front shortened before the last name, the whole path on hover. */
function PathText({ path, home }: { path: string; home: string | null | undefined }) {
  const { head, tail } = pathParts(homePath(path, home));
  return (
    <Hint label={path}>
      <span className="inline-flex min-w-0 max-w-full overflow-hidden font-mono" tabIndex={0} data-store-path={path}>
        {head ? <span className="min-w-0 truncate">{head}</span> : null}
        <span className="shrink-0 whitespace-nowrap">{tail}</span>
      </span>
    </Hint>
  );
}

/**
 * The unfolded row, a band one step below the dialog (so its quiet buttons
 * keep their edge in both themes) indented under the name: a grid of one
 * label column (출처, 파일, 상태; 명령, 키; 문제) and then the item's quiet
 * actions.
 */
function RowDetail({ item, columns, page }: { item: StoreItem; columns: ShownColumn[]; page: Page }) {
  const { t } = useInterfaceTranslation();
  const files = item.files ?? [];
  const executables = files.filter((file) => file.executable).length;
  const cells = columns.map((column) => ({ column, cell: cellOf(item, column) }));
  const fixes = cells.filter(({ cell }) => cell.state === "failed" || cell.state === "hand_written");
  // A row on for some agents and off for others says so, and what its switch does (D-40).
  const offFor = cells.filter(({ cell }) => cell.state === "off").map(({ column }) => columnName(column, t("store.column.shared")));
  const partial = offFor.length > 0 && cells.some(({ cell }) => cellOn(cell.state));
  const label = "self-start pt-xxs text-caption text-muted-foreground";
  const value = "flex min-w-0 items-center gap-sm text-body text-subtle-foreground";
  return (
    <div className="grid grid-cols-[var(--size-store-label)_minmax(0,1fr)] items-center gap-x-md gap-y-sm bg-card py-md pl-(--size-store-detail-inset) pr-md" data-store-detail={item.id}>
      {item.server ? (
        <>
          <span className={label}>{t(item.server.reach.kind === "local" ? "store.detail.command" : "store.detail.address")}</span>
          <span className={cn(value, "font-mono")}><span className="truncate" title={serverLine(item.server)}>{serverLine(item.server)}</span></span>
          {keyNames(item.server).length > 0 ? (
            <>
              <span className={label}>{t("store.detail.keys")}</span>
              <span className={cn(value, "flex-wrap")} data-store-keys={item.id}>
                {keyNames(item.server).map((key) => (
                  <span key={key} className="font-mono">{key} <span className="text-muted-foreground">••••••</span></span>
                ))}
                <Button variant="secondary" size="sm" disabled={page.readOnly} onClick={() => page.openKeys(item)} data-store-edit-keys={item.id}>
                  {t("store.detail.editKeys")}
                </Button>
              </span>
            </>
          ) : null}
        </>
      ) : (
        <>
          <span className={label}>{t("store.detail.source")}</span>
          <span className={value}>
            {item.source.kind === "folder" ? <PathText path={item.source.path} home={page.report.home} /> : <span className="truncate font-mono" title={sourceLine(item)}>{sourceLine(item)}</span>}
            {item.folder_missing ? <span className="shrink-0 text-muted-foreground">· {t("store.detail.lastCopy")}</span> : null}
          </span>
          {files.length > 0 ? (
            <>
              <span className={label}>{t("store.detail.files")}</span>
              <span className={value}>
                {t("store.detail.fileCount", { count: files.length })}
                {executables > 0 ? <span className="text-warning">· {t("store.detail.executables", { count: executables })}</span> : null}
              </span>
            </>
          ) : null}
        </>
      )}
      {partial ? (
        <>
          <span className={label}>{t("store.detail.state")}</span>
          <span className={cn(value, "text-muted-foreground")} data-store-partial={item.id}>{t("store.detail.partial", { agents: offFor.join(", ") })}</span>
        </>
      ) : null}
      {fixes.map(({ column, cell }, index) => (
        <CellFix key={column.id} first={index === 0} item={item} column={column} cell={cell} page={page} />
      ))}
      <span />
      <RowActions item={item} page={page} />
    </div>
  );
}

/**
 * A cell that needs a press, in one line of the 문제 rows: the agent's logo,
 * its name and a few words for the kind of failure, the file, and
 * [다시 시도]; or a copy Hide did not write with [가져오기]. The kit's own
 * sentence is the log's (design principle 13).
 */
function CellFix({ first, item, column, cell, page }: { first: boolean; item: StoreItem; column: ShownColumn; cell: StoreCell; page: Page }) {
  const { t } = useInterfaceTranslation();
  const request = useStoreRequest(page.device.id, page.actions);
  const name = columnName(column, t("store.column.shared"));
  const failed = cell.state === "failed";
  const words = failed ? t(failureKey(cell.failure, item.kind)) : t("store.cell.handWritten");
  return (
    <>
      <span className="self-start pt-xxs text-caption text-muted-foreground">{first ? t("store.detail.problem") : null}</span>
      <span className="flex min-w-0 items-center gap-sm text-body" data-store-fix={`${item.id}:${column.id}`}>
        {column.agent ? <AgentMark kind={column.agent} /> : null}
        <span className="flex min-w-0 flex-1 flex-col gap-xxs">
          <span className={cn("line-clamp-2 break-words", failed ? "text-destructive" : "text-warning")} title={t("store.failure.line", { agent: name, words })}>{t("store.failure.line", { agent: name, words })}</span>
          {cell.path ? <span className="flex min-w-0 text-caption text-muted-foreground"><PathText path={cell.path} home={page.report.home} /></span> : null}
          {request.refusal ? <span className="text-caption text-destructive">{refusalText(request.refusal, t)}</span> : null}
        </span>
        {failed ? (
          <Button variant="secondary" size="sm" className="shrink-0" disabled={request.pending} onClick={() => request.send([{ op: "set", item: item.id, column: column.id, on: columnWants(item, column.id) }])} data-store-retry={`${item.id}:${column.id}`}>
            {t("store.retry")}
          </Button>
        ) : (
          <Button variant="secondary" size="sm" className="shrink-0" disabled={page.readOnly} onClick={() => page.openAdopt({ kind: item.kind, name: item.name, from: cell.place ?? undefined, path: cell.path ? (item.kind === "skill" ? joinPath(cell.path, item.name) : cell.path) : null, server: cell.server ?? null })} data-store-take-over={`${item.id}:${column.id}`}>
            {t("store.cell.takeOver")}
          </Button>
        )}
      </span>
    </>
  );
}

/**
 * What can be done to an item, one row of quiet buttons: its update first
 * when there is one, its files and source, then Delete, which goes on a
 * second press (the first turns it into its confirm word) and leaves Undo.
 */
function RowActions({ item, page }: { item: StoreItem; page: Page }) {
  const { t } = useInterfaceTranslation();
  const source = openableSource(item, page.local);
  const [confirming, setConfirming] = useState(false);
  return (
    <div className="flex flex-wrap items-center gap-sm">
      {item.update ? (
        <Button variant="secondary" size="sm" onClick={() => page.openUpdate([item])} data-store-update={item.id}>
          <UpdateDot />
          {t("store.action.update")}
        </Button>
      ) : null}
      {(item.files ?? []).length > 0 ? (
        <Button variant="secondary" size="sm" onClick={() => page.openFiles(item)} data-store-files={item.id}>
          {t("store.action.files")}
        </Button>
      ) : null}
      {source ? (
        <Button variant="secondary" size="sm" onClick={() => (source.kind === "url" ? page.actions.openLink(source.url, true) : page.actions.revealExternal(source.path))} data-store-open-source={item.id}>
          {t(source.kind === "url" ? "store.action.openSource" : "store.action.openFolder")}
        </Button>
      ) : null}
      <Button
        variant={confirming ? "destructive" : "secondary"}
        size="sm"
        disabled={page.removing === item.id}
        onClick={() => (confirming ? page.remove(item) : setConfirming(true))}
        onBlur={() => setConfirming(false)}
        data-store-delete={item.id}
        data-store-delete-confirm={confirming ? "true" : undefined}
      >
        {t(confirming ? "store.action.deleteConfirm" : "store.action.delete")}
      </Button>
    </div>
  );
}

/** 에이전트에만 있음 (B19): what agents hold that Hide did not write, each with Apply to all. */

/** 에이전트에만 있음 (B19): what agents hold that Hide did not write, in the tables' row style, each with Apply to all. */
function Unmanaged({ rows, report, shown, page }: { rows: StoreUnmanaged[]; report: StoreReport; shown: ReadonlySet<string>; page: Page }) {
  const { t } = useInterfaceTranslation();
  return (
    <section className="mb-xl" data-store-section="unmanaged">
      <div className="pb-sm"><SectionLabel title={t("store.section.unmanaged")} count={rows.length} caption={t("store.section.unmanagedCaption")} /></div>
      <div className={cn(page.away && "opacity-(--opacity-disabled)")} inert={page.readOnly || undefined}>
        {rows.map((row) => <UnmanagedRow key={`${row.kind}:${row.name}`} row={row} report={report} shown={shown} page={page} />)}
      </div>
    </section>
  );
}

function UnmanagedRow({ row, report, shown, page }: { row: StoreUnmanaged; report: StoreReport; shown: ReadonlySet<string>; page: Page }) {
  const { t } = useInterfaceTranslation();
  const columns = shownColumns(report, row.kind, shown);
  const where = (agents: string[]) => {
    const names = columns.filter((column) => column.readers.some((agent) => agents.includes(agent)) || agents.some((agent) => column.agent === agent)).map((column) => columnName(column, t("store.column.shared")));
    return names.length > 0 ? names.join(", ") : agents.map(agentShortName).join(", ");
  };
  const kind = t(row.kind === "skill" ? "store.section.skill" : "store.section.mcp");
  const agents = [...new Set(row.copies.flatMap((copy) => copy.agents))];
  const Glyph = row.kind === "skill" ? FolderIcon : PackageIcon;
  return (
    <div className="flex flex-wrap items-center gap-x-md gap-y-xs border-t border-border py-sm" data-store-unmanaged={`${row.kind}:${row.name}`}>
      <span aria-hidden="true" className="flex size-(--size-control-lg) shrink-0 items-center justify-center rounded-md border border-border bg-secondary text-subtle-foreground">
        <Glyph className="size-(--size-icon)" />
      </span>
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="truncate text-subhead font-semibold text-foreground">{row.name}</span>
        <span className="truncate text-caption text-muted-foreground">
          {kind} · {where(agents)}
          {row.same ? null : <span className="text-warning"> · {t("store.unmanaged.differs")}</span>}
        </span>
      </span>
      {row.same || row.copies.length < 2 ? (
        <Button variant="secondary" size="sm" disabled={page.readOnly} onClick={() => page.openAdopt({ kind: row.kind, name: row.name, path: copyPath(row.copies[0]), server: row.copies[0]?.server ?? null })} data-store-adopt={`${row.kind}:${row.name}`}>
          {t("store.unmanaged.applyAll")}
        </Button>
      ) : (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="secondary" size="sm" disabled={page.readOnly} data-store-adopt={`${row.kind}:${row.name}`}>
              {t("store.unmanaged.applyAll")}
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            {row.copies.map((copy) => (
              <DropdownMenuItem key={copy.place} onSelect={() => page.openAdopt({ kind: row.kind, name: row.name, from: copy.place, path: copyPath(copy), server: copy.server ?? null })} data-store-adopt-from={copy.place}>
                {t("store.unmanaged.fromWhich", { agent: where(copy.agents) })}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      )}
    </div>
  );
}

/** Where a copy is on its machine: the folder a link there leads to, else the copy itself. */
function copyPath(copy: StoreFoundCopy | undefined): string | null {
  return copy ? (copy.link ?? copy.path) : null;
}

/** A name under a folder of the machine, with the separator the folder's path has. */
function joinPath(folder: string, name: string): string {
  const separator = folder.includes("\\") && !folder.includes("/") ? "\\" : "/";
  return `${folder.replace(/[\\/]+$/, "")}${separator}${name}`;
}

function Trash({ report, page }: { report: StoreReport; page: Page }) {
  const { t } = useInterfaceTranslation();
  const [open, setOpen] = useState(false);
  const rows = [...report.trash].sort((a, b) => b.at_unix_ms - a.at_unix_ms);
  const Chevron = open ? ChevronDownIcon : ChevronRightIcon;
  return (
    <section className="mb-xl" data-store-trash="true">
      <Button variant="text" size="inline" className="text-body" aria-expanded={open} onClick={() => setOpen((now) => !now)} data-store-trash-toggle="true">
        <Chevron aria-hidden="true" className="size-(--size-icon)" />
        {t("store.trash.count", { count: rows.length })}
      </Button>
      {open ? (
        <div className="mt-xs" inert={page.readOnly || undefined}>
          {rows.map((entry) => <TrashRow key={entry.id} entry={entry} page={page} />)}
        </div>
      ) : null}
    </section>
  );
}

function TrashRow({ entry, page }: { entry: StoreReport["trash"][number]; page: Page }) {
  const { t } = useInterfaceTranslation();
  const restore = useStoreRequest(page.device.id, page.actions);
  const forget = useStoreRequest(page.device.id, page.actions);
  const kind = t(entry.kind === "skill" ? "store.section.skill" : "store.section.mcp");
  return (
    <div className="flex flex-wrap items-center gap-x-md gap-y-xs border-t border-border py-sm pl-(--size-icon-lg)" data-store-trash-entry={entry.id}>
      <span className="flex min-w-0 flex-1 items-baseline gap-sm">
        <span className="truncate text-subhead text-foreground">{entry.name}</span>
        <span className="truncate text-caption text-muted-foreground">
          {kind}
          {entry.cause.kind === "replaced" ? ` · ${t("store.trash.replaced")}` : null}
        </span>
      </span>
      {restore.refusal === "name_taken" ? <Note tone="error" data-store-trash-taken={entry.id}>{t("store.trash.nameTaken", { name: entry.name })}</Note> : restore.refusal ? <Note tone="error">{refusalText(restore.refusal, t)}</Note> : null}
      <Button variant="secondary" size="sm" disabled={restore.pending || forget.pending} onClick={() => restore.send([{ op: "restore", entry: entry.id }])} data-store-restore={entry.id}>
        {t("store.trash.restore")}
      </Button>
      <Button variant="secondary" size="sm" disabled={restore.pending || forget.pending} onClick={() => forget.send([{ op: "forget", entry: entry.id }])} data-store-forget={entry.id}>
        {t("store.trash.forget")}
      </Button>
    </div>
  );
}


/** The notice a delete leaves on the page, with Undo while it stands (scene 5). */
function UndoToast({ item, report, page, onDone }: { item: StoreItem; report: StoreReport; page: Page; onDone: () => void }) {
  const { t } = useInterfaceTranslation();
  const restore = useStoreRequest(page.device.id, page.actions);
  const entry = trashFor(report, item);
  useEffect(() => {
    if (restore.done) onDone();
  }, [restore.done, onDone]);
  return (
    <div className="pointer-events-none sticky bottom-md z-10 mt-md flex justify-end">
      <Toast
        level="info"
        timeout={10_000}
        onDismiss={onDone}
        dismissLabel={t("common.close")}
        className="w-(--size-toast-w) max-w-full"
        data-store-undo={item.id}
        actions={
          entry ? (
            <Button variant="link" size="inline" disabled={restore.pending} onClick={() => restore.send([{ op: "restore", entry: entry.id }])} data-store-undo-restore={entry.id}>
              {t("store.undo")}
            </Button>
          ) : null
        }
      >
        {t("store.deleted", { name: item.name })}
        {restore.refusal === "name_taken" ? <span className="block text-destructive">{t("store.trash.nameTaken", { name: item.name })}</span> : null}
      </Toast>
    </div>
  );
}
