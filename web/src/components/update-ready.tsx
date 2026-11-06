// A ready update of this app beside Help (docs/UI_BEHAVIOR.md, Help menu): a
// primary ↑ button at the head of the View, Help and Settings group while a
// downloaded update waits, which stays until the restart, and its card. The
// card opens by itself once per ready version, so an operator who keeps the
// app open learns of it without opening Help; closing it or Later folds it
// back into the button, and the host's updater keeps that close per version
// (`dismissed`), so only a newer ready version opens it again. Opened by
// itself it leaves the keyboard, Escape and outside clicks to what the
// operator works in; opened from the button it is an ordinary popover.
// State and actions are `updates.ts`'s; this file draws them.

import { ArrowUpIcon, XIcon } from "lucide-react";
import { useId, useState } from "react";
import type { Actions } from "../actions";
import { useInterfaceTranslation } from "../i18n/client";
import { cardVersion, dismissUpdateCard, readyVersion, releaseNotesUrl, restartIntoUpdate, useUpdateStatus } from "../updates";
import { Button } from "./ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "./ui/popover";
import { Hint } from "./ui/tooltip";
import { hintSide, menuSide, type MenuPlace } from "./menu-place";

/** The card's words and actions for `version`, from values a caller holds (the gallery draws it this way). */
export function UpdateReadyCardBody({ version, onRestart, onLater, onReleaseNotes }: { version: string; onRestart: () => void; onLater: () => void; onReleaseNotes: () => void }) {
  const { t } = useInterfaceTranslation();
  const title = useId();
  return (
    <section aria-labelledby={title} className="flex flex-col gap-sm" data-update-card={version}>
      <div className="flex items-center gap-xs">
        <h2 id={title} className="min-w-0 flex-1 text-body font-medium text-foreground">
          {t("sidebar.help.updateReady")}
        </h2>
        <Hint label={t("common.close")}>
          <Button variant="ghost" size="icon-xs" aria-label={t("common.close")} onClick={onLater} className="text-muted-foreground" data-update-card-close="true">
            <XIcon />
          </Button>
        </Hint>
      </div>
      <div className="flex flex-col gap-xxs">
        <p className="text-body text-foreground">{t("sidebar.updateCard.version", { version })}</p>
        <p className="text-caption text-muted-foreground">{t("sidebar.updateCard.keep")}</p>
      </div>
      <div className="flex flex-wrap items-center gap-xs">
        <Button size="sm" onClick={() => onRestart()} data-update-card-restart="true">
          {t("settings.update.restartNow")}
        </Button>
        <Button variant="ghost" size="sm" onClick={onLater} data-update-card-later="true">
          {t("sidebar.updateCard.later")}
        </Button>
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
  const pending = cardVersion(status);
  const auto = pending !== null && pending !== closed;
  if (ready === null) return null;
  const open = opened || auto;
  const fold = () => {
    setOpened(false);
    if (auto) {
      setClosed(ready);
      dismissUpdateCard(ready);
    }
  };
  const label = t("sidebar.help.versionReady", { version: ready });
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
        <UpdateReadyCardBody version={ready} onRestart={restartIntoUpdate} onLater={fold} onReleaseNotes={() => actions.openLink(releaseNotesUrl(ready), true)} />
      </PopoverContent>
    </Popover>
  );
}
