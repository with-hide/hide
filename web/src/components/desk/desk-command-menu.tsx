import { Trans } from "react-i18next";
import { LoaderCircleIcon, type LucideIcon } from "lucide-react";
import type { DeskCommand, DeskCommandsUnread } from "../../snapshot";
import { useInterfaceTranslation } from "../../i18n/client";
import { Command, CommandGroup, CommandItem } from "../ui/command";
import { Kbd } from "../ui/kbd";
import { DeskMenuLine, DeskMenuList } from "./desk-menu";

/**
 * One of Hide's own rows at the head of the `/` menu, the same for every
 * agent: its name and what it does in the reading face, its icon before the
 * name; a pick runs it (Desk's `새 대화`, operator decision 2026-11-27).
 */
export type HideCommand = { id: string; name: string; description: string; icon: LucideIcon; run: () => void };

/**
 * What the `/` panel shows (feedback 1 frame P 가): the agent's commands
 * that match, a line that none does, a line while Hide reads the agent's
 * list or its session opens, a line saying why an agent has no list
 * (`unread`: its sign-in, not installed, it sends none, the read failed,
 * Desk off), or, for an agent Desk does not speak with, a line that none
 * is known (`unknown`); on each line but the reading one ↵ sends the words
 * as typed.
 */
export type DeskCommandPanel =
  | { kind: "rows"; commands: readonly DeskCommand[] }
  | { kind: "none" }
  | { kind: "waiting" }
  | { kind: "unread"; why: Exclude<DeskCommandsUnread, "reading"> }
  | { kind: "unknown" };

const UNREAD_WORDS = {
  sign_in: "desk.command.unread.signIn",
  not_installed: "desk.command.unread.notInstalled",
  agent_off: "desk.command.unread.agentOff",
  no_list: "desk.command.unread.noList",
  failed: "desk.command.unread.failed",
  desk_off: "desk.command.unread.deskOff",
} as const;

/**
 * The `/` menu in the panel Desk's composer menus share (`desk-menu.tsx`):
 * each row the command as it is typed in the monospace face with the hint
 * for what follows it muted beside it, and what it does under it in the
 * reading face; while words follow `/`, the part of each name they match
 * stays bright and the rest is muted. `↵` or a press writes the command
 * into the box. `active` is the highlighted row's name, which the composer
 * moves.
 */
export function DeskCommandMenu({ agent, own = [], panel, query = "", active, onActive, onPickOwn, onPick }: {
  /** The agent's name, which heads its list. */
  agent: string;
  /** Hide's own rows that match, drawn first under `Hide`. */
  own?: readonly HideCommand[];
  /** The agent's part, none when Hide's rows answer alone. */
  panel: DeskCommandPanel | null;
  /** What follows `/`, which each name's bright part matches. */
  query?: string;
  /** The highlighted row: a Hide row's `hide:<id>` or a command's name. */
  active: string;
  onActive: (value: string) => void;
  onPickOwn?: (command: HideCommand) => void;
  onPick: (command: DeskCommand) => void;
}) {
  const { t, i18n } = useInterfaceTranslation();
  const heading = t("desk.command.heading", { agent });
  const sendsAsIs = <span className="flex items-center gap-xxs"><Trans i18n={i18n} t={t} i18nKey="desk.command.sendsAsIs" components={{ key: <Kbd /> }} /></span>;
  // The agent's part in place of its rows: one line under its name.
  const line = panel === null || panel.kind === "rows" ? null
    : panel.kind === "none" ? (
      <DeskMenuLine heading={heading} data-desk-command-menu="none">
        {t("desk.command.none")} ·
        {sendsAsIs}
      </DeskMenuLine>
    )
    : panel.kind === "waiting" ? (
      <DeskMenuLine heading={heading} data-desk-command-menu="waiting">
        <LoaderCircleIcon aria-hidden="true" className="size-(--size-icon-sm) motion-safe:animate-spin" />
        {t("desk.command.waiting")}
      </DeskMenuLine>
    )
    : (
      <DeskMenuLine heading={heading} data-desk-command-menu={panel.kind} data-desk-command-unread={panel.kind === "unread" ? panel.why : undefined}>
        {t(panel.kind === "unread" ? UNREAD_WORDS[panel.why] : "desk.command.unknown")} ·
        {sendsAsIs}
      </DeskMenuLine>
    );
  if (own.length === 0 && line) return line;
  return (
    <Command value={active} onValueChange={onActive} shouldFilter={false} loop aria-label={t("desk.command.menu")} className="bg-transparent" data-desk-command-menu="rows">
      <DeskMenuList>
        {own.length > 0 ? (
          <CommandGroup heading={t("desk.command.hideHeading")}>
            {own.map((command) => (
              <CommandItem key={command.id} value={`hide:${command.id}`} onSelect={() => onPickOwn?.(command)} className="flex-col items-start gap-none" data-desk-hide-command={command.id}>
                <span className="flex w-full min-w-0 items-center gap-xs text-body font-medium text-foreground">
                  <command.icon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />
                  <span className="min-w-0 truncate">{command.name}</span>
                </span>
                <span className="w-full truncate text-body text-subtle-foreground">{command.description}</span>
              </CommandItem>
            ))}
          </CommandGroup>
        ) : null}
        {panel?.kind === "rows" ? (
          <CommandGroup heading={heading}>
            {panel.commands.map((command) => (
              <CommandItem key={command.name} value={command.name} onSelect={() => onPick(command)} className="flex-col items-start gap-none" data-desk-command={command.name}>
                <span className="flex w-full min-w-0 items-baseline gap-sm font-mono text-body">
                  <Name name={command.name} query={query} />
                  {command.hint ? <span className="min-w-0 truncate text-muted-foreground" data-desk-command-hint="true">{command.hint}</span> : null}
                </span>
                {command.description ? <span className="w-full truncate font-sans text-body text-subtle-foreground" data-desk-command-description="true">{command.description}</span> : null}
              </CommandItem>
            ))}
          </CommandGroup>
        ) : null}
      </DeskMenuList>
      {line}
    </Command>
  );
}

/** A command's name: whole and bright with nothing typed; with words after `/`, the part they match bright and bold, the rest muted. */
function Name({ name, query }: { name: string; query: string }) {
  const sigil = /^[^\p{L}\p{N}]/u.test(name) ? 1 : 0;
  const found = query ? name.toLocaleLowerCase().indexOf(query.toLocaleLowerCase(), sigil) : -1;
  if (found < 0) return <span className="shrink-0 font-medium text-foreground">{name}</span>;
  // A name the words start, after its `/` or `$`, keeps that mark bright with them.
  const start = found === sigil ? 0 : found;
  const end = found + query.length;
  return (
    <span className="shrink-0 text-muted-foreground" data-desk-command-match={name.slice(found, end)}>
      {name.slice(0, start)}
      <span className="font-semibold text-foreground">{name.slice(start, end)}</span>
      {name.slice(end)}
    </span>
  );
}
