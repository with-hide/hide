import { ArrowUpIcon, LoaderCircleIcon } from "lucide-react";
import { REPLY_MAX_LENGTH, useAgentReply } from "../agentReply";
import type { Actions } from "../actions";
import type { MessageKey } from "../i18n/catalogs";
import { useInterfaceTranslation } from "../i18n/client";
import { composingKey } from "../keys";
import { cn } from "../lib/utils";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Hint } from "./ui/tooltip";

/** The refusals the field words itself; any other reads as a plain failure. */
const REASONS = new Set(["pane_gone", "pane_changed", "session_changed", "unknown", "asleep", "input_refused", "unavailable", "device_unreachable", "uncertain", "too_long", "control_characters", "in_flight", "busy", "offline"]);

/**
 * A one-line reply to an agent without opening its session (the Overview's
 * 나를 기다리는 것 and its session detail): Enter or the round send types the
 * words and Enter into the agent's pane, as the phone's reply does. The words
 * stay while they are sent and after a refusal, with its reason under them.
 */
/** `session` is the digest of the session drawn in the pane, which the core checks it still runs before typing. */
export function AgentReplyField({ paneId, session, name, actions, className }: { paneId: string; session: string | null; name: string; actions: Pick<Actions, "replyToAgent">; className?: string }) {
  const { t } = useInterfaceTranslation();
  const reply = useAgentReply(paneId, session, actions.replyToAgent);
  const reason = reply.refused ? t(`agentReply.reason.${REASONS.has(reply.refused) ? reply.refused : "failed"}` as MessageKey) : null;
  const send = t("agentReply.send");
  return (
    <div className={cn("flex min-w-0 flex-col gap-xxs", className)} data-agent-reply={paneId} data-agent-reply-state={reply.sending ? "sending" : reply.refused ? "refused" : "idle"}>
      <div className="flex min-w-0 items-center gap-xs">
        <Input
          className="h-(--size-control-sm) px-xs text-caption"
          aria-label={t("agentReply.label", { name })}
          aria-invalid={reason ? true : undefined}
          placeholder={t("agentReply.placeholder")}
          maxLength={REPLY_MAX_LENGTH}
          value={reply.text}
          readOnly={reply.sending}
          onChange={(event) => reply.setText(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey && !composingKey(event)) {
              event.preventDefault();
              reply.submit();
            }
          }}
        />
        <Hint label={send}>
          <Button variant="secondary" size="icon-sm" className="shrink-0 rounded-full" aria-label={send} disabled={reply.sending || !reply.text.trim()} onClick={reply.submit}>
            {reply.sending ? <LoaderCircleIcon className="animate-spin" /> : <ArrowUpIcon />}
          </Button>
        </Hint>
      </div>
      {reason ? <p role="alert" className="m-0 text-caption text-destructive">{reason}</p> : null}
    </div>
  );
}
