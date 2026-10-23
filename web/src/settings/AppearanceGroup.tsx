import { useEffect, useState, type ReactNode } from "react";
import type { Actions } from "../actions";
import { RadioGroup, RadioGroupItem } from "../components/ui/radio-group";
import { Slider } from "../components/ui/slider";
import { ToggleGroup, ToggleGroupItem } from "../components/ui/toggle-group";
import { Hint } from "../components/ui/tooltip";
import { Group, Note, Row, Value } from "../components/settings-rows";
import { ACCENT_STORED_HEX } from "../generated/accents";
import { useInterfaceTranslation } from "../i18n/client";
import { INTERFACE_LANGUAGES, LANGUAGE_NAMES, isInterfaceLanguage } from "../i18n/locale";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../components/ui/select";
import { ACCENT_CHOICES, FONT_SIZE_BASE, FONT_SIZE_MAX, FONT_SIZE_MIN, TERMINAL_FONT_SIZE_BASE, TERMINAL_FONT_SIZE_MAX, TERMINAL_FONT_SIZE_MIN, usableAccent, usableFontSize, usableTerminalFontSize } from "../settings";
import { commandLabel } from "../shortcutLabels";
import { useShellStore } from "../store";
import { COLOR_THEMES, THEME_CHOICES, accentNameOf, readColorTheme, readTheme, type AccentName, type ThemeChoice } from "../theme";
import { useErrorSince } from "./useErrorSince";

/** The displayed choice always comes from the core, never an optimistic edit. */
function LanguageRow({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const choice = useShellStore((state) => state.rest?.ui_state?.interface_language ?? null);
  const connected = useShellStore((state) => state.connection === "live");
  const [changedAt, setChangedAt] = useState<number | null>(null);
  const error = useErrorSince(changedAt, ["interface_language."]);
  return (
    <Row label={t("common.language")} data-appearance-row="language" detail={error ? <Note tone="error">{t("settings.notSaved", { reason: error })}</Note> : null}>
      <Select
        value={choice ?? "system"}
        disabled={!connected}
        onValueChange={(value) => {
          if (value !== "system" && !isInterfaceLanguage(value)) throw new Error("invalid_interface_language");
          const language = value === "system" ? null : value;
          if (language === choice) return;
          setChangedAt(Date.now());
          actions.setInterfaceLanguage(language);
        }}
      >
        <SelectTrigger aria-label={t("common.language")} data-interface-language={choice ?? "system"}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="system" data-language-option="system">
            {t("common.systemLanguage")}
          </SelectItem>
          {INTERFACE_LANGUAGES.map((language) => (
            <SelectItem key={language} value={language} data-language-option={language}>
              {LANGUAGE_NAMES[language]}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </Row>
  );
}

/** The swatch beside each color theme: its brand color, the same in Light and Dark (hide-color-theme D-15). */
const COLOR_THEME_SWATCH = { hide: "bg-swatch-hide", neutral: "bg-swatch-neutral" } as const;

/**
 * One text size slider: the value moves under the thumb while it drags and is
 * sent to the core once, on release.
 */
function TextSizeRow({ label, sliderLabel, row, marker, size, min, max, detail, onCommit }: {
  label: string;
  sliderLabel: string;
  row: string;
  /** The slider's own data attribute, the handle tests and other screens find it by. */
  marker: "data-font-size" | "data-terminal-font-size";
  size: number;
  min: number;
  max: number;
  detail?: ReactNode;
  onCommit: (size: number) => void;
}) {
  const { t } = useInterfaceTranslation();
  const [draft, setDraft] = useState(size);
  useEffect(() => setDraft(size), [size]);
  return (
    <Row label={label} data-appearance-row={row} detail={detail}>
      <Slider
        min={min}
        max={max}
        step={1}
        value={[draft]}
        aria-label={sliderLabel}
        {...{ [marker]: "true" }}
        className="w-(--size-settings-control-w)"
        onValueChange={([next]) => next !== undefined && setDraft(next)}
        onValueCommit={([next]) => next !== undefined && next !== size && onCommit(next)}
      />
      <Value>{t("settings.fontPoints", { size: draft })}</Value>
    </Row>
  );
}

/**
 * Language, color theme, mode, accent, interface text and terminal text, one
 * group (PRD settings-cleanup B2; hide-color-theme B1).
 */
export function AppearanceGroup({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const accent = useShellStore((s) => usableAccent(s.rest?.ui_state?.accent_hex));
  const theme = useShellStore((s) => readTheme(s.rest?.ui_state?.theme).choice);
  const colorTheme = useShellStore((s) => readColorTheme(s.rest?.ui_state?.color_theme).choice);
  const connected = useShellStore((s) => s.connection === "live");
  const fontSize = useShellStore((s) => usableFontSize(s.rest?.ui_state?.font_size)) ?? FONT_SIZE_BASE;
  const terminalFontSize = useShellStore((s) => usableTerminalFontSize(s.rest?.ui_state?.terminal_font_size)) ?? TERMINAL_FONT_SIZE_BASE;
  const textLarger = useShellStore((s) => commandLabel("text_larger", s.rest?.ui_state));
  const textSmaller = useShellStore((s) => commandLabel("text_smaller", s.rest?.ui_state));
  const [changedAt, setChangedAt] = useState<number | null>(null);
  const error = useErrorSince(changedAt, ["ui_state."]);
  const chosenAccent = accentNameOf(accent);
  return (
    <Group title={t("settings.appearance")} data-settings-group="appearance">
      <LanguageRow actions={actions} />
      <Row label={t("settings.colorTheme")} data-appearance-row="color-theme">
        <ToggleGroup
          type="single"
          value={colorTheme}
          aria-label={t("settings.colorTheme")}
          data-color-theme-choice={colorTheme}
          // A choice made while the socket reconnects would be dropped; wait for it, as Language does.
          disabled={!connected}
          onValueChange={(value) => {
            // A second press on the chosen item would clear it; a color theme is always chosen.
            if (!value || value === colorTheme) return;
            const choice = COLOR_THEMES.find((name) => name === value);
            if (!choice) throw new Error(`unknown color theme ${value}`);
            setChangedAt(Date.now());
            actions.setColorTheme(choice);
          }}
        >
          {COLOR_THEMES.map((choice) => (
            <ToggleGroupItem key={choice} value={choice} aria-label={t(`settings.colorTheme.${choice}`)} data-color-theme-option={choice}>
              <span aria-hidden="true" className={`size-(--size-icon-sm) rounded-xs ring-1 ring-border ${COLOR_THEME_SWATCH[choice]}`} />
              {t(`settings.colorTheme.${choice}`)}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
      </Row>
      <Row label={t("settings.theme")} data-appearance-row="mode">
        <ToggleGroup
          type="single"
          value={theme}
          aria-label={t("settings.theme")}
          data-theme-choice={theme}
          onValueChange={(value) => {
            // A second press on the chosen item would clear it; a theme is always chosen.
            if (!value || value === theme) return;
            setChangedAt(Date.now());
            actions.setTheme(value as ThemeChoice);
          }}
        >
          {THEME_CHOICES.map((choice) => (
            <ToggleGroupItem key={choice.id} value={choice.id} aria-label={t(`settings.theme.${choice.id}`)} data-theme-option={choice.id}>
              {t(`settings.theme.${choice.id}`)}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
      </Row>
      <Row label={t("settings.accent")} data-appearance-row="accent" detail={error ? <Note tone="error" data-appearance-error="true">{t("settings.notSaved", { reason: error })}</Note> : null}>
        <RadioGroup
          aria-label={t("settings.accent")}
          value={chosenAccent ?? ""}
          className="flex items-center gap-sm"
          onValueChange={(name) => {
            const hex = ACCENT_STORED_HEX[name as AccentName];
            setChangedAt(Date.now());
            actions.setAccent(hex.toUpperCase());
          }}
        >
          {ACCENT_CHOICES.map((choice) => (
            <Hint key={choice.id} label={t("settings.accentName", { name: t(`settings.accent.${choice.id}`) })}>
              <RadioGroupItem
                value={choice.id}
                data-accent={choice.id}
                className={`border-0 ${choice.swatch} ring-1 ring-border data-[state=checked]:ring-2 data-[state=checked]:ring-foreground [&_svg]:hidden`}
              />
            </Hint>
          ))}
        </RadioGroup>
      </Row>
      <TextSizeRow
        label={t("settings.interfaceText")}
        sliderLabel={t("settings.interfaceFontSize")}
        row="interface-text"
        marker="data-font-size"
        size={fontSize}
        min={FONT_SIZE_MIN}
        max={FONT_SIZE_MAX}
        onCommit={(size) => {
          setChangedAt(Date.now());
          actions.setFontSize(size);
        }}
      />
      <TextSizeRow
        label={t("settings.terminalText")}
        sliderLabel={t("settings.terminalFontSize")}
        row="terminal-text"
        marker="data-terminal-font-size"
        size={terminalFontSize}
        min={TERMINAL_FONT_SIZE_MIN}
        max={TERMINAL_FONT_SIZE_MAX}
        detail={<Note>{t("settings.terminalTextDescription", { larger: textLarger, smaller: textSmaller })}</Note>}
        onCommit={(size) => {
          setChangedAt(Date.now());
          actions.setTerminalFontSize(size);
        }}
      />
    </Group>
  );
}
