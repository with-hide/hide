import { ChevronDownIcon, ChevronUpIcon, FileIcon, GitPullRequestIcon, GlobeIcon, ImageIcon } from "lucide-react";
import { useState } from "react";
import type { DeskOutput } from "../../desk";
import type { DeskOutputKind } from "../../snapshot";
import { useInterfaceTranslation } from "../../i18n/client";
import type { MessageKey } from "../../i18n/catalogs";
import { cn } from "../../lib/utils";
import { Button } from "../ui/button";
import { DeskOutputs } from "./desk-outputs";
import { UnseenMark } from "./desk-job-card";

/** The shelf's groups in the order they stand when none waits to be seen (Doop B4, shelf 나). */
const KINDS: readonly DeskOutputKind[] = ["image", "page", "doc", "pr"];

const WORD: Record<DeskOutputKind, MessageKey> = {
  image: "desk.shelf.image",
  page: "desk.shelf.page",
  doc: "desk.shelf.doc",
  pr: "desk.shelf.pr",
};

const ICON = "size-(--size-icon-sm) shrink-0 text-muted-foreground";

type Group = { kind: DeskOutputKind; outputs: DeskOutput[]; unseen: boolean };

/**
 * The session's outputs grouped by kind (Doop B4, shelf 나): the groups that
 * hold an output not looked at yet first, each in the kinds' order. A PR is
 * not a View, so it is never unseen.
 */
export function shelfGroups(outputs: readonly DeskOutput[]): Group[] {
  const groups = KINDS.map((kind) => {
    const mine = outputs.filter((output) => output.kind === kind);
    return { kind, outputs: mine, unseen: kind !== "pr" && mine.some((output) => !output.seen) };
  }).filter((group) => group.outputs.length > 0);
  return [...groups.filter((group) => group.unseen), ...groups.filter((group) => !group.unseen)];
}

/** Two of the group's pictures overlapping, the second behind the first; a picture not drawn yet is the image mark. */
function Thumbs({ outputs }: { outputs: readonly DeskOutput[] }) {
  const shown = outputs.slice(0, 2);
  return (
    <span className="relative flex shrink-0 items-center" aria-hidden="true" data-desk-shelf-thumbs={shown.length}>
      {shown.map((output, index) => (
        <span key={output.id} className={cn("flex h-(--size-icon) w-(--size-control-sm) items-center justify-center overflow-hidden rounded-xs border border-border bg-muted", index === 0 ? "relative z-10" : "-ml-sm")}>
          {output.thumb ? <img src={output.thumb} alt="" draggable={false} className="size-full object-cover object-top" /> : <ImageIcon className={ICON} />}
        </span>
      ))}
    </span>
  );
}

function GroupLead({ group }: { group: Group }) {
  switch (group.kind) {
    case "image":
      return <Thumbs outputs={group.outputs} />;
    case "page":
      return <GlobeIcon aria-hidden="true" className={ICON} />;
    case "doc":
      return <FileIcon aria-hidden="true" className={ICON} />;
    case "pr":
      return <GitPullRequestIcon aria-hidden="true" className={ICON} />;
  }
}

/**
 * The shelf under the session panel's head (Doop B4, shelf 나): one line of
 * the session's outputs grouped by kind, `이미지 n · 보기 n · 파일 n · PR n`,
 * a group with an output not looked at yet ending with the look eye the
 * sidebar uses, which goes once that output is opened. An image, a View or
 * a file alone of its kind shows itself instead of its count, its name and
 * an image's picture (operator decision 2026-11-25). A group of one opens
 * its output in the tab panel; a larger group opens to its list under the
 * line, pictures larger with their names and the rest as rows, whose press
 * opens one there. The output shown in the tab panel is ringed. A session
 * with no output has no shelf.
 */
export function OutputShelf({ outputs, selected = null, onOpen }: { outputs: readonly DeskOutput[]; selected?: string | null; onOpen: (output: DeskOutput) => void }) {
  const { t } = useInterfaceTranslation();
  const [open, setOpen] = useState<DeskOutputKind | null>(null);
  const groups = shelfGroups(outputs);
  if (groups.length === 0) return null;
  const shown = groups.find((group) => group.kind === open) ?? null;
  return (
    <section className="flex shrink-0 flex-col border-b border-border" aria-label={t("desk.outputs.title")} data-desk-shelf={outputs.length}>
      <div className="flex min-w-0 flex-wrap items-center gap-xs px-md py-xs">
        {groups.map((group) => {
          const one = group.outputs.length === 1 ? group.outputs[0]! : null;
          const opened = open === group.kind;
          const Chevron = opened ? ChevronUpIcon : ChevronDownIcon;
          return (
            <Button
              key={group.kind}
              variant="outline"
              size="sm"
              className={cn("h-(--size-control-sm) max-w-full gap-xs px-sm text-caption font-normal text-foreground", (opened || (one && selected === one.id)) && "ring-1 ring-ring")}
              aria-expanded={one ? undefined : opened}
              aria-pressed={one ? selected === one.id : undefined}
              data-desk-shelf-group={group.kind}
              data-desk-shelf-unseen={group.unseen ? "true" : undefined}
              onClick={() => (one ? onOpen(one) : setOpen(opened ? null : group.kind))}
            >
              <GroupLead group={group} />
              {one && group.kind !== "pr" ? (
                <span className="min-w-0 max-w-(--size-pr-popover) truncate" title={one.label} data-desk-shelf-one={one.id}>{one.label}</span>
              ) : (
                <>
                  <span>{t(WORD[group.kind])}</span>
                  <span className="text-muted-foreground tabular-nums">{group.outputs.length}</span>
                </>
              )}
              {group.unseen ? <UnseenMark /> : null}
              {one ? null : <Chevron aria-hidden="true" className="size-(--size-icon-sm) text-muted-foreground" />}
            </Button>
          );
        })}
      </div>
      {shown ? (
        <div className="max-h-(--size-overview-graph-root-height) min-w-0 overflow-y-auto px-md pb-sm" data-desk-shelf-list={shown.kind}>
          <DeskOutputs outputs={shown.outputs} selected={selected} wide onOpen={onOpen} />
        </div>
      ) : null}
    </section>
  );
}
