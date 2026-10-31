import { EllipsisIcon } from "lucide-react";
import { useState } from "react";
import type { Actions } from "../actions";
import { useInterfaceTranslation } from "../i18n/client";
import { missionOpen } from "../missions";
import type { Mission } from "../snapshot";
import { EntryDropdown, type MenuEntry } from "./entry-menu";
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from "./ui/alert-dialog";
import { Button } from "./ui/button";

// 미션 종료 at the end of an open mission's head, behind its ⋯ menu, since
// ending is rare. A Conductor that holds the mission is asked to end it and
// still asks about the open pull requests; a mission no Conductor holds is
// cancelled by Hide at once, its pull requests, branches and workers left as
// they are. Either is confirmed first: a cancelled mission never reopens.

export function MissionEnd({ mission, actions }: { mission: Mission; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const [confirm, setConfirm] = useState(false);
  if (!missionOpen(mission)) return null;
  const held = mission.conductor.present;
  // A Conductor already asked has it in hand; once it is gone, the press cancels the mission itself.
  const unavailable = mission.stale ? t("mission.end.stale") : held && mission.end_asked ? t("mission.end.asked") : null;
  const items: MenuEntry<"end">[] = [{ id: "end", label: t("mission.end.menu"), unavailable, destructive: true }];
  const way = held ? "ask" : "cancel";
  return (
    <>
      <EntryDropdown
        label={t("mission.more")}
        title={mission.title}
        items={items}
        onSelect={() => setConfirm(true)}
        data-mission-more-menu={mission.id}
        trigger={
          <Button variant="ghost" size="icon-sm" className="text-muted-foreground" aria-label={t("mission.more")} data-mission-more={mission.id}>
            <EllipsisIcon />
          </Button>
        }
      />
      <AlertDialog open={confirm} onOpenChange={setConfirm}>
        <AlertDialogContent data-mission-end-confirm={way}>
          <AlertDialogHeader>
            <AlertDialogTitle>{t(`mission.end.${way}.title`)}</AlertDialogTitle>
            <AlertDialogDescription>{t(`mission.end.${way}.body`)}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel data-mission-end-keep="true">{t("mission.end.keep")}</AlertDialogCancel>
            <AlertDialogAction variant={held ? "default" : "destructive"} onClick={() => actions.endMission(mission.id)} data-mission-end-action={way}>
              {t(`mission.end.${way}.action`)}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}
