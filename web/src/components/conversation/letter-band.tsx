// A letter in a conversation (mockup v3 section 5): one blue band of the same shape whether the session sent it or
// received it, only the arrow telling which. Pressing the band opens the whole letter in place and folds it again;
// only the other session's name opens that session, and the screen hands that name in.

import { ChevronDownIcon, ChevronUpIcon } from "lucide-react";
import type { ReactNode } from "react";
import { useInterfaceTranslation } from "../../i18n/translator";
import { RowOverlayButton } from "../ui/row-button";
import type { Letter } from "./rows";

const KIND_WORD = {
  request: "conversation.letter.kind.request",
  reply: "conversation.letter.kind.reply",
  report: "conversation.letter.kind.report",
  progress: "conversation.letter.kind.progress",
  block: "conversation.letter.kind.block",
  watch: "conversation.letter.kind.watch",
} as const;

/**
 * A letter's band: → when this session sent it, ← when it received it, then the other session's name (`name`, the
 * screen's own chip, or the name the record wrote), the kind's word (요청, 답장, 보고, 진행, 막힘; 편지 for one with no
 * kind), its first line while folded, and a chevron. Open, the band holds the whole letter under its head, words a
 * reader can select. The band spans the conversation's width, as the doorbell's rows span a terminal's.
 */
export function LetterBand({ letter, open, onToggle, name }: { letter: Letter; open: boolean; onToggle: () => void; name?: ReactNode }) {
  const { t } = useInterfaceTranslation();
  const peer = letter.peer?.name ?? t("conversation.letter.someone");
  const Chevron = open ? ChevronUpIcon : ChevronDownIcon;
  return (
    <div className="relative -mx-md bg-terminal-letter-band px-md py-xs text-body text-terminal-letter-text" data-conversation-letter={letter.direction} data-letter-kind={letter.kind ?? "letter"} data-letter-open={open ? "true" : undefined}>
      <RowOverlayButton className="rounded-none" aria-label={t(letter.direction === "sent" ? "conversation.letter.sent" : "conversation.letter.received", { name: peer })} aria-expanded={open} onClick={onToggle} />
      <div className="pointer-events-none relative flex min-w-0 items-center gap-sm">
        <span aria-hidden="true" className="w-(--size-icon) shrink-0 text-center font-bold">{letter.direction === "sent" ? "→" : "←"}</span>
        <span className="pointer-events-auto min-w-0 max-w-2/5 shrink-0 truncate font-semibold" data-letter-peer="true">{name ?? peer}</span>
        <span className="shrink-0 rounded-xs border border-terminal-letter-text/40 px-xs text-micro">{t(letter.kind ? KIND_WORD[letter.kind] : "conversation.letter.kind.letter")}</span>
        <span className="min-w-0 flex-1 truncate opacity-(--opacity-secondary)">{open ? null : letter.first}</span>
        <Chevron aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />
      </div>
      {open ? <p className="relative mt-xs whitespace-pre-wrap break-keep pl-[calc(var(--size-icon)+var(--spacing-sm))] leading-relaxed [overflow-wrap:anywhere]" data-letter-text="true">{letter.text}</p> : null}
    </div>
  );
}
