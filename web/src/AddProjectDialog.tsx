// Add a project: a Host selector beside the title and one start view the
// same on every machine (PRD device-add-project D-01): Browse folder, Clone
// from URL and Create new project as three tiles, then the projects to import. Browse folder opens the
// system's own folder window on the machine this window runs on when the
// desktop app can open one, and the dialog's own folder list anywhere else
// (`hostWays`, `FolderBrowser`); Clone and Create run on the chosen machine
// and pick their parent with the same two. Every way sends one
// `create_workspace` and reads its answer through `useRegistration`; a folder
// outside home is asked about once and sent again with the operator's
// confirmation of that path (PRD wsl-device B15).

import { FolderOpenIcon, FolderPlusIcon, LinkIcon, type LucideIcon } from "lucide-react";
import type { TFunction } from "i18next";
import { useEffect, useRef, useState, type ComponentProps, type Ref } from "react";
import type { Actions } from "./actions";
import { addProjectHosts, alreadyRegistered, folderLabel, hostWays, initialHost, rememberedFolder, trimFolder } from "./addProject";
import { CloneFromUrl, cloneRunning } from "./CloneFromUrl";
import { CreateProjectView } from "./CreateProjectView";
import { FolderBrowser } from "./FolderBrowser";
import { ImportableProjects } from "./ImportableProjects";
import { Dialog, DialogBody, DialogContent, DialogHeader, DialogTitle } from "./components/ui/dialog";
import { Button } from "./components/ui/button";
import { Select, SelectContent, SelectItem, SelectSeparator, SelectTrigger, SelectValue } from "./components/ui/select";
import { RegistrationStatus, useRegistration } from "./registration";
import type { MessageKey } from "./i18n/catalogs";
import { useInterfaceTranslation } from "./i18n/client";
import { localDeviceId, type FirstRunProjectForm, type WorkspaceRegistration } from "./snapshot";
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
  // Read once at mount: the surface that opened the dialog may have named the device to add to.
  const [device] = useState(() => useUiStore.getState().addProjectDevice);
  return (
    <Dialog open onOpenChange={(next) => { if (!next) close(); }}>
      <DialogContent
        showCloseButton
        aria-describedby={undefined}
        data-add-project={host ?? ""}
        // The views hand the keyboard to their own primary control once mounted.
        onOpenAutoFocus={(event) => event.preventDefault()}
      >
        <AddProjectViews actions={actions} title={(t) => t("addProject.title")} onLeave={close} onHost={setHost} draft={device ? { device } : undefined} />
      </DialogContent>
    </Dialog>
  );
}

/** The other ways to add, each a view of this dialog; their lines name the chosen machine. */
type OtherWay = { id: "clone" | "create"; label: MessageKey; icon: LucideIcon };

const OTHER_WAYS: readonly OtherWay[] = [
  { id: "clone", label: "addProject.wayClone", icon: LinkIcon },
  { id: "create", label: "addProject.wayCreate", icon: FolderPlusIcon },
];

/** The Host list's last entry; it opens Add device instead of choosing a host. */
const ADD_DEVICE = "__add_device__";

/** The folder list over the dialog: adding a folder, or picking the parent of a clone or a new project. */
type Browsing = { mode: "add" } | { mode: "pick"; for: OtherWay["id"]; start: string };

/**
 * The ways to add a project, inside a dialog that holds them: Add a project's
 * own, or the first-run wizard's third step (PRD first-run-onboarding B22,
 * device-add-project D-16), which titles it and watches the project appear
 * itself. `onLeave` closes the holder when Add device takes its place; a
 * holder without one (the wizard, which keeps its steps) has no Add device
 * entry.
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
  draft?: { device?: string | null; form?: FirstRunProjectForm };
  onDraft?: (draft: { project_device?: string; project_form?: FirstRunProjectForm }) => void;
}) {
  const { t } = useInterfaceTranslation();
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const focused = useShellStore((s) => s.rest?.navigator?.focused_device_id);
  const uiState = useShellStore((s) => s.rest?.ui_state);
  const registrations = uiState?.workspace_registrations ?? NO_REGISTRATIONS;
  const hosts = addProjectHosts(devices, t);
  // A clone still running when the dialog opens is shown where it started, on its machine.
  const running = useShellStore.getState().rest?.repository_clone;
  const [chosen, setChosen] = useState(() => (cloneRunning(running) && running?.device_id) || initialHost(hosts, draft?.device ?? focused));
  const host = hosts.find((row) => row.id === chosen) ?? hosts[0]!;
  const device = devices?.find((row) => row.id === host.id);
  const ways = hostWays(host.id, devices, hostBridge() !== null);
  const offline = host.offline;
  // Where a WSL device shows its PC's drives, as its helper reports it; null on any other machine.
  const drives = device?.host?.windows_drives ?? null;
  const windows = /^windows/i.test(device?.host?.platform ?? "") && !drives;
  const node = localDeviceId(useShellStore.getState().rest);
  const [view, setView] = useState<"add" | OtherWay["id"]>(() => (cloneRunning(running) ? "clone" : draft?.form?.view || "add"));
  const [browsing, setBrowsing] = useState<Browsing | null>(null);
  const [form, setForm] = useState<FirstRunProjectForm>(draft?.form ?? {});
  const saveForm = (patch: FirstRunProjectForm) => {
    const next = { ...form, ...patch };
    setForm(next);
    onDraft?.({ project_form: next });
  };
  const registration = useRegistration(registrations);
  const { pending } = registration;
  const browseRef = useRef<HTMLButtonElement>(null);
  const hostChanged = useRef(false);
  // The way Back returns from, whose row takes the keyboard again.
  const returnTo = useRef<OtherWay["id"] | "browse" | null>(null);

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
    if (pending || offline) return;
    if (!ways.native) {
      registration.reset();
      setBrowsing({ mode: "add" });
      return;
    }
    const folder = await actions.pickFolder();
    // A cancelled pick leaves the dialog as it was.
    if (folder !== null) add(trimFolder(folder));
  };

  /**
   * A parent folder for Clone or Create: the system's folder window, or the
   * dialog's own list on another machine, which opens on `from`, the location
   * the view shows and would use.
   */
  const pickParent = async (forWay: OtherWay["id"], from: string): Promise<string | null> => {
    if (!ways.native) {
      setBrowsing({ mode: "pick", for: forWay, start: from });
      return null;
    }
    const folder = await actions.pickFolder();
    return folder === null ? null : trimFolder(folder);
  };

  useEffect(() => onHost?.(host.id), [onHost, host.id]);
  // The keyboard starts on Browse folder, the primary row.
  useEffect(() => {
    browseRef.current?.focus();
    // Only on mount: a host chosen later moves the keyboard through the select's own close.
  }, []);

  if (browsing) {
    const leave = () => {
      if (browsing.mode === "add") returnTo.current = "browse";
      registration.reset();
      setBrowsing(null);
    };
    return (
      <FolderBrowser
        actions={actions}
        device={host.id}
        deviceLabel={host.label}
        local={ways.own}
        drives={drives}
        windows={windows}
        mode={browsing.mode}
        start={browsing.mode === "pick" ? browsing.start : (rememberedFolder(uiState, host.id, node) ?? "~")}
        remembered={rememberedFolder(uiState, host.id, node)}
        registrations={registrations}
        offline={offline}
        busy={pending}
        onAdd={(path) => add(path)}
        onPick={(label) => {
          saveForm({ parent: label });
          setBrowsing(null);
        }}
        onBack={leave}
        status={browsing.mode === "add" ? <RegistrationStatus registration={registration} drives={drives} onConfirm={(path) => add(path, true)} /> : null}
      />
    );
  }

  return (
      <>
        {view === "create" ? (
          <CreateProjectView actions={actions} device={host.id} hostLabel={host.label} registrations={registrations} pickParent={(from) => pickParent("create", from)} onBack={() => open("add")} draft={form} onDraft={saveForm} />
        ) : view === "clone" ? (
          <CloneFromUrl actions={actions} device={host.id} hostLabel={host.label} registrations={registrations} pickParent={(from) => pickParent("clone", from)} onBack={() => open("add")} draft={form} onDraft={saveForm} />
        ) : (
          <>
            <DialogHeader>
              <div className="flex min-w-0 items-center gap-sm pr-xl">
                <DialogTitle>{title(t)}</DialogTitle>
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
                  <SelectTrigger size="sm" className="w-auto min-w-0" aria-label={t("addProject.host")} data-add-project-host="true">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent
                    onCloseAutoFocus={(event) => {
                      // A new host hands the keyboard to Browse folder rather than back to the selector.
                      if (!hostChanged.current) return;
                      hostChanged.current = false;
                      event.preventDefault();
                      browseRef.current?.focus();
                    }}
                  >
                    {hosts.map((row) => (
                      // A device that is not connected is listed dimmed and cannot be chosen (PRD device-add-project B31).
                      <SelectItem key={row.id} value={row.id} disabled={row.offline} data-host-option={row.id} data-host-offline={row.offline || undefined}>
                        {row.offline ? `${row.label} · ${t("addProject.hostOffline")}` : row.label}
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
            </DialogHeader>
            <DialogBody className="flex flex-col gap-lg">
              <div className="grid grid-cols-3 gap-sm" data-add-project-other-ways="true">
                <WayButton
                  ref={(row: HTMLButtonElement | null) => {
                    browseRef.current = row;
                    if (row && returnTo.current === "browse") {
                      returnTo.current = null;
                      row.focus();
                    }
                  }}
                  icon={FolderOpenIcon}
                  label={t("addProject.browseTitle")}
                  disabled={pending || offline}
                  onClick={() => void browse()}
                  data-add-project-browse={ways.native ? "native" : "list"}
                />
                {OTHER_WAYS.map((way) => (
                  <WayButton
                    key={way.id}
                    ref={(row: HTMLButtonElement | null) => {
                      if (row && returnTo.current === way.id) {
                        returnTo.current = null;
                        row.focus();
                      }
                    }}
                    icon={way.icon}
                    label={t(way.label)}
                    disabled={pending || offline}
                    onClick={() => open(way.id)}
                    data-add-project-way={way.id}
                  />
                ))}
              </div>
              <RegistrationStatus registration={registration} drives={drives} onConfirm={(path) => add(path, true)} />
              <ImportableProjects actions={actions} device={host.id} hostLabel={host.label} registrations={registrations} offline={offline} busy={pending} />
            </DialogBody>
          </>
        )}
      </>
  );
}

/** One way to add, as a tile: its icon over its name. Browse folder, the first, holds the keyboard so Enter picks a folder. */
function WayButton({ icon: Icon, label, ref, ...props }: { icon: LucideIcon; label: string; ref?: Ref<HTMLButtonElement> } & Omit<ComponentProps<typeof Button>, "children">) {
  return (
    <Button ref={ref} variant="secondary" className="h-auto min-w-0 flex-col gap-xs whitespace-normal px-sm py-md text-center" {...props}>
      <Icon />
      <span className="min-w-0 text-body">{label}</span>
    </Button>
  );
}
