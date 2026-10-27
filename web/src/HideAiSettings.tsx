import { BrainIcon, ChevronRightIcon, SettingsIcon } from "lucide-react";
import { useState } from "react";
import type { Actions } from "./actions";
import { AgentLogo } from "./components/agent-logo";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "./components/ui/alert-dialog";
import { Button } from "./components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "./components/ui/popover";
import { RadioGroup, RadioGroupItem } from "./components/ui/radio-group";
import { Switch } from "./components/ui/switch";
import { Hint } from "./components/ui/tooltip";
import { useInterfaceTranslation } from "./i18n/client";
import type { MessageKey } from "./i18n/catalogs";
import { cn } from "./lib/utils";
import { catalogWorkspaces, type ProjectSuggestions, type SuggestionApplyMode, type SuggestionQuantity } from "./snapshot";
import { useShellStore } from "./store";
import { suggestionTitle } from "./suggestionList";

const QUANTITIES: readonly SuggestionQuantity[] = ["few", "normal", "many"];
const APPLY_MODES: readonly SuggestionApplyMode[] = ["confirm", "auto_pr"];
const SIGNALS_ON_GITHUB = new Set(["check_failed", "change_request"]);

/**
 * The Hide AI tool's ⚙ (PRD hide-ai-suggestions D-26, D-27, B5, B13, B16,
 * B19): the AI that investigates, which Settings › Hide AI chooses; the
 * 제안 group (its own switch, how many, how fixes are applied, the signals,
 * the lines set to 안 함); the Memory group (its own switch and deleting its
 * memories); and one line on what is sent. With the head switch off every
 * switch here is dimmed: nothing is read either way.
 */
export function HideAiSettings({ project, actions }: { project: ProjectSuggestions; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const [open, setOpen] = useState(false);
  const id = project.workspace_id;
  const label = useShellStore((state) => catalogWorkspaces(state.rest).find((row) => row.id === id)?.label ?? id);
  const settings = project.settings;
  const off = !project.enabled;
  const suggestionsOff = off || !settings.suggestions_enabled;
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <Hint label={t("hideAiTool.settings")}>
        <PopoverTrigger asChild>
          <Button variant="ghost" size="icon-sm" className="text-muted-foreground" aria-label={t("hideAiTool.settings")} aria-pressed={open} data-hide-ai-settings-open={id}>
            <SettingsIcon />
          </Button>
        </PopoverTrigger>
      </Hint>
      <PopoverContent align="end" className="flex max-h-(--radix-popover-content-available-height) flex-col gap-md overflow-auto p-md" aria-label={t("hideAiTool.settings")} data-hide-ai-settings={id}>
        <h4 className="flex min-w-0 items-center gap-xs text-body font-semibold">
          <BrainIcon aria-hidden="true" className="size-(--size-icon) shrink-0" />
          <span className="shrink-0">{t("hideAiTool.title")}</span>
          <span className="min-w-0 truncate font-normal text-subtle-foreground">· {label}</span>
        </h4>
        <Provider project={project} actions={actions} onLeave={() => setOpen(false)} />
        <section className={cn("flex flex-col gap-sm border-t border-border pt-md", off && "opacity-(--opacity-dimmed)")} data-hide-ai-settings-group="suggestions">
          <GroupSwitch name={t("suggestions.settings.suggestions")} note={t("suggestions.settings.suggestionsNote")} checked={settings.suggestions_enabled} disabled={off} onChange={(on) => actions.suggestionSettings(id, { suggestions_enabled: on })} data="suggestions" />
          <Choice
            name={t("suggestions.settings.quantity")}
            value={settings.quantity}
            options={QUANTITIES.map((quantity) => ({ value: quantity, label: t(`suggestions.settings.quantity.${quantity}` as MessageKey), note: t(`suggestions.settings.quantity.${quantity}.note` as MessageKey) }))}
            disabled={suggestionsOff}
            onChange={(quantity) => actions.suggestionSettings(id, { quantity: quantity as SuggestionQuantity })}
            data="quantity"
          />
          <Choice
            name={t("suggestions.settings.applyMode")}
            value={settings.apply_mode}
            options={APPLY_MODES.map((mode) => ({ value: mode, label: t(`suggestions.settings.apply.${mode}` as MessageKey), note: t(`suggestions.settings.apply.${mode}.note` as MessageKey) }))}
            disabled={suggestionsOff}
            onChange={(mode) => actions.suggestionSettings(id, { apply_mode: mode as SuggestionApplyMode })}
            data="apply_mode"
          />
          <div className="flex flex-col gap-xs" data-hide-ai-signals="true">
            <span className="text-micro text-muted-foreground">{t("suggestions.settings.signals")}</span>
            {settings.signals.map((row) => {
              const switchId = `hide-ai-signal-${id}-${row.signal}`;
              return (
                <div key={row.signal} className="flex min-w-0 items-center gap-sm" data-hide-ai-signal={row.signal} data-hide-ai-signal-unavailable={row.unavailable ?? undefined}>
                  <label htmlFor={switchId} className="flex min-w-0 flex-1 flex-col">
                    <span className="flex items-center gap-xs text-caption text-foreground">
                      {t(`suggestions.signal.${row.signal}` as MessageKey)}
                      {SIGNALS_ON_GITHUB.has(row.signal) ? <span className="rounded-xs border border-border px-xxs text-micro text-muted-foreground">GitHub</span> : null}
                    </span>
                    {row.unavailable ? <span className="text-micro text-muted-foreground">{t(`suggestions.signalUnavailable.${row.unavailable}` as MessageKey)}</span> : null}
                  </label>
                  <Switch id={switchId} checked={row.enabled && row.unavailable === null} disabled={suggestionsOff || row.unavailable !== null} onCheckedChange={(on) => actions.suggestionSettings(id, { signal: { signal: row.signal, enabled: on } })} />
                </div>
              );
            })}
          </div>
          <Dismissed project={project} actions={actions} disabled={suggestionsOff} />
        </section>
        <MemoryGroup project={project} actions={actions} />
        <p className="border-t border-border pt-sm text-caption text-muted-foreground" data-hide-ai-sent="true">{t("hideAiTool.sent")}</p>
      </PopoverContent>
    </Popover>
  );
}

/** The AI chip (D-27, B19): the agent Settings › Hide AI asks now; a press opens that page. OpenCode cannot promise to only read, so investigating does not run on it. */
function Provider({ project, actions, onLeave }: { project: ProjectSuggestions; actions: Actions; onLeave: () => void }) {
  const { t } = useInterfaceTranslation();
  const ai = useShellStore((state) => state.rest?.status?.background_ai ?? null);
  const answering = ai?.refusal?.using ?? ai?.provider ?? null;
  const provider = answering ? ai?.providers.find((row) => row.id === answering) ?? null : null;
  return (
    <div className="flex min-w-0 flex-col gap-xxs" data-hide-ai-provider={project.ai}>
      <div className="flex min-w-0 items-center gap-sm">
        <span className="min-w-0 flex-1 text-caption text-subtle-foreground">{t("suggestions.settings.ai")}</span>
        <Button size="sm" variant="secondary" className="min-w-0" onClick={() => { onLeave(); actions.openSettings("hideAi"); }} data-hide-ai-provider-chip={provider?.agent ?? "none"}>
          {provider ? <AgentLogo agent={provider.agent} label={provider.label} /> : null}
          <span className="truncate">{provider ? provider.label : t("suggestions.settings.aiNone")}</span>
          <ChevronRightIcon aria-hidden="true" />
        </Button>
      </div>
      {project.ai === "cannot_investigate" ? <span className="self-end text-caption text-warning" data-hide-ai-cannot-investigate="true">{t("suggestions.settings.cannotInvestigate")}</span> : null}
      {project.ai === "not_ready" ? <span className="self-end text-caption text-muted-foreground">{t("suggestions.settings.aiNotReady")}</span> : null}
    </div>
  );
}

function GroupSwitch({ name, note, checked, disabled, onChange, data }: { name: string; note: string; checked: boolean; disabled: boolean; onChange: (on: boolean) => void; data: string }) {
  const switchId = `hide-ai-group-${data}`;
  return (
    <div className="flex min-w-0 items-center gap-sm">
      <label htmlFor={switchId} className="flex min-w-0 flex-1 flex-col">
        <span className="text-body font-medium text-foreground">{name}</span>
        <span className="break-keep text-micro text-muted-foreground">{note}</span>
      </label>
      <Switch id={switchId} checked={checked} disabled={disabled} onCheckedChange={onChange} data-hide-ai-group-switch={data} />
    </div>
  );
}

/** A choice of a few levels, each with what it means; the core keeps the value. */
function Choice({ name, value, options, disabled, onChange, data }: { name: string; value: string; options: { value: string; label: string; note: string }[]; disabled: boolean; onChange: (value: string) => void; data: string }) {
  return (
    <div className="flex flex-col gap-xs" data-hide-ai-choice={data} data-hide-ai-choice-value={value}>
      <span className="text-micro text-muted-foreground">{name}</span>
      <RadioGroup value={value} disabled={disabled} onValueChange={onChange} aria-label={name} className="gap-xxs">
        {options.map((option) => {
          const itemId = `hide-ai-${data}-${option.value}`;
          return (
            <label key={option.value} htmlFor={itemId} className={cn("grid cursor-pointer grid-cols-[auto_1fr] items-start gap-x-sm rounded-sm px-xs py-xs", value === option.value && "bg-secondary")}>
              <RadioGroupItem id={itemId} value={option.value} className="mt-xxs" />
              <span className="flex min-w-0 flex-col">
                <span className="text-caption text-foreground">{option.label}</span>
                <span className="break-keep text-micro text-muted-foreground">{option.note}</span>
              </span>
            </label>
          );
        })}
      </RadioGroup>
    </div>
  );
}

/** `'안 함'으로 둔 제안 N` (D-43, B16): each with `되살리기`, which lets the core judge it again on what has piled up. */
function Dismissed({ project, actions, disabled }: { project: ProjectSuggestions; actions: Actions; disabled: boolean }) {
  const { t } = useInterfaceTranslation();
  const [open, setOpen] = useState(false);
  const dismissed = project.settings.dismissed;
  if (dismissed.length === 0) return null;
  return (
    <div className="flex flex-col gap-xs" data-hide-ai-dismissed={dismissed.length}>
      <button type="button" aria-expanded={open} className="flex items-center gap-xs self-start text-caption text-subtle-foreground underline-offset-4 hover:text-foreground hover:underline" onClick={() => setOpen(!open)}>
        <ChevronRightIcon aria-hidden="true" className={cn("size-(--size-icon-sm)", open && "rotate-90")} />
        {t("suggestions.settings.dismissed", { count: dismissed.length })}
      </button>
      {open ? dismissed.map((row) => (
        <div key={row.id} className="flex min-w-0 items-center gap-sm pl-md" data-hide-ai-dismissed-row={row.id}>
          <span className="min-w-0 flex-1 truncate text-caption text-foreground" title={suggestionTitle(t, row)}>{suggestionTitle(t, row)}</span>
          <Button size="sm" variant="ghost" disabled={disabled} onClick={() => actions.suggestionAction(project.workspace_id, row.id, "restore")} data-hide-ai-restore={row.id}>{t("suggestions.settings.restore")}</Button>
        </div>
      )) : null}
    </div>
  );
}

/** The Memory group: Memory alone on or off (`memory_action`), and deleting this Project's memories, which asks first because it cannot be undone. */
function MemoryGroup({ project, actions }: { project: ProjectSuggestions; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const memory = useShellStore((state) => (state.memory?.project?.workspace_id === project.workspace_id ? state.memory.project : null));
  const label = useShellStore((state) => catalogWorkspaces(state.rest).find((row) => row.id === project.workspace_id)?.label ?? project.workspace_id);
  const [deleting, setDeleting] = useState(false);
  const off = !project.enabled;
  const busy = !memory || memory.loading || memory.pending_action !== null || memory.unavailable !== null;
  const count = memory?.rows.length ?? 0;
  return (
    <section className={cn("flex flex-col gap-sm border-t border-border pt-md", off && "opacity-(--opacity-dimmed)")} data-hide-ai-settings-group="memory">
      <GroupSwitch name={t("suggestions.settings.memory")} note={t("suggestions.settings.memoryNote")} checked={memory?.enabled ?? false} disabled={off || busy} onChange={(on) => actions.memoryAction(project.workspace_id, on ? "enable" : "disable")} data="memory" />
      {count > 0 ? (
        <button type="button" disabled={busy} className="self-start text-caption text-subtle-foreground underline-offset-4 hover:text-foreground hover:underline disabled:opacity-(--opacity-disabled)" onClick={() => setDeleting(true)} data-memory-delete="settings">
          {t("suggestions.settings.deleteMemory")}
        </button>
      ) : null}
      <AlertDialog open={deleting} onOpenChange={setDeleting}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("memory.delete.title", { project: label })}</AlertDialogTitle>
            <AlertDialogDescription>{t("memory.delete.body", { count })}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("memory.delete.keep")}</AlertDialogCancel>
            <AlertDialogAction onClick={() => actions.memoryAction(project.workspace_id, "delete")} data-memory-delete="confirm">
              {t("memory.delete.confirm", { count })}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  );
}
