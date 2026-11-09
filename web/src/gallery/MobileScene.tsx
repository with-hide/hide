// Production Settings > Mobile with invented relay states. No network or keys.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { MobileTab } from "../MobileTab";
import type { MobileState } from "../mobileSettings";
import { TooltipProvider } from "../components/ui/tooltip";
import { InterfaceLanguageBoundary } from "../i18n/client";
import { useShellStore } from "../store";
import { MobilePhoneScene, PHONE_SCENES, type PhoneSceneState } from "./MobilePhoneScene";

function fixture(state: string): MobileState {
  const now = Date.now();
  const reason = state === "outdated" ? "unsupported_version" : state === "cap" ? "daily_limit" : undefined;
  const qr = `https://app.withhide.dev/#pair=${btoa(JSON.stringify({v:2,name:"Home computer",device_key:"A".repeat(43),noise_key:"B".repeat(43),room:"C".repeat(43),code:"D".repeat(22)}))}`;
  return {
    enabled: state !== "off", method: "relay",
    exposure: ["connected", "qr", "phones", "cleanup"].includes(state) ? "exposed" : state === "off" ? "off" : "checking",
    relay: { state: reason ? "refused" : state === "retrying" ? "retrying" : state === "off" ? "off" : state === "connecting" ? "connecting" : "connected", reason },
    checklist: null, download_url: "https://tailscale.com/download", admin_url: "https://login.tailscale.com/admin/dns",
    foreign_target: null, failure: state === "cleanup" ? {step:"remove",message:"Tailscale CLI unavailable"} : null, url: "https://app.withhide.dev/", qr: state === "qr" ? qr : null,
    code_expires_at_ms: state === "qr" ? now + 300_000 : null,
    phones: state === "phones" ? [{id:"gallery-phone",name:"iPhone",last_seen_ms:now,connected:true,notifications:"on",revoke_at_ms:now+604800000}] : [],
    max_phones:4, push_mode:"off", now_ms:now,
  };
}
/** Settings > Mobile on the desktop, and beside it the phone's own screens (`state=phone-list`, `phone-list-cut`, `phone-devices`, `phone-missions`, `phone-missions-cut`, `phone-mission`, `phone-mission-views`, `phone-prs`, `phone-pr`, `phone-pr-agent`, `phone-pr-merge`, `phone-pr-delegate`, `phone-looks` and the `phone-look-*` viewers; `offline=1` dims the 볼 것 ones). */
export function MobileScene({ params }: { params: URLSearchParams }) {
  const state = params.get("state") ?? "connected";
  if (PHONE_SCENES.includes(state)) {
    return <MobilePhoneScene state={state as PhoneSceneState} language={params.get("lang") ?? "ko"} theme={params.get("theme") ?? "dark"} scale={params.get("scale") === "large" ? 17 / 13 : 1} offline={params.get("offline") === "1"} />;
  }
  return <MobileSettingsScene params={params} state={state} />;
}

function MobileSettingsScene({ params, state }: { params: URLSearchParams; state: string }) {
  const language = params.get("lang") ?? "ko";
  const theme = params.get("theme") ?? "dark";
  const scale = params.get("scale") === "large" ? 17 / 13 : 1;
  const actions = useMemo(() => createActions((event) => {
    const current = useShellStore.getState().mobile;
    if (!current) return true;
    if (event.kind === "mobile_show_code") useShellStore.setState({mobile:fixture("qr")});
    if (event.kind === "mobile_hide_code") useShellStore.setState({mobile:{...current,qr:null,code_expires_at_ms:null}});
    return true;
  }), []);
  useLayoutEffect(() => {
    useShellStore.setState({connection:"live",mobile:fixture(state),rest:{ui_state:{interface_language:language}}} as never);
    document.documentElement.classList.toggle("dark",theme === "dark");
    document.documentElement.classList.toggle("light",theme === "light");
    document.documentElement.style.setProperty("--interface-scale", String(scale));
    return () => { document.documentElement.style.removeProperty("--interface-scale"); };
  }, [state,language,theme,scale]);
  return <TooltipProvider><InterfaceLanguageBoundary />
    <div className="w-full max-w-[calc(var(--size-settings-sheet-w)-var(--size-settings-nav-w))] bg-background px-lg py-lg" data-gallery-scene="mobile" data-scene-ready="true">
      <MobileTab actions={actions} />
    </div>
  </TooltipProvider>;
}
