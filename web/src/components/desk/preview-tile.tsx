import { LoaderCircleIcon, PlayIcon, TriangleAlertIcon, WifiOffIcon } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { browserStillOf, useBrowserStore } from "../../browserViews";
import type { DeskOutput } from "../../desk";
import { deviceConnected } from "../../devices";
import { forgetPreview, watchPreview, type Preview } from "../../filePreview";
import { useInterfaceTranslation } from "../../i18n/client";
import { formatBytes } from "../../i18n/format";
import { requireInterfaceLanguage } from "../../i18n/locale";
import { useFileSource } from "../../imageFile";
import { cn } from "../../lib/utils";
import { machineName } from "../../screenMachine";
import { localDeviceId } from "../../snapshot";
import { useShellStore } from "../../store";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import { RowOverlayButton } from "../ui/row-button";
import { ViewKindMark } from "../../viewMark";
import { UnseenMark } from "./desk-job-card";

/**
 * A file or page a tile shows: an output's (`tileOf`), or a path an answer
 * named. `device` and `root` say where the file is read; a page has its
 * `url` and no path.
 */
export type TileFile = { id: string; label: string; path: string | null; url: string | null; device: string | null; root: string | null };

export function tileOf(output: DeskOutput): TileFile {
  return { id: output.id, label: output.label, path: output.path, url: output.url, device: output.device_id, root: output.checkout };
}

/** The preview `file` shows now: made once in this window (`filePreview.ts`), a page's still, or why there is none. */
function usePreview(file: TileFile): { preview: Preview; retry: () => void } {
  const offline = useShellStore((s) => file.device !== null && file.device !== localDeviceId(s.rest) && !deviceConnected(s.rest, file.device));
  const source = useFileSource(file.device, file.root);
  const changes = useShellStore((s) => (file.path ? (s.fileChanges[file.path] ?? 0) : 0));
  const [attempt, setAttempt] = useState(0);
  const key = JSON.stringify([source?.device ?? null, source?.root ?? null, file.path, changes, attempt]);
  const [made, setMade] = useState<Preview>({ state: "reading" });
  // A page's still taken after the tile appeared redraws it.
  useBrowserStore((s) => (file.url ? s.stills[file.id] : null));
  const path = offline ? null : file.path;
  useEffect(() => {
    if (path === null) return undefined;
    const watch = watchPreview(key, path, source, setMade);
    setMade(watch.now);
    return watch.stop;
    // `key` names the source, the path and its version.
  }, [key]);
  const retry = () => {
    forgetPreview(key);
    setAttempt((now) => now + 1);
  };
  if (offline) return { preview: { state: "offline" }, retry };
  if (file.path) return { preview: made, retry };
  const still = file.url ? browserStillOf(file.id, file.url) : null;
  return { preview: still ? { state: "picture", picture: still, pages: null, seconds: null } : { state: "none" }, retry };
}

function clock(seconds: number): string {
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

/** Inside the tile: the picture with its marks, a text's head, or the kind icon and why there is no preview. */
function TileFace({ file, preview, onRetry }: { file: TileFile; preview: Preview; onRetry: () => void }) {
  const { t, i18n } = useInterfaceTranslation();
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  if (preview.state === "picture") {
    return (
      <>
        <img src={preview.picture} alt="" aria-hidden="true" draggable={false} className="size-full object-cover object-top" data-desk-tile-picture="true" />
        {preview.seconds !== null ? (
          <span className="absolute inset-0 flex items-center justify-center" aria-label={t("desk.tile.play", { time: clock(preview.seconds) })}>
            <span className="flex size-(--size-control-sm) items-center justify-center rounded-full bg-background/80 text-foreground"><PlayIcon aria-hidden="true" className="size-(--size-icon-sm) fill-current" /></span>
          </span>
        ) : null}
        {preview.pages !== null ? <Badge variant="secondary" className="absolute right-xxs bottom-xxs" data-desk-tile-pages={preview.pages}>{t("desk.tile.pages", { count: preview.pages })}</Badge> : null}
        {preview.seconds !== null ? <Badge variant="secondary" className="absolute right-xxs bottom-xxs tabular-nums" data-desk-tile-seconds={preview.seconds}>{clock(preview.seconds)}</Badge> : null}
      </>
    );
  }
  if (preview.state === "text") {
    return (
      <span className="flex size-full min-w-0 flex-col gap-xxs bg-card px-sm py-xs text-left" data-desk-tile-text="true">
        <span className="truncate text-caption font-semibold text-foreground">{preview.title || file.label}</span>
        <span className="line-clamp-3 break-keep text-micro text-muted-foreground [overflow-wrap:anywhere]">{preview.lines.join(" ")}</span>
      </span>
    );
  }
  const reason: ReactNode = preview.state === "reading" ? t("desk.tile.reading")
    : preview.state === "failed" ? <>{t("desk.tile.failed")} · <Button variant="text" size="inline" className="pointer-events-auto font-semibold text-foreground" onClick={onRetry} data-desk-tile-retry="true">{t("desk.tile.retry")}</Button></>
    : preview.state === "too_large" ? t("desk.tile.tooLarge", { size: formatBytes(requireInterfaceLanguage(i18n.language), preview.bytes) })
    : preview.state === "offline" && file.device ? t("desk.tile.offline", { device: machineName(devices, file.device, t) })
    : null;
  const icon = preview.state === "reading" ? <LoaderCircleIcon aria-hidden="true" className="size-(--size-icon-sm) text-muted-foreground motion-safe:animate-spin" />
    : preview.state === "failed" ? <TriangleAlertIcon aria-hidden="true" className="size-(--size-icon-sm) text-muted-foreground" />
    : preview.state === "offline" ? <WifiOffIcon aria-hidden="true" className="size-(--size-icon-sm) text-muted-foreground" />
    // A page (an address and no path) is marked as a page View is, never by its label, which is its title.
    : <ViewKindMark kind={file.path === null && file.url !== null ? "browser" : "file"} label={file.path ?? file.label} />;
  return (
    <span className="flex size-full min-w-0 flex-col items-center justify-center gap-xxs px-xs text-center" data-desk-tile-reason={preview.state}>
      {icon}
      {reason ? <span className="line-clamp-2 text-micro text-muted-foreground">{reason}</span> : null}
    </span>
  );
}

/**
 * One file as a tile (feedback 2 frame M [1]), the same in Desk's answers,
 * the session's opened-view line, the shelf's list and a file chip's card:
 * 150 by 100 with its name under it. An image is its picture, a PDF its
 * first page with its page count, a video its first frame with the play
 * mark and its length, a page the still the tab panel's page view has, a
 * Markdown or text file its title and first three lines; with no preview,
 * its kind icon and why (읽는 중, 못 읽음 · 다시, 너무 큼, 기기 연결 끊김).
 * Pressing it opens it (`onOpen`); without, it is the picture alone, as in
 * a chip's card. `fill` sizes it to its grid cell instead of 150 wide;
 * `named={false}` leaves out the name under it, where a card says the path.
 */
export function PreviewTile({ file, ...rest }: { file: TileFile; selected?: boolean; unseen?: boolean; fill?: boolean; named?: boolean; onOpen?: () => void } & Record<`data-${string}`, string>) {
  const { preview, retry } = usePreview(file);
  return <PreviewTileView file={file} preview={preview} onRetry={retry} {...rest} />;
}

/** A tile drawn from a preview a caller holds: `PreviewTile`'s face, and the gallery's for each state. */
export function PreviewTileView({ file, preview, onRetry, selected = false, unseen = false, fill = false, named = true, onOpen, ...data }: { file: TileFile; preview: Preview; onRetry: () => void; selected?: boolean; unseen?: boolean; fill?: boolean; named?: boolean; onOpen?: () => void } & Record<`data-${string}`, string>) {
  const { t } = useInterfaceTranslation();
  return (
    <span className={cn("flex min-w-0 flex-col gap-xxs", !fill && "w-(--size-desk-output-thumb)")} data-desk-tile={preview.state} {...data}>
      <span className={cn("relative block aspect-[3/2] w-full overflow-hidden rounded-sm border border-border bg-muted", onOpen && "hover:border-ring", selected && "border-ring ring-1 ring-ring")}>
        {onOpen ? <RowOverlayButton hover={false} className="z-10 rounded-sm" aria-label={unseen ? `${file.label} · ${t("desk.outputs.unseen")}` : file.label} aria-pressed={selected} title={file.label} onClick={onOpen} data-desk-tile-open="true" /> : null}
        <span className={cn("pointer-events-none relative block size-full", onOpen && "z-20")}><TileFace file={file} preview={preview} onRetry={onRetry} /></span>
        {unseen ? <UnseenMark className="absolute top-xxs right-xxs z-20" /> : null}
      </span>
      {named ? <span className="min-w-0 truncate text-caption text-subtle-foreground" title={file.label}>{file.label}</span> : null}
    </span>
  );
}

/** A tile's picture alone, drawn small (the shelf's chip): the same preview, `fallback` while it has none. */
export function PreviewPicture({ file, className, fallback }: { file: TileFile; className: string; fallback: ReactNode }) {
  const { preview } = usePreview(file);
  return preview.state === "picture" ? <img src={preview.picture} alt="" aria-hidden="true" draggable={false} className={className} /> : fallback;
}
