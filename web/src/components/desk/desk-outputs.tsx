import type { DeskOutput } from "../../desk";
import { useInterfaceTranslation } from "../../i18n/client";
import { cn } from "../../lib/utils";
import { RowButton } from "../ui/row-button";
import { OutputMark, UnseenDot } from "./desk-job-card";

const PICTURE = "relative flex aspect-[3/2] w-(--size-desk-output-thumb) overflow-hidden rounded-sm border border-border bg-muted outline-none hover:border-ring focus-visible:ring-1 focus-visible:ring-ring";

/**
 * A thread's 결과물 (D-65, B30, B31), drawn as the session record's gallery
 * draws its own (`RecordGallery`): an image or a page with a picture as a
 * small thumbnail in a wrapping row, a document or a PR as a compact row. The output
 * shown in the 보기 column is ringed; an output not looked at yet carries a
 * dot. Pressing one opens it there.
 */
export function DeskOutputs({ outputs, selected = null, onOpen }: {
  outputs: readonly DeskOutput[];
  selected?: string | null;
  onOpen: (output: DeskOutput) => void;
}) {
  const { t } = useInterfaceTranslation();
  const pictures = outputs.filter((output) => output.thumb && (output.kind === "image" || output.kind === "page"));
  const rows = outputs.filter((output) => !pictures.includes(output));
  if (outputs.length === 0) return <p className="text-caption text-muted-foreground" data-desk-outputs="0">{t("desk.outputs.empty")}</p>;
  return (
    <div className="flex min-w-0 flex-col gap-xs" data-desk-outputs={outputs.length}>
      {pictures.length ? (
        <ul className="flex min-w-0 flex-wrap gap-xs">
          {pictures.map((output) => (
            <li key={output.id}>
              <RowButton className={cn(PICTURE, "p-0", selected === output.id && "border-ring ring-1 ring-ring")} aria-label={output.seen ? output.label : `${output.label} · ${t("desk.outputs.unseen")}`} aria-pressed={selected === output.id} title={output.label} onClick={() => onOpen(output)} data-desk-output={output.id} data-desk-output-thumb={output.kind}>
                <img src={output.thumb!} alt="" aria-hidden="true" draggable={false} className="size-full object-cover object-top" />
                {!output.seen ? <UnseenDot className="absolute right-xxs top-xxs" /> : null}
              </RowButton>
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
                {!output.seen ? <UnseenDot /> : null}
              </RowButton>
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}
