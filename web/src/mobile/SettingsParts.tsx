// The pieces the phone's Settings screen is built from, with no store in them: the Settings tab uses them inside a
// computer, and the start page (no computer yet) uses the same ones, so display and about look and behave alike.
// Each takes the translator of the page it is on.

import { ChevronRightIcon, ExternalLinkIcon } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import type { TFunction } from "i18next";
import { Button } from "../components/ui/button";
import { RadioGroup, RadioGroupItem } from "../components/ui/radio-group";
import { RowButton } from "../components/ui/row-button";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "../components/ui/sheet";
import { INTERFACE_LANGUAGES, LANGUAGE_NAMES, isInterfaceLanguage } from "../i18n/locale";
import type { ThemeChoice } from "../theme";
import { appInfo } from "./appBridge";
import { appHost } from "./appHost";
import { PHONE_PROTOCOL } from "./phoneVersion";
import { choosePhoneSettings, usePhoneSettings } from "./settings";

/** The pages the store listing and these links name (5.1.1(i)); their content is the site's, and both open outside the app. */
export const PRIVACY_POLICY_URL = "https://withhide.dev/privacy";
export const SUPPORT_URL = "https://withhide.dev/support";
const FOLLOW = "follow";
const THEMES = ["system", "light", "dark"] as const satisfies readonly ThemeChoice[];

export function Section({ title, children, ...marker }: { title: string; children: ReactNode } & Record<`data-${string}`, string>) {
  return (
    <section className="px-xl pt-lg" {...marker}>
      <h2 className="pb-xs text-body text-muted-foreground">{title}</h2>
      <div className="divide-y divide-border">{children}</div>
    </section>
  );
}

/** A row that tells a value and, when it is pressable, where it leads. */
export function ValueRow({ label, value, onPress, marker }: { label: string; value: ReactNode; onPress?: () => void; marker: string }) {
  const body = (
    <>
      <span className="min-w-0 flex-1 break-keep text-title text-foreground wrap-anywhere">{label}</span>
      <span className="flex min-w-0 items-center gap-xs text-subhead text-muted-foreground">{value}{onPress ? <ChevronRightIcon aria-hidden="true" className="size-(--size-icon) shrink-0" /> : null}</span>
    </>
  );
  const className = "min-h-(--size-touch-target) gap-md px-none py-sm";
  return onPress
    ? <RowButton className={className} data-phone-setting={marker} onClick={onPress}>{body}</RowButton>
    : <div className={`flex items-center ${className}`} data-phone-setting={marker}>{body}</div>;
}

/** A link row that opens the page in the system browser, never inside the app. */
function LinkRow({ label, href, marker }: { label: string; href: string; marker: string }) {
  return (
    <Button asChild variant="ghost" className="h-auto min-h-(--size-touch-target) w-full justify-between gap-md rounded-none px-none py-sm text-title font-normal text-foreground">
      <a href={href} target="_blank" rel="noopener noreferrer" data-phone-setting={marker}>
        <span>{label}</span>
        <ExternalLinkIcon aria-hidden="true" className="size-(--size-icon) shrink-0 text-muted-foreground" />
      </a>
    </Button>
  );
}

/** One choice as a row the whole width is the target of; the radio on the right marks the chosen one. */
function ChoiceRow({ value, name, children, lang, ...marker }: { value: string; name: string; children: string; lang?: string } & Record<`data-${string}`, string>) {
  const id = `phone-settings-${name}-${value}`;
  return (
    <label htmlFor={id} className="flex min-h-(--size-touch-target) items-center justify-between gap-md py-sm text-title text-foreground">
      <span lang={lang} className="min-w-0 break-keep wrap-anywhere">{children}</span>
      <RadioGroupItem id={id} value={value} {...marker} />
    </label>
  );
}

/** The sheet a value row opens: the choices of one setting, applied at once, closed by a press outside or on the grabber. */
function ChoiceSheet({ t, title, open, onOpenChange, returnTo, children }: { t: TFunction; title: string; open: boolean; onOpenChange: (open: boolean) => void; returnTo: () => HTMLElement | null; children: ReactNode }) {
  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent side="bottom" returnFocusTo={returnTo} className="phone-safe-bottom max-h-dvh overflow-y-auto rounded-t-xl px-lg" data-phone-settings-sheet="true">
        <SheetHeader className="px-sm py-sm">
          <SheetTitle className="text-headline">{title}</SheetTitle>
        </SheetHeader>
        <SheetDescription className="sr-only">{t("mobile.settings.description")}</SheetDescription>
        <div className="px-sm pb-lg">{children}</div>
      </SheetContent>
    </Sheet>
  );
}

function useAppVersion(): string | null {
  const [version, setVersion] = useState<string | null>(null);
  useEffect(() => {
    if (!appHost()) return;
    let current = true;
    void appInfo().then(({ version: short, build }) => { if (current) setVersion(build ? `${short} (${build})` : short); }, () => undefined);
    return () => { current = false; };
  }, []);
  return version;
}

/** Language and appearance as one-line values that open a sheet; a choice applies at once and stays on this phone. */
export function DisplaySection({ t, openSheet = null }: { t: TFunction; /** The gallery's: the sheet already open. */ openSheet?: "language" | "theme" | null }) {
  const language = usePhoneSettings((state) => state.language);
  const theme = usePhoneSettings((state) => state.theme);
  const kept = usePhoneSettings((state) => state.kept);
  const [sheet, setSheet] = useState<"language" | "theme" | null>(openSheet);
  const languageRow = useRef<HTMLElement | null>(null);
  const themeRow = useRef<HTMLElement | null>(null);
  const chosenLanguage = language ?? FOLLOW;
  return (
    <>
      <Section title={t("mobile.settings.display")} data-phone-settings-section="display">
        <div ref={(node) => { languageRow.current = node; }}>
          <ValueRow label={t("common.language")} marker="language" value={language ? LANGUAGE_NAMES[language] : t("mobile.settings.followMac")} onPress={() => setSheet("language")} />
        </div>
        <div ref={(node) => { themeRow.current = node; }}>
          <ValueRow label={t("mobile.settings.appearance")} marker="theme" value={t(`mobile.settings.${theme}`)} onPress={() => setSheet("theme")} />
        </div>
        <p className="py-sm text-body text-muted-foreground" data-phone-settings-note={kept ? "kept" : "not-kept"} role={kept ? undefined : "status"}>
          {t(kept ? "mobile.settings.note" : "mobile.settings.notKept")}
        </p>
      </Section>
    <ChoiceSheet t={t} title={t("common.language")} open={sheet === "language"} onOpenChange={(open) => setSheet(open ? "language" : null)} returnTo={() => languageRow.current?.querySelector("button") ?? null}>
      <RadioGroup
        value={chosenLanguage}
        aria-label={t("common.language")}
        data-phone-settings-language={chosenLanguage}
        className="gap-none"
        onValueChange={(value) => {
          if (value === FOLLOW) choosePhoneSettings({ language: null });
          else if (isInterfaceLanguage(value)) choosePhoneSettings({ language: value });
          setSheet(null);
        }}
      >
        <ChoiceRow name="language" value={FOLLOW} data-phone-language-option={FOLLOW}>{t("mobile.settings.followMac")}</ChoiceRow>
        {INTERFACE_LANGUAGES.map((code) => (
          <ChoiceRow key={code} name="language" value={code} lang={code} data-phone-language-option={code}>{LANGUAGE_NAMES[code]}</ChoiceRow>
        ))}
      </RadioGroup>
    </ChoiceSheet>
    <ChoiceSheet t={t} title={t("mobile.settings.appearance")} open={sheet === "theme"} onOpenChange={(open) => setSheet(open ? "theme" : null)} returnTo={() => themeRow.current?.querySelector("button") ?? null}>
      <RadioGroup
        value={theme}
        aria-label={t("mobile.settings.appearance")}
        data-phone-settings-theme={theme}
        className="gap-none"
        onValueChange={(value) => {
          const choice = THEMES.find((candidate) => candidate === value);
          if (choice) choosePhoneSettings({ theme: choice });
          setSheet(null);
        }}
      >
        {THEMES.map((choice) => (
          <ChoiceRow key={choice} name="theme" value={choice} data-phone-theme-option={choice}>{t(`mobile.settings.${choice}`)}</ChoiceRow>
        ))}
      </RadioGroup>
    </ChoiceSheet>
    </>
  );
}

/** The privacy policy and support, which open outside the app, then the app's version and the phone protocol. */
export function AboutSection({ t }: { t: TFunction }) {
  const appVersion = useAppVersion();
  return (
    <Section title={t("mobile.settings.about")} data-phone-settings-section="about">
      <LinkRow label={t("mobile.settings.privacy")} href={PRIVACY_POLICY_URL} marker="privacy" />
      <LinkRow label={t("mobile.settings.support")} href={SUPPORT_URL} marker="support" />
      {appHost() && appVersion ? <ValueRow label={t("mobile.settings.appVersion")} value={<span className="font-mono">{appVersion}</span>} marker="app-version" /> : null}
      <ValueRow label={t("mobile.settings.protocol")} value={<span className="font-mono">{PHONE_PROTOCOL}</span>} marker="protocol" />
    </Section>
  );
}
