import { CopyIcon } from "lucide-react";
import type { Actions } from "../actions";
import { Button } from "../components/ui/button";
import { useInterfaceTranslation } from "../i18n/client";

/** The one command that fixes something, in its own type, with Copy; a command is never translated. */
export function FixCommand({ command, what, actions, ...data }: { command: string; what: string; actions: Actions } & Record<`data-${string}`, string>) {
  const { t } = useInterfaceTranslation();
  return (
    <span className="flex max-w-full shrink-0 items-center gap-xxs rounded-sm bg-muted py-xxs pl-sm pr-xxs" {...data}>
      <code className="min-w-0 break-all font-mono text-body text-foreground">{command}</code>
      <Button variant="ghost" size="icon" aria-label={t("coreMove.copy")} onClick={() => actions.copyText(command, what)}>
        <CopyIcon aria-hidden="true" />
      </Button>
    </span>
  );
}
