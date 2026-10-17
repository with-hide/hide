// The web Settings sheet (PRD S5 B1-B10, B18-B22; settings-left-nav B1-B4):
// a list of pages on the left and the chosen page on the right, each row
// reading a value the core or the daemon reported and each edit sent as the
// one core event that owns it. Nothing here decides a value; a pending edit
// shows as pending until the snapshot says it landed. The sheet is the shell:
// each page is its own module under `settings/`.

import { BellIcon, GaugeIcon, BotIcon, InfoIcon, KeyboardIcon, MonitorSmartphoneIcon, SlidersHorizontalIcon, SmartphoneIcon, SparklesIcon, XIcon, type LucideIcon } from "lucide-react";
import { GithubIcon } from "./components/github-icon";
import { useState } from "react";
import type { Actions } from "./actions";
import { Button } from "./components/ui/button";
import { Dialog, DialogContent, DialogTitle } from "./components/ui/dialog";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "./components/ui/tabs";
import { Hint } from "./components/ui/tooltip";
import { useInterfaceTranslation } from "./i18n/client";
import { MobileTab } from "./MobileTab";
import { awayDevice, coreMachineName, machineName } from "./screenMachine";
import { SETTINGS_GROUPS, type SettingsTab } from "./settings";
import { AboutTab } from "./settings/AboutTab";
import { AgentsTab } from "./settings/AgentsTab";
import { DevicesTab } from "./settings/DevicesTab";
import { GeneralTab } from "./settings/GeneralTab";
import { GithubTab } from "./settings/GithubTab";
import { HideAiTab } from "./settings/HideAiTab";
import { UsageTab } from "./settings/UsageTab";
import { NotificationsTab } from "./settings/NotificationsTab";
import { ShortcutsTab } from "./settings/ShortcutsTab";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { ProjectSettingsSheet } from "./ProjectSettingsSheet";

export function SettingsGate({ actions }: { actions: Actions }) {
  const overlay = useUiStore((s) => s.overlay);
  return overlay === "settings" ? <SettingsSheet actions={actions} /> : overlay === "project_settings" ? <ProjectSettingsSheet actions={actions} /> : null;
}

/** Each page's mark in the list. */
const TAB_ICONS: Record<SettingsTab, LucideIcon> = {
  general: SlidersHorizontalIcon,
  notifications: BellIcon,
  usage: GaugeIcon,
  devices: MonitorSmartphoneIcon,
  shortcuts: KeyboardIcon,
  agents: BotIcon,
  hideAi: SparklesIcon,
  mobile: SmartphoneIcon,
  github: GithubIcon,
  about: InfoIcon,
};

function SettingsSheet({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const close = () => useUiStore.getState().closeOverlay("settings");
  const [tab, setTab] = useState<SettingsTab>(() => useUiStore.getState().settingsTab);
  // The device in front when this window does not run on it, the core's machine included on a node's window.
  const selectedId = useShellStore((s) => awayDevice(s.rest)?.id ?? null);
  const selectedName = useShellStore((s) => (selectedId ? machineName(s.rest?.navigator?.devices, selectedId, t) : ""));
  const selected = selectedId ? { id: selectedId, name: selectedName } : null;
  // Hide AI, Mobile and GitHub are the core's machine's, which a window on another machine names by its name.
  const machineGroup = useShellStore((s) => coreMachineName(s.rest?.navigator?.devices, t));
  const groupName = (id: (typeof SETTINGS_GROUPS)[number]["id"]) => (id === "app" ? t("settings.groups.app") : machineGroup);
  return (
    <Dialog open onOpenChange={(next) => { if (!next) close(); }}>
      {/* A Shortcuts chip that is recording owns Escape: it cancels the recording, not the sheet (B61).
          A window narrower or shorter than the sheet keeps the window inset clear on every side (settings-left-nav B1, B13). */}
      <DialogContent
        data-settings="true"
        aria-describedby={undefined}
        className="h-(--size-settings-sheet-h-max) max-h-[calc(100%-2*var(--size-settings-sheet-window-inset))] w-(--size-settings-sheet-w) max-w-[calc(100%-2*var(--size-settings-sheet-window-inset))]"
        onEscapeKeyDown={(event) => {
          if (useUiStore.getState().recordingShortcut) event.preventDefault();
        }}
      >
        {/* Radix Tabs owns the roving tabindex: Up and Down (plus Home and End)
            move through the list and show each page, Tab enters the page (B3). */}
        <Tabs value={tab} onValueChange={(value) => setTab(value as SettingsTab)} orientation="vertical" className="min-h-0 flex-1 flex-row gap-none">
          <nav className="flex min-h-0 w-(--size-settings-nav-w) shrink-0 flex-col overflow-hidden border-r border-border bg-sidebar px-sm pb-md pt-sm">
            <DialogTitle className="shrink-0 px-sm pb-xs text-title font-semibold">{t("common.settings")}</DialogTitle>
            <TabsList aria-label={t("settings.section")} className="min-h-0 w-full flex-1 flex-col items-stretch gap-xxs rounded-none bg-transparent p-none">
              <div className="flex min-h-0 flex-1 flex-col gap-xxs overflow-y-auto" data-settings-page-list="true">
                {SETTINGS_GROUPS.map((group) => (
                  <div key={group.id} className="flex shrink-0 flex-col gap-xxs" data-settings-nav-group={group.id}>
                    <div className="truncate px-sm pb-xs pt-xs text-micro font-medium text-muted-foreground">{groupName(group.id)}</div>
                    {group.tabs.map((row) => (
                      <NavItem key={row} tab={row} label={t(`settings.tabs.${row}`)} />
                    ))}
                  </div>
                ))}
              </div>
              <div className="flex shrink-0 flex-col pt-xs">
                <NavItem tab="about" label={t("settings.tabs.about")} />
              </div>
            </TabsList>
          </nav>
          <div className="flex min-w-0 flex-1 flex-col">
            <header className="flex items-center gap-sm px-xl pb-lg pt-lg">
              <div className="flex min-w-0 flex-1 items-center gap-sm">
                <h2 className="truncate text-headline font-semibold" data-settings-page-title={tab}>{t(`settings.tabs.${tab}`)}</h2>
                {/* Another machine's name shows only while it is the selected device (B6), beside the page title (B4). */}
                {selected ? (
                  <Hint label={t("settings.ownerRemote", { device: selected.name })}>
                    <span className="inline-block min-w-0 max-w-full truncate rounded-sm border border-border px-xs text-caption text-muted-foreground outline-none focus-visible:ring-1 focus-visible:ring-ring" tabIndex={0} data-settings-device={selected.id}>
                      {selected.name}
                    </span>
                  </Hint>
                ) : null}
              </div>
              <Hint label={t("settings.close")} shortcut="Esc">
                <Button variant="ghost" size="icon-sm" aria-label={t("settings.close")} onClick={close} data-settings-close="true">
                  <XIcon />
                </Button>
              </Hint>
            </header>
            {/* The page scrolls independently; large text can also scroll the page list above its fixed About row. */}
            <TabsContent value={tab} className="min-h-0 overflow-auto px-xl pb-xl focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-ring" data-settings-page={tab}>
              {tab === "general" ? <GeneralTab actions={actions} /> : null}
              {tab === "notifications" ? <NotificationsTab actions={actions} /> : null}
              {tab === "usage" ? <UsageTab actions={actions} /> : null}
              {tab === "devices" ? <DevicesTab actions={actions} /> : null}
              {tab === "shortcuts" ? <ShortcutsTab actions={actions} /> : null}
              {tab === "agents" ? <AgentsTab actions={actions} /> : null}
              {tab === "hideAi" ? <HideAiTab actions={actions} /> : null}
              {tab === "mobile" ? <MobileTab actions={actions} /> : null}
              {tab === "github" ? <GithubTab actions={actions} /> : null}
              {tab === "about" ? <AboutTab /> : null}
            </TabsContent>
          </div>
        </Tabs>
      </DialogContent>
    </Dialog>
  );
}

/** One page in the list: its mark and its name, filled while it is the page shown. */
function NavItem({ tab, label }: { tab: SettingsTab; label: string }) {
  const Icon = TAB_ICONS[tab];
  return (
    <TabsTrigger
      value={tab}
      data-settings-tab={tab}
      className="h-auto min-h-(--size-control) w-full justify-start gap-sm rounded-sm px-sm py-xs text-subhead font-normal data-[state=active]:font-medium"
    >
      <Icon aria-hidden="true" />
      <span className="truncate">{label}</span>
    </TabsTrigger>
  );
}
