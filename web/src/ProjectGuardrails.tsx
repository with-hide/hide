// Working style's Guardrails group (PRD agent-guardrails B1 to B4, B10, B15 to
// B17, B25, B29): four switches, the merge checkbox under the base branch one,
// and the protected path list under the files one. Each change sends the whole
// set of protections; the core saves it once the project's machine has it.

import { PlusIcon, XIcon } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { Group, Note, Row } from "./components/settings-rows";
import { Button } from "./components/ui/button";
import { Checkbox } from "./components/ui/checkbox";
import { Input } from "./components/ui/input";
import { useEscapeLayer } from "./components/ui/layer";
import { Switch } from "./components/ui/switch";
import { useInterfaceTranslation } from "./i18n/client";
import { guardPathRefusal, type Guardrails } from "./projectSettings";
import type { KitAgent } from "./snapshot";
import { composingKey } from "./keys";

/** The control a change came from, so its saving state shows beside that row. */
export type GuardPart = "worktree_folder" | "base_branch" | "agent_merge" | "git_commands" | "files" | "paths";

const SWITCHES = ["worktree_folder", "base_branch", "agent_merge", "git_commands", "files"] as const;

/** Which control a failed change moved, for a receipt read back without its intent; none for one that saved, whose
 * sent value is the confirmed one. */
export function guardPartOf(sent: Guardrails, confirmed: Guardrails): GuardPart | undefined {
  const moved = SWITCHES.find((key) => sent[key] !== confirmed[key]);
  if (moved) return moved;
  const same = sent.paths.length === confirmed.paths.length && sent.paths.every((path, index) => path === confirmed.paths[index]);
  return same ? undefined : "paths";
}

export function ProjectGuardrails({ rules, isGit, base, disabled, unguarded, save, stateFor, onOpenAgents }: {
  rules: Guardrails;
  isGit: boolean;
  /** The branch the base protection guards; null when the project names none. */
  base: string | null;
  disabled: boolean;
  unguarded: KitAgent[];
  save: (part: GuardPart, rules: Guardrails) => void;
  stateFor: (part: GuardPart) => ReactNode;
  onOpenAgents: () => void;
}) {
  const { t } = useInterfaceTranslation();
  const [adding, setAdding] = useState(false);
  const [draft, setDraft] = useState("");
  const [refusal, setRefusal] = useState<string | null>(null);
  // The path sent and not yet confirmed: its field stays open with it, so a refusal shows where it was typed (B16).
  const [sent, setSent] = useState<string | null>(null);
  const addButton = useRef<HTMLButtonElement>(null);
  const field = useRef<HTMLInputElement>(null);
  const change = (part: GuardPart, next: Partial<Guardrails>) => save(part, { ...rules, ...next });
  useEffect(() => {
    if (sent === null || !rules.paths.includes(sent)) return;
    setSent(null);
    setDraft("");
    setAdding(false);
  }, [rules.paths, sent]);
  // Focus returns to Add path once the field closes or a chip is removed and the save that disabled it settles,
  // rather than to the page.
  const refocus = useRef(false);
  useEffect(() => {
    if (!adding && !disabled && refocus.current) {
      refocus.current = false;
      addButton.current?.focus();
    }
  });
  const cancel = () => {
    refocus.current = true;
    setAdding(false);
    setDraft("");
    setSent(null);
    setRefusal(null);
  };
  // Escape leaves the field and keeps the dialog open: the shell answers Escape with the innermost layer.
  useEscapeLayer(adding, cancel, false, true);
  // A refused add leaves the field open with what was typed; the save that disabled it no longer holds focus.
  const wasDisabled = useRef(disabled);
  useEffect(() => {
    if (wasDisabled.current && !disabled && adding) field.current?.focus();
    wasDisabled.current = disabled;
  }, [disabled, adding]);
  // A plain folder's three Git protections are off and unavailable, said once (B17).
  const gitReason = isGit ? null : <Note>{t("workspace.unavailable.notGit")}</Note>;
  const add = () => {
    const path = draft.trim();
    const refused = guardPathRefusal(rules.paths, path);
    if (refused) { setRefusal(refused); return; }
    setRefusal(null);
    setSent(path);
    refocus.current = true;
    change("paths", { paths: [...rules.paths, path] });
  };
  const row = (part: (typeof SWITCHES)[number], label: string, note: ReactNode, extra?: ReactNode, unavailable = false) =>
    <Row label={label} data-project-guard={part} detail={<>{note}{extra}{stateFor(part)}</>}>
      <Switch aria-label={label} checked={rules[part]} disabled={disabled || unavailable} onCheckedChange={(checked) => change(part, { [part]: checked })} />
    </Row>;
  return <Group title={t("projectSettings.guard")} caption={t("projectSettings.guardCaption")} data-project-guardrails="true"
    note={unguarded.length > 0 ? <span className="flex flex-wrap items-baseline gap-x-sm gap-y-xs" data-project-guard-uncovered="true">
      {t("projectSettings.guardUncovered", { agents: unguarded.map((agent) => agent.label).join(", ") })}
      <Button variant="link" size="sm" className="h-auto p-none" onClick={onOpenAgents}>{t("projectSettings.guardAgents")}</Button>
    </span> : undefined}>
    {row("worktree_folder", t("projectSettings.guardWorktree"), <><Note>{t("projectSettings.guardWorktreeNote")}</Note>{gitReason}</>, null, !isGit)}
    {row("base_branch", t("projectSettings.guardBase"),
      !isGit ? <Note>{t("projectSettings.guardBaseNoteAny")}</Note> : base ? <Note>{t("projectSettings.guardBaseNote", { branch: base })}</Note> : <Note tone="warn">{t("projectSettings.guardBaseUnknown")}</Note>,
      rules.base_branch ? <div className="mt-sm flex flex-col gap-xs border-t border-border pt-sm" data-project-guard="agent_merge">
        <label className="flex items-center gap-sm text-body text-foreground">
          <Checkbox checked={rules.agent_merge} disabled={disabled || !isGit} onCheckedChange={(checked) => change("agent_merge", { agent_merge: checked === true })} />
          <span className="min-w-0 break-keep">{t("projectSettings.guardMerge")}</span>
        </label>
        {stateFor("agent_merge")}
      </div> : null,
      !isGit || (!base && !rules.base_branch))}
    {row("git_commands", t("projectSettings.guardGit"), <Note>{t("projectSettings.guardGitNote")}</Note>, null, !isGit)}
    {row("files", t("projectSettings.guardFiles"), <Note>{t("projectSettings.guardFilesNote")}</Note>, rules.files ? <div className="mt-sm flex flex-col gap-xs">
      <ul className="flex min-w-0 flex-wrap items-center gap-xs" aria-label={t("projectSettings.guardFiles")}>
        {rules.paths.map((path) => <li key={path} className="inline-flex min-w-0 max-w-full items-center gap-xxs rounded-xs border border-border bg-secondary px-xs font-mono text-caption text-foreground" data-project-guard-path={path}>
          <span className="min-w-0 break-all">{path}</span>
          <button type="button" aria-label={t("projectSettings.guardPathRemove", { path })} disabled={disabled} onClick={() => { refocus.current = true; change("paths", { paths: rules.paths.filter((held) => held !== path) }); }}
            className="inline-flex size-(--size-icon) shrink-0 items-center justify-center rounded-xs text-muted-foreground outline-none hover:bg-accent hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-(--opacity-disabled)"><XIcon aria-hidden="true" className="size-(--size-icon-sm)" /></button>
        </li>)}
        {!adding ? <li><Button ref={addButton} variant="ghost" size="sm" disabled={disabled} onClick={() => setAdding(true)}><PlusIcon aria-hidden="true" />{t("projectSettings.guardPathAdd")}</Button></li> : null}
      </ul>
      {adding ? <div className="flex min-w-0 max-w-full flex-wrap gap-xs">
        <Input ref={field} autoFocus aria-label={t("projectSettings.guardPathAdd")} placeholder={t("projectSettings.guardPathHint")} value={draft} disabled={disabled}
          className="w-(--size-settings-control-w) max-w-full font-mono"
          onChange={(event) => { setDraft(event.target.value); setRefusal(null); setSent(null); }}
          onKeyDown={(event) => { if (!composingKey(event) && event.key === "Enter") add(); }} />
        <Button size="sm" disabled={disabled} onClick={add}>{t("projectSettings.apply")}</Button>
      </div> : null}
      {refusal ? <Note tone="error">{t(pathRefusalKey(refusal))}</Note> : null}
      {stateFor("paths")}
    </div> : null)}
  </Group>;
}

export function pathRefusalKey(code: string) {
  switch (code) {
    case "project_settings_paths_limit": return "projectSettings.errorPathsLimit" as const;
    case "project_settings_path_too_long": return "projectSettings.errorPathTooLong" as const;
    case "project_settings_path_duplicate": return "projectSettings.errorPathDuplicate" as const;
    default: return "projectSettings.errorPathInvalid" as const;
  }
}
