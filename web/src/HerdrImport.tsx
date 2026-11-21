// Importing a machine's Herdr save into Hide's own engine (PRD engine-bundle-4-herdr-removal D-7, D-8, D-10): the
// sidebar's line until it is imported or while a result waits (B5, B11, B15, B16), and the read-only import Settings
// offers (B16), for this machine and for each device of that engine under its name (B18). The core decides what is
// offered and owns the import (`status.herdr_import`, `status.herdr_import_devices`).

import { LoaderCircleIcon } from "lucide-react";
import type { Actions } from "./actions";
import { Group, Note, Row } from "./components/settings-rows";
import { Button } from "./components/ui/button";
import { importLine, offersImport } from "./herdrImportRules";
import { useInterfaceTranslation } from "./i18n/client";
import { machineName } from "./screenMachine";
import type { HerdrImport } from "./snapshot";
import { useShellStore } from "./store";

const NO_DEVICES: HerdrImport[] = [];

/** This machine's save first, then each device's (B18). */
function useImports(): HerdrImport[] {
  const here = useShellStore((s) => s.rest?.status?.herdr_import ?? null);
  const devices = useShellStore((s) => s.rest?.status?.herdr_import_devices ?? NO_DEVICES);
  return here ? [here, ...devices] : devices;
}

/**
 * The sidebar's lines above its footer, one per machine with something to say, a device's under its name (B18):
 * the read-only import of a save no Hide Herdr runs (B16); a save that could not be read with its retry, or the
 * workspaces an import left out, each with why (B11); the conversations an import left open in a Herdr that has
 * closed since, with their resume (B15).
 */
export function HerdrImportLine({ actions }: { actions: Actions }) {
  const imports = useImports();
  return imports.map((found) => <ImportLineRow key={found.device ?? ""} actions={actions} found={found} />);
}

function ImportLineRow({ actions, found }: { actions: Actions; found: HerdrImport }) {
  const { t } = useInterfaceTranslation();
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const line = importLine(found);
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
