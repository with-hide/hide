// The phone a chapter about the phone puts beside the screen: one agent's
// conversation as the phone draws it, from the phone's own parts
// (`MessageItem`, `ActivityLine`, `AskCardView`, and for a reply's files
// `ComposerFiles`, `SentFiles` and the reply bar), standing at the window's
// foot like a phone held beside the computer. Its presses go to the world as
// the computer would take them (`world.ts`, `PracticePhoneScreen`); nothing
// leaves the page, and there is no relay or phone behind it.

import { agentProductName } from "../agentAdapters";
import { AgentLogo } from "../components/agent-logo";
import { useElapsed } from "../components/elapsed";
import { useInterfaceTranslation } from "../i18n/translator";
import { AskCardView } from "../mobile/AskCard";
import { AttachButton, ComposerFiles, FileNoticeLine } from "../mobile/Attachments";
import { ActivityLine, Composer } from "../mobile/layout";
import { MessageItem } from "../mobile/messages";
import type { CoreEvent, PracticePhoneScreen } from "./world";

export function PracticePhone({ screen, send }: { screen: PracticePhoneScreen; send: (event: CoreEvent) => void }) {
  const { t } = useInterfaceTranslation();
  const waited = useElapsed(screen.ask?.since_ms ?? null);
  return (
    <aside
      aria-label={t("practice.phone")}
      className="absolute right-xl bottom-0 z-20 flex h-(--size-practice-phone-h) max-h-full w-(--size-practice-phone-w) flex-col overflow-hidden rounded-t-3xl border-4 border-b-0 border-subtle-foreground bg-background shadow-overlay"
      data-practice-phone={screen.pane}
    >
      <div className="flex items-center gap-xs border-b border-border px-lg py-sm text-body">
        <AgentLogo agent={screen.kind} label={agentProductName(screen.kind)} size="sm" />
        <span className="min-w-0 flex-1 truncate font-semibold">{screen.title}</span>
        <span className="rounded-full bg-secondary px-sm text-caption">{t("mobile.tab.conversation")}</span>
      </div>
      <div className="flex min-h-0 flex-1 flex-col justify-end overflow-hidden px-lg py-md">
        <ol aria-label={t("mobile.tab.conversation")} className="flex flex-col gap-md">
          {screen.said.map((said, index) => (
            <MessageItem key={index} message={{ id: index, who: said.who, text: said.text, truncated: false, at_ms: 0 }} timed={false} files={screen.attached?.[index]} />
          ))}
        </ol>
        {screen.running !== null ? <ActivityLine word={t("mobile.runningCommand")} since={screen.running} clock /> : null}
        {screen.ask ? (
          <AskCardView
            ask={screen.ask}
            disabled={false}
            waited={waited}
            picked={screen.picked}
            answer={screen.answered ? "sent" : null}
            error={null}
            onPick={(choice) => send({ kind: "practice_phone_pick", payload: { choice } })}
            onAnswer={(choice) => send({ kind: "practice_phone_answer", payload: { choice } })}
            onTerminal={() => {}}
          />
        ) : null}
      </div>
      {screen.reply ? (
        <div className="px-lg pb-md">
          <ComposerFiles files={screen.reply.files} onRemove={(id) => send({ kind: "practice_phone_remove", payload: { id } })} onRetry={() => {}} />
          <FileNoticeLine notice={screen.reply.notice} />
          <Composer
            value={screen.reply.text}
            onChange={() => {}}
            onSubmit={() => send({ kind: "practice_phone_send", payload: {} })}
            placeholder={t("mobile.replyPlaceholder")}
            disabled={false}
            sendDisabled={!screen.reply.sendable}
            lines
            lead={<AttachButton onPick={() => send({ kind: "practice_phone_attach", payload: {} })} disabled={false} />}
            send={{ "data-phone-send": "true" }}
          />
        </div>
      ) : null}
    </aside>
  );
}
