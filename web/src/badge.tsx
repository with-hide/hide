import type { Actions } from "./actions";
import { Button } from "./components/ui/button";
import { windowStrip } from "./connection";
import { useInterfaceTranslation } from "./i18n/client";
import { coreMachineName, moveMachines } from "./screenMachine";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { cn } from "./lib/utils";
import { FixCommand } from "./settings/FixCommand";
import { SignInPrompt } from "./settings/SignInPrompt";

export function ConnectionBadge({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const connection = useShellStore((s) => s.connection);
  const refused = useShellStore((s) => s.refused);
  const move = useShellStore((s) => s.coreMove);
  const link = useShellStore((s) => s.coreLink);
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const strip = windowStrip({ connection, refused, move, link, machines: move ? moveMachines(devices, move, t) : null, coreMachine: coreMachineName(devices, t) }, t);
  if (!strip) return null;
  const line = (
    <div
      role="status"
      className={cn("flex items-center gap-xs text-caption text-subtle-foreground", strip.signIn ? null : "border-b border-border bg-card px-md py-xs")}
      data-connection={strip.kind}
    >
      {strip.mark ? (
        <span aria-hidden="true" className={cn("shrink-0", strip.mark === "warn" ? "font-semibold text-warning" : "text-muted-foreground")}>
          {strip.mark === "warn" ? "!" : "…"}
        </span>
      ) : null}
      <span className="min-w-0 truncate">{strip.text}</span>
      {strip.action ? <span aria-hidden="true">·</span> : null}
      {strip.action === "reconnect" ? (
        <Button variant="link" size="sm" className="h-auto shrink-0 p-0 text-caption" data-core-reconnect="true" onClick={() => actions.coreLink("reconnect")}>
          {t("coreMove.reconnect")}
        </Button>
      ) : null}
      {strip.command ? <FixCommand command={strip.command} what="ssh command" actions={actions} data-core-link-command={strip.command} /> : null}
      {strip.action === "retry" || strip.action === "sign_in" ? (
        // A sign-in's Retry reads Sign in, as the device row's does: it starts the dial that may ask.
        <Button variant="link" size="sm" className="h-auto shrink-0 p-0 text-caption" data-core-retry={strip.action} onClick={() => actions.coreLink("retry")}>
          {strip.action === "sign_in" ? t("devices.signIn.start") : t("common.retry")}
        </Button>
      ) : null}
      {strip.action === "give_up" ? (
        // It only opens the move dialog at its confirm; nothing is given up from the strip.
        <Button variant="link" size="sm" className="h-auto shrink-0 p-0 text-caption" data-core-move-strip-give-up="true" onClick={() => useUiStore.getState().openMoveGiveUp()}>
          {move?.direction === "back" ? t("coreMove.takeHere.take") : t("coreMove.giveUp.stop")}
        </Button>
      ) : null}
    </div>
  );
  if (!strip.signIn) return line;
  // What ssh asks the dial the operator's connect or Retry started, under the strip's line as on a device's row
  // (PRD system-ssh-device-link B33).
  return (
    <div className="space-y-xs border-b border-border bg-card px-md py-xs" data-core-link-sign-in={strip.signIn.kind}>
      {line}
      <div className="max-w-2xl">
        <SignInPrompt id="core-link" wait={strip.signIn} actions={actions} answer={(question, reply) => actions.answerCoreLinkSignIn(question, reply)} />
      </div>
    </div>
  );
}
