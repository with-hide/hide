import { useEffect, useRef, useState } from "react";
import type { TFunction } from "i18next";
import { decodeQr, type PairingQr } from "../../../hide-relay-protocol/web";
import { Button } from "../components/ui/button";
import { CameraOffIcon, XIcon } from "lucide-react";
import { relayReady } from "./relayRuntime";
import { nativeCode, scanQr } from "./appBridge";

/** The app's own camera view: it asks for the camera when the person taps, reads one Hide link (any other code is ignored) and gives its text to the same decoder. */
export async function scanInApp(t: TFunction, onPair: (pair: PairingQr) => void, fail: (error: "cameraError" | "cameraBusy" | "qrError") => void): Promise<void> {
  let text: string;
  try {
    text = await scanQr({
      title: t("mobile.scanner.title"), hint: t("mobile.scanner.hint"), whereTitle: t("mobile.scanner.whereTitle"), whereBody: t("mobile.scanner.whereBody"),
      deniedTitle: t("mobile.scanner.deniedTitle"), deniedBody: t("mobile.scanner.deniedBody"), openSettings: t("mobile.scanner.openSettings"), close: t("common.close"),
    });
  } catch (error) { const code = nativeCode(error); if (code !== "cancelled") fail(code === "busy" ? "cameraBusy" : "cameraError"); return; }
  try { await relayReady(); onPair(decodeQr(text)); }
  catch { fail("qrError"); }
}

/**
 * The browser's camera screen, in place of the add page and drawn like the app's own scanner: a full-screen camera with the frame in the middle and the rest dimmed,
 * where only what is inside the frame is read. One camera, the frame's pixels capped at 640 at four scans a second, stopped on close,
 * hide or unmount. Only a Hide pairing link ends the scan; any other code is ignored. A camera that cannot open says so and offers the paste.
 */
export function WebScanner({ t, onPair, onClose, onPaste }: { t: TFunction; onPair: (pair: PairingQr) => void; onClose: () => void; onPaste: () => void }) {
  const [denied, setDenied] = useState(false);
  const video = useRef<HTMLVideoElement>(null);
  const frame = useRef<HTMLDivElement>(null);
  const callback = useRef({ onPair, onClose });
  callback.current = { onPair, onClose };
  useEffect(() => {
    let ended = false;
    let stream: MediaStream | null = null;
    let timer: ReturnType<typeof setTimeout> | null = null;
    const close = () => {
      ended = true;
      if (timer) clearTimeout(timer);
      stream?.getTracks().forEach((track) => track.stop());
    };
    const visibility = () => { if (document.hidden) { close(); callback.current.onClose(); } };
    document.addEventListener("visibilitychange", visibility);
    const canvas = document.createElement("canvas");
    const context = canvas.getContext("2d", { willReadFrequently: true });
    let readQr: typeof import("jsqr").default;
    const scan = () => {
      if (ended) return;
      const element = video.current;
      const box = frame.current?.getBoundingClientRect();
      if (element?.videoWidth && box?.width && context) {
        // The video covers the screen (object-cover), so the frame's square on screen is a centred square of the video.
        const shown = element.getBoundingClientRect();
        const cover = Math.max(shown.width / element.videoWidth, shown.height / element.videoHeight);
        const side = Math.min(element.videoWidth, element.videoHeight, box.width / cover);
        const x = element.videoWidth / 2 + (box.left + box.width / 2 - shown.left - shown.width / 2) / cover - side / 2;
        const y = element.videoHeight / 2 + (box.top + box.height / 2 - shown.top - shown.height / 2) / cover - side / 2;
        const size = Math.max(1, Math.round(Math.min(640, side)));
        canvas.width = canvas.height = size;
        context.drawImage(element, x, y, side, side, 0, 0, size, size);
        const qr = readQr(context.getImageData(0, 0, size, size).data, size, size);
        if (qr) {
          let pair: PairingQr | null = null;
          try { pair = decodeQr(qr.data); } catch { /* Not a Hide link: keep looking, as the app's scanner does. */ }
          if (pair) { close(); callback.current.onPair(pair); return; }
        }
      }
      timer = setTimeout(scan, 250);
    };
    void (async () => {
      await relayReady();
      readQr = (await import("jsqr")).default;
      if (ended) return;
      const acquired = await navigator.mediaDevices.getUserMedia({ video: { facingMode: "environment", width: { ideal: 1280 }, height: { ideal: 1280 } }, audio: false });
      stream = acquired;
      if (ended) { close(); return; }
      if (!video.current || !context) throw new Error("camera_unavailable");
      video.current.srcObject = stream;
      await video.current.play();
      scan();
    })().catch(() => { const wasEnded = ended; close(); if (!wasEnded) setDenied(true); });
    return () => { close(); document.removeEventListener("visibilitychange", visibility); };
  }, []);
  // Always dark, whatever the page's theme: the camera is the ground, as in the app's scanner, whose layout this follows.
  return <main className="dark relative h-full overflow-hidden bg-background text-center text-foreground" data-phone-web-scanner="true">
    {denied ? null : <>
      <video ref={video} muted playsInline className="absolute inset-0 size-full object-cover" />
      <div ref={frame} aria-hidden="true" className="phone-scan-frame absolute rounded-lg border-2 border-foreground" />
    </>}
    <div className="phone-safe-top phone-safe-bottom relative flex h-full flex-col px-xl">
      <Button variant="ghost" size="icon" className="mt-md self-end min-h-(--size-touch-target) min-w-(--size-touch-target) text-foreground" aria-label={t("common.close")} data-phone-scanner-close="true" onClick={onClose}>
        <XIcon aria-hidden="true" className="size-(--size-icon-lg)" />
      </Button>
      {denied ? <div className="flex flex-1 flex-col items-center justify-center gap-md pb-xxxl" data-phone-scanner-denied="true">
        <CameraOffIcon aria-hidden="true" className="size-(--size-touch-target) text-muted-foreground" />
        <h1 className="text-title font-semibold">{t("mobile.scanner.deniedTitle")}</h1>
        <p className="max-w-(--size-mobile-copy) break-keep text-subhead text-subtle-foreground wrap-anywhere">{t("mobile.scanner.browserDeniedBody")}</p>
        <Button variant="secondary" className="mt-sm min-h-(--size-touch-target) px-xl text-subhead" data-phone-scanner-paste="true" onClick={onPaste}>{t("mobile.relay.paste")}</Button>
      </div> : <>
        <h1 className="text-title font-semibold">{t("mobile.scanner.title")}</h1>
        <p className="break-keep pt-xs text-body text-subtle-foreground wrap-anywhere">{t("mobile.scanner.hint")}</p>
        <div className="flex-1" />
        <div className="mb-xl flex flex-col gap-xxs rounded-lg bg-foreground/15 px-lg py-md">
          <b className="text-subhead font-semibold">{t("mobile.scanner.whereTitle")}</b>
          <span className="break-keep text-body wrap-anywhere">{t("mobile.scanner.whereBody")}</span>
        </div>
      </>}
    </div>
  </main>;
}
