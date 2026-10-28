// Add a project: a Host selector and, on the machine this window runs on,
// Browse folder, which opens the system's own folder picker through the
// desktop app; any other machine's folders are not this window's to browse,
// so it gets a `~/` path field its own node judges. A browser tab has no
// picker, so its own machine gets that field too. The other ways to add
// (Clone from URL, Create new project), each its own view of this dialog, sit
// on the core's machine, the one a new project is made on (`hostWays`). Every
// way sends one `create_workspace` and reads its answer through
// `useRegistration`; a folder outside home is asked about once and sent again
// with the operator's confirmation of that path (PRD wsl-device B15).

import { CornerDownLeftIcon, FolderOpenIcon, FolderPlusIcon, LinkIcon, type LucideIcon } from "lucide-react";
import type { TFunction } from "i18next";
import { useEffect, useRef, useState } from "react";
import type { Actions } from "./actions";
import { addProjectHosts, alreadyRegistered, folderLabel, hostWays, initialHost, trimFolder, wslDrivePath } from "./addProject";
import { CloneFromUrl, cloneRunning } from "./CloneFromUrl";
import { CreateProjectView } from "./CreateProjectView";
import { Button } from "./components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogHeader, DialogTitle } from "./components/ui/dialog";
import { Input } from "./components/ui/input";
import { Kbd } from "./components/ui/kbd";
import { Select, SelectContent, SelectItem, SelectSeparator, SelectTrigger, SelectValue } from "./components/ui/select";
import { RegistrationStatus, useRegistration } from "./registration";
import type { MessageKey } from "./i18n/catalogs";
import { useInterfaceTranslation } from "./i18n/client";
import type { FirstRunProjectForm, WorkspaceRegistration } from "./snapshot";
import { hostBridge } from "./host";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

const NO_REGISTRATIONS: WorkspaceRegistration[] = [];

export function AddProjectDialog({ actions }: { actions: Actions }) {
  const open = useUiStore((s) => s.overlay === "add_project");
  // Mounted only while open, so every opening starts on the focused device with nothing sent.
  return open ? <AddProject actions={actions} /> : null;
}

function AddProject({ actions }: { actions: Actions }) {
  const close = () => useUiStore.getState().closeOverlay("add_project");
  const [host, setHost] = useState<string | null>(null);
  return (
    <Dialog open onOpenChange={(next) => { if (!next) close(); }}>
      <DialogContent
        showCloseButton
        aria-describedby={undefined}
        data-add-project={host ?? ""}
        // The views hand the keyboard to their own primary control once mounted.
        onOpenAutoFocus={(event) => event.preventDefault()}
      >
        <AddProjectViews actions={actions} title={(t) => t("addProject.title")} onLeave={close} onHost={setHost} />
      </DialogContent>
    </Dialog>
  );
}

/** The other ways to add on the core's machine, each a view of this dialog. */
type OtherWay = { id: "clone" | "create"; label: MessageKey; detail: MessageKey; icon: LucideIcon };

const OTHER_WAYS: readonly OtherWay[] = [
  { id: "clone", label: "addProject.wayClone", detail: "addProject.wayCloneDetail", icon: LinkIcon },
  { id: "create", label: "addProject.wayCreate", detail: "addProject.wayCreateDetail", icon: FolderPlusIcon },
];

/** The Host list's last entry; it opens Add device instead of choosing a host. */
const ADD_DEVICE = "__add_device__";

/**
 * The ways to add a project, inside a dialog that holds them: Add a project's
 * own, or the first-run wizard's third step (PRD first-run-onboarding B22),
 * which titles it and watches the project appear itself. `onLeave` closes
 * the holder when Add device takes its place; a holder without one (the
 * wizard, which keeps its steps) has no Add device entry.
 */
export function AddProjectViews({
  actions,
  title,
  onLeave,
  onHost,
  draft,
  onDraft,
}: {
  actions: Actions;
  title: (t: TFunction<"translation">) => string;
  onLeave?: () => void;
  /** The host the views add to, for the holder to name. */
  onHost?: (host: string) => void;
  draft?: { device?: string | null; path?: string; form?: FirstRunProjectForm };
  onDraft?: (draft: { project_device?: string; project_path?: string; project_form?: FirstRunProjectForm }) => void;
}) {
  const { t } = useInterfaceTranslation();
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const focused = useShellStore((s) => s.rest?.navigator?.focused_device_id);
  const registrations = useShellStore((s) => s.rest?.ui_state?.workspace_registrations ?? NO_REGISTRATIONS);
  const hosts = addProjectHosts(devices, t);
  const [chosen, setChosen] = useState(() => initialHost(hosts, draft?.device ?? focused));
  const host = hosts.find((row) => row.id === chosen) ?? hosts[0]!;
  const ways = hostWays(host.id, devices, hostBridge() !== null);
  // Where a WSL device shows its PC's drives, as its helper reports it; null on any other machine.
  const drives = devices?.find((row) => row.id === host.id)?.host?.windows_drives ?? null;
  // A clone still running when the dialog opens is shown where it started.
  const [view, setView] = useState<"add" | OtherWay["id"]>(() => (cloneRunning(useShellStore.getState().rest?.repository_clone) ? "clone" : draft?.form?.view || "add"));
  const [form, setForm] = useState<FirstRunProjectForm>(draft?.form ?? {});
  const saveForm = (patch: FirstRunProjectForm) => {
    const next = { ...form, ...patch };
    setForm(next);
    onDraft?.({ project_form: next });
  };
  const [text, setText] = useState(draft?.path || "~/");
  const registration = useRegistration(registrations);
  const { pending } = registration;
  const browseRef = useRef<HTMLButtonElement>(null);
  const fieldRef = useRef<HTMLInputElement>(null);
  const hostChanged = useRef(false);
  // The way Back returns from, whose row takes the keyboard again.
  const returnTo = useRef<OtherWay["id"] | null>(null);

  const add = (path: string, confirmed = false) => {
    if (alreadyRegistered(path, host.id, registrations)) {
      registration.refuse(path, "already_registered");
      return;
    }
    registration.send(host.id, path, () => actions.createWorkspace(path, folderLabel(path), host.id, confirmed));
  };

  const open = (next: typeof view) => {
    registration.reset();
    if (next === "add" && view !== "add") returnTo.current = view;
    setView(next);
    saveForm({ view: next });
  };

  const browse = async () => {
    if (pending) return;
    const folder = await actions.pickFolder();
    // A cancelled pick leaves the dialog as it was.
    if (folder !== null) add(trimFolder(folder));
  };

  const submitField = () => {
    const path = trimFolder(wslDrivePath(text, drives));
    if (pending || !path || path === "~") return;
    setText(path);
    add(path);
  };

  useEffect(() => onHost?.(host.id), [onHost, host.id]);
  // The keyboard starts on the primary control of the host in front.
  useEffect(() => {
    (ways.browse ? browseRef.current : fieldRef.current)?.focus();
    // Only on mount: a host chosen later moves the keyboard through the select's own close.
  }, []);

  return (
      <>
        {view === "create" ? (
          <CreateProjectView actions={actions} registrations={registrations} pickParent={ways.pickParent} onBack={() => open("add")} draft={form} onDraft={saveForm} />
        ) : view === "clone" ? (
          <CloneFromUrl actions={actions} registrations={registrations} pickParent={ways.pickParent} onBack={() => open("add")} draft={form} onDraft={saveForm} />
        ) : (
          <>
            <DialogHeader>
              <DialogTitle>{title(t)}</DialogTitle>
            </DialogHeader>
            <DialogBody className="flex flex-col gap-md">
              <div className="flex items-center gap-sm">
                <span className="text-body text-subtle-foreground">{t("addProject.host")}</span>
                <Select
                  value={host.id}
                  onValueChange={(next) => {
                    // The list's last entry is Add device, the one form every entry point opens (PRD home-device-rail B13).
                    if (next === ADD_DEVICE) {
                      onLeave?.();
                      actions.openAddDevice();
                      return;
                    }
                    setChosen(next);
                    onDraft?.({ project_device: next });
                    registration.reset();
                    hostChanged.current = true;
                  }}
                >
                  <SelectTrigger size="sm" className="w-auto" aria-label={t("addProject.host")} data-add-project-host="true">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent
                    onCloseAutoFocus={(event) => {
                      // A new host hands the keyboard to its own primary control rather than back to the selector.
                      if (!hostChanged.current) return;
                      hostChanged.current = false;
                      event.preventDefault();
                      (browseRef.current ?? fieldRef.current)?.focus();
                    }}
                  >
                    {hosts.map((row) => (
                      <SelectItem key={row.id} value={row.id} data-host-option={row.id}>
                        {row.label}
                      </SelectItem>
                    ))}
                    {onLeave ? (
                      <>
                        <SelectSeparator />
                        <SelectItem value={ADD_DEVICE} data-host-option-add-device="true">
                          {t("sidebar.addDevice")}
                        </SelectItem>
                      </>
                    ) : null}
                  </SelectContent>
                </Select>
              </div>
              {ways.browse ? (
                <button
                  ref={browseRef}
                  type="button"
                  disabled={pending}
                  onClick={() => void browse()}
                  data-add-project-browse="true"
                  className="flex w-full items-center gap-md rounded-md border border-border bg-card px-md py-sm text-left outline-none transition-colors hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-(--opacity-disabled)"
                >
                  <span aria-hidden="true" className="flex size-(--size-control) shrink-0 items-center justify-center rounded-sm bg-secondary text-foreground [&_svg]:size-(--size-icon)">
                    <FolderOpenIcon />
                  </span>
                  <span className="flex min-w-0 flex-1 flex-col">
                    <span className="text-subhead font-medium text-foreground">{t("addProject.browseTitle")}</span>
                    <span className="text-body text-muted-foreground">{t("addProject.browseDetail")}</span>
                  </span>
                  <Kbd aria-hidden="true">
                    <CornerDownLeftIcon />
                  </Kbd>
                </button>
              ) : (
                <div className="flex flex-col gap-xs">
                  <div className="flex items-center gap-sm">
                    <Input
                      ref={fieldRef}
                      mono
                      value={text}
                      placeholder="~/…"
                      aria-label={t("addProject.folderOn", { host: host.label })}
                      data-add-project-path="true"
                      onChange={(event) => { setText(event.target.value); onDraft?.({ project_path: event.target.value }); }}
                      onPaste={(event) => {
                        // A Windows path pasted on a WSL device is shown as that device's own path (PRD wsl-device B16).
                        const pasted = event.clipboardData.getData("text");
                        const spelled = wslDrivePath(pasted, drives);
                        if (spelled === pasted) return;
                        event.preventDefault();
                        setText(spelled);
                        onDraft?.({ project_path: spelled });
                      }}
                      onKeyDown={(event) => {
                        if (event.nativeEvent.isComposing || event.key !== "Enter") return;
                        event.preventDefault();
                        submitField();
                      }}
                    />
                    <Button variant="secondary" disabled={pending} onClick={submitField} data-add-project-submit="true">
                      {t("common.add")}
                    </Button>
                  </div>
                  <p className="text-body text-muted-foreground">{ways.own ? t("addProject.homeNoteOwn") : t("addProject.homeNote", { host: host.label })}</p>
                </div>
              )}
              <RegistrationStatus registration={registration} drives={drives} onConfirm={(path) => add(path, true)} />
              {ways.otherWays ? (
                <div className="flex flex-col gap-xs" data-add-project-other-ways="true">
                  <span className="text-caption text-muted-foreground">{t("addProject.otherWays")}</span>
                  {OTHER_WAYS.map((way) => (
                    <button
                      key={way.id}
                      ref={(row) => {
                        if (row && returnTo.current === way.id) {
                          returnTo.current = null;
                          row.focus();
                        }
                      }}
                      type="button"
                      disabled={pending}
                      onClick={() => open(way.id)}
                      data-add-project-way={way.id}
                      className="flex w-full items-center gap-md rounded-md px-md py-sm text-left outline-none transition-colors hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-(--opacity-disabled)"
                    >
                      <span aria-hidden="true" className="flex size-(--size-control) shrink-0 items-center justify-center rounded-sm bg-secondary text-foreground [&_svg]:size-(--size-icon)">
                        <way.icon />
                      </span>
                      <span className="flex min-w-0 flex-1 flex-col">
                        <span className="text-subhead font-medium text-foreground">{t(way.label)}</span>
                        <span className="text-body text-muted-foreground">{t(way.detail)}</span>
                      </span>
                    </button>
                  ))}
                </div>
              ) : null}
            </DialogBody>
          </>
        )}
      </>
  );
}
