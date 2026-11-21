// What marks the sample host as a sample (docs/UI_BEHAVIOR.md, Phone demo mode): a bar over every screen with the way out, a pill beside the
// host's name wherever a real host's name would show, the host sheet's own panel and the Settings tab's rows, which lead to the real connect flow.

import { ChevronRightIcon, LaptopIcon } from "lucide-react";
import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import { RowButton } from "../components/ui/row-button";
import { useInterfaceTranslation } from "../i18n/translator";
import { leaveDemo } from "./connection";

/** Beside the sample host's name, so the name cannot be taken for a computer. */
export function SamplePill() {
  const { t } = useInterfaceTranslation();
  return (
    <Badge variant="secondary" className="rounded-full border border-border px-sm" data-phone-demo-sample="true">
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
      <DemoHost className="px-sm" />
      <p className="break-keep px-sm text-body text-muted-foreground wrap-anywhere">{t("mobile.demo.panelNote")}</p>
      <Button variant="secondary" className="min-h-(--size-touch-target) px-xl text-subhead" data-phone-demo-connect="true" onClick={() => leaveDemo(true)}>
        {t("mobile.demo.connectReal")}
      </Button>
      <Button variant="ghost" className="min-h-(--size-touch-target) text-body" onClick={() => leaveDemo()}>{t("mobile.demo.exit")}</Button>
    </div>
  );
}

/** The sample host with its pill and its state, as a real computer's row draws them. */
function DemoHost({ className }: { className: string }) {
  const { t } = useInterfaceTranslation();
  return (
    <div className={`flex items-start gap-md py-md ${className}`}>
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
  );
}

/**
 * The Settings tab's 연결된 컴퓨터 in a demo: one row per thing to do inside the section's card, the sample host, the real
 * connect flow and the way out; the section's note under the card says nothing from the demo goes to a computer.
 */
export function DemoHostRows() {
  const { t } = useInterfaceTranslation();
  const row = "min-h-(--size-touch-target) gap-md rounded-none px-md py-sm text-title text-foreground";
  return (
    <>
      <DemoHost className="px-md" />
      <RowButton className={row} data-phone-demo-connect="true" onClick={() => leaveDemo(true)}>
        <span className="min-w-0 flex-1 break-keep wrap-anywhere">{t("mobile.demo.connectReal")}</span>
        <ChevronRightIcon aria-hidden="true" className="size-(--size-icon) shrink-0 text-muted-foreground" />
      </RowButton>
      <RowButton className={row} data-phone-demo-leave="true" onClick={() => leaveDemo()}>
        <span className="min-w-0 flex-1 break-keep wrap-anywhere">{t("mobile.demo.exit")}</span>
      </RowButton>
    </>
  );
}
