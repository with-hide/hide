// The pane's real conversation parts, shown over its terminal and returned to the same screen.
import { MessageSquareIcon } from "lucide-react";
import { useMemo } from "react";
import { useInterfaceTranslation } from "../../i18n/translator";
import { threadLines } from "../../desk";
import { useComposerTray } from "../../deskComposer";
import { ThreadView } from "../desk/thread-view";
import { Button } from "../ui/button";
import { ScenePointer } from "./pointer";
import { SCENE_CSS, type SceneProps } from "./scene";

export const CONVERSATION_LOOP_MS = 9_000;
export const CONVERSATION_STILL_MS = 4_500;
const noop = () => undefined;
export function ConversationScene({ at }: SceneProps) {
  const { t } = useInterfaceTranslation();
  const open = at >= 1_500 && at < 7_500;
  const prompt = t("whatsNewScene.conversation.prompt"), reply = t("whatsNewScene.conversation.reply");
  const lines = useMemo(() => threadLines([
    { sessionUpdate: "user_message_chunk", content: { type: "text", text: prompt } },
    { sessionUpdate: "agent_message_chunk", content: { type: "text", text: reply } },
  ]), [prompt, reply]);
  const tray = useComposerTray();
  return <div className="relative flex aspect-[2/1] min-w-0 flex-col overflow-hidden bg-background text-foreground" data-whats-new-scene="conversation" data-scene-beat={open ? "conversation" : "terminal"} aria-hidden>
    <style>{SCENE_CSS}</style>
    <header className="flex h-(--size-pane-header) shrink-0 items-center gap-sm border-b border-border bg-card px-sm text-caption">
      <span className="min-w-0 flex-1 truncate">{t("whatsNewScene.conversation.title")}</span>
      <Button variant={open ? "tint" : "ghost"} size="sm" aria-pressed={open}><MessageSquareIcon />{t("panes.conversation.label")}</Button>
    </header>
    <div className="relative min-h-0 flex-1">
      <pre className="absolute inset-0 p-md font-mono text-caption text-muted-foreground">{`> ${prompt}\n${reply}\n`}</pre>
      {open ? <div className="absolute inset-0 flex bg-card" inert>
        <ThreadView job={null} lines={lines} composer={{ value: "", tray, onChange: noop, onSend: noop }} onMission={noop} outputOf={(output) => output} onOutput={noop} onSession={noop} onCopy={() => Promise.resolve(true)} />
      </div> : null}
    </div>
    <ScenePointer left={89} top={8} shown={(at > 500 && at < 2_000) || (at > 6_500 && at < 8_000)} pressed={at >= 1_500 && at < 2_000 || at >= 7_500} />
  </div>;
}
