import { BrainIcon, FolderGit2Icon, FolderIcon, GitBranchIcon, LinkIcon, XIcon } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import type { Actions } from "./actions";
import { Group, Note, Row, Status, Value } from "./components/settings-rows";
import { Button } from "./components/ui/button";
import { Dialog, DialogContent, DialogTitle } from "./components/ui/dialog";
import { Input } from "./components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "./components/ui/select";
import { Switch } from "./components/ui/switch";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "./components/ui/tabs";
import { Hint } from "./components/ui/tooltip";
import { deviceConnected } from "./devices";
import { useInterfaceTranslation } from "./i18n/client";
import { MemorySettings, useMemoryProject } from "./MemoryTab";
import { guardPartOf, ProjectGuardrails, pathRefusalKey, type GuardPart } from "./ProjectGuardrails";
import { guardedBase, projectBaseBranch, projectGuardrails, projectIssueSource, projectRepository, projectSettings, readableSettings, refusedForGood, unguardedAgents, type Guardrails, type ProjectSettingsPatch, type ProjectSettingsTarget } from "./projectSettings";
import { issueSourceChoices, issueSourceName } from "./settings";
import { catalogWorkspaces } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { composingKey } from "./keys";

type Page = "connections" | "work" | "memory";
const PAGES = [{ id: "connections", icon: LinkIcon }, { id: "work", icon: GitBranchIcon }, { id: "memory", icon: BrainIcon }] as const;

export function ProjectSettingsSheet({ actions, initialPage = "connections" }: { actions: Actions; initialPage?: Page }) {
  const target = useUiStore((state) => state.projectSettingsTarget);
  return target ? <ProjectSettingsBody key={`${target.deviceId}:${target.root}`} target={target} actions={actions} initialPage={initialPage} /> : null;
}

function ProjectSettingsBody({ target, actions, initialPage }: { target: ProjectSettingsTarget; actions: Actions; initialPage: Page }) {
  const { t } = useInterfaceTranslation();
  const rest = useShellStore((state) => state.rest);
  const live = useShellStore((state) => state.connection === "live");
  const current = catalogWorkspaces(rest).find((row) => row.device_id === target.deviceId && row.path === target.root && !row.is_home) ?? null;
  const lastWorkspace = useRef(current);
  if (current) lastWorkspace.current = current;
  const workspace = current ?? lastWorkspace.current;
  const available = live && current !== null && deviceConnected(rest, target.deviceId);
  const [page, setPage] = useState<Page>(initialPage);
  const [intent, setIntent] = useState<{ id: string; field: keyof ProjectSettingsPatch; part?: GuardPart; patch: ProjectSettingsPatch; state: "pending" | "saved" | "failed" } | null>(null);
  const { record, write } = projectSettings(rest, target, intent?.id);
  const waiting = useRef(false);
  const pending = write?.state === "pending" || (intent !== null && intent.state === "pending" && write?.request_id !== intent.id);
  useEffect(() => {
    if (write && write.request_id === intent?.id && write.state !== "pending") {
      waiting.current = false;
      const state = write.state;
      setIntent((current) => current && current.state !== state ? { ...current, state } : current);
    }
  }, [write, intent?.id]);
  const blocked = !available || pending || !readableSettings(record);
  const previous = useRef<HTMLElement | null>(useUiStore.getState().projectSettingsReturnFocus);
  const close = () => useUiStore.getState().closeOverlay("project_settings");
  const save = (field: keyof ProjectSettingsPatch, patch: ProjectSettingsPatch, part?: GuardPart) => {
    if (blocked || waiting.current) return;
    const id = crypto.randomUUID();
    waiting.current = true;
    const sent = actions.setProjectSettings(target, patch, id);
    if (!sent) waiting.current = false;
    setIntent({ id, field, part, patch, state: sent ? "pending" : "failed" });
  };
  const guardrails = projectGuardrails(record);
  // A receipt read back without its intent (the dialog reopened) is shown beside the control it moved.
  const receiptPart = (patch: ProjectSettingsPatch | undefined) => patch?.guardrails ? guardPartOf(patch.guardrails, guardrails) : undefined;
  const stateFor = (field: keyof ProjectSettingsPatch, part?: GuardPart): ReactNode => {
    const retry = intent?.field === field ? intent.part === part ? intent : null
      : write?.patch?.[field] !== undefined && receiptPart(write.patch) === part ? { id: write.request_id, patch: write.patch } : null;
    if (!retry) return null;
    const state = write?.request_id === retry.id ? write.state : intent?.state ?? "pending";
    return <div className="flex flex-wrap items-center gap-sm" aria-live="polite">
      <Status tone={state === "failed" ? "error" : state === "pending" ? "pending" : "ok"}>{t(`projectSettings.${state === "failed" ? "failed" : state === "pending" ? "pending" : "saved"}`)}</Status>
      {state === "failed" ? <Note tone="error">{t(settingsErrorKey(write?.error))}</Note> : null}
      {state === "failed" && !refusedForGood(write?.error) ? <Button size="sm" variant="ghost" disabled={!available} onClick={() => setIntent({ id: retry.id, field, part, patch: retry.patch, state: actions.setProjectSettings(target, retry.patch, retry.id) ? "pending" : "failed" })}>{t("projectSettings.retry")}</Button> : null}
    </div>;
  };
  useMemoryProject(actions, available ? current.id : null);
  const memory = useShellStore((state) => {
    const opened = state.memory?.project;
    return opened && opened.workspace_id === workspace?.id ? opened : null;
  });
  const [repositoryDraft, setRepositoryDraft] = useState(record?.repository?.mode === "repository" ? record.repository.repository : "");
  const [repositoryEditing, setRepositoryEditing] = useState(record?.repository?.mode === "repository");
  const [repositoryInvalid, setRepositoryInvalid] = useState(false);
  const confirmedRepository = record?.repository?.mode === "repository" ? record.repository.repository : null;
  useEffect(() => {
    if (write?.state === "pending") return;
    setRepositoryDraft(confirmedRepository ?? "");
    setRepositoryEditing(confirmedRepository !== null);
  }, [confirmedRepository, write?.state, intent?.state]);
  const branches = workspace?.branches ?? [];
  const base = workspace ? projectBaseBranch(rest, workspace) : { branch: "", missing: null };
  const baseChoice = record?.base_branch;
  const selectedBase = baseChoice?.mode === "branch" ? baseChoice.branch : baseChoice?.mode === "auto" ? null : rest?.ui_state?.project_base_branches?.[target.root];
  const baseValue = !selectedBase || base.missing ? "auto" : `branch:${selectedBase}`;
  const source = workspace ? issueSourceChoices(workspace, projectIssueSource(rest, workspace), t) : null;
  const resolvedSource = workspace?.tasks?.source;
  const usable = workspace?.checkouts.some((checkout) => checkout.exists) ?? false;
  const applyRepository = () => {
    const value = repositoryDraft.trim();
    if (value.length > 256 || !/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(value)) { setRepositoryInvalid(true); return; }
    setRepositoryInvalid(false);
    save("repository", { repository: { mode: "repository", repository: value } });
  };
  return <Dialog open onOpenChange={(next) => { if (!next) close(); }}>
    <DialogContent data-project-settings="true" aria-describedby={undefined} returnFocusTo={() => previous.current}
      className="h-(--size-project-settings-sheet-h) max-h-[calc(100%-2*var(--size-settings-sheet-window-inset))] w-(--size-settings-sheet-w) max-w-[calc(100%-2*var(--size-settings-sheet-window-inset))]">
      <Tabs value={page} onValueChange={(value) => setPage(value as Page)} orientation="vertical" className="min-h-0 flex-1 flex-row gap-none">
        <nav className="flex min-h-0 w-(--size-settings-nav-w) shrink-0 flex-col overflow-hidden border-r border-border bg-sidebar px-sm pb-md pt-sm">
          <DialogTitle className="shrink-0 px-sm pb-xs text-title font-semibold">{t("projectSettings.title")}</DialogTitle>
          <div className="min-w-0 px-sm py-sm">
            <p className="flex items-start gap-xs text-body font-medium" data-project-settings-name="true">{workspace?.is_git ? <FolderGit2Icon aria-hidden="true" className="size-(--size-icon) shrink-0" /> : <FolderIcon aria-hidden="true" className="size-(--size-icon) shrink-0" />}<span className="line-clamp-2 min-w-0 break-words" title={workspace?.label ?? target.label}>{workspace?.label ?? target.label}</span></p>
            <p className="line-clamp-2 break-all font-mono text-caption text-muted-foreground" title={target.root}>{target.root}</p>
          </div>
          <TabsList aria-label={t("projectSettings.section")} className="min-h-0 w-full flex-1 flex-col items-stretch justify-start gap-xxs rounded-none bg-transparent p-none">
            {PAGES.map(({ id, icon: Icon }) => <TabsTrigger key={id} value={id} className="min-h-(--size-control) w-full shrink-0 justify-start gap-sm whitespace-normal rounded-xs px-sm py-xs text-body data-[state=active]:bg-accent data-[state=active]:shadow-none"><Icon aria-hidden="true" className="size-(--size-icon) shrink-0" />{t(`projectSettings.${id}`)}</TabsTrigger>)}
          </TabsList>
          <p className="shrink-0 break-words px-sm pt-sm text-caption text-muted-foreground">{t("projectSettings.scope")}</p>
        </nav>
        <div className="flex min-w-0 flex-1 flex-col">
          <header className="flex shrink-0 items-center gap-sm px-xl pb-lg pt-lg">
            <h2 className="min-w-0 flex-1 text-headline font-semibold">{t(`projectSettings.${page}`)}</h2>
            <Hint label={t("settings.close")} shortcut="Esc"><Button variant="ghost" size="icon-sm" aria-label={t("settings.close")} onClick={close}><XIcon /></Button></Hint>
          </header>
          <TabsContent value={page} className="min-h-0 overflow-y-auto overflow-x-hidden px-xl pb-xl" data-project-settings-page={page}>
            <div className="flex min-w-0 flex-col gap-lg">
              {!available ? <Note tone="warn">{t("projectSettings.unavailable")} <Button variant="ghost" size="sm" disabled={!live} onClick={() => { const device = rest?.navigator?.devices?.find((row) => row.id === target.deviceId); if (device?.kind === "remote") actions.retryDevice(target.deviceId); else actions.refreshProjectOverview(target.workspaceId); }}>{t("projectSettings.retry")}</Button></Note> : null}
              {!readableSettings(record) ? <Note tone="warn">{t("projectSettings.newerVersion")}</Note> : null}
              {page === "connections" ? <>
                <Group title={t("projectSettings.repository")} note={t("projectSettings.repositoryNote")}>
                  <Row label={t("projectSettings.repository")} detail={<>{workspace?.is_git ? <Value>{(workspace && projectRepository(rest, workspace)) ?? t("projectSettings.repositoryMissing")}</Value> : <Note>{t("projectSettings.folder")}</Note>}{stateFor("repository")}</>}>
                    <Select value={confirmedRepository !== null ? "repository" : "auto"} disabled={blocked || !workspace?.is_git} onValueChange={(value) => { if (value === "repository") setRepositoryEditing(true); if (value === "auto") save("repository", { repository: { mode: "auto" } }); }}>
                      <SelectTrigger aria-label={t("projectSettings.repository")}><SelectValue /></SelectTrigger><SelectContent><SelectItem value="auto">{t("projectSettings.automatic")}</SelectItem><SelectItem value="repository">{t("projectSettings.override")}</SelectItem></SelectContent>
                    </Select>
                  </Row>
                  {repositoryEditing ? <Row label={t("projectSettings.override")} detail={repositoryInvalid ? <Note tone="error">{t("projectSettings.repositoryInvalid")}</Note> : undefined}>
                    <div className="flex min-w-0 max-w-full flex-wrap gap-xs"><Input aria-label={t("projectSettings.repositoryHint")} placeholder={t("projectSettings.repositoryHint")} value={repositoryDraft} disabled={blocked} onChange={(event) => { setRepositoryDraft(event.target.value); setRepositoryInvalid(false); }} onKeyDown={(event) => { if (event.key === "Enter" && !composingKey(event)) applyRepository(); }} /><Button size="sm" disabled={blocked} onClick={applyRepository}>{t("projectSettings.apply")}</Button></div>
                  </Row> : null}
                </Group>
                <Group title={t("issueSettings.sourceMenu")}>
                  <Row label={t("issueSettings.sourceMenu")} detail={<>
                    <Note tone={resolvedSource?.failure ? "error" : undefined}>
                      {resolvedSource ? resolvedSource.failure ? t("board.sourceReadFailed", { source: issueSourceName(resolvedSource, t) }) : resolvedSource.reading ? t("board.reading") : issueSourceName(resolvedSource, t) : t(available ? "board.reading" : "settings.unavailable")}
                    </Note>
                    {resolvedSource?.failure ? <Button size="sm" variant="ghost" disabled={!available} onClick={() => actions.refreshProjectOverview(target.workspaceId)}>{t("projectSettings.retry")}</Button> : null}
                    {workspace?.checkouts.some((checkout) => checkout.github?.failure_category === "not_logged_in") ? <Button size="sm" variant="ghost" onClick={() => actions.openSettings("github")}>{t("projectSettings.accountSettings")}</Button> : null}
                    {!workspace?.is_git ? <Note>{t("projectSettings.folder")}</Note> : null}
                    {stateFor("issue_source")}
                  </>}>
                    <Select value={workspace && !workspace.is_git && projectIssueSource(rest, workspace) === "github" ? "github" : source?.value ?? "auto"} disabled={blocked || !workspace} onValueChange={(value) => save("issue_source", { issue_source: value as "auto" | "github" | "local" })}><SelectTrigger aria-label={t("issueSettings.sourceMenu")}><SelectValue /></SelectTrigger><SelectContent>{!workspace?.is_git ? <SelectItem value="github" disabled>GitHub</SelectItem> : null}{source?.options.map((option) => <SelectItem key={option.id} value={option.id}>{option.label}</SelectItem>)}</SelectContent></Select>
                  </Row>
                </Group>
                <Group title={t("projectSettings.linearName")}><Row label={t("projectSettings.linearName")} detail={<Note>{t("projectSettings.linear")}</Note>}><Button size="sm" variant="outline" disabled>{t("projectSettings.linear")}</Button></Row></Group>
              </> : null}
              {page === "work" ? <>
                <Group title={t("projectSettings.work")}>
                  <Row label={t("projectSettings.newWork")} detail={<><Note>{t("projectSettings.newWorkNote")}</Note>{stateFor("new_work")}</>}>
                    <Select value={record?.new_work ?? "checkout"} disabled={blocked} onValueChange={(value) => save("new_work", { new_work: value as "checkout" | "worktree" })}><SelectTrigger aria-label={t("projectSettings.newWork")}><SelectValue /></SelectTrigger><SelectContent><SelectItem value="checkout">{t("projectSettings.checkout")}</SelectItem><SelectItem value="worktree" disabled={!workspace?.is_git}>{t("projectSettings.worktree")}</SelectItem></SelectContent></Select>
                  </Row>
                  <Row label={t("projectSettings.base")} detail={<><Note>{workspace?.is_git ? t("projectSettings.baseNote") : t("workspace.unavailable.notGit")}</Note>{base.missing ? <Note tone="warn">{t("projectSettings.branchMissing", { branch: base.missing, fallback: base.branch })}</Note> : null}{stateFor("base_branch")}</>}>
                    <Select value={baseValue} disabled={blocked || !workspace?.is_git} onValueChange={(value) => save("base_branch", { base_branch: value === "auto" ? { mode: "auto" } : { mode: "branch", branch: value.slice("branch:".length) } })}><SelectTrigger aria-label={t("projectSettings.base")}><SelectValue /></SelectTrigger><SelectContent><SelectItem value="auto">{t("projectSettings.automatic")}{base.branch ? ` · ${base.branch}` : ""}</SelectItem>{branches.map((branch) => <SelectItem key={branch} value={`branch:${branch}`}>{branch}</SelectItem>)}</SelectContent></Select>
                  </Row>
                  <Row label={t("projectSettings.autoFetch")} detail={<><Note>{workspace?.is_git ? t("projectSettings.autoFetchNote") : t("workspace.unavailable.notGit")}</Note>{stateFor("auto_fetch")}</>}>
                    <Switch aria-label={t("projectSettings.autoFetch")} checked={workspace?.is_git ? record?.auto_fetch ?? true : false} disabled={blocked || !workspace?.is_git} onCheckedChange={(checked) => save("auto_fetch", { auto_fetch: checked })} />
                  </Row>
                </Group>
                <ProjectGuardrails rules={guardrails} isGit={workspace?.is_git ?? false} base={workspace ? guardedBase(rest, workspace, record) : null} disabled={blocked}
                  unguarded={unguardedAgents(rest?.navigator?.devices?.find((device) => device.id === target.deviceId)?.kit?.agents)}
                  save={(part, rules: Guardrails) => save("guardrails", { guardrails: rules }, part)} stateFor={(part) => stateFor("guardrails", part)}
                  onOpenAgents={() => { close(); actions.openSettings("agents"); }} />
              </> : null}
              {page === "memory" ? <Group title={t("projectSettings.memory")}>
                <Row label={t("projectSettings.memory")} detail={<Note>{t("projectSettings.memoryNote")}</Note>}>
                  {memory ? <MemorySettings project={memory} actions={actions} disabled={!available} /> : <Status tone="pending">{t("memory.reading")}</Status>}
                </Row>
                <Row label={memory ? t("projectSettings.cards", { count: memory.rows.length }) : t("projectSettings.reading")} detail={!usable ? <Note>{t("projectSettings.noCheckout")}</Note> : undefined}><Button size="sm" variant="outline" className="h-auto min-h-(--size-control-sm) min-w-0 max-w-full whitespace-normal break-words py-xs" disabled={!available || !usable} onClick={() => { if (!workspace || !available) return; close(); actions.openProjectMemory(target.deviceId, workspace.id); }}>{t("projectSettings.memoryOpen")}</Button></Row>
              </Group> : null}
            </div>
          </TabsContent>
        </div>
      </Tabs>
    </DialogContent>
  </Dialog>;
}

function settingsErrorKey(code: string | null | undefined) {
  switch (code) {
    case "project_settings_busy": return "projectSettings.errorBusy" as const;
    case "project_settings_branch_unavailable": return "projectSettings.errorBranch" as const;
    case "project_settings_newer_version": return "projectSettings.newerVersion" as const;
    case "project_settings_invalid_repository": return "projectSettings.repositoryInvalid" as const;
    case "project_settings_not_git": return "projectSettings.folder" as const;
    case "project_settings_limit": case "project_settings_link_limit": return "projectSettings.errorLimit" as const;
    case "project_settings_device_unavailable": case "project_settings_project_unavailable": case "project_settings_project_missing": return "projectSettings.unavailable" as const;
    case "project_settings_paths_limit": case "project_settings_path_too_long": case "project_settings_path_duplicate": case "project_settings_path_invalid": return pathRefusalKey(code);
    case "project_settings_base_unknown": return "projectSettings.errorBaseUnknown" as const;
    case "project_settings_policy_failed": return "projectSettings.errorPolicy" as const;
    default: return "projectSettings.errorSave" as const;
  }
}
