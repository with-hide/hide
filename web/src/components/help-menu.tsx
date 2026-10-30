// The sidebar footer's Help menu (PRD sidebar-titlebar-refresh D-11, D-12,
// B13, B14): the shortcut sheet with its chord, the practice window, Copy
// diagnostics with the same text as Settings' About, the repository's
// new-issue page in the default browser, and the version, which is a line to read rather than an action
// until an update of this app is ready (PRD auto-update D-03, B15): then a dot
// sits on the menu's button and the line names the ready version and restarts
// into it. Nothing else of an update shows here (D-21).

import { CircleHelpIcon } from "lucide-react";
import { useState } from "react";
import type { Actions } from "../actions";
import { useInterfaceTranslation } from "../i18n/client";
import { openPractice } from "../practice/channel";
import { diagnosticsInputOf, diagnosticsText } from "../settings";
import { commandLabel } from "../shortcutLabels";
import { useShellStore } from "../store";
import { readyVersion, restartIntoUpdate, useUpdateStatus } from "../updates";
import { Button } from "./ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuShortcut, DropdownMenuTrigger } from "./ui/dropdown-menu";
import { Hint } from "./ui/tooltip";
import { hintSide, menuSide, type MenuPlace } from "./menu-place";

/** Where Report an issue goes: the repository's new-issue page. */
export const NEW_ISSUE_URL = "https://github.com/with-hide/hide/issues/new";

/** How long `Copied` stays beside Copy diagnostics. */
const COPIED_SHOWN_MS = 1500;

export function HelpMenu({ actions, place = "footer" }: { actions: Actions; place?: MenuPlace }) {
  const { t } = useInterfaceTranslation();
  const version = useShellStore((s) => s.daemon?.version ?? null);
  const ready = readyVersion(useUpdateStatus());
  const label = ready ? `${t("sidebar.help")} · ${t("sidebar.help.updateReady")}` : t("sidebar.help");
  const chord = useShellStore((s) => commandLabel("shortcuts", s.rest?.ui_state));
  const [copy, setCopy] = useState<"idle" | "copied" | "failed">("idle");
  const copyDiagnostics = (event: Event) => {
    // The menu stays open so the outcome is read where it was asked for (B14).
    event.preventDefault();
    const text = diagnosticsText(diagnosticsInputOf(useShellStore.getState(), navigator.userAgent));
    void actions.copyText(text, "diagnostics").then((copied) => {
      if (!copied) return setCopy("failed");
      setCopy("copied");
      window.setTimeout(() => setCopy((now) => (now === "copied" ? "idle" : now)), COPIED_SHOWN_MS);
    });
  };
  return (
    <DropdownMenu onOpenChange={(open) => (open ? setCopy("idle") : undefined)}>
      <Hint label={label} side={hintSide(place)}>
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="icon-sm" aria-label={label} data-help-menu-trigger="true" className="relative">
            <CircleHelpIcon />
            {ready ? <span role="img" aria-label={t("sidebar.help.updateReady")} className="absolute right-xxs top-xxs size-(--size-tab-status-dot) rounded-full bg-primary" data-help-update-dot="true" /> : null}
          </Button>
        </DropdownMenuTrigger>
      </Hint>
      <DropdownMenuContent align="end" side={menuSide(place)} aria-label={t("sidebar.help")} data-help-menu="true">
        <DropdownMenuItem data-help-item="shortcuts" onSelect={() => actions.openShortcuts()}>
          {t("sidebar.help.shortcuts")}
          {chord ? <DropdownMenuShortcut>{chord}</DropdownMenuShortcut> : null}
        </DropdownMenuItem>
        <DropdownMenuItem data-help-item="practice" onSelect={() => openPractice()}>
          {t("sidebar.help.practice")}
        </DropdownMenuItem>
        <DropdownMenuItem data-help-item="copy_diagnostics" onSelect={copyDiagnostics}>
          {t("settings.copyDiagnostics")}
          {copy === "copied" ? (
            <span role="status" className="ml-auto text-caption text-success" data-help-copied="true">
              {t("common.copied")}
            </span>
          ) : copy === "failed" ? (
            <span role="alert" className="ml-auto text-caption text-destructive" data-help-copy-failed="true">
              {t("sessions.copyFailed")}
            </span>
          ) : null}
        </DropdownMenuItem>
        <DropdownMenuItem data-help-item="report_issue" onSelect={() => actions.openLink(NEW_ISSUE_URL, true)}>
          {t("sidebar.help.reportIssue")}
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        {ready ? (
          <DropdownMenuItem data-help-update={ready} onSelect={restartIntoUpdate}>
            <span className="font-mono text-caption">{t("sidebar.help.versionReady", { version: ready })}</span>
            <span className="ml-auto text-caption font-medium text-primary">{t("sidebar.help.restart")}</span>
          </DropdownMenuItem>
        ) : (
          <DropdownMenuLabel className="font-mono text-caption font-normal text-muted-foreground" data-help-version={version ?? ""}>
            {`hide ${version ?? t("settings.unavailable")}`}
          </DropdownMenuLabel>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
