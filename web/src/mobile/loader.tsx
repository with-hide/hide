import { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { decodeServer, encodeClient, type PairingQr } from "../../../hide-relay-protocol/web";
import "../index.css";
import "./mobile.css";
import { useLoaderTranslation } from "./loaderTranslation";
import { Button } from "../components/ui/button";
import { DeviceConnecting, HostList, VersionScreen } from "./HostList";
import { Landing, DeviceIdentity } from "./Landing";
import { scanInApp } from "./ScanQr";
import { AddScreen, FirstScreen, StartSettings } from "./StartScreens";
import { bundlePath, HOST_LIST_LINK, hostStore, hostToOpen, MAX_HOSTS, type PairedHost } from "./hosts";
import { appHost, markAppHost, serviceWorkerAllowed } from "./appHost";
import { RELAY_URL, relayReady, takePairing } from "./relayRuntime";
import { followPhoneTheme } from "./settings";
import { followNativeLinks } from "./nativeLink";
import { followPushOpens, pushOpenAddress } from "./pushOpen";
import { PHONE_PROTOCOL } from "./phoneVersion";
import { PlayIcon } from "lucide-react";

markAppHost();
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
  const [error, setError] = useState<"qrError" | "cameraError" | "cameraBusy" | "storage" | "loadError" | "limit" | null>(null);
  const [version, setVersion] = useState<{ device: number; supported: number[] } | null>(null);
  // How many computers this phone knows: none shows the first screen, any shows the list (one opens at once, hostToOpen). Null while it is read.
  const [known, setKnown] = useState<number | null>(null);
  const [settings, setSettings] = useState(false);
  const [openingNotice, setOpeningNotice] = useState(appHost() && new URLSearchParams(location.search).has("open"));
  useEffect(() => {
    // A pairing link that arrives while the list is read wins over opening a computer by itself.
    let paired = false;
    const hosts = pairing.then((qr) => { if (qr) { paired = true; setPair(qr); } }, () => { paired = true; setError("qrError"); })
      .then(() => hostStore().list());
    // Read the native launch intent alongside the saved computers. A notification chooses its computer in this
    // loader, rather than loading the start page a second time after it has already painted another destination.
    let ready!: () => void;
    const initialOpen = new Promise<void>((resolve) => { ready = resolve; });
    const stopOpens = followPushOpens((opened) => {
      window.history.replaceState(null, "", pushOpenAddress(opened));
      setOpeningNotice(true);
      void hosts.then((saved) => {
        if (!paired) setTarget(hostToOpen(new URLSearchParams(location.search), saved));
      }, () => setError("storage"));
    }, ready);
    void Promise.all([hosts, initialOpen]).then(([hosts]) => {
      // The computer the address names, or the only one kept, opens without the list; both land in one render.
      const open = paired ? null : hostToOpen(new URLSearchParams(location.search), hosts);
      if (open) setTarget(open);
      setKnown(hosts.length);
    }, () => { setKnown(-1); if (new URLSearchParams(location.search).has("host")) setError("storage"); });
    const followQr = () => {
      void takePairing().then((qr) => {
        if (!qr) return;
        paired = true;
        setTarget(null); setVersion(null); setError(null); setAdding(false); setPair(qr);
      }).catch(() => setError("qrError"));
    };
    window.addEventListener("hashchange", followQr);
    // A link that opened the app lands on the same pairing screen a scanned QR does, and waits for the person's tap.
    const stopLinks = followNativeLinks((qr) => { paired = true; setTarget(null); setVersion(null); setError(null); setAdding(false); setPair(qr); }, () => setError("qrError"));
    if (serviceWorkerAllowed()) void navigator.serviceWorker.register("/sw.js", { scope: "/" }).catch(() => setError("loadError"));
    return () => { window.removeEventListener("hashchange", followQr); stopLinks(); stopOpens(); };
  }, []);
  useEffect(() => {
    if (!target) return;
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout> | null = null;
    const open = async () => {
      try {
        if ("code" in target) {
          const hosts = await hostStore().list();
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
  if (target && openingNotice && !error) return <main className="min-h-full" />;
  if (target) return <DeviceConnecting t={t} name={target.name} error={error}
    onChoose={() => { setTarget(null); setPair(null); setAdding(false); setError(null); }} />;
  const demo = () => window.location.assign(`/p/${PHONE_PROTOCOL}/?demo=1`);
  if (!adding && known === null) return <main className="min-h-full" />;
  if (!adding && known === 0) {
    if (settings) return <StartSettings t={t} onBack={() => setSettings(false)} />;
    return <>
      <FirstScreen t={t} onDemo={demo} onSettings={() => setSettings(true)}
        // The app reads the code in its own camera view at once; a browser has the camera and paste page.
        onScan={() => { setError(null); if (appHost()) void scanInApp(t, (qr) => setPair(qr), (failure) => setError(failure)); else setAdding(true); }} />
      {error ? <p role="alert" className="px-xl pb-lg text-center text-subhead text-warning">{t(`mobile.relay.${error}`)}</p> : null}
    </>;
  }
  // Back from the add page: the list when a computer is kept, else the first screen, at an address that opens neither by itself.
  if (adding) return <AddScreen t={t} full={(known ?? 0) >= MAX_HOSTS} onPair={(qr) => { setError(null); setPair(qr); setAdding(false); }}
    onBack={() => { setError(null); setAdding(false); window.history.replaceState(null, "", known ? HOST_LIST_LINK : "/"); }} />;
  return <main className="phone-safe-top phone-safe-bottom flex min-h-full flex-col gap-lg px-xl">
    <h1 className="pt-lg text-headline font-semibold">hide</h1>
    <h2 className="text-title">{t("mobile.relay.devices")}</h2>
    <HostList t={t} choose={(host) => { setError(null); setTarget(host); }} />
    <div className="flex flex-col gap-xs">
      <Button variant="ghost" className="min-h-(--size-touch-target) justify-start gap-sm px-sm text-body" data-phone-demo-entry="true" onClick={demo}>
        <PlayIcon aria-hidden="true" className="size-(--size-icon)" />{t("mobile.demo.try")}
      </Button>
      <p className="px-sm text-body text-muted-foreground">{t("mobile.demo.tryHint")}</p>
    </div>
    {error ? <p role="alert" className="text-subhead text-warning">{t(`mobile.relay.${error}`)}</p> : null}
  </main>;
}

const root = document.getElementById("root");
if (!root) throw new Error("root missing");
createRoot(root).render(<Loader />);
