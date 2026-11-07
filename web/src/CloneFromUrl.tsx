// Add a project's Clone from URL: a Git URL, the parent folder the clone goes
// into, and Clone, on the machine the dialog chose (PRD device-add-project
// D-07). The folder the URL names is shown as it is typed, and that machine
// is asked whether it is free under that parent (`clone_target`: hided's
// `$HOME` line for the core's machine, the device's node for a device), so
// Clone is offered only for a clone that can start. The clone itself is the
// core's (`repository_clone`), one at a time whichever machine runs it: its
// progress, its cancel and its end are read from the snapshot. A finished
// clone is registered on its machine with the same `create_workspace` a
// picked folder sends, and its answer is read the same way
// (`useRegistration`): the dialog closes when that project appears. Closing
// the dialog leaves a running clone running; opening it again shows it here.

import { ArrowLeftIcon, FolderOpenIcon } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Trans } from "react-i18next";
import type { Actions } from "./actions";
import { cloneUrlDraft, defaultProjectParent, folderLabel, parseCloneUrl, refusalText, trimFolder, TYPED_ASK_DELAY_MS } from "./addProject";
import { Note, Status } from "./components/settings-rows";
import { Button } from "./components/ui/button";
import { DialogBody, DialogDescription, DialogHeader, DialogTitle } from "./components/ui/dialog";
import { Input } from "./components/ui/input";
import { RegistrationStatus, useRegistration } from "./registration";
import type { RepositoryClone, WorkspaceRegistration } from "./snapshot";
import { useShellStore } from "./store";
import { useInterfaceTranslation } from "./i18n/client";
import { cloneFailureText, cloneStageText, coreErrorText } from "./coreError";
import { QuotedLine } from "./QuotedLine";
import { useErrorSince } from "./errorSince";
import { composingKey } from "./keys";

/** Whether a clone is still running: nothing else may start until it ends. */
export function cloneRunning(clone: RepositoryClone | null | undefined): boolean {
  return clone?.phase === "cloning" || clone?.phase === "cancelling";
}

/**
 * Which clone this view follows: one running when it opened, or the first one
 * newer than `after`, the slot's id when Clone was pressed.
 */
type Following = { id: number } | { after: number; at: number; path: string };

/**
 * `device` is the machine the clone runs on and `hostLabel` its name;
 * `pickParent` opens that machine's folder picker and answers the folder, or
 * null when it was cancelled or the dialog's own list took over.
 */
export function CloneFromUrl({ actions, device, hostLabel, registrations, pickParent, onBack, draft, onDraft }: { actions: Actions; device: string; hostLabel: string; registrations: readonly WorkspaceRegistration[]; pickParent: () => Promise<string | null>; onBack: () => void; draft?: { url?: string; parent?: string }; onDraft?: (draft: { url?: string; parent?: string }) => void }) {
  const { t, i18n } = useInterfaceTranslation();
  const slot = useShellStore((s) => s.rest?.repository_clone ?? null);
  const target = useShellStore((s) => s.cloneTarget);
  const pathRefusal = useShellStore((s) => s.pathRefusal);
  const [url, setUrl] = useState(draft?.url ?? "");
  const [parent, setParent] = useState(() => draft?.parent || defaultProjectParent(registrations, device));
  const chooseParent = (parent: string) => { setParent(parent); onDraft?.({ parent }); };
  const [following, setFollowing] = useState<Following | null>(() => {
    const running = useShellStore.getState().rest?.repository_clone;
    return running && cloneRunning(running) ? { id: running.id } : null;
  });
  const urlRef = useRef<HTMLInputElement>(null);

  const parsed = parseCloneUrl(url);
  const parentPath = trimFolder(parent);
  const name = parsed.ok ? parsed.name : null;
  const clone = slot && following && ("id" in following ? slot.id === following.id : slot.id > following.after) ? slot : null;
  const running = cloneRunning(clone);
  const sentAt = following && "at" in following ? following.at : null;
  // The registration a finished clone makes; its refusal and its closing the dialog are `useRegistration`'s.
  const registration = useRegistration(registrations);
  const cloneError = useErrorSince(sentAt, ["repository.clone", "workspace.control_unavailable"]);
  const hidedRefusal =
    following && "path" in following && pathRefusal?.kind === "clone_repository" && pathRefusal.path === following.path ? pathRefusal.reason : null;
  // A refusal of the clone itself (hided's, or the core's), or the core
  // unable to register the finished folder; a running clone has neither.
  const refusal = hidedRefusal ? refusalText(hidedRefusal, t) : cloneError && !running ? coreErrorText(cloneError, t, "addProject.clone.failed") : null;
  const ended = clone?.phase === "failed" || clone?.phase === "cancelled" || refusal !== null;
  const answer = target && name && target.device_id === device && target.parent === parentPath && target.name === name ? target : null;
  const waiting = following !== null && !ended && registration.shown === null;
  const canClone = parsed.ok && answer !== null && answer.reason === null && !waiting;

  // A clone that ended without a folder leaves no registration to wait for.
  const { reset } = registration;
  useEffect(() => {
    if (ended) reset();
  }, [ended, reset]);

  useEffect(() => {
    if (!name || !parentPath) return;
    const timer = window.setTimeout(() => actions.checkCloneTarget(parentPath, name, device), TYPED_ASK_DELAY_MS);
    return () => window.clearTimeout(timer);
    // A clone that ended may have taken the folder, so its end asks again.
  }, [actions, device, name, parentPath, clone?.phase]);

  const start = () => {
    if (!canClone || !name) return;
    const path = `${parentPath.replace(/\/+$/, "")}/${name}`;
    setFollowing({ after: slot?.id ?? 0, at: Date.now(), path });
    registration.send(device, path, () => actions.cloneRepository(url.trim(), parentPath, name, device));
  };

  const browse = async () => {
    const folder = await pickParent();
    if (folder !== null) chooseParent(folder);
  };

  return (
    <>
      <DialogHeader>
        <Button type="button" variant="ghost" size="sm" className="-ml-sm mb-xs self-start" disabled={running} onClick={onBack} data-add-project-back="true">
          <ArrowLeftIcon />
          {t("common.back")}
        </Button>
        <DialogTitle>{t("addProject.wayClone")}</DialogTitle>
        <DialogDescription>{t("addProject.wayCloneDetail", { host: hostLabel })}</DialogDescription>
      </DialogHeader>
      <DialogBody className="flex flex-col gap-md">
        <label className="flex flex-col gap-xs">
          <span className="text-body text-subtle-foreground">{t("addProject.clone.urlLabel")}</span>
          <Input
            ref={urlRef}
            autoFocus
            mono
            value={url}
            disabled={running}
            placeholder="https://github.com/user/repo.git"
            spellCheck={false}
            autoCapitalize="off"
            autoCorrect="off"
            aria-invalid={url.trim() !== "" && !parsed.ok}
            data-clone-url="true"
            onChange={(event) => { setUrl(event.target.value); onDraft?.({ url: cloneUrlDraft(event.target.value) }); }}
            onKeyDown={(event) => {
              if (composingKey(event) || event.key !== "Enter") return;
              event.preventDefault();
              start();
            }}
          />
          {url.trim() !== "" && !parsed.ok ? (
            <Note tone="error" data-clone-url-reason="true">
              {t(parsed.reason)}
            </Note>
          ) : null}
        </label>
        <div className="flex flex-col gap-xs">
          <label htmlFor="clone-parent" className="text-body text-subtle-foreground">
            {t("addProject.clone.parentLabel")}
          </label>
          <div className="flex items-center gap-sm">
            <Input
              id="clone-parent"
              mono
              value={parent}
              disabled={running}
              spellCheck={false}
              data-clone-parent="true"
              onChange={(event) => chooseParent(event.target.value)}
            />
            <Button variant="secondary" size="icon" disabled={running} aria-label={t("addProject.clone.chooseParent")} onClick={() => void browse()} data-clone-browse="true">
              <FolderOpenIcon />
            </Button>
          </div>
          {name && answer?.reason ? (
            <Note tone="error" data-clone-target-reason={answer.reason}>
              {refusalText(answer.reason, t)}
            </Note>
          ) : name ? (
            <p className="break-all text-caption text-muted-foreground" data-clone-target={answer?.path ?? ""}>
              <Trans i18n={i18n} t={t} i18nKey="addProject.clone.into" values={{ name }} components={{ name: <span className="font-mono text-foreground" /> }} />
            </p>
          ) : null}
        </div>
        <CloneProgress clone={clone} following={following !== null} />
        {refusal ? (
          <div role="alert" className="flex min-w-0 flex-col gap-xxs" data-clone-refused={hidedRefusal ?? "core"} data-core-error={hidedRefusal ? undefined : cloneError?.kind}>
            <Status tone="error">{refusal}</Status>
          </div>
        ) : clone?.phase === "finished" ? (
          <RegistrationStatus registration={registration} />
        ) : null}
        {clone?.phase === "finished" && (refusal || registration.shown) ? (
          <Note data-clone-kept={clone.path}>{t("addProject.clone.kept")}</Note>
        ) : null}
        {running && clone ? (
          <Button variant="secondary" size="lg" disabled={clone.phase === "cancelling"} onClick={() => actions.cancelRepositoryClone(clone.id)} data-clone-cancel="true">
            {clone.phase === "cancelling" ? t("addProject.clone.cancelling") : t("common.cancel")}
          </Button>
        ) : (
          <Button size="lg" disabled={!canClone} onClick={start} data-clone-submit="true">
            {t("addProject.clone.submit")}
          </Button>
        )}
      </DialogBody>
    </>
  );
}

/** What the followed clone is doing, in the smallest form each state needs. */
function CloneProgress({ clone, following }: { clone: RepositoryClone | null; following: boolean }) {
  const { t } = useInterfaceTranslation();
  if (!following) return null;
  if (clone === null) {
    return (
      <Status tone="pending" data-clone-phase="sent">
        {t("addProject.clone.sending")}
      </Status>
    );
  }
  const name = folderLabel(clone.path);
  if (clone.phase === "cloning" || clone.phase === "cancelling") {
    return (
      <div className="flex flex-col gap-xs" data-clone-phase={clone.phase}>
        <Status tone="pending">
          {clone.phase === "cancelling" ? t("addProject.clone.cancellingNamed", { name }) : t("addProject.clone.cloning", { name, host: clone.host })}
        </Status>
        <div
          role="progressbar"
          aria-label={clone.stage ? cloneStageText(clone.stage, t) : t("addProject.clone.stageFallback")}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={clone.percent ?? undefined}
          className="h-xxs overflow-hidden rounded-full bg-secondary"
        >
          <div className="h-full bg-primary transition-[width]" style={{ width: `${clone.percent ?? 0}%` }} />
        </div>
        <span className="text-caption text-muted-foreground" data-clone-stage={clone.stage ?? ""}>
          {clone.stage ? `${cloneStageText(clone.stage, t)}${clone.percent === null ? "" : ` ${clone.percent}%`}` : t("settings.connection.connecting")}
        </span>
      </div>
    );
  }
  if (clone.phase === "failed") {
    return (
      <div role="alert" className="flex min-w-0 flex-col gap-xxs" data-clone-phase="failed" data-core-error={clone.failure ?? undefined}>
        <Status tone="error">{cloneFailureText(clone.failure, t)}</Status>
        {clone.failure === "git" && clone.git_line ? <QuotedLine line={clone.git_line} /> : null}
      </div>
    );
  }
  if (clone.phase === "cancelled") {
    return (
      <Note tone="muted" data-clone-phase="cancelled">
        {t("addProject.clone.cancelled")}
      </Note>
    );
  }
  // Finished: the registration it hands over says `Adding <folder>…` itself.
  return null;
}
