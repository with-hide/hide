import { GitCompareIcon, GlobeIcon } from "lucide-react";
import { useState } from "react";
import type { Actions } from "../actions";
import type { ToolView } from "../agentScope";
import { fileIcon } from "../fileIcons";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import { Button } from "./ui/button";
import { Elapsed } from "./elapsed";
import { FileMark } from "./file-mark";
import { RowButton } from "./ui/row-button";

/** What a row needs of the shell: to open its View where it is. */
type ViewActions = Pick<Actions, "openViewAt">;

/**
 * The Views a tool or an agent it keeps opened (PRD hide-external-control D-55, B48), as its cell lists them in
 * the sidebar and the Overview: newest first, each one a row that opens it where it is, with a mark until the
 * operator has looked at it. Nothing here is the operator's turn, so nothing takes the warning colour: the mark
 * is the text colour and the label reads a step heavier. `limit` shows that many with the rest one press away.
 */
export function ToolViewList({ views, limit, actions }: { views: readonly ToolView[]; limit?: number; actions: ViewActions }) {
  const { t } = useInterfaceTranslation();
  const [all, setAll] = useState(false);
  const shown = all || limit === undefined ? views : views.slice(0, limit);
  const more = views.length - shown.length;
  return (
    <ul className="flex min-w-0 flex-col" data-tool-views={views.length}>
      {shown.map((view) => (
        <li key={`${view.device_id}:${view.checkout_path}:${view.view_id}`} className="min-w-0">
          <ToolViewRow view={view} actions={actions} />
        </li>
      ))}
      {more > 0 ? (
        <li className="px-xs py-xxs">
          <Button variant="text" size="inline" className="text-micro text-muted-foreground" onClick={() => setAll(true)} data-tool-views-more={more}>
            {t("taskLens.tool.viewsMore", { count: more })}
          </Button>
        </li>
      ) : null}
    </ul>
  );
}

function ToolViewRow({ view, actions }: { view: ToolView; actions: ViewActions }) {
  const { t } = useInterfaceTranslation();
  return (
    <RowButton
      className="gap-xs text-caption"
      title={view.label}
      aria-label={[view.label, view.unseen ? t("taskLens.tool.viewUnseen") : null, view.opened_by ?? null].filter(Boolean).join(", ")}
      onClick={() => actions.openViewAt(view.device_id, view.checkout_path, view.view_id)}
      data-tool-view={view.view_id}
      data-tool-view-unseen={view.unseen ? "" : undefined}
    >
      <Mark view={view} />
      <span className="flex min-w-0 flex-1 flex-col">
        <span className={cn("min-w-0 truncate", view.unseen ? "font-medium text-foreground" : "text-subtle-foreground")}>{view.label}</span>
        <span className="flex min-w-0 items-baseline gap-xs text-micro text-muted-foreground">
          {view.opened_by ? <span className="min-w-0 truncate">{view.opened_by}</span> : null}
          {view.opened_by ? <span aria-hidden="true">·</span> : null}
          <Elapsed since={view.opened_unix_ms} className="shrink-0" />
        </span>
      </span>
      {view.unseen ? <span aria-hidden="true" className="size-(--size-tab-status-dot) shrink-0 rounded-full bg-foreground" data-tool-view-dot="true" /> : null}
    </RowButton>
  );
}

/** The View's kind as its tab draws it: a page's globe, a diff's pair, a file's type. */
function Mark({ view }: { view: ToolView }) {
  if (view.kind === "browser") return <GlobeIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />;
  if (view.kind === "diff") return <GitCompareIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />;
  return <FileMark icon={fileIcon(view.label)} className="size-(--size-icon-sm)" />;
}
