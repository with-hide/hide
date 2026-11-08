// Add a project's "Projects to import" (PRD device-add-project D-09 to D-14,
// B20 to B29): the projects the chosen machine's node found its agents,
// Paseo and Herdr working in. Chips above the rows say how many each source
// found and show that source's rows alone; each row carries its source marks
// and how long ago it was active, and is checked to be added with the others
// checked while the dialog stays open.
//
// Every add is the `create_workspace` a picked folder sends, named by a
// request id of its own, and answered by its own receipt in the core
// (`status.project_adds`): added, or refused with why, kept past any later
// event. A refusal hided gives before the core sees the add names that id
// too (`path_refused`). So an add made at the same time from another window
// or the CLI answers no row here. The adds go one at a time, so a row
// refused stays with its reason, still checked, to be added again (B25).

import { InfoIcon } from "lucide-react";
import { useEffect, useState } from "react";
import type { Actions } from "./actions";
import { refusalText } from "./addProject";
import { AgentLogo } from "./components/agent-logo";
import { Elapsed } from "./components/elapsed";
import { Badge } from "./components/ui/badge";
import { Button } from "./components/ui/button";
import { Checkbox } from "./components/ui/checkbox";
import { ToggleGroup, ToggleGroupItem } from "./components/ui/toggle-group";
import { Hint } from "./components/ui/tooltip";
import { Status } from "./components/settings-rows";
import { coreErrorText } from "./coreError";
import { useInterfaceTranslation } from "./i18n/client";
import { filterRows, importFilters, importRows, parentLabel, sourceIsApp, sourceName } from "./importProjects";
import type { WorkspaceRegistration } from "./snapshot";
import type { ImportableProject, ImportSource } from "./store";
import { useShellStore } from "./store";

/** One add on its way: the row's folder and the request id it was sent with. */
type Sent = { path: string; request: string };

/** The chip value that shows every row. */
const ALL = "all";

let nextRequest = 0;

export function ImportableProjects({
  actions,
  device,
  hostLabel,
  registrations,
  offline,
  busy,
}: {
  actions: Actions;
  device: string;
  hostLabel: string;
  registrations: readonly WorkspaceRegistration[];
  /** The machine is not connected, so it cannot be asked or added to. */
  offline: boolean;
  /** The dialog is adding a folder another way. */
  busy: boolean;
}) {
  const { t } = useInterfaceTranslation();
  const answer = useShellStore((s) => s.importable[device]);
  const pathRefusal = useShellStore((s) => s.pathRefusal);
  const [after, setAfter] = useState(() => useShellStore.getState().homeFolderSeq);
  const [queue, setQueue] = useState<string[]>([]);
  const [sent, setSent] = useState<Sent | null>(null);
  const [failed, setFailed] = useState<Readonly<Record<string, string>>>({});
  // Rows whose registration landed, gone whatever spelling the registration keeps.
  const [added, setAdded] = useState<ReadonlySet<string>>(new Set());
  const [checked, setChecked] = useState<ReadonlySet<string>>(new Set());
  const [filter, setFilter] = useState<ImportSource | null>(null);
  const disabled = offline || busy;

  // Each machine chosen is asked once; the area stays empty until its answer arrives (B29).
  useEffect(() => {
    setQueue([]);
    setSent(null);
    setFailed({});
    setAdded(new Set());
    setChecked(new Set());
    setFilter(null);
    if (offline) return;
    setAfter(useShellStore.getState().homeFolderSeq);
    actions.listImportableProjects(device);
    // A machine coming back from not connected is asked again.
  }, [device, offline]);

  const rows = importRows(answer, after, device, registrations).filter((row) => !added.has(row.path));
  const filters = importFilters(rows);
  // A chip whose rows have all been added is gone, and the list shows every row again.
  const source = filter !== null && filters.some((chip) => chip.source === filter) ? filter : null;
  const shown = filterRows(rows, source);
  const receipt = useShellStore((s) => (sent ? s.rest?.status?.project_adds?.findLast((row) => row.request_id === sent.request && row.path === sent.path) : undefined));
  const refused = sent && pathRefusal?.request_id === sent.request ? pathRefusal.reason : null;

  // The add on its way is answered by its receipt: added (its row leaves through the registrations), or refused with why.
  useEffect(() => {
    if (!sent) return;
    if (receipt?.state === "added") {
      setAdded((previous) => new Set(previous).add(sent.path));
      setChecked((previous) => without(previous, [sent.path]));
      setSent(null);
      return;
    }
    const reason = refused
      ? refusalText(refused, t)
      : receipt?.state === "refused"
        ? coreErrorText(receipt, t, "coreError.project.addFailed")
        : null;
    if (reason === null) return;
    setFailed((previous) => ({ ...previous, [sent.path]: reason }));
    setSent(null);
  }, [receipt?.state, refused]);

  // The next row waiting is sent once the one before it is answered.
  useEffect(() => {
    if (sent || queue.length === 0) return;
    const [path, ...rest] = queue;
    setQueue(rest);
    const project = rows.find((row) => row.path === path);
    if (!project) return;
    nextRequest += 1;
    const request = `import:${Date.now().toString(36)}:${nextRequest}`;
    setSent({ path: project.path, request });
    actions.createWorkspace(project.path, project.name, device, false, request);
  }, [sent, queue]);

  if (rows.length === 0) return null;

  const waiting = (path: string) => sent?.path === path || queue.includes(path);
  // What Add would send: the checked rows still listed and not on their way.
  const ready = rows.filter((row) => checked.has(row.path) && !waiting(row.path)).map((row) => row.path);
  const choosable = shown.filter((row) => !waiting(row.path));
  const shownChecked = choosable.filter((row) => checked.has(row.path)).length;
  const all = choosable.length > 0 && shownChecked === choosable.length ? true : shownChecked > 0 ? "indeterminate" : false;
  const check = (paths: string[], on: boolean) => setChecked((previous) => (on ? new Set([...previous, ...paths]) : without(previous, paths)));
  const add = () => {
    if (ready.length === 0) return;
    setFailed((previous) => Object.fromEntries(Object.entries(previous).filter(([path]) => !ready.includes(path))));
    setQueue((previous) => [...previous, ...ready]);
  };

  return (
    <section className="flex flex-col gap-sm" aria-labelledby="add-project-import-title" data-add-project-import={device}>
      <div className="flex items-center gap-xxs">
        <span id="add-project-import-title" className="text-caption font-medium text-subtle-foreground">
          {t("addProject.import.title")}
        </span>
        <Hint label={t("addProject.import.detail", { host: hostLabel })}>
          <Button variant="ghost" size="icon-xs" className="text-muted-foreground" data-add-project-import-detail="true">
            <InfoIcon />
          </Button>
        </Hint>
      </div>
      {filters.length > 0 ? (
        <ToggleGroup
          type="single"
          variant="pill"
          value={source ?? ALL}
          // Pressing the chip already on shows every row again.
          onValueChange={(next) => setFilter(next === "" || next === ALL ? null : next)}
          aria-label={t("addProject.import.filter")}
          data-add-project-import-filters="true"
        >
          <ToggleGroupItem value={ALL} variant="pill" className="text-caption" data-add-project-import-filter={ALL}>
            {t("addProject.import.all")}
            <span className="opacity-(--opacity-secondary)">{rows.length}</span>
          </ToggleGroupItem>
          {filters.map((chip) => (
            <ToggleGroupItem key={chip.source} value={chip.source} variant="pill" className="text-caption" data-add-project-import-filter={chip.source}>
              {sourceIsApp(chip.source) ? null : <AgentLogo agent={chip.source} label={sourceName(chip.source)} size="sm" />}
              {sourceName(chip.source)}
              <span className="opacity-(--opacity-secondary)">{chip.count}</span>
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
      ) : null}
      <div className="flex flex-col">
        <label className="flex items-center gap-sm border-b border-border px-sm pb-xs text-caption text-muted-foreground">
          <Checkbox
            checked={all}
            disabled={disabled || choosable.length === 0}
            onCheckedChange={() => check(choosable.map((row) => row.path), all !== true)}
            data-add-project-import-check-all={choosable.length}
          />
          {source === null ? t("addProject.import.checkAll") : t("addProject.import.checkShown", { count: choosable.length })}
          <span className="ml-auto">{t("addProject.import.newestFirst")}</span>
        </label>
        <ul className="flex max-h-(--size-folder-list-h) flex-col overflow-y-auto py-xxs">
          {shown.map((project) => (
            <ImportRow
              key={project.path}
              project={project}
              checked={checked.has(project.path)}
              onChecked={(on) => check([project.path], on)}
              waiting={waiting(project.path)}
              disabled={disabled}
              failed={failed[project.path] ?? null}
            />
          ))}
        </ul>
      </div>
      <div className="flex justify-end">
        <Button size="sm" disabled={disabled || ready.length === 0} onClick={add} data-add-project-import-add={ready.length}>
          {ready.length > 0 ? t("addProject.import.addChecked", { count: ready.length }) : t("common.add")}
        </Button>
      </div>
    </section>
  );
}

/** One project to import: its check, name and folder, source marks, and how long ago it was active or that it is being added. */
function ImportRow({
  project,
  checked,
  onChecked,
  waiting,
  disabled,
  failed,
}: {
  project: ImportableProject;
  checked: boolean;
  onChecked: (on: boolean) => void;
  waiting: boolean;
  disabled: boolean;
  failed: string | null;
}) {
  const { t } = useInterfaceTranslation();
  const id = `add-project-import-${project.path}`;
  return (
    <li className="flex min-w-0 flex-col gap-xxs rounded-md px-sm py-xs hover:bg-accent" data-add-project-import-row={project.path}>
      <label htmlFor={id} className="flex min-w-0 cursor-pointer items-center gap-sm">
        <Checkbox id={id} checked={checked} disabled={disabled || waiting} onCheckedChange={(next) => onChecked(next === true)} data-add-project-import-check={project.path} />
        <span className="flex min-w-0 flex-1 flex-col">
          <span className="truncate text-body font-medium text-foreground">{project.name}</span>
          <span className="truncate font-mono text-caption text-muted-foreground" title={project.label}>
            {parentLabel(project.label)}
          </span>
        </span>
        <span role="group" className="flex shrink-0 items-center gap-xxs" aria-label={t("addProject.import.sources", { sources: project.sources.map(sourceName).join(", ") })}>
          {project.sources.map((source) =>
            sourceIsApp(source) ? (
              <Badge key={source} variant="outline" aria-hidden="true" data-add-project-import-source={source}>
                {sourceName(source)}
              </Badge>
            ) : (
              <span key={source} title={sourceName(source)} data-add-project-import-source={source}>
                <AgentLogo agent={source} label={sourceName(source)} size="sm" />
              </span>
            ),
          )}
        </span>
        <span className="shrink-0 text-right text-caption text-muted-foreground">
          {waiting ? t("addProject.import.adding") : <Elapsed since={project.active_at_unix_ms} />}
        </span>
      </label>
      {failed ? (
        <Status tone="error" data-add-project-import-failed={project.path}>
          {failed}
        </Status>
      ) : null}
    </li>
  );
}

function without(set: ReadonlySet<string>, paths: readonly string[]): ReadonlySet<string> {
  const next = new Set(set);
  for (const path of paths) next.delete(path);
  return next;
}
