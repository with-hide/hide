import type { DeskOutput } from "../../desk";
import { useInterfaceTranslation } from "../../i18n/client";
import { cn } from "../../lib/utils";
import { PrMark } from "../pr-mark";
import { RowButton } from "../ui/row-button";
import { OutputMark } from "./desk-job-card";

const PICTURE = "relative flex aspect-[5/2] w-full min-w-0 overflow-hidden rounded-sm border border-border bg-muted outline-none hover:border-ring focus-visible:ring-1 focus-visible:ring-ring";

/**
 * A thread's 결과물 (D-65, B30, B31), drawn as the session record's gallery
 * draws its own (`RecordGallery`): an image or a page with a picture as a
 * three-column thumbnail, a document or a PR as a compact row. The output
 * shown in the 보기 column is ringed; an output not looked at yet carries a
 * dot. Pressing one opens it there.
 */
export function DeskOutputs({ outputs, selected = null, prs = {}, onOpen }: {
  outputs: readonly DeskOutput[];
  selected?: string | null;
  /** A PR output's state, by its id, for its mark. */
  prs?: Readonly<Record<string, Parameters<typeof PrMark>[0]["state"]>>;
  onOpen: (output: DeskOutput) => void;
}) {
  const { t } = useInterfaceTranslation();
  const pictures = outputs.filter((output) => output.thumb && (output.kind === "image" || output.kind === "page"));
  const rows = outputs.filter((output) => !pictures.includes(output));
  if (outputs.length === 0) return <p className="text-caption text-muted-foreground" data-desk-outputs="0">{t("desk.outputs.empty")}</p>;
  return (
    <div className="flex min-w-0 flex-col gap-xs" data-desk-outputs={outputs.length}>
      {pictures.length ? (
        <ul className="grid min-w-0 grid-cols-3 gap-xs">
          {pictures.map((output) => (
            <li key={output.id} className="min-w-0">
              <RowButton className={cn(PICTURE, "p-0", selected === output.id && "border-ring ring-1 ring-ring")} aria-label={output.label} aria-pressed={selected === output.id} title={output.label} onClick={() => onOpen(output)} data-desk-output={output.id} data-desk-output-thumb={output.kind}>
                <img src={output.thumb!} alt="" aria-hidden="true" draggable={false} className="size-full object-cover object-top" />
                {!output.seen ? <span aria-hidden="true" className="absolute right-xxs top-xxs size-(--size-tab-status-dot) rounded-full bg-primary" data-desk-output-unseen="true" /> : null}
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
                {output.kind === "pr" && prs[output.id] ? <PrMark state={prs[output.id]!} compact /> : <OutputMark output={output} />}
                <span className="min-w-0 flex-1 truncate text-subtle-foreground">{output.label}</span>
                {!output.seen ? <span aria-hidden="true" className="size-(--size-tab-status-dot) shrink-0 rounded-full bg-primary" data-desk-output-unseen="true" /> : null}
              </RowButton>
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}
