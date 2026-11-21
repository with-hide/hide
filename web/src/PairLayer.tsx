import { Columns2Icon } from "lucide-react";
import type { Actions } from "./actions";
import { Button } from "./components/ui/button";
import { Kbd } from "./components/ui/kbd";
import { useInterfaceTranslation } from "./i18n/client";
import { allAgents } from "./paneAgents";
import { PairPaneCanvas } from "./PaneGrid";
import { commandLabel } from "./shortcutLabels";
import { useShellStore } from "./store";

// Pair view (PRD lane-desk-pair-view B17, B22, D-16, D-17): the body between
// the sidebar and the rail, under the tab row, drawn as the strip that says
// so and the paired pane's own terminal on the left; the lane places the
// tab's desk on the right half, or nothing while the pane is drawn alone at
// the body's full width (D-53). The tab's own canvas stays under it with its
// layout and zoom as they were.

/** The strip and the pane: the left half, `paneWidth` wide (0 until the body is measured), or the whole width while `alone`. */
export function PairLayer({ paneId, alone, paneWidth, covered, actions }: { paneId: string; alone: boolean; paneWidth: number; covered: boolean; actions: Actions }) {
  const width = alone ? "100%" : paneWidth > 0 ? paneWidth : "50%";
  return (
    <div className="absolute inset-x-0 bottom-0 top-(--size-tab-strip) z-10 flex min-h-0 min-w-0 flex-col bg-background" inert={covered} data-pair-layer={paneId} data-pair-alone={alone ? "true" : undefined}>
      <PairStrip paneId={paneId} actions={actions} />
      <div className="min-h-0 flex-1" style={{ width }}>
        <PairPaneCanvas paneId={paneId} actions={actions} />
      </div>
    </div>
  );
}

/**
 * The accent strip at the top of the body (B22): the mark and word `짝 보기`,
 * the paired agent's name, and the way back with its chord, so the screen
 * is never read as the usual one.
 */
function PairStrip({ paneId, actions }: { paneId: string; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const name = useShellStore((s) => allAgents(s.agents, s.rest?.status?.remote).find((row) => row.pane_id === paneId)?.identity_label ?? null);
  const chord = commandLabel("pair_view");
  return (
    <div className="flex h-(--size-tab-strip) shrink-0 items-center gap-sm border-b-2 border-pair-view bg-pair-view-band px-md text-caption text-foreground" role="status" data-pair-strip={paneId}>
      <span className="flex shrink-0 items-center gap-xxs rounded-full bg-pair-view px-sm font-medium text-primary-foreground">
        <Columns2Icon aria-hidden="true" className="size-(--size-icon-sm)" />
        {t("panes.pair.strip")}
      </span>
      <span className="min-w-0 flex-1 truncate font-semibold" data-pair-strip-agent="true">{name ?? t("panes.pair.terminal")}</span>
      <Button variant="outline" size="sm" className="shrink-0" onClick={() => actions.togglePairView(paneId)} data-pair-leave="true">
        {t("panes.menu.pairOff")}
        {chord ? <Kbd>{chord}</Kbd> : null}
      </Button>
    </div>
  );
}
