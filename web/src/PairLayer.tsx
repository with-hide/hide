import { Columns2Icon } from "lucide-react";
import type { Actions } from "./actions";
import { Button } from "./components/ui/button";
import { Kbd } from "./components/ui/kbd";
import { useInterfaceTranslation } from "./i18n/client";
import { PairPaneCanvas } from "./PaneGrid";
import { paneTitle } from "./PaneView";
import { commandLabel } from "./shortcutLabels";
import type { Checkout } from "./snapshot";
import { useShellStore } from "./store";

// Focus mode (PRD lane-desk-pair-view B17, D-16, D-17, D-61): the body
// between the sidebar and the rail, drawn as the band in the tab row's place
// and, under it, the pane's own terminal on the left; the lane places the
// tab's desk on the right half, or nothing while the pane is drawn alone at
// the body's full width (D-53, D-57). The band takes the tab row's height, so
// the body is as tall as it is outside the mode, and the tab row with its
// group selector and + is hidden under it; ⌃⇥ and the sidebar move between
// panes. The tab's own canvas stays under it with its layout and zoom as
// they were.

/** The band and the pane: the left half, `paneWidth` wide (0 until the body is measured), or the whole width while `alone`. */
export function PairLayer({ checkout, paneId, alone, paneWidth, covered, actions }: { checkout: Checkout; paneId: string; alone: boolean; paneWidth: number; covered: boolean; actions: Actions }) {
  const width = alone ? "100%" : paneWidth > 0 ? paneWidth : "50%";
  return (
    <div className="absolute inset-0 z-10 flex min-h-0 min-w-0 flex-col bg-background" inert={covered} data-pair-layer={paneId} data-pair-alone={alone ? "true" : undefined}>
      <PairBand checkout={checkout} paneId={paneId} actions={actions} />
      <div className="min-h-0 flex-1" style={{ width }}>
        <PairPaneCanvas paneId={paneId} actions={actions} />
      </div>
    </div>
  );
}

/**
 * The band in the tab row's place (D-61): the mode's mark and word, the
 * name the left pane's head shows, which follows the pane the mode moves
 * to, and the way back with its chord. Nothing else (D-62).
 */
function PairBand({ checkout, paneId, actions }: { checkout: Checkout; paneId: string; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const pane = checkout.tabs.flatMap((tab) => tab.panes).find((row) => row.id === paneId) ?? null;
  const chord = useShellStore((s) => commandLabel("pair_view", s.rest?.ui_state));
  return (
    <div className="flex h-(--size-tab-strip) shrink-0 items-center gap-sm border-b border-border bg-pair-view-band px-md text-caption text-foreground" role="status" data-pair-strip={paneId}>
      <span className="flex shrink-0 items-center gap-xxs rounded-full bg-pair-view px-sm font-medium text-primary-foreground">
        <Columns2Icon aria-hidden="true" className="size-(--size-icon-sm)" />
        {t("panes.pair.strip")}
      </span>
      <span className="min-w-0 flex-1 truncate font-semibold" data-pair-strip-name="true">{pane ? paneTitle(pane) : t("panes.pair.terminal")}</span>
      <Button variant="outline" size="sm" className="shrink-0" onClick={() => actions.togglePairView(paneId)} data-pair-leave="true">
        {t("panes.menu.pairOff")}
        {chord ? <Kbd>{chord}</Kbd> : null}
      </Button>
    </div>
  );
}
