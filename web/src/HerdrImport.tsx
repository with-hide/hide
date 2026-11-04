// Moving a machine's Herdr save into Hide's own engine (PRD engine-bundle-4-herdr-removal D-6 to D-8, D-10): the
// question while Hide's own Herdr runs (B2, B3, B4), the sidebar's line until it moves or while a result waits
// (B5, B11, B16), and the read-only import Settings offers (B16), for this machine and for each device of that engine
// under its name (B18). The core decides what is offered and owns the move (`status.herdr_import`,
// `status.herdr_import_devices`); the question's "Later" is this page's, so the next launch of the app asks again.

import { LoaderCircleIcon } from "lucide-react";
import { create } from "zustand";
import type { Actions } from "./actions";
import { Group, Note, Row } from "./components/settings-rows";
import { Button } from "./components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "./components/ui/dialog";
import { asksToMove, importLine, offersImport } from "./herdrImportRules";
import { useInterfaceTranslation } from "./i18n/client";
import { machineName } from "./screenMachine";
import type { HerdrImport } from "./snapshot";
import { useShellStore } from "./store";

/** The machines whose question is put off on this page (B5): `""` is this machine, a device goes by its id. */
export const useHerdrImportLater = create<{ later: readonly string[]; set: (later: boolean, device?: string) => void }>((set) => ({
  later: [],
  set: (later, device = "") =>
    set((state) => ({ later: later ? [...new Set([...state.later, device])] : state.later.filter((one) => one !== device) })),
}));

const NO_DEVICES: HerdrImport[] = [];

/** This machine's save first, then each device's (B18). */
function useImports(): HerdrImport[] {
  const here = useShellStore((s) => s.rest?.status?.herdr_import ?? null);
  const devices = useShellStore((s) => s.rest?.status?.herdr_import_devices ?? NO_DEVICES);
  return here ? [here, ...devices] : devices;
}

const putOff = (later: readonly string[], found: HerdrImport) => later.includes(found.device ?? "");

/**
 * The question (B2): what runs in Hide's Herdr, what moving ends, and two buttons named by their result with none
 * chosen beforehand, so Enter presses nothing. It stays while the move runs and says why when Herdr would not stop.
 */
export function HerdrImportQuestion({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const imports = useImports();
  const later = useHerdrImportLater((s) => s.later);
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  // One question at a time: this machine's first, then each device's.
  const found = imports.find((one) => asksToMove(one, putOff(later, one)));
  if (!found) return null;
  const moving = found.stage === "moving";
  const device = found.device;
  const title = device === undefined
    ? found.agents > 0 ? t("herdrImport.ask.title", { tabs: found.tabs, agents: found.agents }) : t("herdrImport.ask.titleNoAgents", { tabs: found.tabs })
    : found.agents > 0
      ? t("herdrImport.ask.deviceTitle", { device: machineName(devices, device, t), tabs: found.tabs, agents: found.agents })
      : t("herdrImport.ask.deviceTitleNoAgents", { device: machineName(devices, device, t), tabs: found.tabs });
  return (
    <Dialog open>
      <DialogContent
        initialFocus="container"
        showCloseButton={false}
        data-herdr-import-question={found.stage}
        data-herdr-import-device={device}
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
              <Button variant="ghost" onClick={() => useHerdrImportLater.getState().set(true, device)} data-herdr-import-later="true">{t("herdrImport.ask.later")}</Button>
              <Button variant="destructive" onClick={() => actions.herdrImport("move", device)} data-herdr-import-move="true">
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
 * The sidebar's lines above its footer, one per machine with something to say, a device's under its name (B18):
 * what is not moved with its Move, which asks the question again (B4, B5); the read-only import of a save no Hide
 * Herdr runs (B16); a save that could not be read with its retry, or the workspaces a move left out, each with why
 * (B11); the conversations an import left open in a Herdr that has closed since, with their resume (B15).
 */
export function HerdrImportLine({ actions }: { actions: Actions }) {
  const imports = useImports();
  const later = useHerdrImportLater((s) => s.later);
  return imports.map((found) => (
    <ImportLineRow key={found.device ?? ""} actions={actions} found={found} asking={asksToMove(found, putOff(later, found))} />
  ));
}

function ImportLineRow({ actions, found, asking }: { actions: Actions; found: HerdrImport; asking: boolean }) {
  const { t } = useInterfaceTranslation();
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const line = importLine(found, asking);
  if (!line) return null;
  const device = found.device;
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
        {action(t("herdrImport.line.retry"), () => actions.herdrImport("look", device), "retry")}
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
        {action(t("herdrImport.line.done"), () => actions.herdrImport("continue", device), "done")}
      </>
    );
  } else if (line.kind === "waiting") {
    body = (
      <>
        <span className="min-w-0 flex-1 truncate text-caption text-foreground">{t("herdrImport.line.waiting", { count: line.count })}</span>
        {action(t("herdrImport.line.waitingAction"), () => actions.herdrImport("import", device), "resume")}
      </>
    );
  } else if (line.kind === "move") {
    body = (
      <>
        <span className="min-w-0 flex-1 truncate text-caption text-foreground">
          {line.agents > 0 ? t("herdrImport.line.move", { count: line.agents }) : t("herdrImport.line.moveTabs", { count: line.tabs })}
        </span>
        {action(t("herdrImport.line.moveAction"), () => useHerdrImportLater.getState().set(false, device), "ask")}
      </>
    );
  } else {
    body = (
      <>
        <span className="flex min-w-0 flex-1 flex-col gap-xxs">
          <span className="truncate text-caption text-foreground">{t("herdrImport.line.import", { count: line.tabs })}</span>
          {line.running ? <span className="truncate text-caption text-muted-foreground">{t("herdrImport.line.running")}</span> : null}
        </span>
        {action(t("herdrImport.line.importAction"), () => actions.herdrImport("import", device), "import")}
      </>
    );
  }
  return (
    <div className="flex shrink-0 items-center gap-sm px-md py-xs" data-herdr-import-line={line.kind} data-herdr-import-device={device}>
      {device === undefined ? null : <span className="max-w-1/3 shrink-0 truncate text-caption text-muted-foreground">{machineName(devices, device, t)}</span>}
      {body}
    </div>
  );
}

/** Settings' read-only import, for each machine whose save no Hide Herdr runs offers one (B16, B18, B20). */
export function HerdrImportGroup({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const offered = useImports().filter(offersImport);
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  if (offered.length === 0) return null;
  return offered.map((found) => (
    <Group key={found.device ?? ""} data-herdr-import-settings="true" data-herdr-import-device={found.device}>
      <Row
        label={found.device === undefined ? t("herdrImport.import.title") : t("herdrImport.import.deviceTitle", { device: machineName(devices, found.device, t) })}
        detail={t("herdrImport.import.note", { count: found.tabs })}
      >
        <Button variant="secondary" size="sm" onClick={() => actions.herdrImport("import", found.device)} data-herdr-import-settings-import="true">
          {t("herdrImport.import.button")}
        </Button>
      </Row>
      {found.herdr_running ? <Note>{t("herdrImport.line.running")}</Note> : null}
    </Group>
  ));
}
