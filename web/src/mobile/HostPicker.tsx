import { useState } from "react";
import { ChevronDownIcon, LaptopIcon } from "lucide-react";
import { Button } from "../components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "../components/ui/popover";
import { useInterfaceTranslation } from "../i18n/translator";
import { usePhone } from "./store";
import { HostList } from "./HostList";

export function HostPicker({ name }: { name: string }) {
  const { t } = useInterfaceTranslation();
  const active = usePhone();
  const [open, setOpen] = useState(false);
  return <Popover open={open} onOpenChange={setOpen}>
    <PopoverTrigger asChild><Button variant="ghost" className="mt-xs max-w-full justify-start gap-xs text-subhead">
      <LaptopIcon className="size-(--size-icon) shrink-0" /><span className="min-w-0 truncate">{name}</span><ChevronDownIcon className="size-(--size-icon) shrink-0" />
    </Button></PopoverTrigger>
    <PopoverContent align="start">{open ? <HostList t={t} active={{ online: active.connected, lastSeen: null, needs: active.groups?.find((g) => g.group === "needs_you")?.count, working: active.groups?.find((g) => g.group === "working")?.count }} /> : null}</PopoverContent>
  </Popover>;
}

