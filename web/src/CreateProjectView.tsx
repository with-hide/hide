// Add a project's Create new project: a name and the folder it goes in, and
// one `create_workspace` with `new_folder` that makes the folder, starts a Git
// repository in it and registers it, on the machine the dialog chose (PRD
// device-add-project D-07). That machine answers a `project_target` probe as
// the operator types (hided's `$HOME` line for the core's machine, the
// device's node for a device), by the same line the create is checked on, so
// an existing folder, a parent outside home or a name that is not one folder
// is said here before anything is sent.

import { ArrowLeftIcon, FolderOpenIcon, GitBranchIcon } from "lucide-react";
import { useEffect, useState } from "react";
import type { Actions } from "./actions";
import { alreadyRegistered, defaultProjectParent, projectNameProblem, projectPath, refusalText, TYPED_ASK_DELAY_MS } from "./addProject";
import { Note } from "./components/settings-rows";
import { Button } from "./components/ui/button";
import { DialogBody, DialogDescription, DialogHeader, DialogTitle } from "./components/ui/dialog";
import { Input } from "./components/ui/input";
import { RowButton } from "./components/ui/row-button";
import { RegistrationStatus, useRegistration } from "./registration";
import type { WorkspaceRegistration } from "./snapshot";
import { useShellStore } from "./store";
import { useInterfaceTranslation } from "./i18n/client";

/**
 * `device` is the machine the project is made on and `hostLabel` its name;
 * `pickParent` opens that machine's folder picker and answers the folder, or
 * null when it was cancelled or the dialog's own list took over.
 */
export function CreateProjectView({ actions, device, hostLabel, registrations, pickParent, onBack, draft, onDraft }: { actions: Actions; device: string; hostLabel: string; registrations: readonly WorkspaceRegistration[]; pickParent: () => Promise<string | null>; onBack: () => void; draft?: { name?: string; parent?: string }; onDraft?: (draft: { name?: string; parent?: string }) => void }) {
  const { t } = useInterfaceTranslation();
  const [parent, setParent] = useState(() => draft?.parent || defaultProjectParent(registrations, device));
  const [typed, setTyped] = useState(draft?.name ?? "");
  const registration = useRegistration(registrations);
  const { pending, shown } = registration;
  const name = typed.trim();
  const nameProblem = projectNameProblem(name);
  const asked = nameProblem === null ? name : "";
  const answer = useShellStore((s) => s.projectTarget);
  const target = answer && answer.device_id === device && answer.parent === parent && answer.name === asked ? answer : null;

  // Asked again when a create is refused or fails, since a failed create may
  // have left its folder behind, which the next create continues into.
  const failure = shown ? `${shown.reason}\n${shown.text}` : "";
  // Typing settles first: on a device each probe is a round trip to its node.
  useEffect(() => {
    const timer = window.setTimeout(() => actions.probeProjectTarget(parent, asked, device), TYPED_ASK_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [actions, device, parent, asked, failure]);

  const registered = target?.path ? alreadyRegistered(target.path, device, registrations) : false;
  const problem = nameProblem ? t(nameProblem) : registered ? refusalText("already_registered", t) : target?.reason ? refusalText(target.reason, t) : null;
  const ready = name !== "" && problem === null && target?.path != null && !pending;
  const shownParent = target?.parent_label ?? parent;
  const shownPath = target?.parent_path ? projectPath(target.parent_path, name) : null;

  const create = () => {
    if (!ready || !target?.path) return;
    const path = target.path;
    registration.send(device, path, () => actions.createProject(path, name, device));
  };

  const choose = async () => {
    if (pending) return;
    const folder = await pickParent();
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
        <DialogDescription>{t("addProject.wayCreateDetail", { host: hostLabel })}</DialogDescription>
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
        <RowButton
          disabled={pending}
          onClick={() => void choose()}
          aria-label={t("addProject.create.locationAria", { parent: shownParent })}
          data-create-project-location={target?.parent_path ?? parent}
          className="gap-md rounded-md border border-border bg-card px-md py-sm"
        >
          <ProjectLocation parent={shownParent} path={shownPath} />
          <FolderOpenIcon aria-hidden="true" className="size-(--size-icon) shrink-0 text-muted-foreground" />
        </RowButton>
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
