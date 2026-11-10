import { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { decodeServer, encodeClient, type PairingQr } from "../../../hide-relay-protocol/web";
import "../index.css";
import "./mobile.css";
import { useLoaderTranslation } from "./loaderTranslation";
import { Button } from "../components/ui/button";
import { DeviceConnecting, HostList, VersionScreen } from "./HostList";
import { Landing, DeviceIdentity } from "./Landing";
import { ScanQr } from "./ScanQr";
import { bundlePath, readHosts, MAX_HOSTS, type PairedHost } from "./hosts";
import { appHost, serviceWorkerAllowed } from "./appHost";
import { RELAY_URL, relayReady, takePairing } from "./relayRuntime";
import { followPhoneTheme } from "./settings";

followPhoneTheme();
// Take and clear secrets synchronously before any React effects or resource loading.
const pairing = takePairing();

/** Ask only the one plaintext fact; no code or credential leaves the loader. */
function protocolOf(room: string, signal: AbortSignal): Promise<number> {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(RELAY_URL);
    const finish = (value: number | null) => {
      clearTimeout(timer);
      signal.removeEventListener("abort", abort);
      ws.onopen = ws.onmessage = ws.onclose = ws.onerror = null;
      ws.close();
      if (value !== null) resolve(value); else reject(new Error("device_unavailable"));
    };
    const abort = () => finish(null);
    const timer = setTimeout(abort, 15_000);
    signal.addEventListener("abort", abort, { once: true });
    ws.onopen = () => ws.send(encodeClient({ type: "phone", version: 1, room }));
    ws.onmessage = (event) => {
      try { const message = decodeServer(String(event.data)); finish(message.type === "joined" ? message.phone_protocol : null); }
      catch { finish(null); }
    };
    ws.onerror = ws.onclose = abort;
    if (signal.aborted) abort();
  });
}

function Loader() {
  const { t } = useLoaderTranslation();
  const [pair, setPair] = useState<PairingQr | null>(null);
  const [adding, setAdding] = useState(new URLSearchParams(location.search).has("add"));
  const [target, setTarget] = useState<PairedHost | PairingQr | null>(null);
  const [error, setError] = useState<"qrError" | "storage" | "loadError" | "limit" | null>(null);
  const [version, setVersion] = useState<{ device: number; supported: number[] } | null>(null);
  useEffect(() => {
    void pairing.then((qr) => {
      if (qr) { setPair(qr); return; }
      const host = new URLSearchParams(location.search).get("host");
      try { if (host) setTarget(readHosts(localStorage).find((candidate) => candidate.room === host) ?? null); }
      catch { setError("storage"); }
    }).catch(() => setError("qrError"));
    const followQr = () => {
      void takePairing().then((qr) => {
        if (!qr) return;
        setTarget(null); setVersion(null); setError(null); setAdding(false); setPair(qr);
      }).catch(() => setError("qrError"));
    };
    window.addEventListener("hashchange", followQr);
    if (serviceWorkerAllowed()) void navigator.serviceWorker.register("/sw.js", { scope: "/" }).catch(() => setError("loadError"));
    return () => window.removeEventListener("hashchange", followQr);
  }, []);
  useEffect(() => {
    if (!target) return;
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout> | null = null;
    const open = async () => {
      try {
        if ("code" in target) {
          const hosts = readHosts(localStorage);
          if (hosts.length >= MAX_HOSTS && !hosts.some((host) => host.room === target.room)) { setError("limit"); return; }
        }
        await relayReady();
        const response = await fetch("/protocols.json", { signal: controller.signal });
        if (!response.ok) throw new Error("bundles_unavailable");
        const supported: unknown = await response.json();
        if (!Array.isArray(supported) || !supported.every((n) => Number.isSafeInteger(n) && n > 0) || supported.length > 100) throw new Error("bundles_invalid");
        // An offline paired host opens its cached bundle so its existing reconnect path owns recovery.
        let protocol: number;
        try { protocol = await protocolOf(target.room, controller.signal); }
        catch { if ("protocol" in target) protocol = target.protocol; else throw new Error("device_unavailable"); }
        if (controller.signal.aborted) return;
        const path = bundlePath(protocol, supported);
        if (!path) { setVersion({ device: protocol, supported }); return; }
        const query = new URLSearchParams({ host: target.room });
        const tag = new URLSearchParams(location.search).get("open");
        if (tag) query.set("open", tag);
        let hash = "";
        if ("code" in target) {
          query.set("connect", "1");
          const bytes = new TextEncoder().encode(JSON.stringify(target));
          hash = "#pair=" + btoa(String.fromCharCode(...bytes)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
        }
        window.location.replace(`${path}?${query}${hash}`);
      } catch {
        if (controller.signal.aborted) return;
        setError("loadError");
        timer = setTimeout(() => { void open(); }, 4000 + Math.random() * 2000);
      }
    };
    void open();
    return () => { controller.abort(); if (timer) clearTimeout(timer); };
  }, [target]);
  if (version && target) return <VersionScreen t={t} name={target.name} protocol={version.device} app={version.supported.join(", ")} appBehind={appHost() && version.device > Math.max(...version.supported)} />;
  if (pair) return <Landing note={<>{t("mobile.relay.encrypted")}<br />{t("mobile.pair.expiry")}</>}>
    <DeviceIdentity name={pair.name} large />
    <p className="max-w-(--size-mobile-copy) text-subhead leading-relaxed text-subtle-foreground">{t("mobile.pair.description")}</p>
    <Button variant="secondary" className="min-h-(--size-touch-target) rounded-lg px-xl text-subhead" disabled={!!target} onClick={() => setTarget(pair)}>{t(target ? "mobile.relay.connecting" : "mobile.pair.connect")}</Button>
    {target ? <Button variant="ghost" className="min-h-(--size-touch-target) text-body" onClick={() => { setTarget(null); setPair(null); setAdding(false); setError(null); }}>{t("mobile.relay.choose")}</Button> : null}
    {error ? <p role="alert" className="max-w-(--size-mobile-copy) text-body text-warning">{t(`mobile.relay.${error}`)}</p> : null}
  </Landing>;
  if (target) return <DeviceConnecting t={t} name={target.name} error={error}
    onChoose={() => { setTarget(null); setPair(null); setAdding(false); setError(null); }} />;
  return <main className="phone-safe-top phone-safe-bottom flex min-h-full flex-col gap-lg px-xl">
    <h1 className="pt-lg text-headline font-semibold">hide</h1>
    {adding ? <ScanQr t={t} onPair={(qr) => { setPair(qr); setAdding(false); }} /> : <>
      <h2 className="text-title">{t("mobile.relay.devices")}</h2>
      <HostList t={t} choose={(host) => { setError(null); setTarget(host); }} />
    </>}
    {error ? <p role="alert" className="text-subhead text-warning">{t(`mobile.relay.${error}`)}</p> : null}
  </main>;
}

const root = document.getElementById("root");
if (!root) throw new Error("root missing");
createRoot(root).render(<Loader />);
