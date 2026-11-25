import type { Actions } from "../actions";
import { SettingsIcon } from "lucide-react";
import { useInterfaceTranslation } from "../i18n/client";
import { commandLabel } from "../shortcutLabels";
import { Button } from "./ui/button";
import { Hint } from "./ui/tooltip";
import { HelpMenu } from "./help-menu";
import { UpdateReadyButton } from "./update-ready";
import { ViewMenu } from "./view-menu";
import { useFollowsViewer } from "../drawnDisplay";
import { hintSide, type MenuPlace } from "./menu-place";

/**
 * A ready update's ↑, then View, Help and Settings, in that order, as one
 * group wherever they stand. Another computer's screen that draws with the
 * showing window's display choices has no View menu: the window's own, on
 * its rail, is the one that changes what that screen draws.
 */
export function SidebarMenus({ actions, place }: { actions: Actions; place: MenuPlace }) {
  const { t } = useInterfaceTranslation();
  const followsViewer = useFollowsViewer();
  return (
    <>
      <UpdateReadyButton actions={actions} place={place} />
      {followsViewer ? null : <ViewMenu actions={actions} place={place} />}
      <HelpMenu actions={actions} place={place} />
      <Hint label={`${t("common.settings")} (${commandLabel("settings")})`} side={hintSide(place)}>
        <Button variant="ghost" size="icon-sm" data-open-settings="true" onClick={() => actions.openSettingsInFront()}>
          <SettingsIcon />
        </Button>
      </Hint>
    </>
  );
}
