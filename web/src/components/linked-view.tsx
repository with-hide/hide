import { CheckIcon, FileDiffIcon, GlobeIcon } from "lucide-react";
import { fileIcon } from "../fileIcons";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import type { LinkedView, LookAt } from "../snapshot";
import { FileMark } from "./file-mark";
import { StatusMark } from "./status-mark";
import { Button } from "./ui/button";
import { RowButton } from "./ui/row-button";

/**
 * One View a mission links, as the decision card's `볼 자료` and a done node's
 * `결과` list it: its kind's mark, its name, the eye while the operator has
 * not looked at it, a check once they have, `닫힘` once it closed, who linked
 * it where that tells rows apart, and the press's word. Pressing an open one
 * brings it forward; pressing a closed one opens it again from what its link
 * kept (`actions.openLinkedView`).
 */
export function LinkedViewRow({ view, onOpen, by = true, data }: { view: LinkedView; onOpen: (view: LinkedView) => void; by?: boolean; data?: Record<`data-${string}`, string> }) {
  const { t } = useInterfaceTranslation();
  return (
    <RowButton className="gap-xs px-xxs py-xxs text-body" title={view.label} onClick={() => onOpen(view)} data-linked-view-state={view.closed ? "closed" : view.unseen ? "unseen" : "seen"} {...data}>
      <ViewMark view={view} />
      <span className={cn("min-w-0 flex-1 truncate", view.closed && "text-muted-foreground")}>{view.label}</span>
      {view.closed ? (
        <span className="shrink-0 text-caption text-muted-foreground">{t("mission.banner.closed")}</span>
      ) : view.unseen ? (
        <StatusMark mark="look" label={t("agents.mark.look")} />
      ) : (
        <CheckIcon role="img" aria-label={t("mission.banner.seen")} className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />
      )}
      {by ? <span className="shrink-0 text-caption text-muted-foreground">{t(`mission.banner.by.${view.by}`)}</span> : null}
      <span className="ml-xs shrink-0 text-caption font-semibold text-subtle-foreground">{t(view.closed ? "mission.banner.reopen" : "mission.banner.open")}</span>
    </RowButton>
  );
}

/** A View's kind as its tab draws it: a page's globe, a diff's mark, a file's type. */
function ViewMark({ view }: { view: LinkedView }) {
  if (view.kind === "browser") return <GlobeIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />;
  if (view.kind === "diff") return <FileDiffIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />;
  return <FileMark icon={fileIcon(view.label)} className="size-(--size-icon-sm)" />;
}

/**
 * What an agent opened that the operator has not looked at, as a chip on a
 * row whose own line says something else (a question, a decision, a run's
 * result): its file mark and name, `외 N` for the others, the unseen dot. A
 * press brings it forward as the row's 볼 것 does (operator decision
 * 2026-11-21); it stops there, so the row's own press is not also taken.
 */
export function LookChip({ look, onOpen, className }: { look: LookAt; onOpen: () => void; className?: string }) {
  const { t } = useInterfaceTranslation();
  const more = look.count > 1 ? t("agents.look.more", { count: look.count - 1 }) : null;
  return (
    <Button
      type="button"
      variant="outline"
      size="inline"
      title={look.label}
      aria-label={t("agents.look.open", { label: look.label })}
      className={cn("pointer-events-auto relative min-w-0 max-w-full shrink gap-xxs rounded-xs px-xxs text-micro font-normal text-subtle-foreground", className)}
      onPointerDown={(event) => event.stopPropagation()}
      onClick={(event) => {
        event.stopPropagation();
        onOpen();
      }}
      data-look-chip={look.view_id}
    >
      <span aria-hidden="true" className="size-(--size-status-mark) shrink-0 rounded-full bg-warning" />
      <FileMark icon={fileIcon(look.label)} className="size-(--size-icon-sm)" />
      <span className="min-w-0 truncate">{look.label}</span>
      {more ? <span className="shrink-0 text-muted-foreground">{more}</span> : null}
    </Button>
  );
}
