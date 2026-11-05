// The phone's 볼 것 (PRD mobile-companion-ui B10-B19, B23-B27, D-06..D-10): the Views the agent opened that are open now, split
// into not seen and seen, and one View opened full screen as a change list, a document, an image or the words that say why
// there is nothing to draw. The phone holds no path: a row is a View id, and hided finds the file (D-16).

import { FileDiffIcon, FileIcon, FileTextIcon, GlobeIcon, ImageIcon, LaptopIcon, Loader2Icon, RefreshCwIcon } from "lucide-react";
import { useMemo, useRef, useState, type PointerEvent, type ReactNode } from "react";
import { Elapsed, useElapsed } from "../components/elapsed";
import { Button } from "../components/ui/button";
import { RowButton } from "../components/ui/row-button";
import { formatBytes } from "../i18n/format";
import { requireInterfaceLanguage } from "../i18n/locale";
import { useInterfaceTranslation } from "../i18n/translator";
import { clampOffset, countState, fileTag, splitPath, zoomBy, type LookContent } from "./lookContent";
import { openLook, refreshLook, retryLook } from "./connection";
import { renderMarkdown } from "./markdown";
import { deviceNameOf, splitLooks, type LookFile, type LookKind, type LookRow } from "./protocol";
import { usePhone, type Looks } from "./store";

const KIND_ICON: Record<LookKind, typeof FileIcon> = { changes: FileDiffIcon, doc: FileTextIcon, image: ImageIcon, page: GlobeIcon, other: FileIcon };

/** The kind's word, as the list and the viewer's head say it (B12). */
export function useKindWord() {
  const { t } = useInterfaceTranslation();
  return (kind: LookKind) => t(`mobile.look.kind.${kind}`);
}

/** The list: 안 본 것 and 본 것, each with its rows; nothing is drawn until hided has listed the Views (B12). */
export function LooksList({ looks }: { looks: Looks | null }) {
  const { t } = useInterfaceTranslation();
  const connected = usePhone((s) => s.connected);
  const parts = useMemo(() => splitLooks(looks?.rows ?? []), [looks]);
  return (
    <div className="min-h-0 flex-1 overflow-auto overscroll-contain bg-background px-lg pb-md" data-phone-looks={looks ? looks.rows.length : "loading"}>
      {!looks ? <p className="pt-md text-body text-muted-foreground">{t("mobile.loading")}</p> : null}
      <LookGroup id="unseen" title={t("mobile.look.unseen", { count: parts.unseen.length })} rows={parts.unseen} disabled={!connected} />
      <LookGroup id="seen" title={t("mobile.look.seen", { count: parts.seen.length })} rows={parts.seen} disabled={!connected} />
      {looks && looks.omitted > 0 ? (
        <p className="pt-md text-body text-muted-foreground" data-phone-looks-omitted={looks.omitted}>
          {t("mobile.look.omitted", { count: looks.omitted })}
        </p>
      ) : null}
    </div>
  );
}

function LookGroup({ id, title, rows, disabled }: { id: "unseen" | "seen"; title: string; rows: LookRow[]; disabled: boolean }) {
  if (rows.length === 0) return null;
  return (
    <section className="pt-lg" data-phone-look-group={id}>
      <h2 className="pb-xs text-body text-muted-foreground">{title}</h2>
      <ul className="divide-y divide-border border-b border-border">
        {rows.map((row) => (
          <li key={row.id}>
            <LookItem row={row} disabled={disabled} />
          </li>
        ))}
      </ul>
    </section>
  );
}

function LookItem({ row, disabled }: { row: LookRow; disabled: boolean }) {
  const { t } = useInterfaceTranslation();
  const word = useKindWord()(row.kind);
  return (
    <RowButton
      disabled={disabled}
      aria-label={[row.unseen ? t("mobile.look.unseen", { count: 1 }) : null, row.name, word].filter(Boolean).join(", ")}
      data-phone-look={row.id}
      data-phone-look-unseen={row.unseen ? "true" : "false"}
      className="min-h-(--size-touch-target) gap-md rounded-none px-none py-md hover:bg-transparent active:bg-accent"
      onClick={() => openLook(row)}
    >
      <Tile row={row} />
      <span className="flex min-w-0 flex-1 flex-col gap-xxs">
        <span className={`line-clamp-2 break-all text-title font-medium ${row.unseen ? "text-foreground" : "text-subtle-foreground"}`}>{row.name}</span>
        <span className="flex min-w-0 gap-xs text-body text-muted-foreground">
          <span className="truncate">{word}</span>
          <span aria-hidden="true">·</span>
          <Elapsed since={row.at} className="shrink-0 font-mono" />
        </span>
      </span>
    </RowButton>
  );
}

/** The preview slot: the kind's icon, with the dot of a View not seen yet (B12). */
function Tile({ row }: { row: LookRow }) {
  const Icon = KIND_ICON[row.kind];
  return (
    <span aria-hidden="true" className="relative flex size-(--size-touch-target) shrink-0 items-center justify-center overflow-hidden rounded-md bg-card text-muted-foreground">
      <Icon className="size-(--size-icon-lg)" />
      {row.unseen ? <span className="absolute left-xxs top-xxs size-(--size-status-mark) rounded-full bg-warning" data-phone-look-dot="true" /> : null}
    </span>
  );
}

/** The viewer's body for the View on screen (B14-B23, B26, B27); `refreshing` is a page's screen being taken again. */
export function LookViewer({ row, content, refreshing }: { row: LookRow; content: LookContent; refreshing: boolean }) {
  const { t } = useInterfaceTranslation();
  const word = useKindWord()(row.kind);
  const connected = usePhone((s) => s.connected);
  switch (content.state) {
    case "loading":
      return (
        <Centered state="loading">
          <Loader2Icon aria-hidden="true" className="size-(--size-icon-lg) animate-spin text-muted-foreground" />
          <p role="status" className="text-body text-muted-foreground">
            {t("mobile.loading")}
          </p>
        </Centered>
      );
    case "text":
      return <TextBody content={content} />;
    case "image":
      return <ImageBody name={row.name} content={content} />;
    case "changes":
      return <ChangesBody content={content} />;
    case "shot":
      return <ShotBody name={row.name} content={content} refreshing={refreshing} />;
    case "page":
      // The address and the one sentence, with the refresh still there (B21): why it cannot be taken is the desktop's log.
      return (
        <div className="flex min-h-0 flex-1 flex-col bg-background px-lg py-md" data-phone-look-state="page">
          <PageAddress address={content.url || row.url || row.name} refreshing={refreshing} />
          <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-sm text-center">
            <GlobeIcon aria-hidden="true" className="size-(--size-icon-lg) text-muted-foreground" />
            <p role="status" className="text-title text-foreground">
              {t("mobile.look.pageUnavailable")}
            </p>
          </div>
        </div>
      );
    case "other":
      return (
        <Centered state="other">
          <p className="max-w-full break-all text-title font-medium text-foreground">{row.name}</p>
          <p className="text-body text-muted-foreground">{word}</p>
          <p className="text-body text-muted-foreground">{t("mobile.look.openOnMac")}</p>
        </Centered>
      );
    case "too_large":
      return (
        <Centered state="too_large">
          <p className="max-w-full break-all text-title font-medium text-foreground">{row.name}</p>
          <p className="text-body text-muted-foreground">{t("mobile.look.imageTooLarge")}</p>
        </Centered>
      );
    case "closed":
      return (
        <Centered state="closed">
          <p role="status" className="text-title text-foreground">
            {t("mobile.look.closed")}
          </p>
        </Centered>
      );
    case "device_unreachable":
      return (
        <Centered state="device_unreachable">
          <p role="status" className="text-title text-foreground">
            {t("mobile.look.deviceUnreachable")}
          </p>
        </Centered>
      );
    case "failed":
      return (
        <Centered state="failed">
          <p role="alert" className="text-title text-foreground">
            {t("mobile.look.failed")}
          </p>
          <Button variant="secondary" className="h-(--size-touch-target) px-lg text-title" data-phone-look-retry="true" disabled={!connected} onClick={retryLook}>
            {t("mobile.look.retry")}
          </Button>
        </Centered>
      );
  }
}

/** A page's address with the button that takes its screen again; the button waits for a connection and for the screen in flight (B20, B45). */
function PageAddress({ address, refreshing }: { address: string; refreshing: boolean }) {
  const { t } = useInterfaceTranslation();
  const connected = usePhone((s) => s.connected);
  return (
    <div className="flex shrink-0 items-center gap-sm pb-sm">
      <span className="flex min-h-(--size-touch-target) min-w-0 flex-1 items-center rounded-md bg-secondary px-md" data-phone-look-address="true">
        <span className="break-all font-mono text-body text-subtle-foreground line-clamp-2">{address}</span>
      </span>
      <Button
        variant="secondary"
        size="icon"
        className="size-(--size-touch-target) shrink-0"
        aria-label={t("mobile.look.refresh")}
        aria-busy={refreshing}
        data-phone-look-refresh="true"
        disabled={!connected || refreshing}
        onClick={refreshLook}
      >
        {refreshing ? <Loader2Icon aria-hidden="true" className="size-(--size-icon) animate-spin" /> : <RefreshCwIcon aria-hidden="true" className="size-(--size-icon)" />}
      </Button>
    </div>
  );
}

/** A page's screen as the desktop app showed it, with who took it and how long ago under it (B20). */
function ShotBody({ name, content, refreshing }: { name: string; content: Extract<LookContent, { state: "shot" }>; refreshing: boolean }) {
  const { t } = useInterfaceTranslation();
  const devices = usePhone((s) => s.devices);
  const macName = usePhone((s) => s.macName);
  const elapsed = useElapsed(content.takenAt);
  return (
    <div className="flex min-h-0 flex-1 flex-col bg-background px-lg py-md" data-phone-look-state="shot">
      <PageAddress address={content.address} refreshing={refreshing} />
      <div className="min-h-0 flex-1 overflow-auto overscroll-contain rounded-md border border-border bg-card">
        <img src={content.url} alt={name} draggable={false} className="block h-auto w-full select-none" data-phone-look-shot="true" />
      </div>
      <p className="shrink-0 pt-sm text-body text-muted-foreground" data-phone-look-taken={content.takenBy}>
        {t("mobile.look.shotTaken", { device: deviceNameOf(devices, content.takenBy, macName), elapsed: elapsed ?? "" })}
      </p>
    </div>
  );
}

function Centered({ state, children }: { state: string; children: ReactNode }) {
  return (
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-sm bg-background px-lg text-center" data-phone-look-state={state}>
      {children}
    </div>
  );
}

/** A Markdown document drawn, any other text in fixed-width letters wrapped to the phone's width; a cut one says so at its end (B16, B17). */
function TextBody({ content }: { content: Extract<LookContent, { state: "text" }> }) {
  const { t } = useInterfaceTranslation();
  const body = useMemo(() => (content.markdown ? renderMarkdown(content.text) : null), [content]);
  return (
    <div className="min-h-0 flex-1 overflow-auto overscroll-contain bg-background px-lg py-md" data-phone-look-state={content.markdown ? "markdown" : "text"}>
      {body ? (
        <div className="space-y-sm wrap-anywhere text-body leading-normal text-foreground">{body}</div>
      ) : (
        <pre className="m-none whitespace-pre-wrap wrap-anywhere font-mono text-body leading-normal text-foreground">{content.text}</pre>
      )}
      {content.truncated ? (
        <p className="pt-md text-body text-muted-foreground" data-phone-look-truncated="true">
          {t("mobile.look.truncated")}
        </p>
      ) : null}
    </div>
  );
}

/** The changed files: how many and how many lines, then each file with its folder and its counts, and no code (B14, B15). */
function ChangesBody({ content }: { content: Extract<LookContent, { state: "changes" }> }) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="min-h-0 flex-1 overflow-auto overscroll-contain bg-background px-lg py-md" data-phone-look-state="changes">
      {content.count === 0 ? (
        <p role="status" className="py-xl text-center text-title text-muted-foreground" data-phone-look-empty="true">
          {t("mobile.look.noChanges")}
        </p>
      ) : (
        <>
          <p className="flex flex-wrap gap-sm pb-md text-body">
            <span className="rounded-md bg-secondary px-sm py-xs text-foreground" data-phone-look-files={content.count}>
              {t("mobile.look.fileCount", { count: content.count })}
            </span>
            <span className="flex gap-xs rounded-md bg-secondary px-sm py-xs text-foreground">
              <span>{t("mobile.look.lines")}</span>
              <Counts added={content.added} removed={content.removed} />
            </span>
          </p>
          <ul className="divide-y divide-border border-y border-border">
            {content.files.map((file) => (
              <li key={file.path}>
                <FileRow file={file} />
              </li>
            ))}
          </ul>
          {content.count > content.files.length ? (
            <p className="pt-sm text-body text-muted-foreground">{t("mobile.look.moreFiles", { count: content.count - content.files.length })}</p>
          ) : null}
          <p className="mt-lg flex items-start gap-sm rounded-md bg-card px-md py-md text-body leading-relaxed text-subtle-foreground">
            <LaptopIcon aria-hidden="true" className="mt-xxs size-(--size-icon) shrink-0" />
            <span className="min-w-0 break-keep wrap-anywhere">{t("mobile.look.codeOnMac")}</span>
          </p>
        </>
      )}
    </div>
  );
}

function Counts({ added, removed }: { added: number; removed: number }) {
  return (
    <span className="font-mono">
      <span className="text-success">+{added}</span> <span className="text-destructive">−{removed}</span>
    </span>
  );
}

function FileRow({ file }: { file: LookFile }) {
  const { t } = useInterfaceTranslation();
  const { name, folder } = splitPath(file.path);
  const created = file.status === "added" || file.status === "untracked";
  const counts = countState(file);
  return (
    <div className="flex min-h-(--size-touch-target) items-center gap-md py-md" data-phone-look-file={file.path}>
      <span aria-hidden="true" className="flex size-(--size-icon-button-standard) shrink-0 items-center justify-center rounded-md bg-secondary font-mono text-micro text-subtle-foreground">
        {fileTag(name)}
      </span>
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="flex min-w-0 items-center gap-sm">
          <span className="min-w-0 break-all text-title font-medium text-foreground">{name}</span>
          {created ? <span className="shrink-0 rounded-xs border border-success px-xs text-micro text-success">{t("mobile.look.newFile")}</span> : null}
        </span>
        {folder ? <span className="break-all text-body text-muted-foreground">{folder}</span> : null}
      </span>
      {counts === "binary" ? <span className="shrink-0 text-body text-muted-foreground">{t("mobile.look.binary")}</span> : null}
      {counts === "counted" ? (
        <span className="shrink-0 text-body">
          <Counts added={file.added ?? 0} removed={file.removed ?? 0} />
        </span>
      ) : null}
    </div>
  );
}

/** An image to the phone's width, zoomed and moved with two fingers, its size and pixels under it (B18). */
function ImageBody({ name, content }: { name: string; content: Extract<LookContent, { state: "image" }> }) {
  const { t, i18n } = useInterfaceTranslation();
  const [pixels, setPixels] = useState<{ width: number; height: number } | null>(null);
  const frame = useRef<HTMLDivElement>(null);
  const [view, setView] = useState({ scale: 1, x: 0, y: 0 });
  const touches = useRef(new Map<number, { x: number; y: number }>());
  const size = formatBytes(requireInterfaceLanguage(i18n.language), content.size);

  const box = () => {
    const rect = frame.current?.getBoundingClientRect();
    return { width: rect?.width ?? 0, height: rect?.height ?? 0 };
  };
  const spread = () => {
    const [a, b] = [...touches.current.values()];
    return a && b ? Math.hypot(a.x - b.x, a.y - b.y) : 0;
  };
  const down = (event: PointerEvent<HTMLDivElement>) => {
    event.currentTarget.setPointerCapture(event.pointerId);
    touches.current.set(event.pointerId, { x: event.clientX, y: event.clientY });
  };
  const move = (event: PointerEvent<HTMLDivElement>) => {
    const before = touches.current.get(event.pointerId);
    if (!before) return;
    const was = spread();
    touches.current.set(event.pointerId, { x: event.clientX, y: event.clientY });
    const { width, height } = box();
    setView((current) => {
      const scale = touches.current.size === 2 && was > 0 ? zoomBy(current.scale, spread() / was) : current.scale;
      // One finger moves a zoomed image; two fingers move it by the first finger's step too.
      const x = clampOffset(current.x + (event.clientX - before.x), scale, width);
      const y = clampOffset(current.y + (event.clientY - before.y), scale, height);
      return scale === 1 ? { scale, x: 0, y: 0 } : { scale, x, y };
    });
  };
  const up = (event: PointerEvent<HTMLDivElement>) => {
    touches.current.delete(event.pointerId);
  };
  return (
    <div className="flex min-h-0 flex-1 flex-col bg-background" data-phone-look-state="image">
      <div
        ref={frame}
        className="relative flex min-h-0 flex-1 items-center justify-center overflow-hidden px-lg"
        style={{ touchAction: "none" }}
        onPointerDown={down}
        onPointerMove={move}
        onPointerUp={up}
        onPointerCancel={up}
        onDoubleClick={() => setView((current) => (current.scale > 1 ? { scale: 1, x: 0, y: 0 } : { scale: 2.5, x: 0, y: 0 }))}
      >
        <img
          src={content.url}
          alt={name}
          draggable={false}
          className="max-h-full max-w-full rounded-md bg-card object-contain select-none"
          style={{ transform: `translate(${view.x}px, ${view.y}px) scale(${view.scale})` }}
          data-phone-look-zoom={view.scale}
          onLoad={(event) => setPixels({ width: event.currentTarget.naturalWidth, height: event.currentTarget.naturalHeight })}
        />
      </div>
      <p className="flex shrink-0 justify-between gap-md px-lg py-sm text-body text-muted-foreground">
        <span className="font-mono" data-phone-look-info="true">
          {pixels ? t("mobile.look.imageInfo", { width: String(pixels.width), height: String(pixels.height), size }) : size}
        </span>
        <span>{t("mobile.look.zoomHint")}</span>
      </p>
    </div>
  );
}
