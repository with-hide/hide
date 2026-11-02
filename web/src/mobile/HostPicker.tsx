import { useState } from "react";
import { ChevronDownIcon, LaptopIcon, XIcon } from "lucide-react";
import { Button } from "../components/ui/button";
import { Sheet, SheetClose, SheetContent, SheetHeader, SheetTitle, SheetTrigger } from "../components/ui/sheet";
import { useInterfaceTranslation } from "../i18n/translator";
import { usePhone } from "./store";
import { HostList } from "./HostList";

export function HostPicker({ name }: { name: string }) {
  const { t } = useInterfaceTranslation();
  const active = usePhone();
  const [open, setOpen] = useState(false);
  return <Sheet open={open} onOpenChange={setOpen}>
    <SheetTrigger asChild><Button variant="ghost" className="mt-xs h-auto min-h-(--size-touch-target) max-w-full justify-start gap-sm px-none py-sm text-subhead">
      <LaptopIcon aria-hidden="true" className="size-(--size-icon) shrink-0" /><span className="min-w-0 whitespace-normal break-keep text-left wrap-anywhere">{name}</span><ChevronDownIcon aria-hidden="true" className="size-(--size-icon) shrink-0" />
    </Button></SheetTrigger>
    <SheetContent side="bottom" aria-describedby={undefined} className="phone-safe-bottom max-h-dvh overflow-y-auto rounded-t-xl px-lg pb-lg">
      <SheetHeader className="flex-row items-center justify-between px-sm py-sm">
        <SheetTitle className="text-body font-medium text-muted-foreground">{t("mobile.relay.devices")}</SheetTitle>
        <SheetClose asChild><Button variant="ghost" size="icon" className="size-(--size-touch-target)" aria-label={t("common.close")}><XIcon aria-hidden="true" /></Button></SheetClose>
      </SheetHeader>
      {open ? <HostList t={t} active={{ online: active.connected, lastSeen: null, needs: active.groups?.find((g) => g.group === "needs_you")?.count, working: active.groups?.find((g) => g.group === "working")?.count }} /> : null}
    </SheetContent>
  </Sheet>;
}
