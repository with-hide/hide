import type { DeskOutput } from "../../desk";
import { useInterfaceTranslation } from "../../i18n/client";
import { cn } from "../../lib/utils";
import { RowButton } from "../ui/row-button";
import { OutputMark, UnseenMark } from "./desk-job-card";
import { PreviewTile, tileOf } from "./preview-tile";

/**
 * A thread's 결과물 (D-65, B30, B31; feedback 2 frame M 나): every file or
 * page as the one preview tile (`PreviewTile`), in a wrapping row, a pull
 * request as a compact row. The output shown on Desk's desk is ringed; an
 * output not looked at yet carries a look eye. Pressing one opens it there.
 */
export function DeskOutputs({ outputs, selected = null, onOpen }: {
  outputs: readonly DeskOutput[];
  selected?: string | null;
  onOpen: (output: DeskOutput) => void;
}) {
  const { t } = useInterfaceTranslation();
  const tiles = outputs.filter((output) => output.kind !== "pr");
  const rows = outputs.filter((output) => output.kind === "pr");
  if (outputs.length === 0) return <p className="text-caption text-muted-foreground" data-desk-outputs="0">{t("desk.outputs.empty")}</p>;
  return (
    <div className="flex min-w-0 flex-col gap-xs" data-desk-outputs={outputs.length}>
      {tiles.length ? (
        <ul className="flex min-w-0 flex-wrap gap-sm">
          {tiles.map((output) => (
            <li key={output.id}>
              <PreviewTile file={tileOf(output)} selected={selected === output.id} unseen={!output.seen} onOpen={() => onOpen(output)} data-desk-output={output.id} data-desk-output-thumb={output.kind} />
            </li>
          ))}
        </ul>
      ) : null}
      {rows.length ? (
        <ul className="flex min-w-0 flex-col">
          {rows.map((output) => (
            <li key={output.id} className="min-w-0">
              <RowButton className={cn("px-xxs py-xxs text-caption", selected === output.id && "bg-accent")} aria-pressed={selected === output.id} onClick={() => onOpen(output)} data-desk-output={output.id} data-desk-output-thumb="row">
                <OutputMark output={output} />
                <span className="min-w-0 flex-1 truncate text-subtle-foreground">{output.label}</span>
                {!output.seen ? <UnseenMark /> : null}
              </RowButton>
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}
