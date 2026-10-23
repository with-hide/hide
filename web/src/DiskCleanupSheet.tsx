import { ChevronRightIcon, GitBranchIcon, HomeIcon, Loader2Icon, LockIcon, RefreshCwIcon, SquareTerminalIcon, Trash2Icon } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { Actions } from "./actions";
import { useInterfaceTranslation } from "./i18n/client";
import { formatBytes as formatSize, formatGigabytes } from "./i18n/format";
import { requireInterfaceLanguage } from "./i18n/locale";
import { AlertDialog, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from "./components/ui/alert-dialog";
import { Button } from "./components/ui/button";
import { Checkbox } from "./components/ui/checkbox";
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "./components/ui/dialog";
import { ToggleGroup, ToggleGroupItem } from "./components/ui/toggle-group";
import { Hint } from "./components/ui/tooltip";
import {
  EMPTY_SELECTION,
  FILTERS,
  FILTER_KEY,
  LAYER_KEY,
  BUSY_KEY,
  cleanupElsewhere,
  bundleRefs,
  bundleState,
  filterCounts,
  footerOf,
  isChecked,
  isIncluded,
  entranceBytes,
  layoutRows,
  needsConfirm,
  planOf,
  pruneSelection,
  resultLines,
  sheetModel,
  toggleBundle,
  toggleCell,
  visibleRows,
  type BundleState,
  type CacheCell,
  type CellRef,
  type Column,
  type Filter,
  type ResultLine,
  type Selection,
  type SheetModel,
  type SheetRow,
} from "./diskCleanup";
import type { MessageKey } from "./i18n/catalogs";
import { cn } from "./lib/utils";
import { prChip } from "./projectBoard";
import { catalogOf, type CacheLayer, type Workspace } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { PrChipMark } from "./components/pr-mark";

// The disk cleanup sheet of a local Git project (PRD disk-layers): one row per
// checkout, one cell per layer a build tool remakes, and the checkboxes that
// pick them. Every rule it draws lives in `diskCleanup.ts`; this file only
// lays the model out and turns a press into one core event. The table is a
// screen-local grid (D-31): no System / Table part exists for it.

/** The sheet's width: the `--size-disk-sheet` token. */
const SHEET_WIDTH = "w-(--size-disk-sheet)";
/** Checkbox, checkout, the two cache layers, the worktree, other and the row total. */
const GRID = "var(--spacing-xl) minmax(0, 2.6fr) repeat(3, minmax(0, 1.2fr)) minmax(0, 0.9fr) minmax(0, 0.9fr)";

/** The dot each layer wears in the usage bar, the tooltip and the legend. */
export const LAYER_DOT = {
  build_cache: "bg-accent-choice-amber",
  dependencies: "bg-file-blue",
  other: "bg-file-yellow",
  source: "bg-file-neutral",
  shared_git: "bg-file-neutral",
} as const;

type Step = "table" | "confirm";

/** How long a confirm may go unanswered before the sheet takes it as dropped; the core answers in one snapshot. */
const CONFIRM_ANSWER_MS = 5000;

/** Opens the sheet on one project, on a filter (the Projects tab's size, `머지됨 N → 정리`, the project panel). */
export function openDiskCleanup(workspaceId: string, filter: Filter) {
  useUiStore.getState().setWorkspaceDialog({ kind: "disk_cleanup", workspaceId, filter });
}

export function DiskCleanupSheet({ actions, workspace, filter: initialFilter, onClose }: { actions: Actions; workspace: Workspace; filter: Filter; onClose: () => void }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  // A node's project carries its cleanup on the node's rows, and the core runs one cleanup at a time for all of them.
  // The answer is a word, so the open sheet redraws when it changes, not on every snapshot of a row.
  const elsewhere = useShellStore((s) => cleanupElsewhere(catalogOf(s.rest?.navigator?.workspaces, s.rest?.status?.remote), workspace.id));
  const model = useMemo(() => sheetModel(workspace, elsewhere, t), [workspace, elsewhere, t]);
  const cleanup = workspace.cleanup ?? null;
  const [filter, setFilter] = useState<Filter>(initialFilter);
  const [selection, setSelection] = useState<Selection>(EMPTY_SELECTION);
  const [foldOpen, setFoldOpen] = useState(false);
  const [step, setStep] = useState<Step>("table");
  // A press on Clean up is one event; until the core answers with `removing` a second press is not another (B23).
  const [sent, setSent] = useState(false);
  const cleanupId = cleanup?.id ?? null;
  const phase = cleanup?.phase ?? null;

  // Opening reviews the project, unless a cleanup that already ran or is running is what there is to show (B20).
  // Another project's review or cleanup that ends while the sheet is open leaves this project unreviewed: review when it does.
  useEffect(() => {
    if (elsewhere || phase === "removing" || phase === "complete") return;
    actions.reviewDiskCleanup(workspace.id);
    // On opening and when the daemon's other cleanup ends; a later Review again is its own press.
  }, [elsewhere === null]);
  // A new review starts from nothing ticked, and a settled press can be made again.
  useEffect(() => {
    setSelection(EMPTY_SELECTION);
    setStep("table");
    setSent(false);
  }, [cleanupId]);
  useEffect(() => {
    if (phase !== "review") setSent(phase === "removing");
  }, [phase]);
  // The core drops a confirm it cannot take (stale id, review gone) with no phase change: after its answer window
  // the press is released, so Clean up never stays disabled with nothing said.
  useEffect(() => {
    if (!sent || phase !== "review") return;
    const timer = window.setTimeout(() => setSent(false), CONFIRM_ANSWER_MS);
    return () => window.clearTimeout(timer);
  }, [sent, phase]);

  const shown = visibleRows(model.rows, filter);
  const counts = filterCounts(model.rows);
  const layout = layoutRows(shown);
  const footer = footerOf(shown, selection, t, language, model.state);
  const plan = planOf(shown, selection);

  const close = () => {
    if (cleanup && phase !== "removing") actions.dismissDiskCleanup();
    onClose();
  };
  const changeFilter = (next: Filter) => {
    setFilter(next);
    setSelection((current) => pruneSelection(current, visibleRows(model.rows, next)));
  };
  const run = () => {
    if (!cleanup || sent) return;
    setSent(true);
    setStep("table");
    actions.confirmDiskCleanup(
      cleanup.id,
      plan.worktrees.map((worktree) => worktree.path),
      plan.cells.map((cell) => ({ path: cell.path, layer: cell.layer })),
    );
  };
  const press = () => (needsConfirm(plan) ? setStep("confirm") : run());
  // The one worktree the confirmation named can stop being removable while it is open (a row turned in use):
  // with none left the sheet is back on the table, where that row says why.
  const confirming = step === "confirm" && plan.worktrees.length > 0;
  /** The confirmation says the panes close, so the button is named by what it does (UI_BEHAVIOR, Destructive buttons). */
  const closingPanes = plan.worktrees.reduce((sum, worktree) => sum + worktree.panes, 0);
  useEffect(() => {
    if (step === "confirm" && plan.worktrees.length === 0) setStep("table");
  }, [step, plan.worktrees.length]);

  const title = t("cleanup.title");
  const body =
    model.state === "removing" ? (
      <RunningView progress={cleanup?.progress ?? null} />
    ) : model.state === "complete" && cleanup ? (
      <ResultView workspace={workspace} onReview={() => actions.reviewDiskCleanup(workspace.id)} onClose={close} />
    ) : (
      <TableView
        workspace={workspace}
        model={model}
        filter={filter}
        counts={counts}
        layout={layout}
        shown={shown}
        selection={selection}
        onSelection={setSelection}
        onFilter={changeFilter}
        foldOpen={foldOpen}
        onFold={() => setFoldOpen((open) => !open)}
        onRetry={() => actions.reviewDiskCleanup(workspace.id)}
      />
    );
  const listing = model.state !== "removing" && model.state !== "complete";
  return (
    <>
      <Dialog open onOpenChange={(next) => { if (!next) close(); }}>
        <DialogContent className={listing ? SHEET_WIDTH : "w-(--size-overview-cleanup)"} showCloseButton aria-label={title} aria-describedby={undefined} data-disk-sheet={workspace.id} data-disk-state={model.state} data-disk-filter={filter}>
          <DialogHeader>
            <DialogTitle>{title}</DialogTitle>
          </DialogHeader>
          <div className="flex min-h-0 flex-1 flex-col overflow-auto px-lg py-md">{body}</div>
          {listing ? (
            <DialogFooter className="items-start justify-between border-t border-border pt-md" data-disk-footer={footer.empty ? "empty" : "ready"}>
              <div className="flex min-w-0 flex-col gap-xxs">
                <span className="text-subhead font-semibold tabular-nums text-foreground" data-disk-summary="true">
                  {footer.summary}
                </span>
              </div>
              <div className="flex items-center gap-sm">
                <Button variant="ghost" onClick={close} data-disk-cancel="true">
                  {t("common.cancel")}
                </Button>
                <Button onClick={press} disabled={footer.empty || model.state !== "ready" || sent} data-disk-clean="true">
                  <Trash2Icon aria-hidden="true" />
                  {model.state === "busy" && elsewhere ? t(BUSY_KEY[elsewhere]) : t("cleanup.clean")}
                </Button>
              </div>
            </DialogFooter>
          ) : null}
        </DialogContent>
      </Dialog>
      <AlertDialog open={confirming} onOpenChange={(next) => { if (!next) setStep("table"); }}>
        <AlertDialogContent data-disk-confirm="true">
          <AlertDialogHeader>
            <AlertDialogTitle>{plan.worktrees.length === 1 ? t("cleanup.deleteNamed", { name: plan.worktrees[0]!.label }) : t("cleanup.deleteWorktrees", { count: plan.worktrees.length })}</AlertDialogTitle>
            <AlertDialogDescription>{t("cleanup.branchesRemain")}</AlertDialogDescription>
          </AlertDialogHeader>
          <ul className="flex flex-col gap-xxs font-mono text-body text-foreground" data-disk-confirm-list="true">
            {plan.worktrees.map((worktree) => (
              <li key={worktree.path} className="flex min-w-0 items-baseline justify-between gap-md">
                <span className="min-w-0 truncate">{worktree.label}</span>
                <span className="flex shrink-0 items-center gap-md text-muted-foreground">
                  {worktree.panes > 0 ? <PaneCount count={worktree.panes} /> : null}
                  {formatSize(language, worktree.bytes)}
                </span>
              </li>
            ))}
          </ul>
          <AlertDialogFooter>
            {/* Plain buttons, not Action and Cancel: neither is the default and neither closes the sheet behind. */}
            <Button variant="secondary" onClick={() => setStep("table")} data-disk-confirm-back="true">
              {t("common.back")}
            </Button>
            <Button variant="destructive" onClick={run} data-disk-confirm-run="true">
              {closingPanes > 0
                ? t("cleanup.confirmClosingPanes", { count: closingPanes, worktrees: t("cleanup.worktreeCount", { count: plan.worktrees.length }) })
                : t("cleanup.confirm", { count: plan.worktrees.length })}
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}

// --- the table ----------------------------------------------------------------------

function TableView({
  workspace,
  model,
  filter,
  counts,
  layout,
  shown,
  selection,
  onSelection,
  onFilter,
  foldOpen,
  onFold,
  onRetry,
}: {
  workspace: Workspace;
  model: SheetModel;
  filter: Filter;
  counts: Record<Filter, number>;
  layout: ReturnType<typeof layoutRows>;
  shown: SheetRow[];
  selection: Selection;
  onSelection: (next: Selection) => void;
  onFilter: (next: Filter) => void;
  foldOpen: boolean;
  onFold: () => void;
  onRetry: () => void;
}) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const all = bundleRefs(shown, selection, ["build_cache", "dependencies"]);
  const columnRefs = (column: Column) => bundleRefs(shown, selection, [column]);
  const measured = model.rows.some((row) => row.measure !== "pending");
  return (
    <div className="flex min-w-0 flex-col gap-md">
      <UsageBar workspace={workspace} />
      {model.state === "unreadable" ? (
        <p className="flex items-center gap-sm text-body text-warning" data-disk-unreadable="true">
          {t("cleanup.inUseUnknown")}
          <Button variant="ghost" size="sm" onClick={onRetry} data-disk-retry="true">
            <RefreshCwIcon aria-hidden="true" />
            {t("common.retry")}
          </Button>
        </p>
      ) : null}
      <div className="flex flex-wrap items-center justify-between gap-md">
        <ToggleGroup type="single" value={filter} onValueChange={(value) => value && onFilter(value as Filter)} aria-label={t("cleanup.filter")} data-disk-filters="true">
          {FILTERS.map((name) => (
            <ToggleGroupItem key={name} value={name} data-disk-filter-item={name}>
              {t(FILTER_KEY[name])}
              <span className="font-mono text-caption text-muted-foreground">{counts[name]}</span>
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
      </div>
      {shown.length === 0 && measured ? (
        <div className="flex flex-col items-center gap-sm py-xl text-body text-muted-foreground" data-disk-empty="true">
          {t("cleanup.noFilteredRows")}
          <Button variant="secondary" size="sm" onClick={() => onFilter("all")} data-disk-show-all="true">
            {t("cleanup.showAll")}
          </Button>
        </div>
      ) : (
        <div role="table" aria-label={t("cleanup.table")} className="flex min-w-0 flex-col" data-disk-table="true">
          <div role="row" className="grid items-end gap-x-sm border-b border-border pb-sm" style={{ gridTemplateColumns: GRID }}>
            <BundleBox state={bundleState(selection, all)} label={t("cleanup.visibleCaches")} onToggle={() => onSelection(toggleBundle(selection, all))} data="all" />
            <span role="columnheader" className="text-caption text-subtle-foreground">
              {t("cleanup.checkout")}
            </span>
            {(["build_cache", "dependencies", "worktree"] as const).map((column) => (
              <ColumnHead key={column} column={column} rows={shown} selection={selection} refs={columnRefs(column)} onToggle={() => onSelection(toggleBundle(selection, columnRefs(column)))} />
            ))}
            <span role="columnheader" className="flex flex-col items-end gap-xxs text-caption text-subtle-foreground">
              {t("cleanup.layer.other")}
              <span className="font-mono text-muted-foreground">{formatSize(language, shown.reduce((sum, row) => sum + (row.other?.bytes ?? 0), 0))}</span>
            </span>
            <span role="columnheader" className="flex flex-col items-end gap-xxs text-caption text-subtle-foreground" data-disk-column="total">
              {t("cleanup.total")}
              <span className="font-mono text-muted-foreground" data-disk-total-head="true">{formatSize(language, shown.reduce((sum, row) => sum + (row.total ?? 0), 0))}</span>
            </span>
          </div>
          {layout.main ? <Row row={layout.main} selection={selection} onSelection={onSelection} /> : null}
          {layout.big.map((row) => (
            <Row key={row.path} row={row} selection={selection} onSelection={onSelection} />
          ))}
          {layout.small.length > 0 ? <FoldedRows layout={layout} open={foldOpen} onToggle={onFold} selection={selection} onSelection={onSelection} /> : null}
        </div>
      )}
    </div>
  );
}

function UsageBar({ workspace }: { workspace: Workspace }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const disk = workspace.disk;
  const layers = disk?.layers;
  const size = entranceBytes(workspace);
  if (!disk || !size || !layers) return null;
  const parts = [
    { key: "build_cache", label: t("cleanup.layer.build_cache"), bytes: layers.build_cache },
    { key: "dependencies", label: t("cleanup.layer.dependencies"), bytes: layers.dependencies },
    { key: "other", label: t("cleanup.layer.other"), bytes: layers.other },
    { key: "source", label: t("cleanup.layer.source"), bytes: layers.source + layers.shared_git },
  ] as const;
  const named = (part: (typeof parts)[number]) => t("cleanup.layerSize", { layer: part.label, size: formatSize(language, part.bytes) });
  return (
    <div className="flex flex-col gap-xs" data-disk-usage="true">
      <div className="flex items-baseline justify-between gap-md text-caption text-subtle-foreground">
        <span>
          {t("cleanup.thisProject")} <span className="font-mono text-foreground">{size.partial ? "≥ " : ""}{formatSize(language, size.bytes)}</span>
        </span>
        {disk.free_bytes != null ? <span className="font-mono" data-disk-free="true">{t("cleanup.diskFree", { size: formatSize(language, disk.free_bytes) })}</span> : null}
      </div>
      <div className="flex h-(--lens-bar-height) w-full gap-px overflow-hidden rounded-xs bg-muted" role="img" aria-label={parts.map(named).join(", ")}>
        {parts.map((part) => (
          <Hint key={part.key} label={named(part)} reveals>
            <span className={LAYER_DOT[part.key]} style={{ flexGrow: part.bytes, flexBasis: 0 }} data-disk-bar-part={part.key} />
          </Hint>
        ))}
      </div>
    </div>
  );
}

function BundleBox({ state, label, onToggle, data }: { state: BundleState; label: string; onToggle: () => void; data: string }) {
  return (
    <Checkbox
      checked={state === "indeterminate" ? "indeterminate" : state === "checked"}
      disabled={state === "none"}
      onCheckedChange={onToggle}
      aria-label={label}
      data-disk-bundle={data}
      data-disk-bundle-state={state}
    />
  );
}

function ColumnHead({ column, rows, selection, refs, onToggle }: { column: Column; rows: SheetRow[]; selection: Selection; refs: CellRef[]; onToggle: () => void }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const state = bundleState(selection, refs);
  const bytes = rows.reduce((sum, row) => sum + (column === "worktree" ? (row.total ?? 0) * (row.isMain ? 0 : 1) : row.cache[column].bytes), 0);
  return (
    <div role="columnheader" className="flex flex-col gap-xxs" data-disk-column={column}>
      <span className="flex items-center gap-xs text-caption font-medium text-foreground">
        <BundleBox state={state} label={t("cleanup.visibleLayer", { layer: t(LAYER_KEY[column]) })} onToggle={onToggle} data={column} />
        {t(LAYER_KEY[column])}
      </span>
      <span className="pl-lg font-mono text-caption text-muted-foreground">{formatSize(language, bytes)}</span>
    </div>
  );
}

function Row({ row, selection, onSelection, nested = false }: { row: SheetRow; selection: Selection; onSelection: (next: Selection) => void; nested?: boolean }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const caches = bundleRefs([row], selection, ["build_cache", "dependencies"]);
  const state = bundleState(selection, caches);
  const included = selection.worktrees.has(row.path);
  const pr = row.checkout.pull_request ? prChip(row.checkout.pull_request) : null;
  const dimmed = row.measure === "unavailable";
  return (
    <div
      role="row"
      className={cn("group grid items-center gap-x-sm border-b border-border py-sm hover:bg-muted", dimmed && "opacity-(--opacity-dimmed)", nested && "bg-card")}
      style={{ gridTemplateColumns: GRID }}
      data-disk-row={row.path}
      data-disk-bucket={row.bucket}
      data-disk-measure={row.measure}
    >
      <span role="cell">
        {included && state === "none" ? (
          <Checkbox checked disabled aria-label={t("cleanup.includedCaches", { name: row.label })} data-disk-bundle="row" data-disk-bundle-state="included" />
        ) : (
          <BundleBox state={state} label={t("cleanup.rowCaches", { name: row.label })} onToggle={() => onSelection(toggleBundle(selection, caches))} data="row" />
        )}
      </span>
      <span role="rowheader" className="flex min-w-0 items-center gap-xs text-body text-foreground">
        {row.isMain ? <HomeIcon aria-hidden="true" className="size-(--size-icon) shrink-0 text-muted-foreground" /> : <GitBranchIcon aria-hidden="true" className="size-(--size-icon) shrink-0 text-muted-foreground" />}
        <Hint label={row.measure === "unavailable" ? t("cleanup.sizeUnknown") : row.label} reveals>
          <span className="min-w-0 truncate" data-disk-label="true">
            {row.label}
          </span>
        </Hint>
        {pr ? (
          <span className="flex shrink-0" data-disk-pr={pr.number}>
            <PrChipMark chip={pr} />
          </span>
        ) : null}
        {row.inUse ? (
          <span className="shrink-0 text-caption text-warning" data-disk-in-use="true">
            {row.inUse}
          </span>
        ) : null}
      </span>
      {(["build_cache", "dependencies"] as const).map((layer) => (
        <CacheCellView key={layer} row={row} layer={layer} selection={selection} onSelection={onSelection} />
      ))}
      <WorktreeCellView row={row} selection={selection} onSelection={onSelection} />
      <OtherCellView row={row} />
      <span role="cell" className="text-right font-mono text-body text-subtle-foreground" data-disk-total={row.path}>
        {row.measure === "measured" && row.total !== null ? formatSize(language, row.total) : row.measure === "pending" ? <Skeleton /> : ""}
      </span>
    </div>
  );
}

function Skeleton() {
  return <span aria-hidden="true" data-disk-skeleton="true" className="inline-block h-(--size-badge-height) w-(--spacing-xxxl) animate-pulse rounded-xs bg-muted" />;
}

function CacheCellView({ row, layer, selection, onSelection }: { row: SheetRow; layer: CacheLayer; selection: Selection; onSelection: (next: Selection) => void }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const cell: CacheCell = row.cache[layer];
  const ref: CellRef = { path: row.path, column: layer };
  const included = isIncluded(selection, ref);
  if (row.measure === "pending") {
    return (
      <span role="cell" className="flex items-center gap-xs" data-disk-cell={`${row.path}:${layer}`} data-disk-cell-state="pending">
        <Checkbox disabled aria-label={t("cleanup.rowLayer", { name: row.label, layer: t(LAYER_KEY[layer]) })} />
        <Skeleton />
      </span>
    );
  }
  if (row.measure === "unavailable") return <span role="cell" data-disk-cell={`${row.path}:${layer}`} data-disk-cell-state="unavailable" />;
  if (cell.bytes === 0) {
    return (
      <span role="cell" className="text-muted-foreground" data-disk-cell={`${row.path}:${layer}`} data-disk-cell-state="empty">
        -
      </span>
    );
  }
  const detail = cell.largest_name ? t("cleanup.folderDetail", { count: cell.folders, name: cell.largest_name }) : t("cleanup.folderCount", { count: cell.folders });
  const name = t("cleanup.rowLayerSize", { name: row.label, layer: t(LAYER_KEY[layer]), size: formatSize(language, cell.bytes) });
  const stateName = included ? "included" : cell.selectable ? "selectable" : "blocked";
  const box = (
    <Checkbox
      checked={included || isChecked(selection, ref)}
      disabled={!cell.selectable || included}
      onCheckedChange={() => onSelection(toggleCell(selection, ref))}
      aria-label={name}
      data-disk-check={`${row.path}:${layer}`}
    />
  );
  return (
    <span role="cell" className={cn("flex min-w-0 items-center gap-xs", (included || !cell.selectable) && "opacity-(--opacity-secondary)")} data-disk-cell={`${row.path}:${layer}`} data-disk-cell-state={stateName}>
      {cell.selectable || included ? (
        box
      ) : (
        <Hint label={cell.why ?? detail} reveals>
          <span className="inline-flex">{box}</span>
        </Hint>
      )}
      <Hint label={cell.why && !cell.selectable ? `${cell.why}\n${detail}` : detail} reveals>
        <span className="font-mono text-body text-foreground" data-disk-bytes="true">
          {formatSize(language, cell.bytes)}
        </span>
      </Hint>
    </span>
  );
}

function WorktreeCellView({ row, selection, onSelection }: { row: SheetRow; selection: Selection; onSelection: (next: Selection) => void }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const key = `${row.path}:worktree`;
  if (row.isMain || !row.worktree) return <span role="cell" data-disk-cell={key} data-disk-cell-state="none" />;
  if (row.measure === "pending") {
    return (
      <span role="cell" className="flex items-center gap-xs" data-disk-cell={key} data-disk-cell-state="pending">
        <Checkbox disabled aria-label={t("cleanup.rowLayer", { name: row.label, layer: t(LAYER_KEY.worktree) })} />
        <Skeleton />
      </span>
    );
  }
  if (row.measure === "unavailable" || row.total === null) return <span role="cell" data-disk-cell={key} data-disk-cell-state="unavailable" />;
  const ref: CellRef = { path: row.path, column: "worktree" };
  if (!row.worktree.selectable) {
    return (
      <span role="cell" className="flex min-w-0 items-center gap-xs text-muted-foreground" data-disk-cell={key} data-disk-cell-state="blocked">
        <Hint label={row.worktree.why ?? t("cleanup.reason.cannotRemove")}>
          <LockIcon aria-hidden="true" className="size-(--size-icon)" data-disk-lock="true" />
        </Hint>
      </span>
    );
  }
  return (
    <span role="cell" className="flex min-w-0 items-center gap-xs" data-disk-cell={key} data-disk-cell-state="selectable">
      <Checkbox
        checked={isChecked(selection, ref)}
        onCheckedChange={() => onSelection(toggleCell(selection, ref))}
        aria-label={t("cleanup.rowLayerSize", { name: row.label, layer: t(LAYER_KEY.worktree), size: formatSize(language, row.total) })}
        data-disk-check={key}
      />
      <span className={cn("font-mono text-body", isChecked(selection, ref) ? "text-destructive" : "text-foreground")} data-disk-bytes="true">
        {formatSize(language, row.total)}
      </span>
      {row.worktree.panes > 0 ? <PaneCount count={row.worktree.panes} /> : null}
    </span>
  );
}

/** The panes that close with a worktree, as a mark and a count: the state shown, not a sentence about it. */
function PaneCount({ count }: { count: number }) {
  const { t } = useInterfaceTranslation();
  const label = t("cleanup.panesClose", { count });
  return (
    <Hint label={label} reveals>
      <span className="inline-flex shrink-0 items-center gap-xxs font-mono text-caption text-muted-foreground" data-disk-panes={count} aria-label={label}>
        <SquareTerminalIcon aria-hidden="true" className="size-(--size-icon)" />
        {count}
      </span>
    </Hint>
  );
}

function OtherCellView({ row }: { row: SheetRow }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const other = row.other;
  if (row.measure !== "measured" || !other || other.bytes === 0) return <span role="cell" data-disk-cell={`${row.path}:other`} />;
  const hint = other.largest_name ? t("cleanup.otherNamedFolder", { name: other.largest_name }) : t("cleanup.unknownFolder");
  return (
    <span role="cell" className="text-right" data-disk-cell={`${row.path}:other`}>
      <Hint label={hint} reveals>
        <span className="font-mono text-body text-subtle-foreground">{formatSize(language, other.bytes)}</span>
      </Hint>
    </span>
  );
}

function FoldedRows({ layout, open, onToggle, selection, onSelection }: { layout: ReturnType<typeof layoutRows>; open: boolean; onToggle: () => void; selection: Selection; onSelection: (next: Selection) => void }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const refs = bundleRefs(layout.small, selection, ["build_cache", "dependencies"]);
  return (
    <>
      <div role="row" className="flex items-center gap-sm border-b border-border py-sm" data-disk-fold={open ? "open" : "closed"}>
        <BundleBox state={bundleState(selection, refs)} label={t("cleanup.smallCaches", { count: layout.small.length })} onToggle={() => onSelection(toggleBundle(selection, refs))} data="fold" />
        <button type="button" aria-expanded={open} className="flex items-center gap-xs rounded-xs text-body text-subtle-foreground outline-none hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring" onClick={onToggle} data-disk-fold-toggle="true">
          <ChevronRightIcon aria-hidden="true" className={cn("size-(--size-icon) transition-transform", open && "rotate-90")} />
          {t("cleanup.smallCheckouts", { count: layout.small.length })} · <span className="font-mono">{formatSize(language, layout.smallBytes)}</span>
        </button>
      </div>
      {open ? layout.small.map((row) => <Row key={row.path} row={row} selection={selection} onSelection={onSelection} nested />) : null}
    </>
  );
}

// --- running and result ---------------------------------------------------------------

function RunningView({ progress }: { progress: { done: number; total: number } | null }) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="flex flex-col gap-sm py-lg" data-disk-running="true">
      <span className="flex items-center gap-sm text-title font-semibold text-foreground">
        <Loader2Icon aria-hidden="true" className="size-(--size-icon) animate-spin text-muted-foreground" />
        {progress ? t("cleanup.clearingProgress", { done: progress.done, total: progress.total }) : t("cleanup.removing")}
      </span>
      <span className="text-body text-muted-foreground">{t("cleanup.continues")}</span>
    </div>
  );
}

const OUTCOME_KEY: Record<ResultLine["outcome"], MessageKey> = { removed: "cleanup.outcome.removed", skipped: "cleanup.outcome.skipped", failed: "cleanup.outcome.failed" };

function ResultView({ workspace, onReview, onClose }: { workspace: Workspace; onReview: () => void; onClose: () => void }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const cleanup = workspace.cleanup!;
  const lines = resultLines(cleanup, t);
  return (
    <div className="flex flex-col gap-md" data-disk-result="true">
      <span className="flex items-baseline gap-sm font-mono text-headline font-semibold text-foreground" data-disk-free-change="true">
        {t("cleanup.free", { size: cleanup.free_before != null ? formatGigabytes(language, cleanup.free_before) : "-" })}
        <span aria-hidden="true">→</span>
        {cleanup.free_after != null ? formatGigabytes(language, cleanup.free_after) : "…"}
      </span>
      <ul className="flex flex-col divide-y divide-border border-y border-border" data-disk-result-lines="true">
        {lines.map((line) => (
          <li key={line.key} className="grid items-baseline gap-x-md py-sm text-body" style={{ gridTemplateColumns: "minmax(0, 1.4fr) minmax(0, 1fr) minmax(0, 1.6fr) minmax(0, 0.5fr)" }} data-disk-result-line={line.outcome}>
            <span className="truncate text-foreground">{line.label}</span>
            <span className="truncate text-muted-foreground">{line.what}</span>
            <span className={cn("truncate text-caption", line.outcome === "removed" ? "text-success" : line.outcome === "failed" ? "text-destructive" : "text-warning")}>
              {t(OUTCOME_KEY[line.outcome])}
              {line.reason ? ` · ${line.outcome === "removed" ? t("cleanup.keptReason", { reason: line.reason }) : line.reason}` : ""}
            </span>
            <span className="text-right font-mono text-caption text-subtle-foreground">{line.bytes !== null ? formatSize(language, line.bytes) : ""}</span>
          </li>
        ))}
        {lines.length === 0 ? <li className="py-sm text-body text-muted-foreground">{t("cleanup.noResults")}</li> : null}
      </ul>
      <div className="flex justify-end gap-sm">
        <Button variant="ghost" onClick={onReview} data-disk-review-again="true">
          <RefreshCwIcon aria-hidden="true" />
          {t("cleanup.reviewAgain")}
        </Button>
        <Button variant="secondary" onClick={onClose} data-disk-close="true">
          {t("common.close")}
        </Button>
      </div>
    </div>
  );
}
