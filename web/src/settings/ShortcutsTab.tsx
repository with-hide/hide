import { RotateCcwIcon, XIcon } from "lucide-react";
import { useEffect, useState, type KeyboardEvent } from "react";
import type { Actions } from "../actions";
import { Button } from "../components/ui/button";
import { Kbd } from "../components/ui/kbd";
import { Hint } from "../components/ui/tooltip";
import { Disclosure, Group, Note, Row, Status } from "../components/settings-rows";
import { commandTitle } from "../commandTitle";
import { hostKind, keySystem, type HostKind } from "../host";
import { useInterfaceTranslation } from "../i18n/client";
import {
  AREA_COMMANDS,
  EDITABLE_PANE_COMMANDS,
  bindingProblem,
  chordEquals,
  chordFromEvent,
  defaultChord,
  displayChord,
  familyModifiers,
  hostChord,
  modifierLabel,
  numberedCommand,
  resolvedRegistry,
  serializeStoredChord,
  sheetRows,
  storedBindings,
  storedKey,
  type BindingProblem,
  type Chord,
  type CommandId,
  type KeySystem,
} from "../shortcuts";
import { bindingProblemText, sheetRowTitle } from "../shortcutLabels";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";
import { coreErrorText } from "../coreError";
import { useRefusalSince } from "../errorSince";

export function ShortcutsTab({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  // Each host edits its own set: the desktop app the macOS set, the browser
  // its own (user decision 2026-09-26).
  const host = hostKind();
  const system = keySystem();
  const stored = useShellStore((s) => storedBindings(s.rest?.ui_state, host));
  const { registry, diagnostic } = resolvedRegistry(stored, host, system);
  const [sentAt, setSentAt] = useState<number | null>(null);
  const [sent, setSent] = useState<Record<string, string> | null>(null);
  const saveError = useRefusalSince(sentAt, ["ui_state."]);
  const saving = sent !== null && JSON.stringify(sent) !== JSON.stringify(stored ?? {}) && saveError === null;
  const apply = (bindings: Record<string, string>) => {
    setSent(bindings);
    setSentAt(Date.now());
    actions.setPaneShortcuts(host, bindings);
  };
  const current = (): Record<string, string> => Object.fromEntries(Object.entries(diagnostic && !diagnostic.includes("retired and was ignored") ? {} : stored ?? {}).filter(([key]) => !["project_home", "toggle_sidebar_view"].includes(key)));
  const rowsFor = (ids: readonly CommandId[]) =>
    ids.map((id) => (
      <ShortcutRow
        key={id}
        id={id}
        host={host}
        system={system}
        registry={registry}
        overridden={!diagnostic && stored?.[storedKey(id, host)] !== undefined}
        onApply={(chord) => {
          const next = current();
          const fallback = defaultChord(id, host, system);
          const text = serializeStoredChord(chord, host, system);
          if ((fallback && chordEquals(fallback, chord)) || text === null) delete next[storedKey(id, host)];
          else next[storedKey(id, host)] = text;
          apply(next);
        }}
        onClear={() => {
          const next = current();
          next[storedKey(id, host)] = "none";
          apply(next);
        }}
        onReset={() => {
          const next = current();
          delete next[storedKey(id, host)];
          apply(next);
        }}
      />
    ));
  // The eight area commands have no chord until the operator sets one, so
  // they fold into one line that says how many still have none (design 8, 9).
  const areaUnset = AREA_COMMANDS.filter((id) => {
    const command = registry.find((row) => row.id === id);
    return !command || hostChord(command, host) === null;
  }).length;
  return (
    <>
      <Group title={t("settings.shortcuts.paneAndNavigation")}>
        {rowsFor(EDITABLE_PANE_COMMANDS.filter((id) => !(AREA_COMMANDS as readonly CommandId[]).includes(id)))}
        <Disclosure
          title={t("settings.shortcuts.areaTitle")}
          summary={areaUnset > 0 ? t("settings.shortcuts.areaUnset", { count: areaUnset }) : t("settings.shortcuts.areaAllSet")}
          data-settings-fold="area-commands"
        >
          {rowsFor(AREA_COMMANDS)}
        </Disclosure>
      </Group>
      <Group title={t("settings.shortcuts.fixedTitle")} data-settings-group="numbered-chords">
        {sheetRows("Tabs", registry, host, system)
          .concat(sheetRows("Navigate", registry, host, system))
          .filter((row) => row.id.startsWith("select_"))
          .map((row) => {
            // A browser host has no numbered chords, so there is nothing to hold.
            const modifiers = row.chord === null ? null : familyModifiers(numberedCommand(row.id)!.family, registry, host);
            return (
              <Row
                key={row.id}
                label={
                  <span className="flex min-w-0 flex-col">
                    <span>{sheetRowTitle(row, t)}</span>
                    {modifiers ? <span className="text-caption text-muted-foreground">{t("settings.shortcuts.holdToSee", { modifier: modifierLabel(modifiers, system) })}</span> : null}
                  </span>
                }
              >
                <Kbd data-shortcut-effective={row.id}>{row.chord ?? "-"}</Kbd>
                {row.chord === null ? <Status tone="muted">{t("settings.shortcuts.notOnHost")}</Status> : null}
              </Row>
            );
          })}
      </Group>
      {diagnostic ? <Note tone="warn" data-shortcut-diagnostic="true">{diagnostic}</Note> : null}
      {saving ? <Note tone="pending">{t("workspace.saving")}</Note> : null}
      {saveError ? <Note tone="error" data-shortcut-save-error="true" data-core-error={saveError.kind}>{coreErrorText(saveError, t, "coreError.settings.notSaved")}</Note> : null}
      <div className="mt-sm flex justify-end">
        <Button variant="secondary" disabled={!stored || Object.keys(stored).length === 0} onClick={() => apply({})} data-shortcut-reset-all="true">
          {t("settings.shortcuts.restoreDefaults")}
        </Button>
      </div>
    </>
  );
}

// The restore and clear controls show on hover or keyboard focus of their row,
// and always where there is no hover (B61); they stay in the layout either
// way, so the chip does not move when they appear.
const ROW_CONTROL = "opacity-0 group-hover/shortcut:opacity-100 group-focus-within/shortcut:opacity-100 [@media(hover:none)]:opacity-100";

/**
 * One editable command: its chord is the control. Pressing it (or Enter on it)
 * waits for the next chord, which applies at once; Escape cancels (B61).
 */
function ShortcutRow({
  id,
  host,
  system,
  registry,
  overridden,
  onApply,
  onReset,
  onClear,
}: {
  id: CommandId;
  host: HostKind;
  system: KeySystem;
  registry: ReturnType<typeof resolvedRegistry>["registry"];
  overridden: boolean;
  onApply: (chord: Chord) => void;
  onReset: () => void;
  onClear: () => void;
}) {
  const { t } = useInterfaceTranslation();
  const command = registry.find((row) => row.id === id);
  const [recording, setRecording] = useState(false);
  const [problem, setProblem] = useState<BindingProblem | "altgr" | null>(null);
  const setRecordingFlag = useUiStore((s) => s.setRecordingShortcut);
  useEffect(() => {
    if (!recording) return;
    setRecordingFlag(true);
    return () => setRecordingFlag(false);
  }, [recording, setRecordingFlag]);
  if (!command) return null;
  const title = commandTitle(id, t);
  const effective = hostChord(command, host);
  const chordText = effective ? displayChord(effective, system) : null;
  const record = (event: KeyboardEvent<HTMLButtonElement>) => {
    // IME composition and lone modifiers are not chords; the recorder waits.
    if (event.nativeEvent.isComposing || event.keyCode === 229) return;
    if (["Meta", "Alt", "Shift", "Control", "CapsLock"].includes(event.key)) return;
    event.preventDefault();
    event.stopPropagation();
    if (event.key === "Escape" && !event.metaKey && !event.altKey && !event.ctrlKey) {
      setRecording(false);
      setProblem(null);
      return;
    }
    const chord = chordFromEvent(event.nativeEvent);
    // AltGr types a character on Windows and Linux layouts (Windows reports it
    // as Ctrl+Alt), so a key pressed with it is never a chord, as the window
    // listener already treats it.
    const reason: BindingProblem | "altgr" | null =
      system === "pc" && event.nativeEvent.getModifierState?.("AltGraph") ? "altgr" : bindingProblem(id, chord, registry, host, system);
    setRecording(false);
    setProblem(reason);
    if (!reason) onApply(chord);
  };
  return (
    <Row
      className="group/shortcut"
      label={title}
      detail={
        problem ? (
          <Note tone="error" data-shortcut-problem={id}>
            {problem === "altgr" ? t("settings.shortcuts.altGr") : bindingProblemText(problem, t)} {chordText ? t("settings.shortcuts.previousChord", { chord: chordText }) : ""}
          </Note>
        ) : null
      }
    >
      <Button
        variant={recording ? "default" : "secondary"}
        size="sm"
        aria-label={recording ? t("settings.shortcuts.recordAria", { command: title }) : chordText ? t("settings.shortcuts.changeAria", { command: title, chord: chordText }) : t("settings.shortcuts.setAria", { command: title })}
        onKeyDown={recording ? record : undefined}
        onBlur={() => setRecording(false)}
        onClick={() => {
          setProblem(null);
          setRecording(true);
        }}
        data-shortcut-record={id}
      >
        {recording ? t("settings.shortcuts.pressChord") : <span data-shortcut-effective={id}>{chordText ?? "-"}</span>}
      </Button>
      {overridden ? (
        <Hint label={t("settings.shortcuts.reset")}>
          <Button variant="ghost" size="icon-sm" className={ROW_CONTROL} aria-label={t("settings.shortcuts.resetAria", { command: title })} onClick={onReset} data-shortcut-reset={id}>
            <RotateCcwIcon />
          </Button>
        </Hint>
      ) : (
        <span aria-hidden="true" className="size-(--size-control-sm)" />
      )}
      {effective ? (
        <Hint label={t("settings.shortcuts.clear")}>
          <Button variant="ghost" size="icon-sm" className={ROW_CONTROL} aria-label={t("settings.shortcuts.clearAria", { command: title })} onClick={onClear} data-shortcut-clear={id}>
            <XIcon />
          </Button>
        </Hint>
      ) : (
        <span aria-hidden="true" className="size-(--size-control-sm)" />
      )}
    </Row>
  );
}
