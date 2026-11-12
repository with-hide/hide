import type { HTMLAttributes } from "react";
import type { LineParts } from "../agentRow";
import { cn } from "../lib/utils";

/**
 * A row's second line in its parts: the leading word and the background tail
 * keep their width and only the sentence between them is cut, so a narrow
 * sidebar still says 대기 or 끝남 and how many tasks are left (status-marks
 * B27, B29). When the lead and tail alone pass the width, the line is
 * clipped at its edge with no ellipsis; no lead word is that long.
 */
export function RowLineText({ parts, className, ...rest }: { parts: LineParts } & HTMLAttributes<HTMLSpanElement>) {
  const pieces: { key: string; text: string; cut: boolean }[] = [];
  if (parts.lead) pieces.push({ key: "lead", text: parts.lead, cut: false });
  if (parts.text) pieces.push({ key: "text", text: parts.text, cut: true });
  if (parts.tail) pieces.push({ key: "tail", text: parts.tail, cut: false });
  return (
    <span className={cn("flex min-w-0 overflow-hidden whitespace-nowrap", className)} {...rest}>
      {pieces.map((piece, index) => (
        <span key={piece.key} className={piece.cut ? "min-w-0 truncate" : "shrink-0"} data-line-part={piece.key}>
          {index > 0 ? <span className="whitespace-pre"> · </span> : null}
          {piece.text}
        </span>
      ))}
    </span>
  );
}
