import type { TFunction } from "i18next";
import { ChevronRightIcon, EllipsisIcon, InfoIcon, PlusIcon } from "lucide-react";
import { useEffect, useMemo, useState, type ReactNode } from "react";
import type { Actions } from "../actions";
import { useMinuteClock } from "../components/elapsed";
import { Disclosure, Group, Note, Row } from "../components/settings-rows";
import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "../components/ui/dropdown-menu";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../components/ui/select";
import { Switch } from "../components/ui/switch";
import { Toast } from "../components/ui/toast";
import { ToggleGroup, ToggleGroupItem } from "../components/ui/toggle-group";
import { Hint } from "../components/ui/tooltip";
import { useInterfaceTranslation } from "../i18n/client";
import { requireInterfaceLanguage } from "../i18n/locale";
import { kitUnavailableText } from "../kitWhy";
import { cn } from "../lib/utils";
import { lastSeen } from "../mobileSettings";
import { machineName, windowDeviceId, windowFirst } from "../screenMachine";
import type { Device, StoreAsk, StoreCell, StoreItem, StoreItemKind, StoreReport, StoreUnmanaged } from "../snapshot";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";
import { agentMachines, type AgentMachine } from "./agentRows";
import { AddDialog, AskDialog, FilesDialog, KeysDialog, UpdateDialog, refusalText } from "./StoreDialogs";
import {
  agentShortName,
  allSwitch,
  cellOf,
  cellOn,
  columnName,
  columnWants,
  itemsOf,
  keyNames,
  openableSource,
  rowSummary,
  saveStoreView,
  savedStoreView,
  serverLine,
  shownAgents,
  shownColumns,
  sourceLine,
  trashFor,
  updatable,
  versionText,
  type RowSummary,
  type ShownColumn,
  type StoreView,
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
  actions: Actions;
  newSession: ReadonlySet<string>;
  openFiles: (item: StoreItem) => void;
  openKeys: (item: StoreItem) => void;
  openUpdate: (items: StoreItem[]) => void;
  /** Deletes the item; the page keeps the request, since the row goes before its answer lands. */
  remove: (item: StoreItem) => void;
  /** The item whose delete is on its way. */
  removing: string | null;
};

/**
 * Settings › 스킬·MCP (PRD skill-mcp-manager; decisions of 2026-11-19, mockup
 * scenes 1-3 to 6): one machine's skills and MCP servers at a time. The list
 * is the default and a row unfolds in place; 목록 | 표 turns to the table,
 * remembered for this viewer. Everything shown is the machine's last store
 * report; every press is one store request the core runs on that machine.
 */
export function SkillsTab({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const remote = useShellStore((s) => s.rest?.status?.remote);
  const own = useShellStore((s) => windowDeviceId(s.rest?.navigator?.devices));
  const asks = useShellStore((s) => s.rest?.status?.store_asks);
  const askRequested = useUiStore((s) => s.storeAskRequested);
  const [selected, setSelected] = useState<string | null>(null);
  const [view, setView] = useState<StoreView>(savedStoreView);
  useAgentsDemand(actions, true);
  const machines = agentMachines(windowFirst(devices), remote);
  // 확인하기 on an agent's ask opens the machine it asks about.
  const askDevice = asks?.find((ask) => ask.id === askRequested)?.device_id ?? null;
  const machine = machines.find((row) => row.device.id === (askDevice ?? selected ?? own)) ?? machines[0];
  if (!machine) return <Note>{t("store.reading")}</Note>;
  const choose = (next: StoreView) => {
    setView(next);
    saveStoreView(next);
  };
  return (
    <MachineStore
      key={machine.device.id}
      machine={machine}
      machines={machines}
      own={own}
      view={view}
      onView={choose}
      onMachine={(id) => {
        setSelected(id);
        useUiStore.getState().clearStoreAsk();
      }}
      devices={devices}
      actions={actions}
    />
  );
}

function MachineStore({ machine, machines, own, view, onView, onMachine, devices, actions }: { machine: AgentMachine; machines: AgentMachine[]; own: string; view: StoreView; onView: (view: StoreView) => void; onMachine: (id: string) => void; devices: Device[] | undefined; actions: Actions }) {
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
    actions,
    newSession,
    openFiles: setFiles,
    openKeys: setKeys,
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
  return (
    <div className="@container/store" data-store-machine={device.id} data-store-view={view}>
      <div className="mb-lg flex flex-wrap items-center gap-sm" data-store-head="true">
        <ToggleGroup type="single" value={view} onValueChange={(next) => next && onView(next as StoreView)} aria-label={t("store.view.label")} data-store-view-switch="true">
          <ToggleGroupItem value="list" data-store-view-choice="list">{t("store.view.list")}</ToggleGroupItem>
          <ToggleGroupItem value="table" data-store-view-choice="table">{t("store.view.table")}</ToggleGroupItem>
        </ToggleGroup>
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
        <span className="flex-1" />
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
        <Button size="sm" disabled={readOnly} onClick={() => setAdding(true)} data-store-add="true">
          <PlusIcon aria-hidden="true" />
          {t("store.add")}
        </Button>
      </div>
      {asks.length > 0 ? (
        <Group data-store-asks="true">
          {asks.map((row) => (
            <Row key={row.id} label={<span className="text-warning" data-store-ask-line={row.id}>{askTitle(row, t)}</span>} controlsEnd>
              <Button size="sm" variant="secondary" onClick={() => actions.openStoreAsk(row.id)} data-store-ask-open={row.id}>
                {t("agents.human.confirm")}
              </Button>
            </Row>
          ))}
        </Group>
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
        // A machine that is away shows its last list, dimmed and with no controls (scene 6).
        <div className={cn(away && "opacity-(--opacity-disabled)")} inert={readOnly || undefined} data-store-lists={readOnly ? "read-only" : "live"}>
          <Kind kind="skill" items={skills} shown={shown} view={view} page={page} />
          <Kind kind="mcp" items={mcps} shown={shown} view={view} page={page} />
          {report.unmanaged.length > 0 ? <Unmanaged rows={report.unmanaged} report={report} shown={shown} page={page} /> : null}
          {report.trash.length > 0 ? <Trash report={report} page={page} /> : null}
          {view === "table" ? <Legend /> : null}
        </div>
      ) : null}
      {removed && removal.refusal ? <Note tone="error" data-store-delete-refused={removed.id}>{refusalText(removal.refusal, t)}</Note> : null}
      {removed && removal.done && !undone && report ? <UndoToast key={removal.sent} item={removed} report={report} page={page} onDone={() => setUndone(true)} /> : null}
      {adding && report ? <AddDialog device={device} name={name} report={report} shown={shown} local={device.id === own} actions={actions} onClose={() => setAdding(false)} /> : null}
      {files ? <FilesDialog item={files} onClose={() => setFiles(null)} /> : null}
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
      <Group>
        <Row label={<Note data-store-away={device.id} data-store-blocked={machine.blocked}>{t("store.disconnected", { device: name })}</Note>} controlsEnd>
          {machine.blocked === "unreachable" ? (
            <Button variant="secondary" size="sm" onClick={() => actions.retryDevice(device.id)} data-store-reconnect={device.id}>
              {t("store.reconnect")}
            </Button>
          ) : null}
        </Row>
      </Group>
    );
  }
  if (machine.blocked === "unavailable") {
    return <Note data-store-unavailable={device.id}>{device.kit?.unavailable ? kitUnavailableText(device.kit.unavailable, t) : t("store.unavailable", { device: name })}</Note>;
  }
  if (report?.refusal === "keys_secret_gone") {
    return (
      <Note tone="error" data-store-refusal="keys_secret_gone">
        {t("store.keysGone", { device: name })}{" "}
        <Button variant="link" size="sm" className="h-auto p-none" data-store-keys-help onClick={() => actions.openLink(KEYS_LOST_URL, true)}>{t("store.keysGoneHelp")}</Button>
      </Note>
    );
  }
  if (report?.refusal === "unreadable") return <Note tone="error" data-store-refusal="unreadable">{t("store.unreadable", { device: name })}</Note>;
  return null;
}

/** The first read of a machine: three quiet rows where its list will stand. */
function Loading() {
  const { t } = useInterfaceTranslation();
  return (
    <Group title={t("store.section.skill")} caption={t("store.reading")} data-store-loading="true">
      {[0, 1, 2].map((row) => (
        <div key={row} className="flex items-center gap-md px-md py-sm" aria-hidden="true">
          <span className="h-(--size-badge-height) w-1/4 animate-pulse rounded-xs bg-muted" />
          <span className="flex-1" />
          <span className="h-(--size-badge-height) w-1/6 animate-pulse rounded-xs bg-muted" />
          <span className="h-(--size-checkbox) w-(--size-control) animate-pulse rounded-xl bg-muted" />
        </div>
      ))}
    </Group>
  );
}

/** 업데이트 n: when the check found newer versions; the hint says when it last looked. */
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

/** One kind's section: its title and count, then the rows or the table. */
function Kind({ kind, items, shown, view, page }: { kind: StoreItemKind; items: StoreItem[]; shown: ReadonlySet<string>; view: StoreView; page: Page }) {
  const { t } = useInterfaceTranslation();
  const columns = shownColumns(page.report, kind, shown);
  if (items.length === 0) return null;
  const title = t(kind === "skill" ? "store.section.skill" : "store.section.mcp");
  if (view === "table") {
    return (
      <section className="mb-lg" data-store-section={kind}>
        <SectionTitle title={title} count={items.length} />
        <div className="overflow-x-auto rounded-md border border-border bg-card">
          <table className="w-full border-collapse text-body" data-store-table={kind}>
            <thead>
              <tr className="text-caption text-muted-foreground">
                <th className="px-md py-xs text-left font-normal"><span className="sr-only">{title}</span></th>
                <th className="px-xs py-xs text-center font-normal">{t("store.column.all")}</th>
                {columns.map((column) => (
                  <th key={column.id} className="px-xs py-xs text-center font-normal whitespace-nowrap" data-store-column={column.id}>
                    <ColumnHead column={column} />
                  </th>
                ))}
                <th className="w-(--size-control) px-xs py-xs"><span className="sr-only">⋯</span></th>
              </tr>
            </thead>
            <tbody className="divide-y divide-border border-t border-border">
              {items.map((item) => <TableRow key={item.id} item={item} columns={columns} page={page} />)}
            </tbody>
          </table>
        </div>
      </section>
    );
  }
  return (
    <section className="mb-lg" data-store-section={kind}>
      <SectionTitle title={title} count={items.length} />
      <div className="divide-y divide-border rounded-md border border-border bg-card">
        {items.map((item) => <ListRow key={item.id} item={item} columns={columns} page={page} />)}
      </div>
    </section>
  );
}

function SectionTitle({ title, count, caption }: { title: string; count?: number; caption?: string }) {
  return (
    <div className="mb-sm flex items-baseline gap-sm">
      <h3 className="text-body font-semibold text-subtle-foreground">{title}</h3>
      {count !== undefined ? <span className="text-caption text-muted-foreground">{count}</span> : null}
      {caption ? <span className="text-caption text-muted-foreground">{caption}</span> : null}
    </div>
  );
}

/** A column's name; the shared folder names the agents that read it in its hint. */
function ColumnHead({ column }: { column: ShownColumn }) {
  const { t } = useInterfaceTranslation();
  const name = columnName(column, t("store.column.shared"));
  if (!column.shared) return <>{name}</>;
  const readers = column.readers.map(agentShortName).join(" · ");
  return (
    <span className="inline-flex items-center gap-xxs">
      {name}
      <Hint label={readers ? t("store.column.sharedHint", { agents: readers }) : t("store.column.sharedNone")}>
        <button type="button" className="inline-flex text-muted-foreground outline-none focus-visible:ring-1 focus-visible:ring-ring" data-store-shared-hint={column.id}>
          <InfoIcon aria-hidden="true" className="size-(--size-icon-sm)" />
        </button>
      </Hint>
    </span>
  );
}

/** The short version, with the dot when a check found a newer one. */
function Version({ item }: { item: StoreItem }) {
  const { t } = useInterfaceTranslation();
  const text = versionText(item.version, t);
  if (!text && !item.update) return null;
  return (
    <span className="inline-flex shrink-0 items-center gap-xxs font-mono text-caption text-muted-foreground" data-store-version={item.id}>
      {text}
      {item.update ? (
        <span title={t("store.version.update")} data-store-update-dot={item.id}>
          <UpdateDot />
          <span className="sr-only">{t("store.version.update")}</span>
        </span>
      ) : null}
    </span>
  );
}

/** What a row says at its end: every agent has it, none does, or only where it differs. */
function Summary({ summary }: { summary: RowSummary }) {
  const { t } = useInterfaceTranslation();
  if (summary.kind === "all") return <span className="text-muted-foreground">{t("store.summary.all")}</span>;
  if (summary.kind === "off") return <span className="text-muted-foreground">{t("store.summary.off")}</span>;
  return (
    <span className="flex min-w-0 flex-wrap justify-end gap-xs">
      {summary.parts.map((part) => {
        const column = columnName(part.column, t("store.column.shared"));
        return (
          <Badge key={part.column.id} variant="outline" className={cn(part.state === "failed" && "border-destructive text-destructive", part.state === "hand_written" && "border-warning text-warning")} data-store-summary-part={`${part.column.id}:${part.state}`}>
            {t(part.state === "failed" ? "store.summary.cellFailed" : part.state === "hand_written" ? "store.summary.cellHandWritten" : "store.summary.cellOff", { column })}
          </Badge>
        );
      })}
    </span>
  );
}

/** A row of the list: name, version, summary, its 전체 switch and the chevron; a press unfolds it in place (scene 1-3). */
function ListRow({ item, columns, page }: { item: StoreItem; columns: ShownColumn[]; page: Page }) {
  const { t } = useInterfaceTranslation();
  const [open, setOpen] = useState(false);
  const request = useStoreRequest(page.device.id, page.actions);
  const summary = rowSummary(item, columns);
  const all = allSwitch(item, columns);
  return (
    <div data-store-row={item.id} data-store-summary={summary.kind} data-store-open={open ? "true" : "false"}>
      <div className="flex items-center gap-md px-md py-sm">
        <button
          type="button"
          aria-expanded={open}
          aria-label={t("store.expand", { name: item.name })}
          onClick={() => setOpen((now) => !now)}
          className="flex min-w-0 flex-1 items-center gap-sm rounded-xs text-left outline-none focus-visible:ring-1 focus-visible:ring-ring"
          data-store-row-toggle={item.id}
        >
          <span className="min-w-0 truncate text-subhead font-medium text-foreground">{item.name}</span>
          <Version item={item} />
          {item.folder_missing ? <Badge variant="outline" className="border-warning text-warning">{t("store.detail.folderMissing")}</Badge> : null}
          <span className="ml-auto min-w-0 text-right text-body">
            <Summary summary={summary} />
          </span>
        </button>
        <Switch checked={all.checked} disabled={request.pending} aria-busy={request.pending} onCheckedChange={() => request.send([all.change])} aria-label={t("store.switch.all", { name: item.name })} data-store-all={item.id} />
        <ChevronRightIcon aria-hidden="true" className={cn("size-(--size-icon) shrink-0 text-muted-foreground transition-transform", open && "rotate-90")} />
      </div>
      {request.refusal ? <div className="px-md pb-sm"><Note tone="error">{refusalText(request.refusal, t)}</Note></div> : null}
      {open ? <RowDetail item={item} columns={columns} page={page} /> : null}
    </div>
  );
}

/** The unfolded row: what the item is, where it comes from, each agent's switch and what can be done to it. */
function RowDetail({ item, columns, page }: { item: StoreItem; columns: ShownColumn[]; page: Page }) {
  const { t } = useInterfaceTranslation();
  const files = item.files ?? [];
  const executables = files.filter((file) => file.executable).length;
  const failed = columns.map((column) => ({ column, cell: cellOf(item, column) })).filter(({ cell }) => cell.state === "failed");
  return (
    <div className="flex flex-col gap-sm border-t border-border px-md py-sm" data-store-detail={item.id}>
      {item.description ? <p className="break-words text-body text-subtle-foreground">{item.description}</p> : null}
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-md gap-y-xs text-body">
        {item.server ? (
          <>
            <dt className="text-muted-foreground">{t(item.server.reach.kind === "local" ? "store.detail.command" : "store.detail.address")}</dt>
            <dd className="break-all font-mono text-subtle-foreground">{serverLine(item.server)}</dd>
            {keyNames(item.server).length > 0 ? (
              <>
                <dt className="text-muted-foreground">{t("store.detail.keys")}</dt>
                <dd className="flex flex-wrap items-center gap-sm" data-store-keys={item.id}>
                  {keyNames(item.server).map((key) => (
                    <span key={key} className="font-mono text-subtle-foreground">{key} ••••••</span>
                  ))}
                  <Button variant="secondary" size="sm" disabled={page.readOnly} onClick={() => page.openKeys(item)} data-store-edit-keys={item.id}>
                    {t("store.detail.editKeys")}
                  </Button>
                </dd>
              </>
            ) : null}
          </>
        ) : (
          <>
            <dt className="text-muted-foreground">{t("store.detail.source")}</dt>
            <dd className="break-all font-mono text-subtle-foreground">
              {sourceLine(item)}
              {item.source.kind === "folder" ? <span className="font-sans text-muted-foreground"> · {item.folder_missing ? t("store.detail.lastCopy") : t("store.detail.follows")}</span> : null}
            </dd>
            {files.length > 0 ? (
              <>
                <dt className="text-muted-foreground">{t("store.detail.files")}</dt>
                <dd className="text-subtle-foreground">
                  {t("store.detail.fileCount", { count: files.length })}
                  {executables > 0 ? <span className="text-warning"> · {t("store.detail.executables", { count: executables })}</span> : null}
                </dd>
              </>
            ) : null}
          </>
        )}
      </dl>
      <div className="flex flex-wrap gap-x-lg gap-y-sm" data-store-agents={item.id}>
        {columns.map((column) => <ColumnSwitch key={column.id} item={item} column={column} page={page} />)}
      </div>
      {failed.map(({ column, cell }) => (
        <Note key={column.id} tone="error" data-store-failed={`${item.id}:${column.id}`}>
          {columnName(column, t("store.column.shared"))}: {cell.reason ?? t("store.cell.failed")}
        </Note>
      ))}
      <RowActions item={item} page={page} />
    </div>
  );
}

/** One agent's switch in an unfolded row, or what stands in its place: a failure to retry, a copy to take over, no support. */
function ColumnSwitch({ item, column, page }: { item: StoreItem; column: ShownColumn; page: Page }) {
  const { t } = useInterfaceTranslation();
  const request = useStoreRequest(page.device.id, page.actions);
  const cell = cellOf(item, column);
  const name = columnName(column, t("store.column.shared"));
  const label = column.shared ? <ColumnHead column={column} /> : name;
  const fresh = page.newSession.has(`${item.id}\u0000${column.id}`);
  const wrap = (control: ReactNode, extra?: ReactNode) => (
    <span className="inline-flex items-center gap-xs text-body text-subtle-foreground" data-store-cell={`${item.id}:${column.id}`} data-store-cell-state={cell.state}>
      {control}
      {label}
      {extra}
      {fresh ? <span className="text-caption text-muted-foreground" data-store-new-session="true">{t("store.cell.newSession")}</span> : null}
      {request.refusal ? <span className="text-caption text-destructive">{refusalText(request.refusal, t)}</span> : null}
    </span>
  );
  switch (cell.state) {
    case "unsupported":
      return wrap(<span className="w-(--size-control) text-center text-muted-foreground" title={t("store.cell.unsupported", { agent: name })}>—</span>);
    case "failed":
      return wrap(
        <span className="w-(--size-control) text-center font-semibold text-destructive" title={cell.reason ?? undefined}>!</span>,
        <Button variant="secondary" size="sm" disabled={request.pending} onClick={() => request.send([{ op: "set", item: item.id, column: column.id, on: columnWants(item, column.id) }])} data-store-retry={`${item.id}:${column.id}`}>
          {t("store.retry")}
        </Button>,
      );
    case "hand_written":
      return wrap(
        <span className="w-(--size-control) text-center text-warning">≠</span>,
        <>
          <span className="text-caption text-warning">{t("store.cell.handWritten")}</span>
          <Button variant="secondary" size="sm" disabled={request.pending} onClick={() => request.send([{ op: "adopt", kind: item.kind, name: item.name, from: cell.place ?? undefined }])} data-store-take-over={`${item.id}:${column.id}`}>
            {t("store.cell.takeOver")}
          </Button>
        </>,
      );
    case "agent_off":
      return wrap(<Switch checked={false} disabled aria-label={t("store.switch.column", { name: item.name, column: name })} title={t("store.cell.agentOff")} />);
    default:
      return wrap(
        <Switch
          checked={cellOn(cell.state)}
          disabled={request.pending}
          aria-busy={request.pending}
          onCheckedChange={(on) => request.send([{ op: "set", item: item.id, column: column.id, on }])}
          aria-label={t("store.switch.column", { name: item.name, column: name })}
          title={cell.state === "waiting" ? t("store.cell.waiting") : undefined}
          data-store-switch={`${item.id}:${column.id}`}
        />,
      );
  }
}

/** What can be done to an item: its update first when there is one, its files and source, and Delete at the end. */
function RowActions({ item, page }: { item: StoreItem; page: Page }) {
  const { t } = useInterfaceTranslation();
  const source = openableSource(item, page.local);
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
      <span className="flex-1" />
      <Button variant="ghost" size="sm" className="text-destructive hover:text-destructive" disabled={page.removing === item.id} onClick={() => page.remove(item)} data-store-delete={item.id}>
        {t("store.action.delete")}
      </Button>
    </div>
  );
}

/** A row of the table (scene 1-2 A): a row every agent has, or none, merges its cells into one dim line until hovered. */
function TableRow({ item, columns, page }: { item: StoreItem; columns: ShownColumn[]; page: Page }) {
  const { t } = useInterfaceTranslation();
  const request = useStoreRequest(page.device.id, page.actions);
  const summary = rowSummary(item, columns);
  const all = allSwitch(item, columns);
  const merged = summary.kind !== "differs";
  const source = openableSource(item, page.local);
  return (
    <tr className="group/row" data-store-row={item.id} data-store-summary={summary.kind}>
      <td className="px-md py-sm">
        <span className="flex min-w-0 items-center gap-sm">
          <span className="truncate text-subhead font-medium text-foreground">{item.name}</span>
          <Version item={item} />
        </span>
        {request.refusal ? <Note tone="error">{refusalText(request.refusal, t)}</Note> : null}
      </td>
      <td className="px-xs py-sm text-center">
        <Switch checked={all.checked} disabled={request.pending} onCheckedChange={() => request.send([all.change])} aria-label={t("store.switch.all", { name: item.name })} data-store-all={item.id} />
      </td>
      {merged ? (
        <td colSpan={columns.length} className="px-xs py-sm text-center text-muted-foreground group-focus-within/row:hidden group-hover/row:hidden" data-store-merged={item.id}>
          {t(summary.kind === "all" ? "store.summary.all" : "store.summary.off")}
        </td>
      ) : null}
      {columns.map((column) => (
        <td key={column.id} className={cn("px-xs py-sm text-center", merged && "hidden group-focus-within/row:table-cell group-hover/row:table-cell")}>
          <CellMark item={item} column={column} cell={cellOf(item, column)} page={page} />
        </td>
      ))}
      <td className="px-xs py-sm text-center">
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="row-action" size="icon-sm" aria-label={t("store.more", { name: item.name })} data-store-more={item.id}>
              <EllipsisIcon aria-hidden="true" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            {item.update ? (
              <DropdownMenuItem onSelect={() => page.openUpdate([item])} data-store-menu-update={item.id}>
                {t("store.action.update")}
                <UpdateDot className="ml-auto" />
              </DropdownMenuItem>
            ) : null}
            {(item.files ?? []).length > 0 ? <DropdownMenuItem onSelect={() => page.openFiles(item)}>{t("store.action.files")}</DropdownMenuItem> : null}
            {item.server && keyNames(item.server).length > 0 ? <DropdownMenuItem onSelect={() => page.openKeys(item)}>{t("store.detail.editKeys")}</DropdownMenuItem> : null}
            {source ? (
              <DropdownMenuItem onSelect={() => (source.kind === "url" ? page.actions.openLink(source.url, true) : page.actions.revealExternal(source.path))}>
                {t(source.kind === "url" ? "store.action.openSource" : "store.action.openFolder")}
              </DropdownMenuItem>
            ) : null}
            <DropdownMenuItem className="text-destructive" onSelect={() => page.remove(item)} data-store-menu-delete={item.id}>
              {t("store.action.delete")}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </td>
    </tr>
  );
}

/** A table cell: ● on, ○ off, — not supported, ! failed (press to retry), ≠ a copy Hide did not write (press to take over). */
function CellMark({ item, column, cell, page }: { item: StoreItem; column: ShownColumn; cell: StoreCell; page: Page }) {
  const { t } = useInterfaceTranslation();
  const request = useStoreRequest(page.device.id, page.actions);
  const name = columnName(column, t("store.column.shared"));
  const data = { "data-store-cell": `${item.id}:${column.id}`, "data-store-cell-state": cell.state };
  if (cell.state === "unsupported") return <span className="text-muted-foreground" title={t("store.cell.unsupported", { agent: name })} {...data}>—</span>;
  const base = "inline-flex size-(--size-control-sm) items-center justify-center rounded-xs outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none";
  if (cell.state === "failed") {
    return (
      <button type="button" className={cn(base, "font-semibold text-destructive")} title={`${t("store.legend.failed")}${cell.reason ? ` · ${cell.reason}` : ""}`} disabled={request.pending} onClick={() => request.send([{ op: "set", item: item.id, column: column.id, on: columnWants(item, column.id) }])} {...data}>
        !
      </button>
    );
  }
  if (cell.state === "hand_written") {
    return (
      <button type="button" className={cn(base, "text-warning")} title={`${t("store.cell.handWritten")} · ${t("store.cell.takeOver")}`} disabled={request.pending} onClick={() => request.send([{ op: "adopt", kind: item.kind, name: item.name, from: cell.place ?? undefined }])} {...data}>
        ≠
      </button>
    );
  }
  const on = cellOn(cell.state);
  const fresh = page.newSession.has(`${item.id}\u0000${column.id}`);
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={t("store.switch.column", { name: item.name, column: name })}
      title={cell.state === "agent_off" ? t("store.cell.agentOff") : cell.state === "waiting" ? t("store.cell.waiting") : fresh ? t("store.cell.newSession") : undefined}
      disabled={cell.state === "agent_off" || request.pending}
      onClick={() => request.send([{ op: "set", item: item.id, column: column.id, on: !on }])}
      className={cn(base, cell.state === "agent_off" && "opacity-(--opacity-disabled)")}
      {...data}
    >
      <span aria-hidden="true" className={cn("size-(--size-status-mark) rounded-full", on ? "bg-primary" : "border border-muted-foreground")} />
    </button>
  );
}

function Legend() {
  const { t } = useInterfaceTranslation();
  return (
    <div className="flex flex-wrap gap-md text-caption text-muted-foreground" data-store-legend="true">
      <span className="inline-flex items-center gap-xs"><span className="size-(--size-status-mark) rounded-full bg-primary" />{t("store.legend.on")}</span>
      <span className="inline-flex items-center gap-xs"><span className="size-(--size-status-mark) rounded-full border border-muted-foreground" />{t("store.legend.off")}</span>
      <span>— {t("store.legend.unsupported")}</span>
      <span><span className="font-semibold text-destructive">!</span> {t("store.legend.failed")}</span>
    </div>
  );
}

/** 에이전트에만 있음 (B19): what agents hold that Hide did not write, each with Apply to all. */
function Unmanaged({ rows, report, shown, page }: { rows: StoreUnmanaged[]; report: StoreReport; shown: ReadonlySet<string>; page: Page }) {
  const { t } = useInterfaceTranslation();
  return (
    <section className="mb-lg" data-store-section="unmanaged">
      <SectionTitle title={t("store.section.unmanaged")} count={rows.length} caption={t("store.section.unmanagedCaption")} />
      <div className="divide-y divide-border rounded-md border border-border bg-card">
        {rows.map((row) => <UnmanagedRow key={`${row.kind}:${row.name}`} row={row} report={report} shown={shown} page={page} />)}
      </div>
    </section>
  );
}

function UnmanagedRow({ row, report, shown, page }: { row: StoreUnmanaged; report: StoreReport; shown: ReadonlySet<string>; page: Page }) {
  const { t } = useInterfaceTranslation();
  const request = useStoreRequest(page.device.id, page.actions);
  const columns = shownColumns(report, row.kind, shown);
  const where = (agents: string[]) => {
    const names = columns.filter((column) => column.readers.some((agent) => agents.includes(agent)) || agents.some((agent) => column.agent === agent)).map((column) => columnName(column, t("store.column.shared")));
    return names.length > 0 ? names.join(", ") : agents.map(agentShortName).join(", ");
  };
  const kind = t(row.kind === "skill" ? "store.section.skill" : "store.section.mcp");
  const agents = [...new Set(row.copies.flatMap((copy) => copy.agents))];
  return (
    <div className="flex flex-wrap items-center gap-x-md gap-y-xs px-md py-sm" data-store-unmanaged={`${row.kind}:${row.name}`}>
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="truncate text-subhead font-medium text-foreground">{row.name}</span>
        <span className="text-caption text-muted-foreground">
          {kind} · {where(agents)}
          {row.same ? null : <span className="text-warning"> · {t("store.unmanaged.differs")}</span>}
        </span>
      </span>
      {request.refusal ? <Note tone="error">{refusalText(request.refusal, t)}</Note> : null}
      {row.same || row.copies.length < 2 ? (
        <Button variant="secondary" size="sm" disabled={request.pending} onClick={() => request.send([{ op: "adopt", kind: row.kind, name: row.name }])} data-store-adopt={`${row.kind}:${row.name}`}>
          {t("store.unmanaged.applyAll")}
        </Button>
      ) : (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="secondary" size="sm" disabled={request.pending} data-store-adopt={`${row.kind}:${row.name}`}>
              {t("store.unmanaged.applyAll")}
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            {row.copies.map((copy) => (
              <DropdownMenuItem key={copy.place} onSelect={() => request.send([{ op: "adopt", kind: row.kind, name: row.name, from: copy.place }])} data-store-adopt-from={copy.place}>
                {t("store.unmanaged.fromWhich", { agent: where(copy.agents) })}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      )}
    </div>
  );
}

/** 지운 항목 n개 (B18): the undo list, newest first, each to restore or remove for good. */
function Trash({ report, page }: { report: StoreReport; page: Page }) {
  const { t } = useInterfaceTranslation();
  const rows = [...report.trash].sort((a, b) => b.at_unix_ms - a.at_unix_ms);
  return (
    <section className="mb-lg rounded-md border border-border bg-card" data-store-trash="true">
      <Disclosure title={t("store.trash.count", { count: rows.length })}>
        {rows.map((entry) => <TrashRow key={entry.id} entry={entry} page={page} />)}
      </Disclosure>
    </section>
  );
}

function TrashRow({ entry, page }: { entry: StoreReport["trash"][number]; page: Page }) {
  const { t } = useInterfaceTranslation();
  const restore = useStoreRequest(page.device.id, page.actions);
  const forget = useStoreRequest(page.device.id, page.actions);
  const kind = t(entry.kind === "skill" ? "store.section.skill" : "store.section.mcp");
  return (
    <div className="flex flex-wrap items-center gap-x-md gap-y-xs px-md py-sm" data-store-trash-entry={entry.id}>
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="truncate text-subhead text-foreground">{entry.name}</span>
        <span className="text-caption text-muted-foreground">
          {kind}
          {entry.cause.kind === "replaced" ? ` · ${t("store.trash.replaced")}` : null}
        </span>
      </span>
      {restore.refusal === "name_taken" ? <Note tone="error" data-store-trash-taken={entry.id}>{t("store.trash.nameTaken", { name: entry.name })}</Note> : restore.refusal ? <Note tone="error">{refusalText(restore.refusal, t)}</Note> : null}
      <Button variant="secondary" size="sm" disabled={restore.pending || forget.pending} onClick={() => restore.send([{ op: "restore", entry: entry.id }])} data-store-restore={entry.id}>
        {t("store.trash.restore")}
      </Button>
      <Button variant="ghost" size="sm" disabled={restore.pending || forget.pending} onClick={() => forget.send([{ op: "forget", entry: entry.id }])} data-store-forget={entry.id}>
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
