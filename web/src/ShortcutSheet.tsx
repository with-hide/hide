import type { Actions } from "./actions";
import { Dialog, DialogBody, DialogContent, DialogHeader, DialogTitle } from "./components/ui/dialog";
import { Kbd } from "./components/ui/kbd";
import { Hint } from "./components/ui/tooltip";
import { hostKind, keySystem } from "./host";
import { resolvedRegistry, sheetRows, storedBindings, type Command } from "./shortcuts";
import { useShellStore } from "./store";
import { useInterfaceTranslation } from "./i18n/client";
import { commandGroupTitle, passthroughText, sheetRowTitle } from "./shortcutLabels";

// The sheet is generated from the registry (PRD S2 B11): every mapping of the
// running host by group, a "moved for Chrome" note on the chords Chrome
// reserves (browser only), and a passthrough note on the one chord the
// terminal keeps. The numbered ⌃1-9, ⌥1-9 and ⌘1-9 selections fold into one row
// each with their range, and read "not on this host" in a browser, which
// has no such chords (PRD electron-digit-shortcuts-hints B3, B4).

const GROUPS: Command["group"][] = ["Tabs", "Navigate", "Panels", "Panes", "Help"];

export function ShortcutSheet({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  // The sheet reads the same effective registry the window listener runs, so
  // a rebound pane chord is what it lists (PRD S5 B9).
  const host = hostKind();
  const system = keySystem();
  const stored = useShellStore((s) => storedBindings(s.rest?.ui_state, host));
  const { registry, diagnostic } = resolvedRegistry(stored, host, system);
  // The gate in App.tsx only mounts this component while the overlay is
  // "shortcuts", so the dialog is always open here; closing it (Escape, a
  // click outside, or the trigger elsewhere) toggles that overlay off.
  return (
    <Dialog open onOpenChange={(next) => { if (!next) actions.openShortcuts(); }}>
      <DialogContent data-shortcut-sheet="true" className="w-(--size-search-sheet-w)">
        <DialogHeader className="flex-row items-baseline justify-between">
          <DialogTitle>{t("commands.shortcuts")}</DialogTitle>
          <span className="text-caption text-muted-foreground">{host === "electron" ? t("shell.desktopApp") : t("shell.browser")}</span>
        </DialogHeader>
        <DialogBody>
          {diagnostic ? (
            <p className="mb-md text-caption text-warning" data-shortcut-diagnostic="true">
              {diagnostic}
            </p>
          ) : null}
          {GROUPS.map((group) => (
            <section key={group} className="mb-md">
              <h3 className="mb-xs text-caption uppercase text-muted-foreground">{commandGroupTitle(group, t)}</h3>
              <ul>
                {sheetRows(group, registry, host, system).map((row) => (
                  <li key={row.id} className="flex items-center gap-md py-xxs" data-shortcut={row.id}>
                    <span className="min-w-0 flex-1 truncate">{sheetRowTitle(row, t)}</span>
                    {host === "browser" && row.moved && row.movedFrom ? (
                      <Hint label={t("shell.chromeReserves", { shortcut: row.movedFrom })}>
                        <span className="text-caption text-warning">{t("shell.movedForChrome", { shortcut: row.movedFrom })}</span>
                      </Hint>
                    ) : null}
                    {row.passthrough ? <span className="text-caption text-muted-foreground">{passthroughText(row.passthrough, t)}</span> : null}
                    {row.chord === null && row.id.startsWith("select_") ? <span className="text-caption text-muted-foreground" data-shortcut-absent={row.id}>{t("shell.notOnHost")}</span> : null}
                    <Kbd>{row.chord ?? "-"}</Kbd>
                  </li>
                ))}
              </ul>
            </section>
          ))}
        </DialogBody>
      </DialogContent>
    </Dialog>
  );
}
