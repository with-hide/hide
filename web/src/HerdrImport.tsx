// Moving this machine's Herdr save into Hide's own engine (PRD engine-bundle-4-herdr-removal D-6 to D-8): the
// question while Hide's own Herdr runs (B2, B3, B4), the sidebar's line until it moves or while a result waits
// (B5, B11, B16), and the read-only import Settings offers (B16). The core decides what is offered and owns the
// move (`status.herdr_import`); the question's "Later" is this page's, so the next launch of the app asks again.

import { LoaderCircleIcon } from "lucide-react";
import { create } from "zustand";
import type { Actions } from "./actions";
import { Group, Note, Row } from "./components/settings-rows";
import { Button } from "./components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "./components/ui/dialog";
import { asksToMove, importLine, offersImport } from "./herdrImportRules";
import { useInterfaceTranslation } from "./i18n/client";
import { useShellStore } from "./store";

/** The question put off on this page (B5). */
export const useHerdrImportLater = create<{ later: boolean; set: (later: boolean) => void }>((set) => ({
  later: false,
  set: (later) => set({ later }),
}));

/**
 * The question (B2): what runs in Hide's Herdr, what moving ends, and two buttons named by their result with none
 * chosen beforehand, so Enter presses nothing. It stays while the move runs and says why when Herdr would not stop.
 */
export function HerdrImportQuestion({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const found = useShellStore((s) => s.rest?.status?.herdr_import ?? null);
  const later = useHerdrImportLater((s) => s.later);
  if (!found || !asksToMove(found, later)) return null;
  const moving = found.stage === "moving";
  const title = found.agents > 0 ? t("herdrImport.ask.title", { tabs: found.tabs, agents: found.agents }) : t("herdrImport.ask.titleNoAgents", { tabs: found.tabs });
  return (
    <Dialog open>
      <DialogContent
        initialFocus="container"
        showCloseButton={false}
        data-herdr-import-question={found.stage}
        className="w-(--size-onboarding-dialog-w) [&_[data-slot=dialog-title]]:text-headline"
        // The question is answered by its buttons only.
        onEscapeKeyDown={(event) => event.preventDefault()}
        onPointerDownOutside={(event) => event.preventDefault()}
        onInteractOutside={(event) => event.preventDefault()}
      >
        <DialogHeader>
          <DialogTitle className="flex items-center gap-sm" aria-busy={moving || undefined}>
            {moving ? <LoaderCircleIcon aria-hidden="true" className="size-(--size-icon) shrink-0 animate-spin text-muted-foreground" /> : null}
            {title}
          </DialogTitle>
          <DialogDescription>{t("herdrImport.ask.description")}</DialogDescription>
        </DialogHeader>
        {found.failure === "not_stopped" && !moving ? (
          <DialogBody>
            <Note tone="error" data-herdr-import-failure="not_stopped">{t("herdrImport.failure.not_stopped")}</Note>
          </DialogBody>
        ) : <div className="h-md" />}
        <DialogFooter>
          <span className="flex-1" />
          {moving ? (
            <span className="text-body text-muted-foreground" data-herdr-import-moving="true">{t("herdrImport.ask.moving")}</span>
          ) : (
            <>
              <Button variant="ghost" onClick={() => useHerdrImportLater.getState().set(true)} data-herdr-import-later="true">{t("herdrImport.ask.later")}</Button>
              <Button variant="destructive" onClick={() => actions.herdrImport("move")} data-herdr-import-move="true">
                {found.agents > 0 ? t("herdrImport.ask.move", { count: found.agents }) : t("herdrImport.ask.moveTabs", { count: found.tabs })}
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/**
 * The sidebar's line above its footer: what is not moved with its Move, which asks the question again (B4, B5); the
 * read-only import of a save no Hide Herdr runs (B16); a save that could not be read with its retry, or the
 * workspaces a move left out, each with why (B11).
 */
export function HerdrImportLine({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const found = useShellStore((s) => s.rest?.status?.herdr_import ?? null);
  const later = useHerdrImportLater((s) => s.later);
  const line = importLine(found, asksToMove(found, later));
  if (!line) return null;
  const action = (label: string, onClick: () => void, data: string) => (
    <Button variant="text" size="inline" className="shrink-0 text-caption" onClick={onClick} {...{ [`data-herdr-import-${data}`]: "true" }}>
      {label}
    </Button>
  );
  let body;
  if (line.kind === "moving") {
    body = (
      <span className="flex min-w-0 items-center gap-xs text-caption text-muted-foreground">
        <LoaderCircleIcon aria-hidden="true" className="size-(--size-icon) shrink-0 animate-spin" />
        {t("herdrImport.ask.moving")}
      </span>
    );
  } else if (line.kind === "unreadable") {
    body = (
      <>
        <span className="flex min-w-0 flex-1 flex-col gap-xxs">
          <span className="truncate text-caption text-foreground">{t("herdrImport.line.unreadable")}</span>
          {line.reason ? <span className="truncate text-caption text-muted-foreground" title={line.reason}>{line.reason}</span> : null}
        </span>
        {action(t("herdrImport.line.retry"), () => actions.herdrImport("look"), "retry")}
      </>
    );
  } else if (line.kind === "refused") {
    body = (
      <>
        <span className="flex min-w-0 flex-1 flex-col gap-xxs">
          <span className="truncate text-caption text-foreground">{t("herdrImport.line.refused", { count: line.refused.length })}</span>
          {line.refused.map((row) => (
            <span key={row.label} className="truncate text-caption text-muted-foreground" title={row.reason}>
              {t("herdrImport.line.refusedRow", { label: row.label, reason: row.reason })}
            </span>
          ))}
        </span>
        {action(t("herdrImport.line.done"), () => actions.herdrImport("continue"), "done")}
      </>
    );
  } else if (line.kind === "move") {
    body = (
      <>
        <span className="min-w-0 flex-1 truncate text-caption text-foreground">
          {line.agents > 0 ? t("herdrImport.line.move", { count: line.agents }) : t("herdrImport.line.moveTabs", { count: line.tabs })}
        </span>
        {action(t("herdrImport.line.moveAction"), () => useHerdrImportLater.getState().set(false), "ask")}
      </>
    );
  } else {
    body = (
      <>
        <span className="flex min-w-0 flex-1 flex-col gap-xxs">
          <span className="truncate text-caption text-foreground">{t("herdrImport.line.import", { count: line.tabs })}</span>
          {line.running ? <span className="truncate text-caption text-muted-foreground">{t("herdrImport.line.running")}</span> : null}
        </span>
        {action(t("herdrImport.line.importAction"), () => actions.herdrImport("import"), "import")}
      </>
    );
  }
  return (
    <div className="flex shrink-0 items-center gap-sm px-md py-xs" data-herdr-import-line={line.kind}>
      {body}
    </div>
  );
}

/** Settings' read-only import, while a save no Hide Herdr runs offers one (B16). */
export function HerdrImportGroup({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const found = useShellStore((s) => s.rest?.status?.herdr_import ?? null);
  if (!found || !offersImport(found)) return null;
  return (
    <Group data-herdr-import-settings="true">
      <Row label={t("herdrImport.import.title")} detail={t("herdrImport.import.note", { count: found.tabs })}>
        <Button variant="secondary" size="sm" onClick={() => actions.herdrImport("import")} data-herdr-import-settings-import="true">
          {t("herdrImport.import.button")}
        </Button>
      </Row>
      {found.herdr_running ? <Note>{t("herdrImport.line.running")}</Note> : null}
    </Group>
  );
}
