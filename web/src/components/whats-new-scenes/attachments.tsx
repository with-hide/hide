// What's new scene for files from the phone (Doop 08, 09): away from the desk, the operator presses the `+` at the start of the
// reply field and picks a screenshot and a log; the photo stands as its thumbnail, the log as its chip with the band of its upload,
// and a 14 MB trace is left out with one line. Once both are in, the reply goes, and the files stand over the operator's bubble.
// On the Mac the agent's own screen takes the files' paths and the words as one message, reads the log, and answers on both. It is
// drawn from the phone's own parts and words (`ComposerFiles`, `SentFiles`, `MessageItem`, `mobile.*`), and the work is an
// example, never the operator's.

import { useMemo } from "react";
import { agentProductName } from "../../agentAdapters";
import { useInterfaceTranslation } from "../../i18n/translator";
import { AttachButton, ComposerFiles, FileNoticeLine } from "../../mobile/Attachments";
import { Composer } from "../../mobile/layout";
import { MessageItem } from "../../mobile/messages";
import type { ComposerFile, SentFile } from "../../mobile/uploads";
import { AgentLogo } from "../agent-logo";
import { sampleShot } from "../sample-shot";
import { ScenePointer } from "./pointer";
import { SCENE_CSS, type SceneProps } from "./scene";

/** One loop, in ms: the beats below fall inside it. */
export const ATTACHMENTS_LOOP_MS = 11_000;
/** The pointer comes to the `+` and presses it; the picked files stand on the bar and the log starts up. */
const POINT = 700;
const PICK = 1_300;
/** The log's bytes are in. */
const READY = 3_100;
/** The pointer moves to send and presses it; the files stand over the bubble. */
const SEND_POINT = 3_400;
const SEND = 4_000;
/** On the Mac the agent's screen took the paths and the words, and reads the log. */
const TYPED = 4_400;
/** The moment a still picture shows: the files sent over the bubble and taken on the agent's own screen. */
export const ATTACHMENTS_STILL_MS = 4_900;
/** The agent answers. */
const ANSWERED = 6_000;

const PHOTO = "IMG_2041.jpg";
const LOG = "holder.log";
const PHOTO_BYTES = 1_258_291;
const LOG_BYTES = 348_160;
/** The trace left out: past the 10 MB a file may be. */
const TRACE = { name: "trace.json", size: 14 * 1024 * 1024 };

export function AttachmentsScene({ at }: SceneProps) {
  const { t } = useInterfaceTranslation();
  const thumb = useMemo(() => sampleShot(), []);
  const picked = at >= PICK && at < SEND;
  const sent = at >= SEND;
  const share = Math.min(1, Math.max(0, (at - PICK) / (READY - PICK)));
  const files: ComposerFile[] = picked
    ? [
        { id: "photo", name: PHOTO, size: PHOTO_BYTES, kind: "photo", thumb, state: "ready", sent: PHOTO_BYTES, reason: null },
        { id: "log", name: LOG, size: LOG_BYTES, kind: "file", thumb: null, state: at >= READY ? "ready" : "uploading", sent: Math.round(LOG_BYTES * share), reason: null },
      ]
    : [];
  const attached: SentFile[] = [
    { stage: "photo", name: PHOTO, size: PHOTO_BYTES, kind: "photo", thumb },
    { stage: "log", name: LOG, size: LOG_BYTES, kind: "file", thumb: null },
  ];
  const pointer = at < SEND_POINT
    ? { left: 54, top: 84, shown: at >= POINT && at < READY, pressed: at >= PICK }
    : { left: 93, top: 84, shown: at < SEND + 500, pressed: at >= SEND };
  return (
    <div className="relative flex aspect-[2/1] gap-md overflow-hidden bg-muted p-md text-foreground" data-whats-new-scene="attachments" data-scene-beat={at >= ANSWERED ? "answered" : sent ? "sent" : picked ? "picked" : "empty"} aria-hidden>
      <style>{SCENE_CSS}</style>
      <Terminal at={at} />
      <div className="relative min-w-0 flex-1" inert>
        {/* The phone rises from the scene's foot: the end of the conversation and the reply bar under it. */}
        <div className="absolute inset-x-0 top-0 -bottom-md flex flex-col overflow-hidden rounded-t-3xl border-4 border-b-0 border-subtle-foreground bg-background">
          <div className="flex items-center gap-xs border-b border-border px-md py-xs text-caption">
            <AgentLogo agent="claude" label={agentProductName("claude")} size="sm" />
            <span className="min-w-0 flex-1 truncate font-semibold">{t("whatsNewScene.attachments.agent")}</span>
            <span className="rounded-full bg-secondary px-sm text-micro">{t("mobile.tab.conversation")}</span>
          </div>
          <ol className="flex min-h-0 flex-1 flex-col justify-end gap-md overflow-hidden px-md pb-sm">
            <MessageItem message={{ id: 1, who: "agent", text: t("whatsNewScene.attachments.before"), truncated: false, at_ms: 0 }} timed={false} />
            {sent ? (
              <div className="wns-rise">
                <MessageItem message={{ id: 2, who: "you", text: t("whatsNewScene.attachments.reply"), truncated: false, at_ms: 0 }} timed={false} files={attached} />
              </div>
            ) : null}
            {at >= ANSWERED ? (
              <div className="wns-rise">
                <MessageItem message={{ id: 3, who: "agent", text: t("whatsNewScene.attachments.answer"), truncated: false, at_ms: 0 }} timed={false} />
              </div>
            ) : null}
          </ol>
          <div className="px-md pb-md">
            <ComposerFiles files={files} onRemove={() => {}} onRetry={() => {}} />
            {picked ? <FileNoticeLine notice={{ kind: "too_large", name: TRACE.name, size: TRACE.size }} /> : null}
            <Composer
              value={picked ? t("whatsNewScene.attachments.reply") : ""}
              onChange={() => {}}
              onSubmit={() => {}}
              placeholder={t("mobile.replyPlaceholder")}
              disabled={false}
              sendDisabled={!picked || at < READY}
              lines
              lead={<AttachButton onPick={() => {}} disabled={false} />}
            />
          </div>
        </div>
      </div>
      <ScenePointer {...pointer} />
    </div>
  );
}

/** The agent's own screen on the Mac: the message the phone's reply typed, the paths of its files before its words. */
function Terminal({ at }: { at: number }) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="flex w-1/2 shrink-0 flex-col overflow-hidden rounded-md border border-border bg-background">
      <div className="flex items-center gap-xs border-b border-border px-sm py-xxs text-caption">
        <AgentLogo agent="claude" label={agentProductName("claude")} size="sm" />
        <span className="min-w-0 flex-1 truncate">{t("whatsNewScene.attachments.agent")}</span>
      </div>
      <div className="flex flex-col gap-xxs px-sm py-xs font-mono text-micro text-subtle-foreground">
        <span className="line-clamp-2">● {t("whatsNewScene.attachments.before")}</span>
        {at >= TYPED ? (
          <>
            <span className="wns-rise flex flex-col text-foreground">
              <span className="truncate">{`> "…/attachments/…-${PHOTO}"`}</span>
              <span className="truncate">{`  "…/attachments/…-${LOG}"`}</span>
              <span className="truncate">{`  ${t("whatsNewScene.attachments.reply")}`}</span>
            </span>
            <span className="wns-rise truncate">{`⏺ Read(${LOG})`}</span>
          </>
        ) : null}
        {at >= ANSWERED ? <span className="wns-rise line-clamp-3 text-foreground">● {t("whatsNewScene.attachments.answer")}</span> : null}
      </div>
    </div>
  );
}
