// The start page's two screens that exist before any computer does: the first screen that says what to do, in order,
// with the QR scan as the one big button, and the settings reached from its corner. They are the loader's own and hold
// no store: the page with a computer keeps the list screen, and its Settings is the fourth tab of the phone app.

import { ChevronLeftIcon, PlayIcon, QrCodeIcon, SettingsIcon } from "lucide-react";
import type { TFunction } from "i18next";
import { Button } from "../components/ui/button";
import { AboutSection, DisplaySection, Section } from "./SettingsParts";

function Step({ n, title, children }: { n: number; title: string; children?: React.ReactNode }) {
  return (
    <li className="flex gap-md">
      <span aria-hidden="true" className="grid size-(--size-icon-lg) shrink-0 place-items-center rounded-full bg-muted text-body font-semibold text-muted-foreground">{n}</span>
      <div className="flex min-w-0 flex-col">
        <b className="break-keep text-subhead font-semibold wrap-anywhere">{title}</b>
        {children ? <span className="break-keep text-body text-muted-foreground wrap-anywhere">{children}</span> : null}
      </div>
    </li>
  );
}

/** The first screen: what Hide is, the three steps, the QR scan as the big button, and the demo under it. */
export function FirstScreen({ t, onScan, onDemo, onSettings }: { t: TFunction; onScan: () => void; onDemo: () => void; onSettings: () => void }) {
  return (
    <main className="phone-safe-top phone-safe-bottom flex min-h-full flex-col px-xl" data-phone-first="true">
      <header className="flex items-center justify-between pt-lg">
        <h1 className="text-headline font-semibold">hide</h1>
        <Button variant="ghost" size="icon" className="min-h-(--size-touch-target) min-w-(--size-touch-target) text-muted-foreground" aria-label={t("mobile.settings.title")} data-phone-first-settings="true" onClick={onSettings}>
          <SettingsIcon aria-hidden="true" className="size-(--size-icon-lg)" />
        </Button>
      </header>
      <h2 className="pt-xl text-headline font-semibold">{t("mobile.first.title")}</h2>
      <p className="pt-xs text-subhead text-subtle-foreground">{t("mobile.first.lead")}</p>
      <ol className="flex flex-col gap-lg pt-xl">
        <Step n={1} title={t("mobile.first.step1")}>{t("mobile.first.step1Body")} · <a className="underline underline-offset-4" href="https://withhide.dev" target="_blank" rel="noopener noreferrer">withhide.dev</a></Step>
        <Step n={2} title={t("mobile.first.step2")}>{t("mobile.first.step2Body")}</Step>
        <Step n={3} title={t("mobile.first.step3")} />
      </ol>
      <div className="min-h-xl flex-1" />
      <Button className="min-h-(--size-touch-target) gap-sm text-subhead" data-phone-scan="true" onClick={onScan}>
        <QrCodeIcon aria-hidden="true" className="size-(--size-icon)" />{t("mobile.relay.scan")}
      </Button>
      <Button variant="ghost" className="mt-xs min-h-(--size-touch-target) gap-sm text-subhead" data-phone-demo-entry="true" onClick={onDemo}>
        <PlayIcon aria-hidden="true" className="size-(--size-icon)" />{t("mobile.demo.try")}
      </Button>
      <p className="pb-lg text-center text-body text-muted-foreground">{t("mobile.demo.tryHint")}</p>
    </main>
  );
}

/** Settings with no computer: the computers section is the empty line, and display and about are the tab's own. */
export function StartSettings({ t, onBack }: { t: TFunction; onBack: () => void }) {
  return (
    <main className="phone-safe-top phone-safe-bottom flex min-h-full flex-col" data-phone-settings="true">
      <header className="flex items-center gap-xs px-lg">
        <Button variant="ghost" size="icon" className="min-h-(--size-touch-target) min-w-(--size-touch-target) text-muted-foreground" aria-label={t("common.back")} data-phone-settings-back="true" onClick={onBack}>
          <ChevronLeftIcon aria-hidden="true" className="size-(--size-icon-lg)" />
        </Button>
        <h1 className="text-headline font-semibold">{t("mobile.settings.title")}</h1>
      </header>
      <Section title={t("mobile.settings.computers")} data-phone-settings-section="computers">
        <p className="py-sm text-body text-muted-foreground">{t("mobile.first.noComputers")}</p>
      </Section>
      <DisplaySection t={t} follows="phone" />
      <AboutSection t={t} />
    </main>
  );
}
