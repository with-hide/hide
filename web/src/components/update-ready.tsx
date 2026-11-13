// A ready update of this app beside Help (docs/UI_BEHAVIOR.md, Help menu): a
// primary ↑ button at the head of the View, Help and Settings group while a
// downloaded update waits, which stays until the restart, and its card. The
// card opens by itself once per ready version, so an operator who keeps the
// app open learns of it without opening Help; closing it or Later folds it
// back into the button, and the host's updater keeps that close per version
// (`dismissed`), so only a newer ready version opens it again. Opened by
// itself it leaves the keyboard, Escape and outside clicks to what the
// operator works in; opened from the button it is an ordinary popover.
// A required update (a release names a minimum version above this app) opens
// its card the same way and keeps it open with no close and no Later, saying
// the work keeps running across the restart, until the operator restarts.
// State and actions are `updates.ts`'s; this file draws them.

import { ArrowUpIcon, ShieldCheckIcon, XIcon } from "lucide-react";
import { useId, useState } from "react";
import type { Actions } from "../actions";
import { useInterfaceTranslation } from "../i18n/client";
import { cardVersion, dismissUpdateCard, readyVersion, releaseNotesUrl, restartIntoUpdate, useUpdateStatus } from "../updates";
import { Button } from "./ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "./ui/popover";
import { Hint } from "./ui/tooltip";
import { hintSide, menuSide, type MenuPlace } from "./menu-place";

/**
 * The card's words and actions for `version`, from values a caller holds (the gallery draws it this way).
 * A required update's card has no close and no Later, and says in the foreground, under a check, that the work keeps running.
 */
export function UpdateReadyCardBody({ version, required = false, onRestart, onLater, onReleaseNotes }: { version: string; required?: boolean; onRestart: () => void; onLater: () => void; onReleaseNotes: () => void }) {
  const { t } = useInterfaceTranslation();
  const title = useId();
  return (
    <section aria-labelledby={title} className="flex flex-col gap-sm" data-update-card={version} data-update-card-required={required || undefined}>
      <div className="flex items-center gap-xs">
        <h2 id={title} className="min-w-0 flex-1 text-body font-medium text-foreground">
          {t(required ? "sidebar.updateCard.required" : "sidebar.help.updateReady")}
        </h2>
        {required ? null : (
          <Hint label={t("common.close")}>
            <Button variant="ghost" size="icon-xs" aria-label={t("common.close")} onClick={onLater} className="text-muted-foreground" data-update-card-close="true">
              <XIcon />
            </Button>
          </Hint>
        )}
      </div>
      <div className="flex flex-col gap-xs">
        <p className="break-keep text-body text-foreground">{t(required ? "sidebar.updateCard.requiredVersion" : "sidebar.updateCard.version", { version })}</p>
        {required ? (
          <p className="flex items-start gap-xs break-keep text-caption text-foreground" data-update-card-keeps="true">
            <ShieldCheckIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-success" />
            <span className="min-w-0">{t("sidebar.updateCard.requiredKeep")}</span>
          </p>
        ) : (
          <p className="text-caption text-muted-foreground">{t("sidebar.updateCard.keep")}</p>
        )}
      </div>
      <div className="flex flex-wrap items-center gap-xs">
        <Button size="sm" onClick={() => onRestart()} data-update-card-restart="true">
          {t("settings.update.restartNow")}
        </Button>
        {required ? null : (
          <Button variant="ghost" size="sm" onClick={onLater} data-update-card-later="true">
            {t("sidebar.updateCard.later")}
          </Button>
        )}
        <Button variant="text-link" size="inline" className="ml-auto text-caption underline" onClick={onReleaseNotes} data-update-card-notes="true">
          {t("sidebar.updateCard.releaseNotes")}
        </Button>
      </div>
    </section>
  );
}

/** The ↑ button and its card while an update is ready; nothing otherwise, and nothing in a browser tab or on the phone, which have no updates. */
export function UpdateReadyButton({ actions, place }: { actions: Actions; place: MenuPlace }) {
  const { t } = useInterfaceTranslation();
  const status = useUpdateStatus();
  const ready = readyVersion(status);
  // The version whose card opened by itself and was closed here, until the host's echo of the close arrives.
  const [closed, setClosed] = useState<string | null>(null);
  const [opened, setOpened] = useState(false);
  const required = status?.required === true;
  const pending = cardVersion(status);
  // A required update's card is never folded here, so a close from before it was required does not keep it shut.
  const auto = pending !== null && (required || pending !== closed);
  if (ready === null) return null;
  const open = opened || auto;
  const fold = () => {
    setOpened(false);
    if (auto && !required) {
      setClosed(ready);
      dismissUpdateCard(ready);
    }
  };
  const label = t(required ? "sidebar.help.versionRequired" : "sidebar.help.versionReady", { version: ready });
  return (
    <Popover open={open} onOpenChange={(next) => (next ? setOpened(true) : fold())} escape={!auto}>
      <Hint label={label} side={hintSide(place)}>
        <PopoverTrigger asChild>
          <Button size="icon-sm" aria-label={label} className="update-ready-ring" data-update-button={ready}>
            <ArrowUpIcon />
          </Button>
        </PopoverTrigger>
      </Hint>
      <PopoverContent
        side={menuSide(place)}
        align="end"
        className="w-(--size-usage-popover) rounded-lg p-md"
        data-update-card-open={auto ? "auto" : "asked"}
        // Opened by itself, the card takes nothing from the pane being typed in and stays until it is answered.
        onOpenAutoFocus={(event) => (auto ? event.preventDefault() : undefined)}
        onInteractOutside={(event) => (auto ? event.preventDefault() : undefined)}
        onEscapeKeyDown={(event) => (auto ? event.preventDefault() : undefined)}
      >
        <UpdateReadyCardBody version={ready} required={required} onRestart={restartIntoUpdate} onLater={fold} onReleaseNotes={() => actions.openLink(releaseNotesUrl(ready), true)} />
      </PopoverContent>
    </Popover>
  );
}
