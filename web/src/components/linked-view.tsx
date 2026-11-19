import { CheckIcon, FileDiffIcon, GlobeIcon } from "lucide-react";
import { fileIcon } from "../fileIcons";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import type { LinkedView } from "../snapshot";
import { FileMark } from "./file-mark";
import { StatusMark } from "./status-mark";
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
