// The window's top row (PRD sidebar-titlebar-refresh D-15 to D-18, B17 to
// B23): one row where macOS's title bar and the Workspace toolbar were. It
// opens with the sidebar's switch, after the traffic lights' space on a macOS
// window that is not full screen, at the same place whether the sidebar shows
// or not, so a hidden sidebar has a way back that is not a chord. Over the rail
// and the sidebar it names the device in front; over the center it carries the
// Workspace's path and icons, and nothing on any other screen.
// The row is where the desktop window is dragged from (`.window-drag`), and
// every control in it takes a press instead.

import { PanelLeftIcon } from "lucide-react";
import type { Actions } from "./actions";
import { DeviceTitle } from "./components/sidebar-header";
import { Button } from "./components/ui/button";
import { Hint } from "./components/ui/tooltip";
import { railShown } from "./devices";
import { useInterfaceTranslation } from "./i18n/client";
import { cn } from "./lib/utils";
import { commandLabel } from "./shortcutLabels";
import { useDeviceTitle, useSidebarColumns } from "./sidebar";
import { useShellStore } from "./store";
import { trafficLightSpace, useWindowChrome } from "./windowChrome";
import { WorkspaceTitle } from "./WorkspaceScreen";

export function TitleRow({ actions }: { actions: Actions }) {
  const chrome = useWindowChrome();
  const lights = trafficLightSpace(chrome);
  const sidebar = useShellStore((s) => s.rest?.ui_state?.left_sidebar_visible ?? true);
  const rail = useShellStore((s) => railShown(s.rest));
  const { title, deviceMenu } = useDeviceTitle(actions);
  // The device's part spans the rail and the sidebar under it, one edge for both.
  const columns = useSidebarColumns();
  // The switch's glyph stands where the device name used to start: a step after the traffic lights, or at the row's own inset without them.
  const start = lights ? "calc(var(--size-traffic-light-inset) + var(--spacing-xs))" : "var(--spacing-sm)";
  return (
    <header className="window-drag flex h-(--size-tab-strip) shrink-0 border-b border-border bg-sidebar" style={columns} data-title-row="true" data-traffic-lights={lights ? "true" : "false"}>
      {/* One box in both states, so the switch keeps its place and its keyboard focus when it hides the sidebar. */}
      <div
        className={cn("flex shrink-0 items-center gap-xs", sidebar && "w-(--sidebar-total) border-r border-input pr-xs")}
        style={{ paddingLeft: start }}
        data-sidebar-title={sidebar ? "true" : undefined}
      >
        <SidebarSwitch visible={sidebar} actions={actions} />
        {sidebar ? <DeviceTitle rail={rail} title={title} deviceMenu={deviceMenu} /> : null}
      </div>
      <div className="flex min-w-0 flex-1 items-stretch" data-title-center="true">
        <WorkspaceTitle actions={actions} />
      </div>
    </header>
  );
}

/**
 * Shows or hides the left sidebar, the same command as its chord (⌘B by
 * default). Like the chord, a click leaves the keyboard where it was, in a
 * terminal above all, as a macOS toolbar button does; Tab still reaches it.
 */
function SidebarSwitch({ visible, actions }: { visible: boolean; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const chord = useShellStore((s) => commandLabel("toggle_left_sidebar", s.rest?.ui_state));
  const label = visible ? t("sidebar.hide") : t("sidebar.show");
  return (
    <Hint label={label} shortcut={chord}>
      <Button variant="ghost" size="icon-sm" className="shrink-0 text-subtle-foreground" aria-label={label} aria-expanded={visible} data-sidebar-switch="true" data-keyboard-stays="true" onMouseDown={(event) => event.preventDefault()} onClick={actions.toggleLeftSidebar}>
        <PanelLeftIcon />
      </Button>
    </Hint>
  );
}
