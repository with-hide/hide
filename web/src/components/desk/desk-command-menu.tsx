import type { DeskCommand } from "../../snapshot";
import { useInterfaceTranslation } from "../../i18n/client";
import { Command, CommandGroup, CommandItem, CommandList } from "../ui/command";

/**
 * The `/` menu: the commands the agent listed, in the `@` menu's popover and
 * rows, filtered by what follows `/` (`commandRows`). Each row is the command
 * as it is typed with the hint for what follows it, and what it does under
 * it; `↵` or a press writes it into the box. `active` is the highlighted
 * row's name, which the composer moves.
 */
export function DeskCommandMenu({ agent, commands, active, onActive, onPick }: {
  /** The agent's name, which heads the list. */
  agent: string;
  commands: readonly DeskCommand[];
  active: string;
  onActive: (name: string) => void;
  onPick: (command: DeskCommand) => void;
}) {
  const { t } = useInterfaceTranslation();
  return (
    <Command value={active} onValueChange={onActive} shouldFilter={false} loop aria-label={t("desk.command.menu")} data-desk-command-menu="true">
      <CommandList>
        <CommandGroup heading={t("desk.command.heading", { agent })}>
          {commands.map((command) => (
            <CommandItem key={command.name} value={command.name} onSelect={() => onPick(command)} className="flex-col items-start gap-none py-xxs" data-desk-command={command.name}>
              <span className="flex w-full min-w-0 items-baseline gap-xs">
                <span className="shrink-0 font-mono font-medium">{command.name}</span>
                {command.hint ? <span className="min-w-0 truncate text-caption text-muted-foreground" data-desk-command-hint="true">{command.hint}</span> : null}
              </span>
              {command.description ? <span className="w-full truncate text-caption text-muted-foreground" data-desk-command-description="true">{command.description}</span> : null}
            </CommandItem>
          ))}
        </CommandGroup>
      </CommandList>
    </Command>
  );
}
