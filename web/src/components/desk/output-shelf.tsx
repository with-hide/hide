import { ChevronDownIcon, ChevronUpIcon, FileIcon, GitPullRequestIcon, GlobeIcon, ImageIcon } from "lucide-react";
import { useState, type ReactNode } from "react";
import { pullLabel } from "../../codeHost";
import type { DeskOutput } from "../../desk";
import { useInterfaceTranslation } from "../../i18n/client";
import type { MessageKey } from "../../i18n/catalogs";
import { cn } from "../../lib/utils";
import { PR_LOOK } from "../../prMark";
import type { DeskOutputKind, PrState, SessionPull } from "../../snapshot";
import { PrMark } from "../pr-mark";
import { Button } from "../ui/button";
import { RowButton } from "../ui/row-button";
import { Hint } from "../ui/tooltip";
import { UnseenMark } from "./desk-job-card";

/** The shelf's groups in the order they stand when none waits to be seen (Doop B4, shelf 나). */
const KINDS: readonly DeskOutputKind[] = ["image", "page", "doc", "pr"];

const WORD: Record<DeskOutputKind, MessageKey> = {
  image: "desk.shelf.image",
  page: "desk.shelf.page",
  doc: "desk.shelf.doc",
  pr: "desk.shelf.pr",
};

/** How many pull requests an open PR group lists before `+ n개 더` (Doop B5). */
const PR_ROWS = 6;

const ICON = "size-(--size-icon-sm) shrink-0 text-muted-foreground";

/**
 * One thing on the shelf: an output a View showed, or a pull request of the
 * session's own list (the pane head's PR mark), which is not a View and so
 * never unseen. `where` is a file's path or a page's address.
 */
export type ShelfItem =
  | { kind: Exclude<DeskOutputKind, "pr">; id: string; label: string; where: string | null; thumb: string | null; unseen: boolean; output: DeskOutput }
  | { kind: "pr"; id: string; label: string; title: string; state: PrState; url: string; output: DeskOutput | null; pull: SessionPull | null };

export type ShelfGroup = { kind: DeskOutputKind; items: ShelfItem[]; unseen: boolean };

/**
 * The session's outputs and pull requests grouped by kind (Doop B4, shelf
 * 나): the groups that hold something not looked at yet first, each in the
 * kinds' order. A pull request the session's list holds is listed once,
 * whether a View also showed it or not.
 */
export function shelfGroups(outputs: readonly DeskOutput[], pulls: readonly SessionPull[]): ShelfGroup[] {
  const items: ShelfItem[] = [];
  for (const output of outputs) {
    if (output.kind === "pr") continue;
    items.push({ kind: output.kind, id: output.id, label: output.label, where: output.path ?? output.url, thumb: output.thumb, unseen: !output.seen, output });
  }
  for (const pull of pulls) {
    items.push({ kind: "pr", id: `pr:${pull.url}`, label: pullLabel(pull), title: pull.title, state: pull.state, url: pull.url, output: outputs.find((output) => output.kind === "pr" && output.url === pull.url) ?? null, pull });
  }
  for (const output of outputs) {
    if (output.kind !== "pr" || !output.url || pulls.some((pull) => pull.url === output.url)) continue;
    if (!output.pr_state) throw new Error(`PR output ${output.id} carries no pr_state`);
    items.push({ kind: "pr", id: output.id, label: output.label, title: "", state: output.pr_state, url: output.url, output, pull: null });
  }
  const groups = KINDS.map((kind) => {
    const mine = items.filter((item) => item.kind === kind);
    return { kind, items: mine, unseen: mine.some(unseen) };
  }).filter((group) => group.items.length > 0);
  return [...groups.filter((group) => group.unseen), ...groups.filter((group) => !group.unseen)];
}

function unseen(item: ShelfItem): boolean {
  return item.kind !== "pr" && item.unseen;
}

/** A picture on a chip, or the image mark while none is drawn yet. */
function Thumb({ item, className }: { item: ShelfItem; className?: string }) {
  return (
    <span className={cn("flex h-(--size-icon) w-(--size-control-sm) shrink-0 items-center justify-center overflow-hidden rounded-xs border border-border bg-muted", className)} aria-hidden="true">
      {item.kind === "image" && item.thumb ? <img src={item.thumb} alt="" draggable={false} className="size-full object-cover object-top" /> : <ImageIcon className={ICON} />}
    </span>
  );
}

/** Two of a group's pictures overlapping, the second behind the first. */
function Thumbs({ items }: { items: readonly ShelfItem[] }) {
  const shown = items.slice(0, 2);
  return (
    <span className="relative flex shrink-0 items-center" data-desk-shelf-thumbs={shown.length}>
      {shown.map((item, index) => <Thumb key={item.id} item={item} className={index === 0 ? "relative z-10" : "-ml-sm"} />)}
    </span>
  );
}

function KindIcon({ kind }: { kind: DeskOutputKind }) {
  if (kind === "page") return <GlobeIcon aria-hidden="true" className={ICON} />;
  if (kind === "doc") return <FileIcon aria-hidden="true" className={ICON} />;
  if (kind === "pr") return <GitPullRequestIcon aria-hidden="true" className={ICON} />;
  return null;
}

/** What hovering a thing alone of its kind shows (Doop B5): its whole name, an image's picture large, its path, address or state. */
function ItemDetails({ item }: { item: ShelfItem }) {
  const { t } = useInterfaceTranslation();
  return (
    <span className="flex max-w-(--size-pr-popover) flex-col gap-xxs" data-desk-shelf-details={item.id}>
      {item.kind === "image" && item.thumb ? <img src={item.thumb} alt="" className="w-full rounded-xs border border-border object-contain" /> : null}
      <span className="break-all font-medium">{item.kind === "pr" && item.title ? `${item.label} ${item.title}` : item.label}</span>
      {item.kind === "pr" ? <span className={PR_LOOK[item.state].tone}>{t(PR_LOOK[item.state].word)}</span> : item.where ? <span className="break-all text-muted-foreground">{item.where}</span> : null}
    </span>
  );
}

const CHIP = "h-(--size-control-lg) min-w-0 gap-xs px-sm text-body font-normal text-foreground";

/**
 * The shelf under the session panel's head (Doop B4 shelf 나, B5): one line
 * of what the session made, grouped by kind, `이미지 n · 보기 n · 파일 n ·
 * PR n`, the image group with two of its pictures; a group holding an output
 * not looked at yet ends with the look eye the sidebar uses, which goes once
 * that output is shown. A thing alone of its kind shows itself in place of
 * its count, its name (an image with its picture, a pull request as its
 * mark), its details on hover, and a press opens it in the tab panel; a
 * larger group opens its list under the line, the images as large pictures
 * two to a row, the pull requests as rows with their state, up to six and
 * `+ n개 더`. The line never wraps: a thing alone of its kind gives up its
 * name's room first. A session with nothing made has no shelf.
 */
export function OutputShelf({ outputs, pulls, selected = null, onOpen, onPull }: {
  outputs: readonly DeskOutput[];
  pulls: readonly SessionPull[];
  /** The output in front of the tab panel, ringed. */
  selected?: string | null;
  onOpen: (output: DeskOutput) => void;
  onPull: (pull: SessionPull) => void;
}) {
  const { t } = useInterfaceTranslation();
  const [open, setOpen] = useState<DeskOutputKind | null>(null);
  const [allPulls, setAllPulls] = useState(false);
  const groups = shelfGroups(outputs, pulls);
  if (groups.length === 0) return null;
  const press = (item: ShelfItem) => {
    if (item.kind === "pr" && item.pull) onPull(item.pull);
    else if (item.output) onOpen(item.output);
  };
  const shown = groups.find((group) => group.kind === open && group.items.length > 1) ?? null;
  return (
    <section className="flex shrink-0 flex-col border-b border-border" aria-label={t("desk.outputs.title")} data-desk-shelf={groups.reduce((sum, group) => sum + group.items.length, 0)}>
      <div className="flex min-w-0 flex-nowrap items-center gap-xs overflow-hidden px-md py-xs">
        {groups.map((group) => {
          const one = group.items.length === 1 ? group.items[0]! : null;
          if (one) {
            const face: ReactNode = one.kind === "pr"
              ? <PrMark state={one.state} label={one.label} className="min-w-0" />
              : <>
                {one.kind === "image" ? <Thumb item={one} /> : <KindIcon kind={one.kind} />}
                <span className="min-w-0 truncate" data-desk-shelf-one={one.id}>{one.label}</span>
              </>;
            return (
              <Hint key={group.kind} label={one.label} content={<ItemDetails item={one} />}>
                <Button variant="outline" size="sm" className={cn(CHIP, "shrink", selected !== null && one.output?.id === selected && "ring-1 ring-ring")} aria-pressed={one.output ? selected === one.output.id : undefined} data-desk-shelf-group={group.kind} data-desk-shelf-unseen={group.unseen ? "true" : undefined} onClick={() => press(one)}>
                  {face}
                  {group.unseen ? <UnseenMark /> : null}
                </Button>
              </Hint>
            );
          }
          const opened = open === group.kind;
          const Chevron = opened ? ChevronUpIcon : ChevronDownIcon;
          return (
            <Button key={group.kind} variant="outline" size="sm" className={cn(CHIP, "shrink-0", opened && "ring-1 ring-ring")} aria-expanded={opened} data-desk-shelf-group={group.kind} data-desk-shelf-unseen={group.unseen ? "true" : undefined} onClick={() => setOpen(opened ? null : group.kind)}>
              {group.kind === "image" ? <Thumbs items={group.items} /> : <KindIcon kind={group.kind} />}
              <span>{t(WORD[group.kind])}</span>
              <span className="text-muted-foreground tabular-nums">{group.items.length}</span>
              {group.unseen ? <UnseenMark /> : null}
              <Chevron aria-hidden="true" className="size-(--size-icon-sm) text-muted-foreground" />
            </Button>
          );
        })}
      </div>
      {shown ? (
        <div className="max-h-(--size-overview-graph-root-height) min-w-0 overflow-y-auto px-md pb-sm" data-desk-shelf-list={shown.kind}>
          {shown.kind === "image" ? (
            <ul className="grid min-w-0 grid-cols-2 gap-sm">
              {shown.items.map((item) => (
                <li key={item.id} className="flex min-w-0 flex-col gap-xxs">
                  <RowButton className={cn("relative h-(--size-desk-shelf-thumb) overflow-hidden rounded-sm border border-border bg-muted p-0 hover:border-ring", item.output?.id === selected && "border-ring ring-1 ring-ring")} aria-label={unseen(item) ? `${item.label} · ${t("desk.outputs.unseen")}` : item.label} onClick={() => press(item)} data-desk-shelf-item={item.id}>
                    {item.kind === "image" && item.thumb ? <img src={item.thumb} alt="" aria-hidden="true" draggable={false} className="size-full object-cover object-top" /> : <ImageIcon aria-hidden="true" className="m-auto size-(--size-icon) text-muted-foreground" />}
                    {unseen(item) ? <UnseenMark className="absolute right-xxs top-xxs" /> : null}
                  </RowButton>
                  <span className="min-w-0 truncate text-caption text-subtle-foreground" title={item.label}>{item.label}</span>
                </li>
              ))}
            </ul>
          ) : (
            <ul className="flex min-w-0 flex-col">
              {(shown.kind === "pr" && !allPulls ? shown.items.slice(0, PR_ROWS) : shown.items).map((item) => (
                <li key={item.id} className="min-w-0">
                  <RowButton className={cn("h-(--size-control-lg) gap-xs rounded-sm px-xs text-body", item.output?.id === selected && "bg-accent")} onClick={() => press(item)} data-desk-shelf-item={item.id}>
                    {item.kind === "pr" ? (
                      <>
                        <PrMark state={item.state} label={item.label} className="shrink-0" />
                        <span className="min-w-0 flex-1 truncate text-foreground" title={item.title}>{item.title}</span>
                        <span className={cn("shrink-0 text-caption", PR_LOOK[item.state].tone)}>{t(PR_LOOK[item.state].word)}</span>
                      </>
                    ) : (
                      <>
                        <KindIcon kind={item.kind} />
                        <span className="min-w-0 flex-1 truncate text-foreground" title={item.where ?? item.label}>{item.label}</span>
                        {unseen(item) ? <UnseenMark /> : null}
                      </>
                    )}
                  </RowButton>
                </li>
              ))}
              {shown.kind === "pr" && !allPulls && shown.items.length > PR_ROWS ? (
                <li>
                  <Button variant="text" size="inline" className="px-xs text-caption text-muted-foreground" data-desk-shelf-more={shown.items.length - PR_ROWS} onClick={() => setAllPulls(true)}>
                    {t("desk.shelf.more", { count: shown.items.length - PR_ROWS, names: shown.items.slice(PR_ROWS).map((item) => item.label).join(" ") })}
                  </Button>
                </li>
              ) : null}
            </ul>
          )}
        </div>
      ) : null}
    </section>
  );
}
