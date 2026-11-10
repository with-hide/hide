import { useEffect, useRef, useState } from "react";
import type { TFunction } from "i18next";
import { decodeQr, type PairingQr } from "../../../hide-relay-protocol/web";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { relayReady } from "./relayRuntime";
import { appHost } from "./appHost";
import { nativeCode, scanQr } from "./appBridge";

/** The app's own camera view: it asks for the camera when the person taps, reads one Hide link (any other code is ignored) and gives its text to the same decoder. */
export async function scanInApp(t: TFunction, onPair: (pair: PairingQr) => void, fail: (error: "cameraError" | "qrError") => void): Promise<void> {
  let text: string;
  try {
    text = await scanQr({
      title: t("mobile.scanner.title"), hint: t("mobile.scanner.hint"), whereTitle: t("mobile.scanner.whereTitle"), whereBody: t("mobile.scanner.whereBody"),
      deniedTitle: t("mobile.scanner.deniedTitle"), deniedBody: t("mobile.scanner.deniedBody"), openSettings: t("mobile.scanner.openSettings"), close: t("common.close"),
    });
  } catch (error) { if (nativeCode(error) !== "cancelled") fail("cameraError"); return; }
  try { await relayReady(); onPair(decodeQr(text)); }
  catch { fail("qrError"); }
}

/** One camera, frames capped at 640 pixels at four scans/second, stopped on hide or unmount. */
export function ScanQr({ t, onPair }: { t: TFunction; onPair: (pair: PairingQr) => void }) {
  const [camera, setCamera] = useState(false);
  const [link, setLink] = useState("");
  const [error, setError] = useState<"cameraError" | "qrError" | null>(null);
  const video = useRef<HTMLVideoElement>(null);
  const callback = useRef(onPair);
  callback.current = onPair;
  useEffect(() => {
    if (!camera) return;
    let ended = false;
    let stream: MediaStream | null = null;
    let timer: ReturnType<typeof setTimeout> | null = null;
    const close = () => {
      ended = true;
      if (timer) clearTimeout(timer);
      stream?.getTracks().forEach((track) => track.stop());
    };
    const visibility = () => { if (document.hidden) { close(); setCamera(false); } };
    document.addEventListener("visibilitychange", visibility);
    const canvas = document.createElement("canvas");
    const context = canvas.getContext("2d", { willReadFrequently: true });
    let readQr: typeof import("jsqr").default;
    const scan = () => {
      if (ended) return;
      const element = video.current;
      if (element?.videoWidth && context) {
        const scale = Math.min(1, 640 / Math.max(element.videoWidth, element.videoHeight));
        canvas.width = Math.max(1, Math.round(element.videoWidth * scale));
        canvas.height = Math.max(1, Math.round(element.videoHeight * scale));
        context.drawImage(element, 0, 0, canvas.width, canvas.height);
        const pixels = context.getImageData(0, 0, canvas.width, canvas.height);
        const qr = readQr(pixels.data, pixels.width, pixels.height);
        if (qr) {
          try { const pair = decodeQr(qr.data); close(); setCamera(false); callback.current(pair); return; }
          catch { setError("qrError"); }
        }
      }
      timer = setTimeout(scan, 250);
    };
    void (async () => {
      await relayReady();
      readQr = (await import("jsqr")).default;
      if (ended) return;
      const acquired = await navigator.mediaDevices.getUserMedia({ video: { facingMode: "environment", width: { ideal: 640 }, height: { ideal: 640 } }, audio: false });
      stream = acquired;
      if (ended) { close(); return; }
      if (!video.current || !context) throw new Error("camera_unavailable");
      video.current.srcObject = stream;
      await video.current.play();
      scan();
    })().catch(() => { const wasEnded = ended; close(); if (!wasEnded) { setCamera(false); setError("cameraError"); } });
    return () => { close(); document.removeEventListener("visibilitychange", visibility); };
  }, [camera]);
  return <div className="flex flex-col gap-md">
    <p className="text-subhead text-muted-foreground">{t("mobile.relay.scanHint")}</p>
    {camera ? <video ref={video} muted playsInline className="aspect-square w-full rounded-lg object-cover" /> : null}
    <Button variant="secondary" className="min-h-(--size-touch-target)" data-phone-scan="true" onClick={() => {
      setError(null);
      if (appHost()) void scanInApp(t, onPair, setError);
      else setCamera(!camera);
    }}>{t(camera ? "common.close" : "mobile.relay.scan")}</Button>
    <Input aria-label={t("mobile.relay.paste")} placeholder={t("mobile.relay.paste")} value={link} autoComplete="off" maxLength={1024} onChange={(event) => setLink(event.target.value)} />
    <Button variant="secondary" disabled={!link} onClick={() => {
      void relayReady().then(() => { const pair = decodeQr(link); setLink(""); onPair(pair); }).catch(() => setError("qrError"));
    }}>{t("mobile.relay.connect")}</Button>
    {error ? <p role="alert" className="text-subhead text-warning">{t(`mobile.relay.${error}`)}</p> : null}
  </div>;
}
