import { useRef, useState } from "react";
import { ChevronDownIcon, LaptopIcon, XIcon } from "lucide-react";
import { Button } from "../components/ui/button";
import { Sheet, SheetClose, SheetContent, SheetHeader, SheetTitle, SheetTrigger } from "../components/ui/sheet";
import { useInterfaceTranslation } from "../i18n/translator";
import { usePhone } from "./store";
import { HostList } from "./HostList";

export function HostPicker({ name }: { name: string }) {
  const { t } = useInterfaceTranslation();
  const active = usePhone();
  // The Mac's own counts: the groups' counts also hold its devices' rows, which the rows under it say.
  const local = active.devices.find((device) => device.local);
  const [open, setOpen] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  return <Sheet open={open} onOpenChange={setOpen}>
    <SheetTrigger asChild><Button ref={trigger} variant="ghost" className="mt-xs h-auto min-h-(--size-touch-target) max-w-full justify-start gap-sm px-none py-sm text-subhead">
      <LaptopIcon aria-hidden="true" className="size-(--size-icon) shrink-0" /><span className="min-w-0 whitespace-normal break-keep text-left wrap-anywhere">{name}</span><ChevronDownIcon aria-hidden="true" className="size-(--size-icon) shrink-0" />
    </Button></SheetTrigger>
    <SheetContent side="bottom" returnFocusTo={() => trigger.current} aria-describedby={undefined} className="phone-safe-bottom max-h-dvh overflow-y-auto rounded-t-xl px-lg">
      <SheetHeader className="flex-row items-center justify-between px-sm py-sm">
        <SheetTitle className="text-body font-medium text-muted-foreground">{t("mobile.relay.devices")}</SheetTitle>
        <SheetClose asChild><Button variant="ghost" size="icon" className="size-(--size-touch-target)" aria-label={t("common.close")}><XIcon aria-hidden="true" /></Button></SheetClose>
      </SheetHeader>
      <div className="pb-lg">{open ? <HostList t={t} machines={active.devices} active={{ online: active.connected, lastSeen: null, needs: local?.needs_you, working: local?.working }} /> : null}</div>
    </SheetContent>
  </Sheet>;
}
