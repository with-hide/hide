// A conversation's messages as the phone draws them (docs/UI_BEHAVIOR.md, The phone app; Doop 02): the operator's words in a
// bubble on the right, the agent's Markdown at full width, an interruption, and the time after each turn. Drawn only, with no
// connection, so a practice chapter draws the same rows as the phone's detail.
import { memo, useMemo } from "react";
import { useInterfaceTranslation } from "../i18n/translator";
import { cn } from "../lib/utils";
import { renderMarkdown } from "./markdown";
import { messageTime, type ConversationMessage } from "./protocol";

/** Memoized: a message never changes once it arrived, so its Markdown is parsed once. */
export const MessageItem = memo(function MessageItem({ message, timed }: { message: ConversationMessage; timed: boolean }) {
  const { t } = useInterfaceTranslation();
  const time = timed && message.at_ms > 0 ? (
    <time dateTime={new Date(message.at_ms).toISOString()} className={cn("block pt-xs font-mono text-body text-muted-foreground", message.who === "you" && "text-right")}>
      {messageTime(message.at_ms)}
    </time>
  ) : null;
  if (message.who === "stopped") {
    return (
      <li data-phone-message="stopped" className="text-body text-muted-foreground">
        {t("mobile.stopped")}
      </li>
    );
  }
  if (message.who === "you") {
    return (
      <li data-phone-message="you" className="flex flex-col items-end">
        {/* The operator's words stand right, in a rounded bubble, as Doop 02 draws them. */}
        <p className="max-w-5/6 whitespace-pre-wrap break-keep rounded-xl bg-secondary px-lg py-sm text-body leading-normal text-foreground wrap-anywhere">{message.text}</p>
        {time}
      </li>
    );
  }
  return <AgentMessage message={message} time={time} />;
});

function AgentMessage({ message, time }: { message: ConversationMessage; time: React.ReactNode }) {
  const { t } = useInterfaceTranslation();
  const body = useMemo(() => renderMarkdown(message.text), [message.text]);
  return (
    <li data-phone-message="agent">
      <div className="space-y-sm break-keep wrap-anywhere text-body leading-normal text-foreground">
        {body}
        {message.truncated ? <p className="text-muted-foreground">{t("mobile.truncated")}</p> : null}
      </div>
      {time}
    </li>
  );
}

