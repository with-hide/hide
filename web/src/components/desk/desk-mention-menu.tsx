import { ChevronDownIcon, ChevronLeftIcon, ChevronRightIcon, HouseIcon, PlusIcon } from "lucide-react";
import type { MentionProject, MentionRow, MentionSession, MentionTree } from "../../deskComposer";
import { AgentMark } from "../../AgentMark";
import { useInterfaceTranslation } from "../../i18n/client";
import { cn } from "../../lib/utils";
import { Elapsed } from "../elapsed";
import { SessionRowBody } from "../session-row";
import { StatusMark } from "../status-mark";
import { Command, CommandGroup, CommandItem, CommandList, CommandSeparator, CommandShortcut } from "../ui/command";
import { Button } from "../ui/button";
import { Kbd } from "../ui/kbd";

export type { MentionProject, MentionSession } from "../../deskComposer";

/** How far a child row sits in under its parent, by depth (D-84); a deeper one stays at the last. */
const INDENT = ["", "pl-xl", "pl-xxl", "pl-xxxl"] as const;

/**
 * The `@` menu (D-70, D-84, B18, B48): the sessions as a tree of two-line
 * rows, parents first with their children folded under `▸ 하위 n`, then the
 * projects, all filtered by what follows `@`; a search adds a `하위 세션`
 * group of the children it found, each with its parent's name. `↵` on a
 * session hands the task to it, on a project starts a new thread there, and
 * `⇥` on a project enters it: then the menu lists its running sessions under
 * `＋ 새 스레드`, and `⇧⇥` or Esc goes back. `active` is the highlighted
 * row's value, `session:<pane>` or `project:<path>`; the composer moves it
 * and folds a row with → and ←, and pressing `하위 n` folds it here.
 */
export function DeskMentionMenu({ tree, searching = false, entered = null, active, onActive, onSession, onProject, onFold, onBack }: {
  tree: MentionTree;
  /** Words follow `@`: the session group is the search's. */
  searching?: boolean;
  /** The project entered with ⇥, and its running sessions. */
  entered?: { project: MentionProject; sessions: readonly MentionSession[] } | null;
  active: string;
  onActive: (value: string) => void;
  onSession: (session: MentionSession) => void;
  onProject: (project: MentionProject) => void;
  onFold: (session: MentionSession, open: boolean) => void;
  onBack: () => void;
}) {
  const { t } = useInterfaceTranslation();
  const session = (row: MentionRow) => {
    const one = row.session;
    const line = [one.project, one.branch, one.detail].filter((part): part is string => Boolean(part)).join(" · ");
    return (
      <CommandItem key={one.id} value={`session:${one.id}`} onSelect={() => onSession(one)} className={cn("items-start py-xxs", INDENT[Math.min(row.depth, INDENT.length - 1)])} data-desk-mention-session={one.id} data-desk-mention-depth={row.depth}>
        <SessionRowBody
          mark={<><StatusMark mark={one.mark} /><AgentMark kind={one.agent_kind} /></>}
          head={<>
            <span className="min-w-0 flex-auto truncate font-medium">
              {row.crumb ? <span className="font-normal text-muted-foreground" data-desk-mention-crumb="true">{t("desk.mention.crumb", { name: row.crumb })}</span> : null}
              {one.name}
            </span>
            {row.children > 0 ? (
              <Button
                variant="text"
                size="inline"
                tabIndex={-1}
                className="shrink-0 gap-xxs text-micro text-muted-foreground"
                aria-expanded={row.open}
                aria-label={t(row.open ? "desk.mention.closeChildren" : "desk.mention.openChildren", { name: one.name, count: row.children })}
                onPointerDown={(event) => event.preventDefault()}
                onClick={(event) => {
                  event.stopPropagation();
                  onFold(one, !row.open);
                }}
                data-desk-mention-fold={row.open ? "open" : "closed"}
              >
                {row.open ? <ChevronDownIcon aria-hidden="true" className="size-(--size-icon-sm)" /> : <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm)" />}
                {t("desk.mention.children", { count: row.children })}
              </Button>
            ) : <Elapsed since={one.last_prompt_at_unix_ms} className="shrink-0 font-mono text-micro text-muted-foreground" />}
          </>}
        >
          {line ? <span className="truncate text-caption text-muted-foreground" data-desk-mention-line="true">{line}</span> : null}
        </SessionRowBody>
      </CommandItem>
    );
  };
  const enteredRows = (entered?.sessions ?? []).map((one) => ({ session: one, depth: 0, children: 0, open: false, crumb: null }));
  return (
    <Command value={active} onValueChange={onActive} shouldFilter={false} loop aria-label={t("desk.mention.menu")} data-desk-mention-menu={entered ? "project" : "top"}>
      <CommandList>
        {entered ? (
          <>
            <Button variant="ghost" size="sm" className="w-full justify-start gap-xs px-sm text-caption font-normal text-muted-foreground" onClick={onBack} data-desk-mention-back="true">
              <ChevronLeftIcon aria-hidden="true" className="size-(--size-icon-sm)" />
              <span className="font-medium text-foreground">{entered.project.name}</span>
              <span className="ml-auto flex items-center gap-xxs">{t("desk.mention.back")}<Kbd>⇧⇥</Kbd></span>
            </Button>
            <CommandItem value={`new:${entered.project.id}`} onSelect={() => onProject(entered.project)} data-desk-mention-new="true">
              <PlusIcon aria-hidden="true" />
              <span className="flex-1">{t("desk.mention.newThread")}</span>
              <CommandShortcut>↵</CommandShortcut>
            </CommandItem>
            <CommandSeparator />
            <CommandGroup heading={t("desk.mention.liveSessions", { count: enteredRows.length })}>
              {enteredRows.map(session)}
            </CommandGroup>
          </>
        ) : (
          <>
            {tree.rows.length ? <CommandGroup heading={t(searching ? "desk.mention.matched" : "desk.mention.sessions")} data-desk-mention-group="sessions">{tree.rows.map(session)}</CommandGroup> : null}
            {tree.found.length ? <CommandGroup heading={t("desk.mention.childSessions")} data-desk-mention-group="children">{tree.found.map(session)}</CommandGroup> : null}
            {(tree.rows.length || tree.found.length) && tree.projects.length ? <CommandSeparator /> : null}
            {tree.projects.length ? (
              <CommandGroup heading={t("desk.mention.projects")} data-desk-mention-group="projects">
                {tree.projects.map((project) => (
                  <CommandItem key={project.id} value={`project:${project.id}`} onSelect={() => onProject(project)} data-desk-mention-project={project.id}>
                    <HouseIcon aria-hidden="true" className="size-(--size-icon-sm) text-mention-project" />
                    <span className="min-w-0 flex-1 truncate">{project.name}</span>
                    <span className="shrink-0 text-micro text-muted-foreground">{t("desk.mention.projectSessions", { count: project.sessions.length })}</span>
                    <Kbd>⇥</Kbd>
                  </CommandItem>
                ))}
              </CommandGroup>
            ) : null}
          </>
        )}
      </CommandList>
    </Command>
  );
}
