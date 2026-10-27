// The sidebar's invitation to practice: a quiet row above the footer for an
// operator who never practiced (`practiceInvited`). Pressing it opens the
// practice window on chapter 0; its × closes it for good, which the core
// keeps (`practice_invite_close`). Opening any chapter writes progress, so
// the row goes with the first step too.

import { GraduationCapIcon, XIcon } from "lucide-react";
import type { Actions } from "../actions";
import { useInterfaceTranslation } from "../i18n/client";
import { FIRST_CHAPTER, openPractice, practiceInvited } from "../practice/channel";
import { useShellStore } from "../store";
import { Button } from "./ui/button";
import { Hint } from "./ui/tooltip";

export function PracticeInvite({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  // No row until the first snapshot says what the core keeps, so it never flashes for an operator who practiced.
  const shown = useShellStore((s) => s.rest?.ui_state !== undefined && practiceInvited(s.rest.ui_state));
  if (!shown) return null;
  return (
    <div className="shrink-0 px-xs py-xs" data-practice-invite="true">
      <div className="group/invite flex min-w-0 items-center gap-xs rounded-xs pr-xs hover:bg-accent focus-within:bg-accent">
        <button
          type="button"
          onClick={() => openPractice(FIRST_CHAPTER)}
          className="flex min-w-0 flex-1 items-center gap-sm py-xs pl-sm text-left outline-none focus-visible:ring-1 focus-visible:ring-ring"
          data-practice-invite-open="true"
        >
          <GraduationCapIcon aria-hidden="true" className="size-(--size-icon) shrink-0 text-(--practice-accent-strong)" />
          <span className="flex min-w-0 flex-col gap-xxs">
            <span className="truncate text-caption font-medium text-foreground">{t("sidebar.practiceInvite.title")}</span>
            <span className="truncate text-caption text-muted-foreground">{t("sidebar.practiceInvite.line")}</span>
          </span>
        </button>
        {/* The row never comes back once closed, so the way that stays is said where it goes. */}
        <Hint label={t("sidebar.practiceInvite.close")}>
          <Button variant="ghost" size="icon-sm" aria-label={t("common.close")} onClick={() => actions.closePracticeInvite()} className="text-muted-foreground" data-practice-invite-close="true">
            <XIcon />
          </Button>
        </Hint>
      </div>
    </div>
  );
}
