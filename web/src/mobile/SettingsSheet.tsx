// The phone's settings sheet (mission-34 node settings): the language and
// light or dark this phone shows, one tap each, applied at once and kept on
// this phone only (settings.ts). It sits behind the gear in the list's header,
// beside Start agent, and opens as the start sheet does.

import { useRef, useState } from "react";
import { SettingsIcon, XIcon } from "lucide-react";
import { Button } from "../components/ui/button";
import { RadioGroup, RadioGroupItem } from "../components/ui/radio-group";
import { Sheet, SheetClose, SheetContent, SheetDescription, SheetHeader, SheetTitle, SheetTrigger } from "../components/ui/sheet";
import { useInterfaceTranslation } from "../i18n/translator";
import { INTERFACE_LANGUAGES, LANGUAGE_NAMES, isInterfaceLanguage } from "../i18n/locale";
import type { ThemeChoice } from "../theme";
import { choosePhoneSettings, usePhoneSettings } from "./settings";

/** The page the store listing and this link both name (5.1.1(i)); its content is the site's. */
export const PRIVACY_POLICY_URL = "https://withhide.dev/privacy";
const FOLLOW = "follow";
const THEMES = ["system", "light", "dark"] as const satisfies readonly ThemeChoice[];

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

export function SettingsSheet({ defaultOpen = false }: { defaultOpen?: boolean }) {
  const { t } = useInterfaceTranslation();
  const language = usePhoneSettings((state) => state.language);
  const theme = usePhoneSettings((state) => state.theme);
  const kept = usePhoneSettings((state) => state.kept);
  const [open, setOpen] = useState(defaultOpen);
  const trigger = useRef<HTMLButtonElement>(null);
  return (
    <Sheet open={open} onOpenChange={setOpen}>
      <SheetTrigger asChild>
        <Button ref={trigger} variant="ghost" size="icon" aria-label={t("mobile.settings.title")} data-phone-settings="true" className="size-(--size-touch-target) rounded-lg text-foreground active:bg-accent">
          <SettingsIcon aria-hidden="true" className="size-(--size-icon-lg)" />
        </Button>
      </SheetTrigger>
      <SheetContent side="bottom" returnFocusTo={() => trigger.current} className="phone-safe-bottom max-h-dvh overflow-y-auto rounded-t-xl px-lg" data-phone-settings-sheet="true">
        <SheetHeader className="flex-row items-center justify-between px-sm py-sm">
          <SheetTitle className="text-headline">{t("mobile.settings.title")}</SheetTitle>
          <SheetClose asChild><Button variant="ghost" size="icon" className="size-(--size-touch-target)" aria-label={t("common.close")}><XIcon aria-hidden="true" /></Button></SheetClose>
        </SheetHeader>
        <SheetDescription className="sr-only">{t("mobile.settings.description")}</SheetDescription>
        <div className="flex flex-col gap-lg px-sm pb-lg">
          <section aria-labelledby="phone-settings-language">
            <h3 id="phone-settings-language" className="pb-xs text-body text-muted-foreground">{t("common.language")}</h3>
            <RadioGroup
              value={language ?? FOLLOW}
              aria-labelledby="phone-settings-language"
              data-phone-settings-language={language ?? FOLLOW}
              className="gap-none"
              onValueChange={(value) => {
                if (value === FOLLOW) choosePhoneSettings({ language: null });
                else if (isInterfaceLanguage(value)) choosePhoneSettings({ language: value });
              }}
            >
              <ChoiceRow name="language" value={FOLLOW} data-phone-language-option={FOLLOW}>{t("mobile.settings.followMac")}</ChoiceRow>
              {INTERFACE_LANGUAGES.map((code) => (
                <ChoiceRow key={code} name="language" value={code} lang={code} data-phone-language-option={code}>{LANGUAGE_NAMES[code]}</ChoiceRow>
              ))}
            </RadioGroup>
          </section>
          <section aria-labelledby="phone-settings-theme">
            <h3 id="phone-settings-theme" className="pb-xs text-body text-muted-foreground">{t("mobile.settings.appearance")}</h3>
            <RadioGroup
              value={theme}
              aria-labelledby="phone-settings-theme"
              data-phone-settings-theme={theme}
              className="gap-none"
              onValueChange={(value) => {
                const choice = THEMES.find((candidate) => candidate === value);
                if (choice) choosePhoneSettings({ theme: choice });
              }}
            >
              {THEMES.map((choice) => (
                <ChoiceRow key={choice} name="theme" value={choice} data-phone-theme-option={choice}>{t(`mobile.settings.${choice}`)}</ChoiceRow>
              ))}
            </RadioGroup>
          </section>
          <Button asChild variant="text-link" className="min-h-(--size-touch-target) justify-start px-none text-title text-foreground">
            <a href={PRIVACY_POLICY_URL} target="_blank" rel="noopener noreferrer" data-phone-privacy-link="true">{t("mobile.settings.privacy")}</a>
          </Button>
          <p className="text-body text-muted-foreground" data-phone-settings-note={kept ? "kept" : "not-kept"} role={kept ? undefined : "status"}>
            {t(kept ? "mobile.settings.note" : "mobile.settings.notKept")}
          </p>
        </div>
      </SheetContent>
    </Sheet>
  );
}
