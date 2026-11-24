import { GlobeIcon } from "lucide-react";
import { FileMark } from "./components/file-mark";
import { fileIcon } from "./fileIcons";
import type { ViewDisplaySnapshot } from "./snapshot";

/** A page's globe, a diff's comparison mark or a file's type mark, never colour alone (D-15): the mark of a View that is not a card. */
export function ViewKindMark({ kind, label }: { kind: ViewDisplaySnapshot["kind"]; label: string }) {
  if (kind === "browser") return <GlobeIcon aria-hidden="true" data-view-mark="browser" className="size-(--size-icon) shrink-0 text-muted-foreground" />;
  if (kind === "diff") {
    return (
      <span aria-hidden="true" data-view-mark="diff" className="shrink-0 font-mono text-warning">
        ±
      </span>
    );
  }
  return <FileMark icon={fileIcon(label)} data-view-mark="file" />;
}
