import { Trans } from "react-i18next";
import { LoaderCircleIcon } from "lucide-react";
import type { DeskCommand } from "../../snapshot";
import { useInterfaceTranslation } from "../../i18n/client";
import { Command, CommandGroup, CommandItem } from "../ui/command";
import { Kbd } from "../ui/kbd";
import { DeskMenuLine, DeskMenuList } from "./desk-menu";

/**
 * What the `/` panel shows (feedback 1 frame P 가): the agent's commands
 * that match, a line that none does, a line while the agent's session
 * opens and its list is on its way, or a line that the list comes with
 * the first message, before the agent's session ever opened.
 */
export type DeskCommandPanel = { kind: "rows"; commands: readonly DeskCommand[] } | { kind: "none" } | { kind: "waiting" } | { kind: "first" };

/**
 * The `/` menu in the panel Desk's composer menus share (`desk-menu.tsx`):
 * each row the command as it is typed in the monospace face with the hint
 * for what follows it muted beside it, and what it does under it in the
 * reading face; while words follow `/`, the part of each name they match
 * stays bright and the rest is muted. `↵` or a press writes the command
 * into the box. `active` is the highlighted row's name, which the composer
 * moves.
 */
export function DeskCommandMenu({ agent, panel, query = "", active, onActive, onPick }: {
  /** The agent's name, which heads the list. */
  agent: string;
  panel: DeskCommandPanel;
  /** What follows `/`, which each name's bright part matches. */
  query?: string;
  active: string;
  onActive: (name: string) => void;
  onPick: (command: DeskCommand) => void;
}) {
  const { t, i18n } = useInterfaceTranslation();
  const heading = t("desk.command.heading", { agent });
  if (panel.kind === "none") {
    return (
      <DeskMenuLine heading={heading} data-desk-command-menu="none">
        {t("desk.command.none")} ·
        <span className="flex items-center gap-xxs"><Trans i18n={i18n} t={t} i18nKey="desk.command.sendsAsIs" components={{ key: <Kbd /> }} /></span>
      </DeskMenuLine>
    );
  }
  if (panel.kind === "waiting") {
    return (
      <DeskMenuLine heading={heading} data-desk-command-menu="waiting">
        <LoaderCircleIcon aria-hidden="true" className="size-(--size-icon-sm) motion-safe:animate-spin" />
        {t("desk.command.waiting")}
      </DeskMenuLine>
    );
  }
  if (panel.kind === "first") return <DeskMenuLine heading={heading} data-desk-command-menu="first">{t("desk.command.firstMessage")}</DeskMenuLine>;
  return (
    <Command value={active} onValueChange={onActive} shouldFilter={false} loop aria-label={t("desk.command.menu")} className="bg-transparent" data-desk-command-menu="rows">
      <DeskMenuList>
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
      </DeskMenuList>
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
