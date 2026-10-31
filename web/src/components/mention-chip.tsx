import type { ReactNode } from "react";
import { cn } from "../lib/utils";
import { Button } from "./ui/button";

/** What an `@` names: a session (its pane or its thread) or a project. */
export type MentionKind = "session" | "project";

const TONE: Record<MentionKind, string> = {
  session: "bg-mention-session/10 text-mention-session",
  project: "bg-mention-project/10 text-mention-project",
};

/** A pressable chip deepens its fill under the pointer and keeps its colour. */
const PRESS: Record<MentionKind, string> = {
  session: "hover:bg-mention-session/20 hover:text-mention-session",
  project: "hover:bg-mention-project/20 hover:text-mention-project",
};

/**
 * An `@name` inside a line of words (PRD session-docs-and-assistant D-72,
 * B26): a session or a project, told apart by colour, in the text's own size
 * so it sits in the sentence. With `onPress` it is the way to what it names,
 * and `label` says where that goes; without, it only names it. `lead` takes
 * the place of `@`: a session's status and agent marks, as a session reads
 * everywhere (B26), or a project's ⌂, on the same colour (D-85).
 */
export function MentionChip(props: {
  kind: MentionKind;
  /** The name after `@`, as the operator typed or Hide shows it. */
  name: string;
  /** A session's status and agent marks, or a project's ⌂, drawn in place of `@`. */
  lead?: ReactNode;
  className?: string;
} & (
  | { onPress?: undefined; label?: undefined }
  /** A pressable chip names what pressing opens: its accessible name. */
  | { onPress: () => void; label: string }
)) {
  const { kind, name, lead, label, onPress, className } = props;
  const look = cn("max-w-full rounded-xs px-xxs align-baseline", lead && "gap-xxs", TONE[kind], className);
  const text = lead ? <><span className="inline-flex shrink-0 items-center gap-xxs self-center">{lead}</span><span className="truncate">{name}</span></> : <span className="truncate">@{name}</span>;
  if (!onPress) {
    return <span className={cn("inline-flex min-w-0 items-baseline", look)} data-mention-chip={kind}>{text}</span>;
  }
  return (
    <Button variant="text" size="inline" className={cn("inline-flex", look, PRESS[kind])} aria-label={label} data-mention-chip={kind} onClick={onPress}>
      {text}
    </Button>
  );
}
