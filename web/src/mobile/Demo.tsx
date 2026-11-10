// What marks the sample host as a sample (docs/UI_BEHAVIOR.md, Phone demo mode): a bar over every screen with the way out, a pill beside the
// host's name wherever a real host's name would show, and the host sheet's own panel, which leads to the real connect flow.

import { LaptopIcon } from "lucide-react";
import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import { useInterfaceTranslation } from "../i18n/translator";
import { leaveDemo } from "./connection";

/** Beside the sample host's name, so the name cannot be taken for a computer. */
export function SamplePill() {
  const { t } = useInterfaceTranslation();
  return (
    <Badge variant="outline" className="rounded-full px-sm" data-phone-demo-sample="true">
      {t("mobile.demo.sample")}
    </Badge>
  );
}

/** The slim line above every screen of a demo: it says the data is a sample, and ends the demo in one tap. */
export function DemoBar() {
  const { t } = useInterfaceTranslation();
  return (
    <div role="status" className="phone-demo-bar flex shrink-0 items-center justify-between gap-md border-b border-border bg-secondary px-xl" data-phone-demo-bar="true">
      <span className="min-w-0 break-keep text-body text-subtle-foreground wrap-anywhere">{t("mobile.demo.bar")}</span>
      <Button variant="text" className="min-h-(--size-touch-target) shrink-0 px-none text-body" data-phone-demo-exit="true" onClick={() => leaveDemo()}>
        {t("mobile.demo.exit")}
      </Button>
    </div>
  );
}

/** The host sheet of a demo: the one sample host, and the ways out (the real QR scan and paste flow, or back to the first screen). */
export function DemoHostPanel() {
  const { t } = useInterfaceTranslation();
  return (
    <div className="flex flex-col gap-md" data-phone-demo-panel="true">
      <div className="flex items-start gap-md px-sm py-md">
        <LaptopIcon aria-hidden="true" className="mt-xs size-(--size-icon-lg) shrink-0 text-muted-foreground" />
        <span className="flex min-w-0 flex-1 flex-col gap-xs">
          <span className="flex min-w-0 items-center gap-sm">
            <span className="min-w-0 whitespace-normal break-keep text-subhead font-medium wrap-anywhere">{t("mobile.demo.host")}</span>
            <SamplePill />
          </span>
          <span className="flex items-center gap-xs text-body text-muted-foreground">
            <span aria-hidden="true" className="size-(--size-status-mark) shrink-0 rounded-full bg-success" />
            {t("mobile.devices.viewing")}
          </span>
        </span>
      </div>
      <p className="px-sm text-body text-muted-foreground">{t("mobile.demo.panelNote")}</p>
      <Button variant="secondary" className="min-h-(--size-touch-target) px-xl text-subhead" data-phone-demo-connect="true" onClick={() => leaveDemo(true)}>
        {t("mobile.demo.connectReal")}
      </Button>
      <Button variant="ghost" className="min-h-(--size-touch-target) text-body" onClick={() => leaveDemo()}>{t("mobile.demo.exit")}</Button>
    </div>
  );
}
