// Add a project's Create new project: a name and the folder it goes in, and
// one `create_workspace` with `new_folder` that makes the folder, starts a Git
// repository in it and registers it (on the core's machine only, whose hided
// makes it, so `node` here is the core's machine). hided answers a
// `project_target` probe as the operator types, on the same `$HOME` line the
// create is checked on, so an existing folder, a parent outside home or a
// name that is not one folder is said here before anything is sent.

import { ArrowLeftIcon, FolderOpenIcon, GitBranchIcon } from "lucide-react";
import { useEffect, useState } from "react";
import type { Actions } from "./actions";
import { alreadyRegistered, defaultProjectParent, projectNameProblem, projectPath, refusalText } from "./addProject";
import { Note } from "./components/settings-rows";
import { Button } from "./components/ui/button";
import { DialogBody, DialogDescription, DialogHeader, DialogTitle } from "./components/ui/dialog";
import { Input } from "./components/ui/input";
import { RegistrationStatus, useRegistration } from "./registration";
import { localDeviceId, type WorkspaceRegistration } from "./snapshot";
import { useShellStore } from "./store";
import { useInterfaceTranslation } from "./i18n/client";

/** `pickParent`: the native folder picker shows the core's folders, which it does only in a window on the core's machine. */
export function CreateProjectView({ actions, registrations, pickParent, onBack, draft, onDraft }: { actions: Actions; registrations: readonly WorkspaceRegistration[]; pickParent: boolean; onBack: () => void; draft?: { name?: string; parent?: string }; onDraft?: (draft: { name?: string; parent?: string }) => void }) {
  const { t } = useInterfaceTranslation();
  const [parent, setParent] = useState(() => draft?.parent || defaultProjectParent(registrations, localDeviceId(useShellStore.getState().rest)));
  const [typed, setTyped] = useState(draft?.name ?? "");
  const registration = useRegistration(registrations);
  const { pending, shown } = registration;
  const name = typed.trim();
  const nameProblem = projectNameProblem(name);
  const asked = nameProblem === null ? name : "";
  const answer = useShellStore((s) => s.projectTarget);
  const target = answer && answer.parent === parent && answer.name === asked ? answer : null;

  // Asked again when a create is refused or fails, since a failed create may
  // have left its folder behind, which the next create continues into.
  const failure = shown ? `${shown.reason}\n${shown.text}` : "";
  useEffect(() => {
    actions.probeProjectTarget(parent, asked);
  }, [actions, parent, asked, failure]);

  const registered = target?.path ? alreadyRegistered(target.path, localDeviceId(useShellStore.getState().rest), registrations) : false;
  const problem = nameProblem ? t(nameProblem) : registered ? refusalText("already_registered", t) : target?.reason ? refusalText(target.reason, t) : null;
  const ready = name !== "" && problem === null && target?.path != null && !pending;
  const shownParent = target?.parent_label ?? parent;
  const shownPath = target?.parent_path ? projectPath(target.parent_path, name) : null;

  const create = () => {
    if (!ready || !target?.path) return;
    const path = target.path;
    registration.send(localDeviceId(useShellStore.getState().rest), path, () => actions.createProject(path, name));
  };

  const choose = async () => {
    if (pending) return;
    const folder = await actions.pickFolder();
    // A cancelled pick keeps the folder that was there.
    if (folder !== null) { setParent(folder); onDraft?.({ parent: folder }); }
  };

  return (
    <form
      className="flex min-h-0 flex-col"
      data-create-project="true"
      onSubmit={(event) => {
        event.preventDefault();
        create();
      }}
    >
      <DialogHeader>
        <Button type="button" variant="ghost" size="sm" className="-ml-sm mb-xs self-start" onClick={onBack} data-create-project-back="true">
          <ArrowLeftIcon />
          {t("common.back")}
        </Button>
        <DialogTitle>{t("addProject.create.title")}</DialogTitle>
        <DialogDescription>{t("addProject.create.description")}</DialogDescription>
      </DialogHeader>
      <DialogBody className="flex flex-col gap-md">
        <label className="block text-body text-subtle-foreground">
          {t("common.name")}
          <Input
            autoFocus
            mono
            value={typed}
            disabled={pending}
            autoComplete="off"
            spellCheck={false}
            placeholder="my-project"
            className="mt-xxs"
            aria-invalid={problem !== null || undefined}
            data-create-project-name="true"
            onChange={(event) => { setTyped(event.target.value); onDraft?.({ name: event.target.value }); }}
          />
        </label>
        {pickParent ? (
          <button
            type="button"
            disabled={pending}
            onClick={() => void choose()}
            aria-label={t("addProject.create.locationAria", { parent: shownParent })}
            data-create-project-location={target?.parent_path ?? parent}
            className="flex w-full items-center gap-md rounded-md border border-border bg-card px-md py-sm text-left outline-none transition-colors hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-(--opacity-disabled)"
          >
            <ProjectLocation parent={shownParent} path={shownPath} />
            <FolderOpenIcon aria-hidden="true" className="size-(--size-icon) shrink-0 text-muted-foreground" />
          </button>
        ) : (
          <div className="flex w-full items-center gap-md rounded-md border border-border bg-card px-md py-sm" data-create-project-location={target?.parent_path ?? parent}>
            <ProjectLocation parent={shownParent} path={shownPath} />
          </div>
        )}
        {problem ? (
          <Note tone="warn" data-create-project-problem={registered ? "already_registered" : (target?.reason ?? "name")}>
            {problem}
          </Note>
        ) : target?.leftover && name ? (
          <Note data-create-project-leftover="true">{t("addProject.create.leftover")}</Note>
        ) : null}
        <Button type="submit" disabled={!ready} className="w-full" data-create-project-submit="true">
          {t("addProject.create.submit")}
        </Button>
        <RegistrationStatus registration={registration} />
      </DialogBody>
    </form>
  );
}

/** Where the new project goes: the folder it is made in and its full path, which follows the name. */
function ProjectLocation({ parent, path }: { parent: string; path: string | null }) {
  const { t } = useInterfaceTranslation();
  return (
    <>
      <span aria-hidden="true" className="flex size-(--size-control) shrink-0 items-center justify-center rounded-sm bg-secondary text-foreground [&_svg]:size-(--size-icon)">
        <GitBranchIcon />
      </span>
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="truncate text-subhead font-medium text-foreground">{t("addProject.create.location", { parent })}</span>
        <span className="break-all font-mono text-caption text-muted-foreground" data-create-project-path={path ?? ""}>
          {path ?? " "}
        </span>
      </span>
    </>
  );
}
