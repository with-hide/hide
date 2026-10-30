import { ChevronLeftIcon, FolderIcon, PlusIcon } from "lucide-react";
import type { RowMark } from "../../snapshot";
import { AgentMark } from "../../AgentMark";
import { useInterfaceTranslation } from "../../i18n/client";
import { StatusMark } from "../status-mark";
import { Command, CommandGroup, CommandItem, CommandList, CommandSeparator, CommandShortcut } from "../ui/command";
import { Button } from "../ui/button";
import { Kbd } from "../ui/kbd";

/** A session the @ menu offers (B18): Desk's or not, on this device or another. */
export type MentionSession = { id: string; name: string; agent_kind: string; mark: RowMark | null; detail: string | null };

/** A project the @ menu offers, with the panes of the sessions running in it now. */
export type MentionProject = { id: string; name: string; sessions: string[] };

/**
 * The `@` menu (D-70, B18): recent sessions above, projects below, both
 * filtered by what follows `@`. `↵` on a session hands the task to it, on a
 * project starts a new thread there, and `⇥` on a project enters it: then
 * the menu lists its running sessions under `＋ 새 스레드`, and `⇧⇥` or Esc
 * goes back. `active` is the highlighted row's value; the composer moves it.
 */
export function DeskMentionMenu({ sessions, projects, entered = null, active, onActive, onSession, onProject, onBack }: {
  sessions: readonly MentionSession[];
  projects: readonly MentionProject[];
  /** The project entered with ⇥, and its running sessions. */
  entered?: { project: MentionProject; sessions: readonly MentionSession[] } | null;
  active: string;
  onActive: (value: string) => void;
  onSession: (session: MentionSession) => void;
  onProject: (project: MentionProject) => void;
  onBack: () => void;
}) {
  const { t } = useInterfaceTranslation();
  const session = (one: MentionSession) => (
    <CommandItem key={one.id} value={`session:${one.id}`} onSelect={() => onSession(one)} data-desk-mention-session={one.id}>
      <StatusMark mark={one.mark} />
      <AgentMark kind={one.agent_kind} />
      <span className="min-w-0 flex-1 truncate">{one.name}</span>
      <span className="shrink-0 text-micro text-muted-foreground">{one.detail}</span>
    </CommandItem>
  );
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
            <CommandGroup heading={t("desk.mention.liveSessions", { count: entered.sessions.length })}>
              {entered.sessions.map(session)}
            </CommandGroup>
          </>
        ) : (
          <>
            {sessions.length ? <CommandGroup heading={t("desk.mention.sessions")}>{sessions.map(session)}</CommandGroup> : null}
            {sessions.length && projects.length ? <CommandSeparator /> : null}
            {projects.length ? (
              <CommandGroup heading={t("desk.mention.projects")}>
                {projects.map((project) => (
                  <CommandItem key={project.id} value={`project:${project.id}`} onSelect={() => onProject(project)} data-desk-mention-project={project.id}>
                    <FolderIcon aria-hidden="true" />
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
