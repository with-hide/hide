import { GlobeIcon, PackageIcon } from "lucide-react";
import { useEffect, useState } from "react";
import type { Actions } from "./actions";
import { browserStillOf, addressShown } from "./browserViews";
import { FileMark } from "./components/file-mark";
import { requestFileBytesWithTotal, type FileSource } from "./fileBytes";
import { fileIcon } from "./fileIcons";
import { useInterfaceTranslation } from "./i18n/client";
import { cn } from "./lib/utils";
import { FIRST_TILES, type GalleryTile, type ViewThumb } from "./galleryTiles";
import type { RecordView } from "./snapshot";
import { localDeviceId } from "./snapshot";
import { useShellStore } from "./store";

const TILE = "flex min-w-0 flex-col overflow-hidden rounded-sm border border-border bg-card text-left";
const PICTURE = "relative flex aspect-video w-full min-w-0 overflow-hidden bg-muted";

/**
 * The 결과물 gallery (PRD session-docs-and-assistant D-40): two columns of
 * tiles, the first `FIRST_TILES` until `+N` shows the rest. Pressing a View's
 * tile brings that View forward where the record's lane is; an output its
 * turns only named opens nothing.
 */
export function RecordGallery({ tiles, omitted, actions }: { tiles: readonly GalleryTile[]; omitted: number; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const [all, setAll] = useState(false);
  const shown = all || tiles.length <= FIRST_TILES ? tiles : tiles.slice(0, FIRST_TILES - 1);
  const more = tiles.length - shown.length;
  return <>
    <ul className="grid min-w-0 grid-cols-2 gap-xs" data-record-gallery={tiles.length}>
      {shown.map((tile) => <li key={tile.kind === "view" ? `view:${tile.view.device_id}:${tile.view.checkout_path}:${tile.view.display_id}` : `artifact:${tile.name}`} className="min-w-0">
        {tile.kind === "view" ? <ViewTile view={tile.view} thumb={tile.thumb} actions={actions} /> : <ArtifactTile name={tile.name} />}
      </li>)}
      {more > 0 ? <li className="min-w-0">
        <button type="button" className={cn(TILE, "h-full items-center justify-center py-sm text-subhead font-semibold text-muted-foreground outline-none hover:bg-accent hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring")} aria-label={t("record.galleryMore", { count: more })} onClick={() => setAll(true)} data-record-gallery-more={more}>+{more}</button>
      </li> : null}
    </ul>
    {omitted > 0 && (all || more === 0) ? <p className="pt-xs text-micro text-muted-foreground" data-record-views-omitted={omitted}>{t("record.viewsOmitted", { count: omitted })}</p> : null}
  </>;
}

function ViewTile({ view, thumb, actions }: { view: RecordView; thumb: ViewThumb; actions: Actions }) {
  const address = view.url ? addressShown(view.url) : null;
  return <button type="button" className={cn(TILE, "w-full outline-none hover:border-ring focus-visible:ring-1 focus-visible:ring-ring")} title={[view.label, address ?? view.path].filter(Boolean).join("\n")} onClick={() => actions.openRecordView(view)} data-record-view={view.display_id} data-record-view-thumb={thumb}>
    <span className={PICTURE}>
      {thumb === "image" ? <ImagePicture view={view} /> : thumb === "lines" ? <LinesPicture lines={view.lines ?? []} /> : thumb === "page" ? <PagePicture view={view} address={address} /> : <MarkPicture view={view} />}
    </span>
    <span className="flex min-w-0 items-center gap-xs px-xs py-xxs text-micro text-subtle-foreground">
      <Mark view={view} />
      <span className="min-w-0 truncate" data-record-view-label="true">{view.label}</span>
    </span>
  </button>;
}

/** An output its turns named (a board, a document, a release): its name, with nothing to open. */
function ArtifactTile({ name }: { name: string }) {
  return <div className={cn(TILE, "h-full")} title={name}>
    <span className={cn(PICTURE, "items-center justify-center")}><PackageIcon aria-hidden="true" className="size-(--size-icon) text-muted-foreground" /></span>
    <span className="min-w-0 truncate px-xs py-xxs text-micro text-subtle-foreground" data-record-artifact={name}>{name}</span>
  </div>;
}

/** The View's kind as its tab draws it: a page's globe, a diff's ±, a file's type. */
function Mark({ view }: { view: RecordView }) {
  if (view.kind === "browser") return <GlobeIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />;
  if (view.kind === "diff") return <span aria-hidden="true" className="shrink-0 font-mono text-warning">±</span>;
  return <FileMark icon={fileIcon(view.label)} className="size-(--size-icon-sm)" />;
}

function MarkPicture({ view }: { view: RecordView }) {
  return <span className="flex w-full items-center justify-center">
    {view.kind === "browser" ? <GlobeIcon aria-hidden="true" className="size-(--size-icon) text-muted-foreground" /> : view.kind === "diff" ? <span aria-hidden="true" className="font-mono text-subhead text-warning">±</span> : <FileMark icon={fileIcon(view.label)} />}
  </span>;
}

/** An open document's first lines, as the editor holds them. */
function LinesPicture({ lines }: { lines: readonly string[] }) {
  return <span className="block w-full overflow-hidden whitespace-pre px-xs py-xxs font-mono text-micro text-muted-foreground" data-record-view-lines={lines.length}>{lines.join("\n")}</span>;
}

/** A page's capture when this window already took one, else its title and address. */
function PagePicture({ view, address }: { view: RecordView; address: string | null }) {
  const still = view.url ? browserStillOf(view.display_id, view.url) : null;
  if (still) return <img src={still} alt="" aria-hidden="true" draggable={false} className="size-full object-cover object-top" data-record-view-still="true" />;
  return <span className="flex w-full min-w-0 flex-col justify-center gap-xxs px-sm" data-record-view-card="true">
    <span className="line-clamp-2 text-caption font-medium text-foreground [overflow-wrap:anywhere]">{view.label}</span>
    {address ? <span className="truncate font-mono text-micro text-muted-foreground">{address}</span> : null}
  </span>;
}

/** An image is its own picture: its bytes, read once and kept for the next draw. */
function ImagePicture({ view }: { view: RecordView }) {
  const node = useShellStore((state) => localDeviceId(state.rest));
  const source: FileSource = view.device_id === node ? null : { device: view.device_id, root: view.checkout_path };
  const url = useImage(view.path, source);
  if (url === "failed") return <MarkPicture view={view} />;
  return url ? <img src={url} alt="" aria-hidden="true" draggable={false} className="size-full object-cover" data-record-view-image="true" /> : null;
}

/** Bytes past this draw the file's mark instead: a tile is no place to read a large file. */
const IMAGE_BYTES = 8 * 1024 * 1024;

/** The image's picture while its tile is drawn: read once, let go when the tile goes. */
function useImage(path: string, source: FileSource): string | "failed" | null {
  const [url, setUrl] = useState<string | "failed" | null>(null);
  const device = source?.device ?? null;
  const root = source?.root ?? null;
  useEffect(() => {
    let live = true;
    let made: string | null = null;
    setUrl(null);
    requestFileBytesWithTotal(path, { length: IMAGE_BYTES }, device && root ? { device, root } : null)
      .then(({ bytes, total }) => {
        if (!live) return;
        if (total > IMAGE_BYTES) return setUrl("failed");
        made = URL.createObjectURL(new Blob([bytes as BlobPart], { type: path.toLowerCase().endsWith(".svg") ? "image/svg+xml" : "image/*" }));
        setUrl(made);
      })
      .catch(() => { if (live) setUrl("failed"); });
    return () => {
      live = false;
      if (made) URL.revokeObjectURL(made);
    };
  }, [path, device, root]);
  return url;
}
