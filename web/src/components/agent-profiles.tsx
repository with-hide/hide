// Agent profiles, drawn from values a caller holds: an agent's profile rows
// with their one state, the add and edit form, the delete confirmation, the
// start panel's profile menu and a project's Agent accounts group. Nothing
// here reads the store or sends an event: Settings › Agents
// (`settings/AgentProfiles.tsx`), the start panel and Project settings pass
// the values and the handlers in, and the gallery's `scene=agent-profiles`
// draws every state with none.

import { ChevronRightIcon, LockIcon, PlusIcon, XIcon } from "lucide-react";
import type { ReactNode } from "react";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import { AgentLogo } from "./agent-logo";
import { Group, Note, Row, Status } from "./settings-rows";
import { AlertDialog, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle, AlertDialogAction } from "./ui/alert-dialog";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { Dialog, DialogBody, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "./ui/dialog";
import { Input } from "./ui/input";
import { RowOverlayButton } from "./ui/row-button";
import { Select, SelectContent, SelectItem, SelectSeparator, SelectTrigger, SelectValue } from "./ui/select";
import { Hint } from "./ui/tooltip";

/** How an agent's definition keeps accounts apart: a folder, a name the agent owns (omp), or not at all (agy). */
export type ProfileAccountWay = "folder" | "name" | "none";
export type ProfileInstall = "installed" | "installing" | "failed" | "removing" | "remove_failed";
export type ProfileLogin = "signed_in" | "signed_out" | "unknown";

export type ProfileView = {
  id: string;
  /** The built-in profile: no account variable, the agent's own folder; never edited or deleted. */
  builtIn: boolean;
  name: string;
  /** The agent's default profile, the one a start uses with no project default. */
  isDefault: boolean;
  /** The account folder or omp name as registered; the agent's own folder on the built-in row. */
  account: string | null;
  program: string | null;
  envKeys: readonly string[];
  /** Null where the profile has no install target of its own (the built-in row, a shared location). */
  install: ProfileInstall | null;
  /** Null where nothing is shown: a wrapper, or no way to check. */
  login: ProfileLogin | null;
  /** Percent left of the account's limit; `shared` when another profile owns the same location. */
  usageLeft: number | "shared" | null;
  /** The program started the agent in another registered folder (B25). */
  wrapperFolder: boolean;
};

export type AgentProfilesView = { agent: string; label: string; way: ProfileAccountWay; program: string; profiles: readonly ProfileView[] };

/** A device's profile list: live, read-only while it is disconnected, or absent on a build that has none. */
export type ProfilesMachine = "live" | "offline" | "unsupported";

type StateWord = { tone: "error" | "warn" | "pending" | "muted"; key: string; action: "reinstall" | "retry" | "open" | null };

/** The one state a profile row shows, the most urgent first: what the operator can act on before what only informs. */
export function profileState(profile: ProfileView): StateWord | null {
  if (profile.install === "failed") return { tone: "error", key: "agentProfiles.state.failed", action: "reinstall" };
  if (profile.install === "remove_failed") return { tone: "error", key: "agentProfiles.state.removeFailed", action: "retry" };
  if (profile.install === "installing") return { tone: "pending", key: "agentProfiles.state.installing", action: null };
  if (profile.install === "removing") return { tone: "pending", key: "agentProfiles.state.removing", action: null };
  if (profile.wrapperFolder) return { tone: "warn", key: "agentProfiles.state.wrapperFolder", action: null };
  if (profile.login === "signed_out") return { tone: "warn", key: "agentProfiles.state.signedOut", action: "open" };
  if (profile.login === "unknown") return { tone: "muted", key: "agentProfiles.state.unknown", action: null };
  return null;
}

function profileName(profile: ProfileView, t: ReturnType<typeof useInterfaceTranslation>["t"]) {
  return profile.builtIn ? t("agentProfiles.builtIn") : profile.name;
}

/** The row's right end: its one state with the action that answers it, or the account's usage when all is well. */
export function ProfileStateCell({ profile, readOnly = false, onAction }: { profile: ProfileView; readOnly?: boolean; onAction?: (action: "reinstall" | "retry" | "open") => void }) {
  const { t } = useInterfaceTranslation();
  const state = profileState(profile);
  if (state) {
    return (
      <span className="flex min-w-0 shrink-0 flex-wrap items-center justify-end gap-x-sm gap-y-xxs" data-profile-state={state.key.slice("agentProfiles.state.".length)}>
        <Status tone={state.tone}>{t(state.key as "agentProfiles.state.failed")}</Status>
        {state.action && !readOnly ? (
          <Button variant="secondary" size="sm" className="pointer-events-auto" onClick={() => onAction?.(state.action!)} data-profile-action={state.action}>
            {t(state.action === "reinstall" ? "agentProfiles.reinstall" : state.action === "retry" ? "agentProfiles.retry" : "agentProfiles.openToSignIn")}
          </Button>
        ) : null}
      </span>
    );
  }
  return <ProfileUsage left={profile.usageLeft} />;
}

function ProfileUsage({ left }: { left: ProfileView["usageLeft"] }) {
  const { t } = useInterfaceTranslation();
  if (left === null) return null;
  if (left === "shared") {
    return (
      <Hint label={t("agentProfiles.state.shared")}>
        <span tabIndex={0} className="pointer-events-auto shrink-0 text-body text-muted-foreground" data-profile-usage="shared">—</span>
      </Hint>
    );
  }
  return <span className="shrink-0 text-body tabular-nums text-subtle-foreground" data-profile-usage={left}>{t("agentProfiles.state.left", { percent: left })}</span>;
}

function DefaultBadge() {
  const { t } = useInterfaceTranslation();
  return <Badge variant="outline" data-profile-default="true">{t("agentProfiles.startsByDefault")}</Badge>;
}

/**
 * One profile as a line: its name with the default mark and its state; where
 * it runs (its account, its program) is the edit dialog's. The whole line
 * opens that dialog, lit on hover and marked by the chevron at its end; the
 * state's own button stays its own, so the press goes to a RowOverlayButton
 * behind the content.
 */
export function ProfileRow({ profile, agentLabel, readOnly = false, onOpen, onAction }: { profile: ProfileView; agentLabel: string; readOnly?: boolean; onOpen?: () => void; onAction?: (action: "reinstall" | "retry" | "open") => void }) {
  const { t } = useInterfaceTranslation();
  const name = profileName(profile, t);
  return (
    <div className="relative min-h-(--size-control) rounded-xs" data-profile-row={profile.id}>
      {onOpen && !readOnly ? <RowOverlayButton aria-label={t("agentProfiles.editTitle", { agent: agentLabel, profile: name })} onClick={onOpen} /> : null}
      <div className="pointer-events-none relative flex min-h-(--size-control) min-w-0 flex-wrap items-center gap-x-md gap-y-xxs px-sm py-xs">
        <span className="flex min-w-0 flex-1 basis-(--size-settings-control-w) flex-wrap items-center gap-x-sm gap-y-xxs">
          <span className="min-w-0 max-w-full truncate text-subhead text-foreground">{name}</span>
          {profile.isDefault ? <DefaultBadge /> : null}
        </span>
        <span className="ml-auto flex min-w-0 items-center gap-sm">
          <ProfileStateCell profile={profile} readOnly={readOnly} onAction={onAction} />
          {onOpen && !readOnly ? <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" data-profile-opens="true" /> : null}
        </span>
      </div>
    </div>
  );
}

function AddProfileButton({ onAdd, disabled }: { onAdd?: () => void; disabled?: boolean }) {
  const { t } = useInterfaceTranslation();
  return (
    <Button variant="text" size="sm" className="self-start" disabled={disabled} onClick={onAdd} data-profile-add="true">
      <PlusIcon aria-hidden="true" />
      {t("agentProfiles.add")}
    </Button>
  );
}

/** Where a device's list cannot be changed, the one line that says why (B30). */
export function ProfilesMachineNote({ machine, device }: { machine: ProfilesMachine; device: string }) {
  const { t } = useInterfaceTranslation();
  if (machine === "live") return null;
  return <Note tone={machine === "offline" ? "warn" : "muted"} data-profiles-machine={machine}>{machine === "offline" ? t("agentProfiles.offline", { device }) : t("agentProfiles.unsupported")}</Note>;
}

/**
 * An agent's profiles unfolded under its row in the installed
 * list, indented to the agent's name. With nothing but the built-in profile
 * only the add button shows (B1). Add and edit open `ProfileFormDialog`.
 */
export function InlineProfiles({ view, machine = "live", onAdd, onOpen, onAction }: { view: AgentProfilesView; machine?: ProfilesMachine; onAdd?: () => void; onOpen?: (id: string) => void; onAction?: (id: string, action: "reinstall" | "retry" | "open") => void }) {
  const readOnly = machine !== "live";
  const own = view.profiles.filter((profile) => !profile.builtIn);
  if (machine === "unsupported") return null;
  return (
    <div className={cn("flex flex-col gap-xxs pb-sm pl-[calc(var(--spacing-md)+var(--size-icon-lg))] pr-sm", readOnly && "opacity-(--opacity-dimmed)")} data-inline-profiles={view.agent}>
      {own.length > 0 ? view.profiles.map((profile) => (
        <ProfileRow key={profile.id} profile={profile} agentLabel={view.label} readOnly={readOnly} onOpen={() => onOpen?.(profile.id)} onAction={(action) => onAction?.(profile.id, action)} />
      )) : null}
      {readOnly ? null : <AddProfileButton onAdd={onAdd} />}
    </div>
  );
}

export type EnvPair = { key: string; value: string };
export type ProfileDraft = { name: string; account: string; program: string; env: readonly EnvPair[] };
/** A refusal under its field, as the node answers it (B4, B5): which field, and the words. */
export type ProfileRefusal = { field: "name" | "account" | "program" | "env"; text: string; envKey?: string };

function Field({ label, optional = false, htmlFor, children, note, refusal }: { label: string; optional?: boolean; htmlFor?: string; children: ReactNode; note?: ReactNode; refusal?: string | null }) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="flex flex-col gap-xs" data-profile-field={label}>
      <label htmlFor={htmlFor} className="flex items-baseline gap-sm text-subhead text-foreground">
        {label}
        {optional ? <span className="text-caption text-muted-foreground">{t("agentProfiles.optional")}</span> : null}
      </label>
      {children}
      {refusal ? <Note tone="error">{refusal}</Note> : null}
      {note ? <p className="break-keep text-caption text-muted-foreground">{note}</p> : null}
    </div>
  );
}

/**
 * The profile's fields, the same for adding and editing (B3, B26). The
 * account field follows the agent's way: a folder with a chooser, an omp name,
 * or a disabled field with the reason. Editing locks the account and saves each
 * field as it is left (`onCommit`), so it has no Save button. Without
 * `onDraft` the fields only show the draft.
 */
export function ProfileForm({ view, mode, draft, refusals = [], builtIn = false, readOnly = false, idPrefix, onDraft, onCommit, onChooseFolder, canChoose = true, envLoading = false }: { view: AgentProfilesView; mode: "add" | "edit"; draft: ProfileDraft; refusals?: readonly ProfileRefusal[]; builtIn?: boolean; readOnly?: boolean; idPrefix: string; onDraft?: (draft: ProfileDraft) => void; onCommit?: (draft: ProfileDraft) => void; onChooseFolder?: () => void; canChoose?: boolean; envLoading?: boolean }) {
  const { t } = useInterfaceTranslation();
  const refusal = (field: ProfileRefusal["field"]) => refusals.find((row) => row.field === field && !row.envKey)?.text ?? null;
  const editing = mode === "edit";
  const off = readOnly || builtIn;
  const fixed = !onDraft;
  const set = (patch: Partial<ProfileDraft>) => onDraft?.({ ...draft, ...patch });
  const commit = () => onCommit?.(draft);
  const setEnv = (index: number, pair: Partial<EnvPair>) => set({ env: draft.env.map((row, at) => (at === index ? { ...row, ...pair } : row)) });
  const removeEnv = (index: number) => {
    const next = { ...draft, env: draft.env.filter((_, at) => at !== index) };
    onDraft?.(next);
    onCommit?.(next);
  };
  const accountLabel = view.way === "name" ? t("agentProfiles.field.ompName") : view.way === "folder" ? t("agentProfiles.field.folder") : t("agentProfiles.field.account");
  const accountNote = view.way === "none" ? t("agentProfiles.noAccountNote") : editing ? null : view.way === "name" ? t("agentProfiles.ompNote") : t("agentProfiles.folderNote");
  return (
    <div className="flex flex-col gap-lg" data-profile-form={mode} data-profile-way={view.way}>
      {builtIn ? <Note>{t("agentProfiles.builtInNote")}</Note> : null}
      {builtIn ? null : (
        <Field label={t("agentProfiles.field.name")} htmlFor={`${idPrefix}-name`} refusal={refusal("name")}>
          <Input id={`${idPrefix}-name`} value={draft.name} readOnly={fixed} onChange={(event) => set({ name: event.target.value })} onBlur={commit} disabled={off} aria-invalid={refusal("name") ? true : undefined} data-profile-input="name" />
        </Field>
      )}
      <Field label={accountLabel} optional={view.way !== "none" && !editing} htmlFor={`${idPrefix}-account`} refusal={refusal("account")} note={editing && view.way !== "none" && !builtIn ? <span className="flex items-center gap-xs"><LockIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />{t("agentProfiles.locked")}</span> : accountNote}>
        {view.way === "none" ? (
          <Input id={`${idPrefix}-account`} disabled value={t("agentProfiles.noAccount")} readOnly data-profile-account="none" />
        ) : (
          <span className="flex min-w-0 gap-sm">
            <Input id={`${idPrefix}-account`} mono value={draft.account} readOnly={editing || fixed} onChange={(event) => set({ account: event.target.value })} disabled={editing || off} aria-invalid={refusal("account") ? true : undefined} data-profile-account={view.way} />
            {view.way === "folder" && !editing && canChoose ? <Button variant="secondary" disabled={off} onClick={onChooseFolder} data-profile-choose="true">{t("agentProfiles.chooseFolder")}</Button> : null}
          </span>
        )}
      </Field>
      {builtIn ? null : (
        <>
          <Field label={t("agentProfiles.field.program")} optional htmlFor={`${idPrefix}-program`} refusal={refusal("program")} note={t("agentProfiles.programNote")}>
            <Input id={`${idPrefix}-program`} mono value={draft.program} readOnly={fixed} onChange={(event) => set({ program: event.target.value })} onBlur={commit} disabled={off} placeholder={t("agentProfiles.programPlaceholder", { program: view.program })} aria-invalid={refusal("program") ? true : undefined} data-profile-input="program" />
          </Field>
          <Field label={t("agentProfiles.field.env")} optional note={t("agentProfiles.envNote")} refusal={refusal("env")}>
            <div className="flex flex-col gap-xs" data-profile-env={envLoading ? "loading" : "true"}>
              {draft.env.map((pair, index) => {
                const refused = refusals.find((row) => row.field === "env" && row.envKey === pair.key)?.text ?? null;
                return (
                  // A row keeps its place while its name is typed, so it is keyed by place.
                  <div key={index} className="flex flex-col gap-xxs" data-profile-env-row={index}>
                    <span className="flex min-w-0 items-center gap-xs">
                      <Input mono aria-label={t("agentProfiles.envKey")} value={pair.key} readOnly={fixed} onChange={(event) => setEnv(index, { key: event.target.value })} onBlur={commit} disabled={off || envLoading} className="basis-2/5" aria-invalid={refused ? true : undefined} />
                      <span aria-hidden="true" className="font-mono text-muted-foreground">=</span>
                      <Input mono aria-label={t("agentProfiles.envValue")} value={pair.value} readOnly={fixed} onChange={(event) => setEnv(index, { value: event.target.value })} onBlur={commit} disabled={off || envLoading} className="flex-1" />
                      <Hint label={t("agentProfiles.envRemove", { key: pair.key })}>
                        <Button variant="ghost" size="icon-sm" disabled={off || envLoading} onClick={() => removeEnv(index)} aria-label={t("agentProfiles.envRemove", { key: pair.key })}><XIcon /></Button>
                      </Hint>
                    </span>
                    {refused ? <Note tone="error">{refused}</Note> : null}
                  </div>
                );
              })}
              <Button variant="text" size="sm" className="self-start" disabled={off || envLoading} onClick={() => set({ env: [...draft.env, { key: "", value: "" }] })} data-profile-env-add="true"><PlusIcon aria-hidden="true" />{t("agentProfiles.envAdd")}</Button>
            </div>
          </Field>
        </>
      )}
      {editing && !builtIn && !readOnly ? <p className="break-keep text-caption text-muted-foreground">{t("agentProfiles.nextStart")}</p> : null}
    </div>
  );
}

/** The edit form's own actions: make it the agent's default, delete it. The built-in row offers only the first (B28). */
function ProfileEditActions({ profile, busy = false, onMakeDefault, onDelete }: { profile: ProfileView; busy?: boolean; onMakeDefault?: () => void; onDelete?: () => void }) {
  const { t } = useInterfaceTranslation();
  return (
    <>
      {profile.isDefault ? null : <Button variant="secondary" disabled={busy} onClick={onMakeDefault} data-profile-make-default="true">{t("agentProfiles.makeDefault")}</Button>}
      {profile.builtIn ? null : <Button variant="text" className="text-destructive" disabled={busy} onClick={onDelete} data-profile-delete="true">{t("agentProfiles.delete")}</Button>}
    </>
  );
}

/** What the dialog does with the operator's presses; the gallery passes none. */
export type ProfileDialogHandlers = {
  onDraft?: (draft: ProfileDraft) => void;
  onCommit?: (draft: ProfileDraft) => void;
  onSubmit?: () => void;
  onMakeDefault?: () => void;
  onDelete?: () => void;
  onChooseFolder?: () => void;
  canChoose?: boolean;
  /** A change is on its way: the form's buttons wait. */
  busy?: boolean;
  /** The variables' values are still being read: their fields wait. */
  envLoading?: boolean;
  /** A refusal that names no field. */
  error?: string | null;
};

/** Adding or editing a profile in a dialog over the Agents list. */
export function ProfileFormDialog({ open, onOpenChange, view, mode, profile, draft, refusals, ...on }: { open: boolean; onOpenChange?: (open: boolean) => void; view: AgentProfilesView; mode: "add" | "edit"; profile?: ProfileView; draft: ProfileDraft; refusals?: readonly ProfileRefusal[] } & ProfileDialogHandlers) {
  const { t } = useInterfaceTranslation();
  const title = mode === "add" || !profile ? t("agentProfiles.addTitle", { agent: view.label }) : t("agentProfiles.editTitle", { agent: view.label, profile: profileName(profile, t) });
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent showCloseButton initialFocus={mode === "edit" ? "container" : "first"} className="w-(--size-add-device-sheet-w)" data-profile-dialog={mode}>
        <DialogHeader>
          <DialogTitle className="flex items-center gap-sm"><AgentLogo agent={view.agent} label={view.label} />{title}</DialogTitle>
        </DialogHeader>
        <DialogBody>
          <ProfileForm view={view} mode={mode} draft={draft} refusals={refusals} builtIn={profile?.builtIn} idPrefix={`profile-${mode}`} onDraft={on.onDraft} onCommit={on.onCommit} onChooseFolder={on.onChooseFolder} canChoose={on.canChoose} envLoading={on.envLoading} />
          {on.error ? <div className="pt-md"><Note tone="error" data-profile-error="true">{on.error}</Note></div> : null}
        </DialogBody>
        <DialogFooter className={mode === "edit" ? "justify-between" : undefined}>
          {mode === "add" ? (
            <>
              <Button variant="secondary" onClick={() => onOpenChange?.(false)}>{t("common.cancel")}</Button>
              <Button disabled={on.busy || (Boolean(on.onSubmit) && !draft.name.trim())} onClick={on.onSubmit} data-profile-submit="true">{t("common.add")}</Button>
            </>
          ) : profile ? <ProfileEditActions profile={profile} busy={on.busy} onMakeDefault={on.onMakeDefault} onDelete={on.onDelete} /> : null}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** An agent running with the profile, or starting with it before it has a pane to show (`starting`). */
export type InUseAgent = { id: string; title: string; place: string; asleep: boolean; starting?: boolean };

/**
 * Deleting a profile (B27): refused while agents run with it, sleeping ones
 * included, each with the way to it; otherwise what the delete takes away and
 * which projects go back to the agent default, before it does.
 */
export function ProfileDeleteDialog({ open, onOpenChange, profile, location, inUse, projects, busy = false, onConfirm, onShow }: { open: boolean; onOpenChange?: (open: boolean) => void; profile: string; location: string; inUse: readonly InUseAgent[]; projects: readonly string[]; busy?: boolean; onConfirm?: () => void; onShow?: (id: string) => void }) {
  const { t } = useInterfaceTranslation();
  const refused = inUse.length > 0;
  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent data-profile-delete-dialog={refused ? "refused" : "confirm"}>
        <AlertDialogHeader>
          <AlertDialogTitle>{refused ? t("agentProfiles.inUseTitle", { profile }) : t("agentProfiles.deleteTitle", { profile })}</AlertDialogTitle>
          <AlertDialogDescription className="break-keep">{refused ? t("agentProfiles.inUse") : t("agentProfiles.deleteBody", { location })}</AlertDialogDescription>
        </AlertDialogHeader>
        {refused ? (
          <ul className="flex flex-col divide-y divide-border rounded-md border border-border" data-profile-in-use="true">
            {inUse.map((agent, index) => (
              <li key={agent.id || `starting-${index}`} className="flex min-w-0 items-center gap-sm px-md py-sm" data-profile-in-use-starting={agent.starting ? "true" : undefined}>
                <span className="min-w-0 flex-1 truncate text-body text-foreground">
                  {agent.title}
                  {agent.place ? <span className="text-muted-foreground"> · {agent.place}</span> : null}
                </span>
                {agent.starting ? <Badge>{t("agentProfiles.starting")}</Badge> : agent.asleep ? <Badge>{t("agentProfiles.asleep")}</Badge> : null}
                {agent.starting ? null : (
                  <Button variant="text" size="inline" onClick={() => onShow?.(agent.id)} data-profile-in-use-open={agent.id}>
                    {t("agentProfiles.show")}
                  </Button>
                )}
              </li>
            ))}
          </ul>
        ) : projects.length > 0 ? (
          <div className="flex flex-col gap-xs" data-profile-delete-projects="true">
            <p className="break-keep text-body text-subtle-foreground">{t("agentProfiles.deleteProjects")}</p>
            <ul className="flex flex-col gap-xxs pl-md">
              {projects.map((project) => <li key={project} className="list-disc text-body text-foreground">{project}</li>)}
            </ul>
          </div>
        ) : null}
        <AlertDialogFooter>
          {refused ? <AlertDialogCancel>{t("common.close")}</AlertDialogCancel> : (
            <>
              <AlertDialogCancel>{t("agentProfiles.deleteKeep")}</AlertDialogCancel>
              <AlertDialogAction
                disabled={busy}
                onClick={(event) => {
                  // The dialog stays until the core answers: a delete it
                  // refuses lists what refused it here.
                  event.preventDefault();
                  onConfirm?.();
                }}
                data-profile-delete-confirm="true"
              >{t("agentProfiles.deleteConfirm", { profile })}</AlertDialogAction>
            </>
          )}
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

/** What an option in a profile menu says under its name: what is left of its account, or why it would not start signed in, and the project default mark. */
function pickDescription(profile: ProfileView, projectDefault: boolean, t: ReturnType<typeof useInterfaceTranslation>["t"]): string | undefined {
  const state = profileState(profile);
  const parts = [
    projectDefault ? t("agentProfiles.projectDefault") : null,
    state ? t(state.key as "agentProfiles.state.failed") : typeof profile.usageLeft === "number" ? t("agentProfiles.state.left", { percent: profile.usageLeft }) : profile.usageLeft === "shared" ? "—" : null,
  ].filter(Boolean);
  return parts.length ? parts.join(" · ") : undefined;
}

/**
 * The start panel's profile menu (B14): drawn right after the agent menu, only
 * when the agent has two profiles or more, with the one D-32 picked chosen.
 */
export function ProfilePicker({ view, value, projectDefault, onChange, disabled = false, defaultOpen }: { view: AgentProfilesView; value: string; projectDefault: string | null; onChange?: (id: string) => void; disabled?: boolean; defaultOpen?: boolean }) {
  const { t } = useInterfaceTranslation();
  if (view.profiles.length < 2) return null;
  return (
    <Select value={value} disabled={disabled} onValueChange={onChange} defaultOpen={defaultOpen}>
      <SelectTrigger variant="ghost" aria-label={t("agentProfiles.pick")} className="w-auto min-w-0" data-profile-pick={value}>
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {view.profiles.map((profile) => (
          <SelectItem key={profile.id} value={profile.id} description={pickDescription(profile, profile.id === projectDefault, t)} data-profile-pick-option={profile.id}>
            {profileName(profile, t)}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

/** A project's chosen profile per agent: an id, or null to follow the agent default. */
export type ProjectProfileChoice = { view: AgentProfilesView; chosen: string | null };

const FOLLOW = "__agent_default";

/**
 * Project settings › Working style › Agent accounts (B15): one row per agent
 * that has profiles on the project's device; none, and the group is absent.
 */
export function ProjectAgentAccounts({ rows, disabled = false, onChange, status }: { rows: readonly ProjectProfileChoice[]; disabled?: boolean; onChange?: (agent: string, profile: string | null) => void; status?: (agent: string) => ReactNode }) {
  const { t } = useInterfaceTranslation();
  const shown = rows.filter((row) => row.view.profiles.some((profile) => !profile.builtIn));
  if (shown.length === 0) return null;
  return (
    <Group title={t("agentProfiles.project.title")} caption={t("agentProfiles.project.caption")} note={t("agentProfiles.project.note")} data-project-agent-accounts="true">
      {shown.map(({ view, chosen }) => {
        const agentDefault = view.profiles.find((profile) => profile.isDefault) ?? view.profiles[0]!;
        return (
          <Row key={view.agent} label={<span className="flex min-w-0 items-center gap-sm"><AgentLogo agent={view.agent} label={view.label} /><span className="min-w-0 truncate">{view.label}</span></span>} detail={status?.(view.agent)} data-project-agent-account={view.agent}>
            <Select value={chosen ?? FOLLOW} disabled={disabled} onValueChange={(next) => onChange?.(view.agent, next === FOLLOW ? null : next)}>
              <SelectTrigger aria-label={t("agentProfiles.project.title")} data-project-agent-account-value={chosen ?? "follow"}><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value={FOLLOW}>{t("agentProfiles.project.follow", { profile: profileName(agentDefault, t) })}</SelectItem>
                <SelectSeparator />
                {view.profiles.map((profile) => <SelectItem key={profile.id} value={profile.id}>{profileName(profile, t)}</SelectItem>)}
              </SelectContent>
            </Select>
          </Row>
        );
      })}
    </Group>
  );
}
