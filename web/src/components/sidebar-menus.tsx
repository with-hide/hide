import type { Actions } from "../actions";
import { SettingsIcon } from "lucide-react";
import { useInterfaceTranslation } from "../i18n/client";
import { commandLabel } from "../shortcutLabels";
import { Button } from "./ui/button";
import { Hint } from "./ui/tooltip";
import { HelpMenu } from "./help-menu";
import { ViewMenu } from "./view-menu";
import { hintSide, type MenuPlace } from "./menu-place";

/** View, Help and Settings, in that order, as one group wherever they stand. */
export function SidebarMenus({ actions, place }: { actions: Actions; place: MenuPlace }) {
  const { t } = useInterfaceTranslation();
  return (
    <>
      <ViewMenu actions={actions} place={place} />
      <HelpMenu actions={actions} place={place} />
      <Hint label={`${t("common.settings")} (${commandLabel("settings")})`} side={hintSide(place)}>
        <Button variant="ghost" size="icon-sm" data-open-settings="true" onClick={() => actions.openSettings()}>
          <SettingsIcon />
        </Button>
      </Hint>
    </>
  );
}
