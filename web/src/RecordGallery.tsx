import { CircleDotIcon, GlobeIcon } from "lucide-react";
import { useState } from "react";
import type { Actions } from "./actions";
import { browserStillOf, addressShown } from "./browserViews";
import { FileMark } from "./components/file-mark";
import type { FileSource } from "./fileBytes";
import { useImageFile } from "./imageFile";
import { fileIcon } from "./fileIcons";
import { useInterfaceTranslation } from "./i18n/client";
import { cn } from "./lib/utils";
import { FIRST_PICTURES, FIRST_ROWS, pictureOf, rowDetail, type GalleryTile } from "./galleryTiles";
import type { RecordView } from "./snapshot";
import { localDeviceId } from "./snapshot";
import { useShellStore } from "./store";

const PICTURE = "relative flex aspect-[5/2] w-full min-w-0 overflow-hidden rounded-sm border border-border bg-muted outline-none hover:border-ring focus-visible:ring-1 focus-visible:ring-ring";
const ROW = "flex w-full min-w-0 items-center gap-xs rounded-xs px-xxs py-xxs text-left text-caption outline-none focus-visible:ring-1 focus-visible:ring-ring";

/**
 * The 결과물 section (PRD session-docs-and-assistant D-40): the items with a
 * real picture as a strip of three-column thumbnails, every other one as a
 * compact row, the first `FIRST_PICTURES` and `FIRST_ROWS` until `+N` shows
 * the rest. Pressing a View brings it forward where the record's lane is;
 * pressing an issue opens its page.
 */
export function RecordGallery({ tiles, omitted, actions }: { tiles: readonly GalleryTile[]; omitted: number; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const [all, setAll] = useState(false);
  const sorted = tiles.map((tile) => ({ tile, picture: pictureOf(tile, (view) => (view.url ? browserStillOf(view.display_id, view.url) : null)) }));
  const pictures = sorted.filter((one) => one.picture !== null);
  const rows = sorted.filter((one) => one.picture === null);
  const shownPictures = all ? pictures : pictures.slice(0, FIRST_PICTURES);
  const shownRows = all ? rows : rows.slice(0, FIRST_ROWS);
  const more = tiles.length - shownPictures.length - shownRows.length;
  return <div className="flex min-w-0 flex-col gap-xs" data-record-gallery={tiles.length}>
    {shownPictures.length ? <ul className="grid min-w-0 grid-cols-3 gap-xs" data-record-gallery-pictures={shownPictures.length}>
      {shownPictures.map(({ tile, picture }) => tile.kind === "view" && picture ? <li key={keyOf(tile)} className="min-w-0">
        <button type="button" className={PICTURE} title={[tile.view.label, tile.view.url ? addressShown(tile.view.url) : tile.view.path].filter(Boolean).join("\n")} aria-label={tile.view.label} onClick={() => actions.openRecordView(tile.view)} data-record-view={tile.view.display_id} data-record-view-thumb={picture.kind}>
          {picture.kind === "still" ? <img src={picture.src} alt="" aria-hidden="true" draggable={false} className="size-full object-cover object-top" data-record-view-still="true" /> : <ImagePicture view={tile.view} />}
        </button>
      </li> : null)}
    </ul> : null}
    {shownRows.length ? <ul className="flex min-w-0 flex-col" data-record-gallery-rows={shownRows.length}>
      {shownRows.map(({ tile }) => <li key={keyOf(tile)} className="min-w-0"><Row tile={tile} actions={actions} /></li>)}
    </ul> : null}
    {more > 0 ? <button type="button" className={cn(ROW, "text-micro text-muted-foreground hover:text-foreground")} aria-label={t("record.galleryMore", { count: more })} onClick={() => setAll(true)} data-record-gallery-more={more}>+{more}</button> : null}
    {omitted > 0 && (all || more === 0) ? <p className="text-micro text-muted-foreground" data-record-views-omitted={omitted}>{t("record.viewsOmitted", { count: omitted })}</p> : null}
  </div>;
}

function keyOf(tile: GalleryTile): string {
  return tile.kind === "view" ? `view:${tile.view.device_id}:${tile.view.checkout_path}:${tile.view.display_id}` : `issue:${tile.issue.repository}#${tile.issue.number}`;
}

/** An item with no picture: its kind's mark, its name and one short muted detail; a closed issue is dimmed. */
function Row({ tile, actions }: { tile: GalleryTile; actions: Actions }) {
  const detail = rowDetail(tile);
  const view = tile.kind === "view" ? tile.view : null;
  const issue = tile.kind === "issue" ? tile.issue : null;
  const name = view ? view.label : issue!.title;
  const open = () => (view ? actions.openRecordView(view) : actions.openLink(issue!.url, false));
  return <button type="button" className={cn(ROW, "hover:bg-accent", issue?.state === "closed" && "opacity-(--opacity-dimmed)")} title={[name, view ? view.url ?? view.path : issue!.url].filter(Boolean).join("\n")} onClick={open}
    data-record-view={view?.display_id} data-record-view-thumb={view ? "row" : undefined} data-record-issue={issue?.number} data-issue-closed={issue?.state === "closed" || undefined}>
    {view ? <Mark view={view} /> : <CircleDotIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />}
    <span className="min-w-0 shrink truncate text-subtle-foreground" data-record-view-label={view ? "true" : undefined}>{name}</span>
    {detail ? <span className="min-w-0 flex-1 truncate text-micro text-muted-foreground" data-record-view-detail="true">{detail}</span> : null}
  </button>;
}

/** The View's kind as its tab draws it: a page's globe, a file's type. */
function Mark({ view }: { view: RecordView }) {
  if (view.kind === "browser") return <GlobeIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />;
  return <FileMark icon={fileIcon(view.label)} className="size-(--size-icon-sm)" />;
}

/** An image is its own picture: its bytes, read once and kept for the next draw. */
function ImagePicture({ view }: { view: RecordView }) {
  const node = useShellStore((state) => localDeviceId(state.rest));
  const source: FileSource = view.device_id === node ? null : { device: view.device_id, root: view.checkout_path };
  const url = useImageFile(view.path, source);
  if (url === "failed") return <span className="flex w-full items-center justify-center"><FileMark icon={fileIcon(view.label)} /></span>;
  return url ? <img src={url} alt="" aria-hidden="true" draggable={false} className="size-full object-cover" data-record-view-image="true" /> : null;
}
