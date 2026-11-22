import { useRef, useState } from "react";
import { ChevronDownIcon, XIcon } from "lucide-react";
import { Button } from "../components/ui/button";
import { Sheet, SheetClose, SheetContent, SheetHeader, SheetTitle, SheetTrigger } from "../components/ui/sheet";
import { useInterfaceTranslation } from "../i18n/translator";
import { usePhone } from "./store";
import { HostList } from "./HostList";
import { DemoHostPanel, SamplePill } from "./Demo";

export function HostPicker({ name, activeRoom, defaultOpen = false }: { name: string; /** The room of the Mac being viewed, when the page address does not name it (the gallery). */ activeRoom?: string; defaultOpen?: boolean }) {
  const { t } = useInterfaceTranslation();
  const active = usePhone();
  // The Mac's own counts: the groups' counts also hold its devices' rows, which the rows under it say.
  const local = active.devices.find((device) => device.local);
  const [open, setOpen] = useState(defaultOpen);
  const trigger = useRef<HTMLButtonElement>(null);
  return <Sheet open={open} onOpenChange={setOpen}>
    {/* The screen's title: the computer's name and a chevron (Doop 01). */}
    <SheetTrigger asChild><Button ref={trigger} variant="ghost" className="-ml-xs h-auto min-h-(--size-touch-target) max-w-full justify-start gap-sm px-xs py-none text-headline font-semibold" data-phone-host-picker="true">
      <span className="min-w-0 whitespace-normal break-keep text-left wrap-anywhere">{name}</span>{active.demo ? <SamplePill /> : null}<ChevronDownIcon aria-hidden="true" className="size-(--size-icon-lg) shrink-0 text-muted-foreground" />
    </Button></SheetTrigger>
    <SheetContent side="bottom" returnFocusTo={() => trigger.current} aria-describedby={undefined} className="phone-safe-bottom max-h-dvh overflow-y-auto rounded-t-xl px-lg">
      <SheetHeader className="flex-row items-center justify-between px-sm py-sm">
        <SheetTitle className="text-body font-medium text-muted-foreground">{t("mobile.relay.devices")}</SheetTitle>
        <SheetClose asChild><Button variant="ghost" size="icon" className="size-(--size-touch-target)" aria-label={t("common.close")}><XIcon aria-hidden="true" /></Button></SheetClose>
      </SheetHeader>
      <div className="pb-lg">{open && active.demo ? <DemoHostPanel /> : open ? <HostList t={t} machines={active.devices} activeRoom={activeRoom} active={{ online: active.connected, lastSeen: null, needs: local?.needs_you, working: local?.working }} /> : null}</div>
    </SheetContent>
  </Sheet>;
}
