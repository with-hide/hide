// The start sheet's two lines of words (docs/UI_BEHAVIOR.md, The phone app),
// drawn as the desktop Start panel draws them: where over the text (the
// device, the project `›` the branch) and who under it (the agent with its
// mark, the model). Each word opens its own list in a sheet over the start
// sheet; the project and branch lists have a search field, as the panel's do.

import { useRef, useState, type ReactNode } from "react";
import { CheckIcon, ChevronDownIcon, FolderIcon, GitBranchIcon, LaptopIcon, ServerIcon } from "lucide-react";
import type { TFunction } from "i18next";
import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import { Command, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList } from "../components/ui/command";
import { Sheet, SheetContent, SheetDescription, SheetTitle } from "../components/ui/sheet";
import { AgentLogo } from "../components/agent-logo";
import { useInterfaceTranslation } from "../i18n/translator";
import { cn } from "../lib/utils";
import { systemMachineKey } from "../screenMachine";
import { takesModel } from "../agentPicker";
import type { StartCatalog } from "./protocol";
import { START_KINDS, modelsOf, startDevices, type StartChoice, type StartDevice, type StartSelection } from "./start";

type List = "project" | "branch" | "kind" | "model";

/** The list's own name for the CLI's default, which no model is called. */
const DEFAULT_MODEL = "__cli_default";

/** A device's name as the lists say it: a registered device's own, the core's machine as its system calls it. */
function deviceName(t: TFunction<"translation">, label: string | null, catalog: StartCatalog | null): string {
  return label ?? t(systemMachineKey(catalog?.core_platform ?? ""));
}

/** One word of a line: a quiet press as tall as a finger, its name cut to one line. */
function Word({ icon, children, onPress, disabled, ...marker }: { icon?: ReactNode; children: ReactNode; onPress: () => void; disabled?: boolean } & Record<`data-${string}`, string>) {
  return (
    <Button variant="ghost" disabled={disabled} onClick={onPress} className="h-(--size-touch-target) min-w-0 max-w-full shrink gap-xs px-sm text-title font-normal text-foreground" {...marker}>
      {icon}
      <span className="min-w-0 truncate">{children}</span>
      <ChevronDownIcon aria-hidden="true" className="size-(--size-icon-sm) text-muted-foreground" />
    </Button>
  );
}

/** A list over the start sheet; the keyboard stays down until the search field is pressed. */
function ListSheet({ title, open, onClose, returnTo, list, search, children }: { title: string; open: boolean; onClose: () => void; returnTo: () => HTMLElement | null; list: List; search?: string; children: ReactNode }) {
  const { t } = useInterfaceTranslation();
  return (
    <Sheet open={open} onOpenChange={(next) => !next && onClose()}>
      <SheetContent side="bottom" returnFocusTo={returnTo} onOpenAutoFocus={(event) => event.preventDefault()} className="phone-safe-bottom max-h-dvh gap-sm rounded-t-xl px-md pt-lg pb-md" data-phone-start-list={list}>
        <SheetTitle className="px-sm text-headline">{title}</SheetTitle>
        <SheetDescription className="sr-only">{title}</SheetDescription>
        <Command className="bg-transparent">
          {search ? <CommandInput placeholder={search} wrapperClassName="h-(--size-touch-target) rounded-lg border-0 bg-input" className="text-title" data-phone-start-search={list} /> : null}
          <CommandList className="pt-xs">
            <CommandEmpty>{t("startPanel.noMatch")}</CommandEmpty>
            {children}
          </CommandList>
        </Command>
      </SheetContent>
    </Sheet>
  );
}

/** A row of a list: a finger's height, lit while pressed rather than by the list's own highlight. */
const ROW = "min-h-(--size-touch-target) gap-md rounded-md px-sm text-title data-[selected=true]:bg-transparent active:bg-accent";

function Chosen() {
  return <CheckIcon aria-hidden="true" className="size-(--size-icon) text-primary" data-phone-start-chosen="true" />;
}

/** Where the start goes: the device, the project, `›`, the branch. */
export function WhereWords({ catalog, selection, choice, disabled, onChange }: { catalog: StartCatalog | null; selection: StartSelection; choice: StartChoice; disabled: boolean; onChange: (choice: StartChoice) => void }) {
  const { t } = useInterfaceTranslation();
  const [list, openList] = useState<"project" | "branch" | null>(null);
  const line = useRef<HTMLDivElement>(null);
  // The word a closing list hands focus back to, kept past the close that clears `list`.
  const opener = useRef<List>("project");
  const setList = (next: "project" | "branch" | null) => {
    if (next) opener.current = next;
    openList(next);
  };
  const devices = startDevices(catalog);
  const { target, project } = selection;
  const device = devices.find((row) => row.deviceId === target?.device_id) ?? null;
  const pick = (id: string) => {
    onChange({ ...choice, target: id });
    setList(null);
  };
  const returnTo = () => line.current?.querySelector<HTMLElement>(`[data-phone-start-${opener.current}]`) ?? null;
  return (
    <div ref={line} className="-mx-sm flex min-w-0 flex-wrap items-center" data-phone-start-where="true">
      {device ? (
        <Word
          icon={device.label === null ? <LaptopIcon aria-hidden="true" className="text-muted-foreground" /> : null}
          onPress={() => setList("project")}
          disabled={disabled}
          data-phone-start-device={device.deviceId}
        >
          {device.label === null ? deviceName(t, null, catalog) : (
            // Another device's start is told apart at a glance, as on the desktop panel.
            <span className="inline-flex min-w-0 items-center gap-xxs rounded-xs bg-device-remote px-xs text-primary-foreground">
              <ServerIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />
              <span className="truncate">{device.label}</span>
            </span>
          )}
        </Word>
      ) : null}
      <Word icon={project ? <FolderIcon aria-hidden="true" className="text-muted-foreground" /> : null} onPress={() => setList("project")} disabled={disabled || !project} data-phone-start-project={project?.key ?? ""} data-phone-start-target={target?.id ?? ""}>
        {project?.label ?? t("mobile.start.noProject")}
      </Word>
      {project && target?.branch ? (
        <>
          <span aria-hidden="true" className="text-title text-muted-foreground">›</span>
          <Word icon={<GitBranchIcon aria-hidden="true" className="text-muted-foreground" />} onPress={() => setList("branch")} disabled={disabled} data-phone-start-branch={target.id}>
            {target.branch}
          </Word>
        </>
      ) : null}
      <ListSheet title={t("startPanel.project")} open={list === "project"} onClose={() => setList(null)} returnTo={returnTo} list="project" search={t("startPanel.findProject")}>
        {devices.map((row) => <DeviceProjects key={row.deviceId} device={row} name={deviceName(t, row.label, catalog)} chosen={project?.key ?? null} onPick={pick} />)}
        {catalog?.omitted ? (
          <p className="px-sm py-sm text-body text-muted-foreground opacity-(--opacity-dimmed)" data-phone-start-targets-omitted={catalog.omitted}>
            {t("mobile.start.targetsOmitted", { count: catalog.omitted })}
          </p>
        ) : null}
      </ListSheet>
      {project ? (
        <ListSheet title={t("startPanel.branch")} open={list === "branch"} onClose={() => setList(null)} returnTo={returnTo} list="branch" search={t("startPanel.findBranch")}>
          <CommandGroup>
            {project.targets.map((row) => (
              <CommandItem key={row.id} value={`${row.branch ?? row.place} ${row.id}`} onSelect={() => pick(row.id)} className={ROW} data-phone-start-option={row.id}>
                <GitBranchIcon aria-hidden="true" />
                <span className="min-w-0 flex-1 truncate">{row.branch ?? row.place}</span>
                {row.main ? <Badge variant="outline" className="text-caption font-normal text-muted-foreground">{t("startPanel.mainCheckout")}</Badge> : null}
                {row.id === target?.id ? <Chosen /> : null}
              </CommandItem>
            ))}
          </CommandGroup>
        </ListSheet>
      ) : null}
    </div>
  );
}

/** One device's projects under its name; a device that is not connected is dimmed and none of its projects can be chosen. */
function DeviceProjects({ device, name, chosen, onPick }: { device: StartDevice; name: string; chosen: string | null; onPick: (id: string) => void }) {
  const { t } = useInterfaceTranslation();
  const Icon = device.label === null ? LaptopIcon : ServerIcon;
  return (
    <CommandGroup
      className={cn(!device.connected && "opacity-(--opacity-dimmed)")}
      heading={
        <span className="flex min-w-0 items-center gap-xs">
          <Icon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />
          <span className="truncate">{device.connected ? name : t("startPanel.deviceOffline", { device: name })}</span>
        </span>
      }
      data-phone-start-device-group={device.deviceId}
    >
      {device.projects.map((project) => (
        <CommandItem
          key={project.key}
          value={`${name} ${project.label} ${project.key}`}
          disabled={!device.connected}
          onSelect={() => onPick(project.targets[0]!.id)}
          className={ROW}
          data-phone-start-option={project.key}
        >
          <FolderIcon aria-hidden="true" />
          <span className="min-w-0 flex-1 truncate">{project.label}</span>
          {project.targets.length > 1 ? <span className="shrink-0 text-body text-muted-foreground">{t("mobile.start.branches", { count: project.targets.length })}</span> : null}
          {project.key === chosen ? <Chosen /> : null}
        </CommandItem>
      ))}
    </CommandGroup>
  );
}

/** Who starts: the agent with its mark, then the model, `CLI 기본값` first; an agent whose start takes no model draws no model word. */
export function WhoWords({ selection, choice, disabled, onChange }: { selection: StartSelection; choice: StartChoice; disabled: boolean; onChange: (choice: StartChoice) => void }) {
  const { t } = useInterfaceTranslation();
  const [list, openList] = useState<"kind" | "model" | null>(null);
  const line = useRef<HTMLDivElement>(null);
  const opener = useRef<List>("kind");
  const setList = (next: "kind" | "model" | null) => {
    if (next) opener.current = next;
    openList(next);
  };
  const kind = START_KINDS.find((row) => row.id === selection.kind);
  const models = modelsOf(selection);
  // A remembered model the catalog does not list stays the one sent, so it is listed too.
  const listed = selection.model && !models.includes(selection.model) ? [selection.model, ...models] : models;
  const returnTo = () => line.current?.querySelector<HTMLElement>(`[data-phone-start-${opener.current}]`) ?? null;
  return (
    <div ref={line} className="-mx-sm flex min-w-0 flex-wrap items-center" data-phone-start-who="true">
      <Word icon={<AgentLogo agent={selection.kind} label={kind?.label ?? selection.kind} size="sm" />} onPress={() => setList("kind")} disabled={disabled} data-phone-start-kind={selection.kind}>
        {kind?.label ?? selection.kind}
      </Word>
      {takesModel(selection.kind) ? (
        <Word onPress={() => setList("model")} disabled={disabled || models.length === 0} data-phone-start-model={selection.model}>
          <span className={cn(!selection.model && "text-muted-foreground")}>{selection.model || t("agentPicker.cliDefault")}</span>
        </Word>
      ) : null}
      <ListSheet title={t("mobile.start.kind")} open={list === "kind"} onClose={() => setList(null)} returnTo={returnTo} list="kind">
        <CommandGroup>
          {START_KINDS.map((row) => (
            <CommandItem
              key={row.id}
              value={row.id}
              onSelect={() => {
                onChange({ ...choice, kind: row.id, model: undefined });
                setList(null);
              }}
              className={ROW}
              data-phone-start-option={row.id}
            >
              <AgentLogo agent={row.id} label={row.label} />
              <span className="min-w-0 flex-1 truncate">{row.label}</span>
              {row.id === selection.kind ? <Chosen /> : null}
            </CommandItem>
          ))}
        </CommandGroup>
      </ListSheet>
      <ListSheet title={t("mobile.start.model")} open={list === "model"} onClose={() => setList(null)} returnTo={returnTo} list="model">
        <CommandGroup>
          {["", ...listed].map((model) => (
            <CommandItem
              key={model || DEFAULT_MODEL}
              value={model || DEFAULT_MODEL}
              onSelect={() => {
                onChange({ ...choice, model });
                setList(null);
              }}
              className={ROW}
              data-phone-start-option={model}
            >
              <span className="min-w-0 flex-1 truncate">{model || t("agentPicker.cliDefault")}</span>
              {model === selection.model ? <Chosen /> : null}
            </CommandItem>
          ))}
        </CommandGroup>
      </ListSheet>
    </div>
  );
}
