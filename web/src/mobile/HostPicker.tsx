import { useId, useRef, useState } from "react";
import { ChevronDownIcon, LaptopIcon, XIcon } from "lucide-react";
import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import { Sheet, SheetClose, SheetContent, SheetHeader, SheetTitle, SheetTrigger } from "../components/ui/sheet";
import { useInterfaceTranslation } from "../i18n/translator";
import { cn } from "../lib/utils";
import { usePhone } from "./store";
import { DeviceTile, HostList } from "./HostList";
import { DemoHostPanel, SamplePill } from "./Demo";
import { headName, machinesName, shownThrough } from "./protocol";

/**
 * The home screen's head on the hosted phone: one line that opens the device sheet, the computer's tile and name (without `.local`),
 * its connection dot, `+N` for the devices the Mac shows through it, and the chevron. Its accessible name is the full `name + devices`.
 */
export function HostPicker({ name, activeRoom, defaultOpen = false }: { name: string; /** The room of the Mac being viewed, when the page address does not name it (the gallery). */ activeRoom?: string; defaultOpen?: boolean }) {
  const { t } = useInterfaceTranslation();
  const active = usePhone();
  // The Mac's own counts: the groups' counts also hold its devices' rows, which the rows under it say.
  const local = active.devices.find((device) => device.local);
  const through = shownThrough(active.devices).length;
  const [open, setOpen] = useState(defaultOpen);
  const trigger = useRef<HTMLButtonElement>(null);
  const stateId = useId();
  return <Sheet open={open} onOpenChange={setOpen}>
    <span id={stateId} className="sr-only">{t(active.connected ? "mobile.connected" : "mobile.disconnected")}</span>
    <SheetTrigger asChild><Button ref={trigger} variant="ghost" aria-label={machinesName(name, active.devices)} aria-describedby={stateId}
      className="-ml-xs h-auto min-h-(--size-touch-target) max-w-full justify-start gap-sm px-xs py-none text-title font-semibold text-foreground" data-phone-host-picker="true">
      <DeviceTile icon={LaptopIcon} compact />
      <span className="min-w-0 truncate">{headName(name)}</span>
      {active.demo ? <SamplePill /> : null}
      <span aria-hidden="true" className={cn("size-(--size-status-mark) shrink-0 rounded-full", active.connected ? "bg-success" : "bg-muted-foreground")} data-phone-head-dot={active.connected ? "on" : "off"} />
      {through > 0 ? <Badge aria-hidden="true" className="font-mono text-caption" data-phone-head-through={through}>+{through}</Badge> : null}
      <ChevronDownIcon aria-hidden="true" className="size-(--size-icon-lg) shrink-0 text-muted-foreground" />
    </Button></SheetTrigger>
    <SheetContent side="bottom" returnFocusTo={() => trigger.current} aria-describedby={undefined} className="phone-safe-bottom max-h-dvh overflow-y-auto rounded-t-xl px-lg">
      <SheetHeader className="flex-row items-center justify-between px-sm py-sm">
        <SheetTitle className="text-subhead font-semibold text-foreground">{t("mobile.relay.devices")}</SheetTitle>
        <SheetClose asChild><Button variant="ghost" size="icon" className="size-(--size-touch-target)" aria-label={t("common.close")}><XIcon aria-hidden="true" /></Button></SheetClose>
      </SheetHeader>
      <div className="pb-lg">{open && active.demo ? <DemoHostPanel /> : open ? <HostList t={t} machines={active.devices} activeRoom={activeRoom} active={{ online: active.connected, lastSeen: null, needs: local?.needs_you, working: local?.working }} /> : null}</div>
    </SheetContent>
  </Sheet>;
}
