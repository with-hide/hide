// The start page's screens that connect a computer: the first screen that says what to do, in order, with the QR scan as
// the one big button, and the settings reached from its corner; and the add page, the same steps with a way back and the
// pasted link folded under the scan. They are the loader's own and hold no store: the page with a computer keeps the list
// screen, and its Settings is the fourth tab of the phone app.

import { useState } from "react";
import { ChevronLeftIcon, ChevronRightIcon, PlayIcon, QrCodeIcon, SettingsIcon } from "lucide-react";
import type { TFunction } from "i18next";
import { decodeQr, type PairingQr } from "../../../hide-relay-protocol/web";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { appHost } from "./appHost";
import { relayReady } from "./relayRuntime";
import { scanInApp, WebScanner } from "./ScanQr";
import { AboutSection, DisplaySection, ROW_INSET, Section } from "./SettingsParts";

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

function Steps({ t }: { t: TFunction }) {
  return (
    <ol className="flex flex-col gap-lg pt-xl">
      <Step n={1} title={t("mobile.first.step1")}>{t("mobile.first.step1Body")} · <a className="underline underline-offset-4" href="https://withhide.dev" target="_blank" rel="noopener noreferrer">withhide.dev</a></Step>
      <Step n={2} title={t("mobile.first.step2")}>{t("mobile.first.step2Body")}</Step>
      <Step n={3} title={t("mobile.first.step3")} />
    </ol>
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
      <Steps t={t} />
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

type AddError = "cameraError" | "cameraBusy" | "qrError";

/**
 * The add page (`/?add=1`): the first screen's steps with a back arrow, the QR scan as the big button, and the pasted link as one
 * small row under it that opens a field and 연결 in place. In the app the scan is the app's own camera; in a browser it is the
 * browser's camera, drawn like it. A browser that keeps as many computers as it can says so before anything is scanned.
 */
export function AddScreen({ t, full, onBack, onPair }: { t: TFunction; full: boolean; onBack: () => void; onPair: (pair: PairingQr) => void }) {
  const [camera, setCamera] = useState(false);
  const [pasting, setPasting] = useState(false);
  const [link, setLink] = useState("");
  const [error, setError] = useState<AddError | null>(null);
  const paste = () => void relayReady().then(() => { const pair = decodeQr(link.trim()); setLink(""); onPair(pair); }).catch(() => setError("qrError"));
  if (camera) return <WebScanner t={t} onPair={(pair) => { setCamera(false); onPair(pair); }} onClose={() => setCamera(false)}
    onPaste={() => { setCamera(false); setPasting(true); }} />;
  return (
    <main className="phone-safe-top phone-safe-bottom flex min-h-full flex-col px-xl" data-phone-add-screen="true">
      <header className="-ml-sm flex items-center gap-xs pt-lg">
        <Button variant="ghost" size="icon" className="min-h-(--size-touch-target) min-w-(--size-touch-target) text-muted-foreground" aria-label={t("common.back")} data-phone-add-back="true" onClick={onBack}>
          <ChevronLeftIcon aria-hidden="true" className="size-(--size-icon-lg)" />
        </Button>
        <h1 className="text-headline font-semibold">hide</h1>
      </header>
      <h2 className="pt-lg text-headline font-semibold">{t("mobile.first.title")}</h2>
      <Steps t={t} />
      <div className="min-h-xl flex-1" />
      {full ? <p role="status" className="break-keep pb-md text-body text-muted-foreground wrap-anywhere" data-phone-add-full="true">{t("mobile.relay.limit")}</p> : null}
      <Button className="min-h-(--size-touch-target) gap-sm text-subhead" data-phone-scan="true" onClick={() => {
        setError(null);
        if (appHost()) void scanInApp(t, onPair, setError);
        else setCamera(true);
      }}>
        <QrCodeIcon aria-hidden="true" className="size-(--size-icon)" />{t("mobile.relay.scan")}
      </Button>
      {error ? <p role="alert" className="break-keep pt-sm text-center text-body text-warning wrap-anywhere">{t(`mobile.relay.${error}`)}</p> : null}
      {pasting ? (
        <form className="flex gap-sm pt-md pb-lg" onSubmit={(event) => { event.preventDefault(); setError(null); paste(); }}>
          <Input autoFocus aria-label={t("mobile.relay.paste")} placeholder={t("mobile.relay.paste")} value={link} autoComplete="off" maxLength={1024}
            className="h-(--size-touch-target) flex-1 text-subhead" onChange={(event) => setLink(event.target.value)} />
          <Button type="submit" variant="secondary" disabled={!link.trim()} className="min-h-(--size-touch-target) shrink-0 px-lg text-subhead" data-phone-paste-connect="true">{t("mobile.relay.connect")}</Button>
        </form>
      ) : (
        <Button variant="text" className="mt-xs mb-lg min-h-(--size-touch-target) gap-xs self-center text-body" data-phone-paste-open="true" onClick={() => setPasting(true)}>
          {t("mobile.relay.paste")}<ChevronRightIcon aria-hidden="true" className="size-(--size-icon)" />
        </Button>
      )}
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
        <p className={`break-keep py-sm text-body text-muted-foreground wrap-anywhere ${ROW_INSET}`}>{t("mobile.first.noComputers")}</p>
      </Section>
      <DisplaySection t={t} follows="phone" />
      <AboutSection t={t} />
    </main>
  );
}
