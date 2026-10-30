import { BriefcaseBusinessIcon, ChevronDownIcon } from "lucide-react";
import { useEffect, useState } from "react";
import type { Actions } from "./actions";
import type { AgentLayout } from "./agentLayout";
import { EntryDropdown, type MenuEntry } from "./components/entry-menu";
import { coreErrorText } from "./coreError";
import { Button } from "./components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "./components/ui/dialog";
import { Hint } from "./components/ui/tooltip";
import { Input } from "./components/ui/input";
import { groupNameProblem, useGroupName, useGroupFeedback } from "./groupTabs";
import { useInterfaceTranslation } from "./i18n/client";
import { catalogWorkspaces } from "./snapshot";
import { useShellStore } from "./store";
import type { ViewWorkspace } from "./viewLayout";
import { composingKey } from "./keys";

export function AgentGroupSelector({ layout, workspace, actions }: { layout: AgentLayout; workspace: ViewWorkspace; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const requested = useGroupFeedback((state) => state.request);
  const error = useShellStore((state) => state.rest?.status?.last_error);
  const failure = requested?.workspace.device_id === workspace.device_id && requested.workspace.path === workspace.path && error?.request_id === requested.id ? coreErrorText(error, t, "groups.saveFailed") : null;
  const groups = layout.groups ?? [];
  const selected = layout.group_id ?? null;
  const name = groups.find((group) => group.id === selected)?.name ?? t("groups.ungrouped");
  const items: MenuEntry[] = groups.map((group) => ({ id: `select:${group.id ?? ""}`, label: group.name ?? t("groups.ungrouped"), unavailable: null }));
  items.push({ id: "create", label: t("groups.create"), icon: "add", separated: true, unavailable: groups.filter((group) => group.id !== null).length >= 32 ? t("groups.limit") : null });
  if (selected !== null) items.push({ id: "rename", label: t("groups.rename"), icon: "rename", unavailable: null }, { id: "dissolve", label: t("groups.dissolve"), unavailable: null });
  if (layout.can_undo_group) items.push({ id: "undo", label: t("groups.undo"), unavailable: null, separated: true });
  return <><EntryDropdown label={t("groups.select")} title={name} hint={name} items={items} onSelect={(id) => {
    if (id.startsWith("select:")) actions.agentGroup(workspace, id.slice(7) || null, { action: "select" });
    else if (id === "create" || id === "rename") useGroupName.getState().open({ workspace, groupId: id === "rename" ? selected : null, name: id === "rename" ? name : "" });
    else actions.agentGroup(workspace, selected, { action: id });
  }} trigger={<button type="button" data-agent-group-selector={selected ?? "ungrouped"} aria-label={`${t("groups.select")}: ${name}`} className="flex min-w-0 max-w-(--size-tab-preferred) shrink items-center gap-xs border-r border-border px-sm text-caption text-foreground outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-ring"><BriefcaseBusinessIcon className="size-(--size-icon-sm) shrink-0" /><span className="truncate">{name}</span><ChevronDownIcon className="size-(--size-icon-sm) shrink-0" /></button>} />{failure ? <Hint label={failure} reveals><span role="alert" data-core-error={error?.kind} className="my-auto max-w-(--size-tab-preferred) truncate px-xs text-caption text-destructive">{failure}</span></Hint> : null}</>;
}

export function GroupNameDialog({ actions }: { actions: Actions }) {
  const intent = useGroupName((state) => state.intent);
  return intent ? <NameForm key={JSON.stringify(intent)} actions={actions} intent={intent} /> : null;
}

function NameForm({ actions, intent }: { actions: Actions; intent: NonNullable<ReturnType<typeof useGroupName.getState>["intent"]> }) {
  const { t } = useInterfaceTranslation();
  const [name, setName] = useState(intent.name);
  const [submitted, setSubmitted] = useState(false);
  const [request, setRequest] = useState<string | null>(null);
  const [timedOut, setTimedOut] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const [errorBefore, setErrorBefore] = useState<number | null>(null);
  const rest = useShellStore((state) => state.rest);
  const groups = catalogWorkspaces(rest).filter((workspace) => workspace.device_id === intent.workspace.device_id).flatMap((workspace) => workspace.checkouts).find((checkout) => checkout.path === intent.workspace.path)?.agent_groups?.groups ?? [];
  const problem = groupNameProblem(name, groups, intent.groupId);
  const failed = request !== null && rest?.status?.last_error?.request_id === request && rest.status.last_error.occurred_at !== errorBefore;
  const pending = request !== null && !failed && !timedOut;
  useEffect(() => {
    if (!request) return;
    const timer = window.setTimeout(() => setTimedOut(true), 90_000);
    return () => window.clearTimeout(timer);
  }, [request, attempt]);
  useEffect(() => {
    if (!request || failed) return;
    const result = groups.find((group) => group.name === name.trim() && (intent.groupId === null || group.id === intent.groupId));
    if (result) useGroupName.getState().close();
  }, [request, failed, groups, name, intent.groupId]);
  const submit = () => {
    setSubmitted(true);
    if (problem || pending) return;
    if (intent.groupId !== null && name.trim() === intent.name) return useGroupName.getState().close();
    setTimedOut(false);
    setAttempt((value) => value + 1);
    setErrorBefore(rest?.status?.last_error?.occurred_at ?? null);
    setRequest(actions.agentGroup(intent.workspace, intent.groupId, { action: intent.groupId === null ? "create" : "rename", name, ...(intent.tabId ? { tab_id: intent.tabId } : {}) }, request ?? undefined));
  };
  return <Dialog open onOpenChange={(open) => { if (!open) useGroupName.getState().close(); }}><DialogContent aria-describedby={undefined}>
    <DialogHeader><DialogTitle>{t(intent.groupId === null ? "groups.createTitle" : "groups.rename")}</DialogTitle></DialogHeader>
    <DialogBody><Input autoFocus value={name} readOnly={pending} aria-label={t("groups.name")} aria-invalid={submitted && !!problem} onChange={(event) => { setName(event.target.value); setRequest(null); setTimedOut(false); }} onKeyDown={(event) => { if (event.key === "Enter" && !composingKey(event)) { event.preventDefault(); submit(); } }} />
      {timedOut ? <p role="status" className="mt-xs text-caption text-muted-foreground">{t("groups.saveUnknown")}</p> : null}
      {submitted && problem ? <p role="alert" className="mt-xs text-caption text-destructive">{t(problem)}</p> : failed ? <p role="alert" data-core-error={rest?.status?.last_error?.kind} className="mt-xs text-caption text-destructive">{rest?.status?.last_error ? coreErrorText(rest.status.last_error, t, "groups.saveFailed") : t("groups.saveFailed")}</p> : null}
    </DialogBody><DialogFooter><Button variant="ghost" onClick={() => useGroupName.getState().close()}>{t("common.cancel")}</Button><Button disabled={pending} onClick={submit}>{t("common.save")}</Button></DialogFooter>
  </DialogContent></Dialog>;
}
