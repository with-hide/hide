import { ChevronDownIcon, ChevronUpIcon, FilesIcon, GlobeIcon, ImageIcon } from "lucide-react";
import { useState, type MouseEvent } from "react";
import { pullLabel } from "../../codeHost";
import type { DeskOutput } from "../../desk";
import { fileIcon } from "../../fileIcons";
import { useInterfaceTranslation } from "../../i18n/client";
import { cn } from "../../lib/utils";
import { CLOSED_DIM, pullsStaleness } from "../../prMark";
import type { AgentRow, PrState, SessionPull } from "../../snapshot";
import { FileMark } from "../file-mark";
import { PrHoverCard, PrListItem, PrMark } from "../pr-mark";
import { Button } from "../ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger } from "../ui/dropdown-menu";
import { Hint } from "../ui/tooltip";
import { UnseenMark } from "./desk-job-card";
import { PreviewPicture, PreviewTile, tileOf } from "./preview-tile";

const ICON = "size-(--size-icon-sm) shrink-0 text-muted-foreground";
const CHIP = "h-(--size-control-lg) min-w-0 gap-xs px-sm text-body font-normal text-foreground data-[state=open]:bg-secondary";

/** A pull request the shelf names: one of the session's own list, or one a View showed that the list does not hold. */
export type ShelfPull = { id: string; label: string; title: string; state: PrState; url: string; pull: SessionPull | null };

/**
 * The Views a session opened, split as the list draws them (PRD desk-own-desk
 * D-08, D-18, B21, B23): those not looked at yet, then those looked at, each
 * newest first as the core lists them. A pull request is not a View.
 */
export function shelfViews(outputs: readonly DeskOutput[]): { toLook: DeskOutput[]; seen: DeskOutput[] } {
  const views = outputs.filter((output) => output.kind !== "pr");
  return { toLook: views.filter((output) => !output.seen), seen: views.filter((output) => output.seen) };
}

/** The session's pull requests: its own list, then any a View showed that the list does not hold, each once. */
export function shelfPulls(outputs: readonly DeskOutput[], pulls: readonly SessionPull[]): ShelfPull[] {
  const rows: ShelfPull[] = pulls.map((pull) => ({ id: `pr:${pull.url}`, label: pullLabel(pull), title: pull.title, state: pull.state, url: pull.url, pull }));
  for (const output of outputs) {
    if (output.kind !== "pr" || !output.url || rows.some((row) => row.url === output.url)) continue;
    if (!output.pr_state) throw new Error(`PR output ${output.id} carries no pr_state`);
    rows.push({ id: output.id, label: output.label, title: "", state: output.pr_state, url: output.url, pull: null });
  }
  return rows;
}

/** Where a View is (B23): a file's folder under its checkout, a page's address. */
export function viewWhere(output: DeskOutput): string | null {
  if (output.url) return output.url.replace(/^[a-z]+:\/\//iu, "").replace(/\/$/u, "");
  if (!output.path) return null;
  const root = output.checkout ? `${output.checkout.replace(/\/$/u, "")}/` : null;
  const inside = root && output.path.startsWith(root) ? output.path.slice(root.length) : output.path;
  const folder = inside.split("/").slice(0, -1).join("/");
  return folder || null;
}

/** A row's kind mark (B23): an image's small picture, a page's globe, a document's file type. */
function ViewMark({ output }: { output: DeskOutput }) {
  if (output.kind === "image") {
    return (
      <span className="flex h-(--size-icon) w-(--size-control-sm) shrink-0 items-center justify-center overflow-hidden rounded-xs border border-border bg-muted" aria-hidden="true">
        <PreviewPicture file={tileOf(output)} className="size-full object-cover object-top" fallback={<ImageIcon aria-hidden="true" className={ICON} />} />
      </span>
    );
  }
  if (output.kind === "page") return <GlobeIcon aria-hidden="true" className={ICON} />;
  return <FileMark icon={fileIcon(output.label)} className={ICON} />;
}

/**
 * The session's outputs under its head (PRD desk-own-desk D-08, D-17, D-18,
 * B21-B27; shelf 안 1): one `뷰 N` chip, with the 볼 것 eye while any View is
 * not looked at yet, and the session's pull request mark, or `PR n` when it
 * has several. Each chip opens its list floating over the conversation, which
 * neither the conversation nor the input box moves for; a press on a row
 * opens it on Desk's desk and closes the list, as Escape and a press outside
 * do. A View row shows its preview tile beside it under the pointer. A
 * session that opened nothing has no `뷰` chip, and one with neither has no
 * shelf.
 */
export function OutputShelf({ outputs, more = false, pulls, onOpen, onPull, row = null }: {
  /** Newest first, at most 32 (`DeskJob.outputs`). */
  outputs: readonly DeskOutput[];
  /** Older Views were left out of `outputs` (B27). */
  more?: boolean;
  pulls: readonly SessionPull[];
  onOpen: (output: DeskOutput) => void;
  /** Opens a pull request of the session's list; `external` (⌘-press) opens it on its host, as every PR mark does. */
  onPull: (pull: SessionPull, external: boolean) => void;
  /** The session's row, whose PR card a lone pull request shows under the pointer as the pane head's mark does. */
  row?: AgentRow | null;
}) {
  const views = shelfViews(outputs);
  const prs = shelfPulls(outputs, pulls);
  const count = views.toLook.length + views.seen.length;
  if (count === 0 && prs.length === 0) return null;
  const openPull = (item: ShelfPull, event: MouseEvent) => {
    if (item.pull) return onPull(item.pull, event.metaKey || event.ctrlKey);
    const output = outputs.find((one) => one.id === item.id);
    if (output) onOpen(output);
  };
  return (
    <section className="flex shrink-0 items-center gap-xs overflow-hidden border-b border-border px-md py-xs" data-desk-shelf={count + prs.length}>
      {count > 0 ? <ViewsChip toLook={views.toLook} seen={views.seen} more={more} onOpen={onOpen} /> : null}
      {prs.length === 1 ? <OnePull item={prs[0]!} row={row} onPress={openPull} onPull={onPull} /> : prs.length > 1 ? <PullsChip items={prs} onPress={openPull} /> : null}
    </section>
  );
}

/** `뷰 N` and its floating list (B21-B25, B27). */
function ViewsChip({ toLook, seen, more, onOpen }: { toLook: DeskOutput[]; seen: DeskOutput[]; more: boolean; onOpen: (output: DeskOutput) => void }) {
  const { t } = useInterfaceTranslation();
  const [open, setOpen] = useState(false);
  const Chevron = open ? ChevronUpIcon : ChevronDownIcon;
  const count = toLook.length + seen.length;
  return (
    <DropdownMenu open={open} onOpenChange={setOpen}>
      <DropdownMenuTrigger asChild>
        <Button variant="outline" size="sm" className={cn(CHIP, "shrink-0")} aria-label={toLook.length > 0 ? `${t("desk.views.chip", { count })}, ${t("desk.outputs.unseen")}` : t("desk.views.chip", { count })} data-desk-views={count} data-desk-views-unseen={toLook.length > 0 ? "true" : undefined}>
          <FilesIcon aria-hidden="true" className={ICON} />
          <span>{t("desk.views.chip", { count })}</span>
          {toLook.length > 0 ? <UnseenMark /> : null}
          <Chevron aria-hidden="true" className={ICON} />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" aria-label={t("desk.views.title")} className="max-h-(--size-desk-shelf-list) w-(--size-pr-popover) overflow-y-auto" data-desk-views-list={count}>
        {toLook.length > 0 ? <>
          <DropdownMenuLabel data-desk-views-group="to_look">{t("desk.views.toLook", { count: toLook.length })}</DropdownMenuLabel>
          {toLook.map((output) => <ViewRow key={output.id} output={output} unseen onOpen={onOpen} />)}
        </> : null}
        {toLook.length > 0 && seen.length > 0 ? <DropdownMenuSeparator /> : null}
        {seen.length > 0 ? <>
          <DropdownMenuLabel data-desk-views-group="seen">{t("desk.views.seen", { count: seen.length })}</DropdownMenuLabel>
          {seen.map((output) => <ViewRow key={output.id} output={output} unseen={false} onOpen={onOpen} />)}
        </> : null}
        {more ? <p className="px-sm py-xs text-caption text-muted-foreground" data-desk-views-older="true">{t("desk.views.older")}</p> : null}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** One View in the list (B23, B24): its kind, its name, where it is, the eye while not looked at; its preview tile beside it under the pointer. */
function ViewRow({ output, unseen, onOpen }: { output: DeskOutput; unseen: boolean; onOpen: (output: DeskOutput) => void }) {
  const where = viewWhere(output);
  return (
    <Hint label={output.label} side="left" content={<PreviewTile file={tileOf(output)} />}>
      <DropdownMenuItem data-desk-view-row={output.id} data-desk-view-unseen={unseen ? "true" : undefined} onSelect={() => onOpen(output)}>
        <span className="flex size-(--size-icon) shrink-0 items-center justify-center"><ViewMark output={output} /></span>
        <span className="min-w-0 flex-1 truncate text-foreground">{output.label}</span>
        {where ? <span className="max-w-[40%] shrink-0 truncate text-caption text-muted-foreground" data-desk-view-where="true">{where}</span> : null}
        {unseen ? <UnseenMark /> : null}
      </DropdownMenuItem>
    </Hint>
  );
}

/** The session's one pull request, as the pane head's PR mark draws it, with its card under the pointer (B26). */
function OnePull({ item, row, onPress, onPull }: { item: ShelfPull; row: AgentRow | null; onPress: (item: ShelfPull, event: MouseEvent) => void; onPull: (pull: SessionPull, external: boolean) => void }) {
  const chip = (
    <Button variant="outline" size="sm" className={cn(CHIP, "shrink")} data-desk-pulls={1} onClick={(event) => onPress(item, event)}>
      <PrMark state={item.state} label={item.label} stale={item.pull?.stale} className={cn("min-w-0", item.state === "closed" && CLOSED_DIM)} />
    </Button>
  );
  if (item.pull && row) {
    const pull = item.pull;
    return <PrHoverCard agent={row} pulls={[pull]} staleness={pullsStaleness([pull])} side="bottom" onOpenPullRequest={(_, external) => onPull(pull, external)}>{chip}</PrHoverCard>;
  }
  return <Hint label={item.title ? `${item.label} ${item.title}` : item.label} side="bottom">{chip}</Hint>;
}

/** `PR n` and its floating list, each row a pull request as the PR list draws it (B26). */
function PullsChip({ items, onPress }: { items: ShelfPull[]; onPress: (item: ShelfPull, event: MouseEvent) => void }) {
  const { t } = useInterfaceTranslation();
  const [open, setOpen] = useState(false);
  const Chevron = open ? ChevronUpIcon : ChevronDownIcon;
  return (
    <DropdownMenu open={open} onOpenChange={setOpen}>
      <DropdownMenuTrigger asChild>
        <Button variant="outline" size="sm" className={cn(CHIP, "shrink-0")} data-desk-pulls={items.length}>
          <span>{t("desk.pulls.chip", { count: items.length })}</span>
          <Chevron aria-hidden="true" className={ICON} />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="max-h-(--size-desk-shelf-list) w-(--size-pr-popover) overflow-y-auto" data-desk-pulls-list={items.length}>
        {items.map((item) => (
          // A ⌘-press opens the pull request on its host; the item's own select would not carry the key.
          <DropdownMenuItem key={item.id} data-desk-pull-row={item.id} onClick={(event) => onPress(item, event)}>
            {item.pull ? <PrListItem pull={item.pull} /> : (
              <>
                <PrMark state={item.state} label={item.label} className={cn("shrink-0", item.state === "closed" && CLOSED_DIM)} />
                <span className={cn("min-w-0 flex-1 truncate text-foreground", item.state === "closed" && CLOSED_DIM)}>{item.title}</span>
              </>
            )}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
