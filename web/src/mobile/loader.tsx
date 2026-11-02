import { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { decodeServer, encodeClient, type PairingQr } from "../../../hide-relay-protocol/web";
import "../index.css";
import "./mobile.css";
import { useLoaderTranslation } from "./loaderTranslation";
import { Button } from "../components/ui/button";
import { HostList, VersionScreen } from "./HostList";
import { ScanQr } from "./ScanQr";
import { bundlePath, readHosts, MAX_HOSTS, type PairedHost } from "./hosts";
import { RELAY_URL, relayReady, takePairing } from "./relayRuntime";

const mode = window.matchMedia("(prefers-color-scheme: dark)");
const theme = () => document.documentElement.classList.toggle("dark", mode.matches);
theme();
mode.addEventListener("change", theme);
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
    if ("serviceWorker" in navigator) void navigator.serviceWorker.register("/sw.js", { scope: "/" }).catch(() => setError("loadError"));
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
  if (version && target) return <VersionScreen t={t} name={target.name} protocol={version.device} app={version.supported.join(", ")} />;
  return <main className="phone-safe-top flex min-h-full flex-col gap-lg px-lg py-xxxl">
    <h1 className="text-display font-semibold">hide</h1>
    {pair ? <>
      <h2 className="truncate text-headline font-semibold" title={pair.name}>{t("mobile.pair.title", { name: pair.name })}</h2>
      <p className="text-subhead text-muted-foreground">{t("mobile.pair.description")}</p>
      <p className="text-subhead text-muted-foreground">{t("mobile.relay.encrypted")}</p>
      <Button className="min-h-(--size-touch-target)" disabled={!!target} onClick={() => setTarget(pair)}>{t(target ? "mobile.relay.connecting" : "mobile.pair.connect")}</Button>
      <p className="text-body text-muted-foreground">{t("mobile.pair.expiry")}</p>
    </> : adding ? <ScanQr t={t} onPair={(qr) => { setPair(qr); setAdding(false); }} /> : <>
      <h2 className="text-title">{t("mobile.relay.devices")}</h2>
      {!target ? <HostList t={t} choose={(host) => { setError(null); setTarget(host); }} /> : null}
    </>}
    {target && !pair ? <p role="status">{t("mobile.relay.connecting")}</p> : null}
    {target ? <Button variant="ghost" onClick={() => { setTarget(null); setPair(null); setAdding(false); setError(null); }}>{t("mobile.relay.choose")}</Button> : null}
    {error ? <p role="alert" className="text-subhead text-warning">{t(`mobile.relay.${error}`)}</p> : null}
  </main>;
}

const root = document.getElementById("root");
if (!root) throw new Error("root missing");
createRoot(root).render(<Loader />);
