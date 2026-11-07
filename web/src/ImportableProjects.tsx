// Add a project's "Projects to import" (PRD device-add-project D-09 to D-14,
// B20 to B29): the projects the chosen machine's node found its agents,
// Paseo and Herdr working in, each with a chip per source, added one at a
// time or all at once while the dialog stays open.
//
// Every add is the `create_workspace` a picked folder sends. The core
// answers a registration through the snapshot and a refusal through its one
// `last_error` (or hided's `path_refused`), so the adds go one at a time:
// each answer then belongs to the one row sent, a row that lands leaves the
// list, and a row refused stays with its reason to be pressed again (B25).

import { useEffect, useState } from "react";
import type { Actions } from "./actions";
import { refusalText } from "./addProject";
import { Badge } from "./components/ui/badge";
import { Button } from "./components/ui/button";
import { Status } from "./components/settings-rows";
import { coreErrorText } from "./coreError";
import { useErrorSince } from "./errorSince";
import { useInterfaceTranslation } from "./i18n/client";
import { importRows, sourceIsApp, sourceName } from "./importProjects";
import type { WorkspaceRegistration } from "./snapshot";
import { useShellStore } from "./store";

/** One add on its way: the row's folder, when it was sent, and the ids of the machine's projects then. */
type Sent = { path: string; at: number; known: ReadonlySet<string> };

const ADD_ERRORS = ["workspace.create", "workspace.remove_in_flight", "workspace.control_unavailable"];

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
  const disabled = offline || busy;

  // Each machine chosen is asked once; the area stays empty until its answer arrives (B29).
  useEffect(() => {
    setQueue([]);
    setSent(null);
    setFailed({});
    setAdded(new Set());
    if (offline) return;
    setAfter(useShellStore.getState().homeFolderSeq);
    actions.listImportableProjects(device);
    // A machine coming back from not connected is asked again.
  }, [device, offline]);

  const rows = importRows(answer, after, device, registrations).filter((row) => !added.has(row.path));
  const coreError = useErrorSince(sent?.at ?? null, ADD_ERRORS);
  const landed = sent !== null && registrations.some((row) => row.device_id === device && !sent.known.has(row.id));
  const refused = sent && pathRefusal?.kind === "create_workspace" && pathRefusal.path === sent.path ? pathRefusal.reason : null;

  // The add on its way is answered: landed (its row leaves through the registrations), or refused with why.
  useEffect(() => {
    if (!sent) return;
    if (landed || coreError?.kind === "workspace.create_registered") {
      setAdded((previous) => new Set(previous).add(sent.path));
      setSent(null);
      return;
    }
    const reason = refused ? refusalText(refused, t) : coreError ? coreErrorText(coreError, t, "coreError.project.addFailed") : null;
    if (reason === null) return;
    setFailed((previous) => ({ ...previous, [sent.path]: reason }));
    setSent(null);
  }, [landed, refused, coreError?.occurred_at]);

  // The next row waiting is sent once the one before it is answered.
  useEffect(() => {
    if (sent || queue.length === 0) return;
    const [path, ...rest] = queue;
    setQueue(rest);
    const project = rows.find((row) => row.path === path);
    if (!project) return;
    useShellStore.getState().clearPathRefusal();
    setSent({ path: project.path, at: Date.now(), known: new Set(registrations.filter((row) => row.device_id === device).map((row) => row.id)) });
    actions.createWorkspace(project.path, project.name, device);
  }, [sent, queue]);

  if (rows.length === 0) return null;

  const waiting = (path: string) => sent?.path === path || queue.includes(path);
  const add = (paths: string[]) => {
    const fresh = paths.filter((path) => !waiting(path));
    if (fresh.length === 0) return;
    setFailed((previous) => Object.fromEntries(Object.entries(previous).filter(([path]) => !fresh.includes(path))));
    setQueue((previous) => [...previous, ...fresh]);
  };

  return (
    <section className="flex flex-col gap-xs" aria-labelledby="add-project-import-title" data-add-project-import={device}>
      <div className="flex items-center justify-between gap-sm">
        <span id="add-project-import-title" className="text-caption text-muted-foreground">
          {t("addProject.import.title")}
        </span>
        <Button
          variant="text"
          size="inline"
          className="text-caption"
          disabled={disabled}
          onClick={() => add(rows.map((row) => row.path))}
          data-add-project-import-all={rows.length}
        >
          {t("addProject.import.addAll", { count: rows.length })}
        </Button>
      </div>
      <span className="text-body text-muted-foreground">{t("addProject.import.detail", { host: hostLabel })}</span>
      <ul className="flex flex-col">
        {rows.map((project) => (
          <li key={project.path} className="flex min-w-0 flex-col gap-xxs rounded-md px-md py-xs hover:bg-accent" data-add-project-import-row={project.path}>
            <div className="flex min-w-0 items-center gap-sm">
              <span className="flex min-w-0 flex-1 flex-col">
                <span className="flex min-w-0 flex-wrap items-center gap-xs">
                  <span className="min-w-0 truncate text-subhead font-medium text-foreground">{project.name}</span>
                  <span className="flex flex-wrap gap-xxs" aria-label={t("addProject.import.sources", { sources: project.sources.map(sourceName).join(", ") })}>
                    {project.sources.map((source) => (
                      <Badge key={source} variant={sourceIsApp(source) ? "outline" : "secondary"} aria-hidden="true" data-add-project-import-source={source}>
                        {sourceName(source)}
                      </Badge>
                    ))}
                  </span>
                </span>
                <span className="truncate font-mono text-caption text-muted-foreground" title={project.label}>
                  {project.label}
                </span>
              </span>
              <Button
                variant="secondary"
                size="sm"
                disabled={disabled || waiting(project.path)}
                aria-label={t("addProject.import.addNamed", { name: project.name })}
                onClick={() => add([project.path])}
                data-add-project-import-add={project.path}
              >
                {waiting(project.path) ? t("addProject.import.adding") : t("common.add")}
              </Button>
            </div>
            {failed[project.path] ? (
              <Status tone="error" data-add-project-import-failed={project.path}>
                {failed[project.path]}
              </Status>
            ) : null}
          </li>
        ))}
      </ul>
    </section>
  );
}
