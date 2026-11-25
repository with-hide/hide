// Production Settings › Devices with its phone group over invented relay
// states (operator review 2026-11-23): this Mac, one paired Mac mini, and the
// phones. `state=connected|off|connecting|retrying|outdated|cap|cleanup|qr|
// expired|empty|full|renamed|tailscale|tailscale-failed` is the `mobile` frame;
// `screen=other` is a screen another machine shows of this one. The Add
// dialog, the QR window and each phone's ⋯ open by pressing them. No network
// or keys.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import type { MobileState, PhoneRow } from "../mobileSettings";
import { NOTHING_UNDER_WAY, type Peer } from "../pairing";
import { DevicesTab } from "../settings/DevicesTab";
import type { Device } from "../snapshot";
import { TooltipProvider } from "../components/ui/tooltip";
import { InterfaceLanguageBoundary } from "../i18n/client";
import { useShellStore } from "../store";
import { EMPTY_GALLERY_SCOPE } from "./agentScopes";
import { MobilePhoneScene, PHONE_SCENES, type PhoneSceneState } from "./MobilePhoneScene";

const DAY = 86_400_000;
const phone = (id: string, name: string, over: Partial<PhoneRow>, now: number): PhoneRow => ({ id, name, last_seen_ms: now, connected: true, notifications: "on", revoke_at_ms: now + 7 * DAY, ...over });

function fixture(state: string): MobileState {
  const now = Date.now();
  const reason = state === "outdated" ? "unsupported_version" : state === "cap" ? "daily_limit" : undefined;
  const qr = `https://app.withhide.dev/#pair=${btoa(JSON.stringify({v:2,name:"Home computer",device_key:"A".repeat(43),noise_key:"B".repeat(43),room:"C".repeat(43),code:"D".repeat(22)}))}`;
  const tailscale = state.startsWith("tailscale");
  const down = ["retrying", "outdated", "cap"].includes(state);
  const phones: PhoneRow[] = state === "empty" ? []
    : state === "full" ? [0, 1, 2, 3].map((n) => phone(`gallery-phone-${n}`, n === 0 ? "iPhone" : `iPhone ${n + 1}`, {}, now))
    : [
      phone("gallery-phone", state === "renamed" ? "회사 iPhone" : "iPhone", { connected: !down && state !== "off", own_name: state === "renamed" ? "iPhone" : null, last_seen_ms: down || state === "off" ? now - 3 * 3_600_000 : now }, now),
      phone("gallery-pixel", "Android", { connected: false, notifications: "unasked", last_seen_ms: now - 2 * DAY, revoke_at_ms: now + 5 * DAY }, now),
    ];
  return {
    enabled: state !== "off", method: tailscale ? "tailscale" : "relay",
    exposure: state === "off" ? "off" : state === "tailscale-failed" ? "checking" : ["connecting", "retrying", "outdated", "cap"].includes(state) ? "checking" : "exposed",
    relay: { state: tailscale || state === "off" ? "off" : reason ? "refused" : state === "retrying" ? "retrying" : state === "connecting" ? "connecting" : "connected", reason },
    checklist: tailscale ? { installed: "ok", logged_in: state === "tailscale-failed" ? "failed" : "ok", https: "ok", host_name: "studio" } : null,
    download_url: "https://tailscale.com/download", admin_url: "https://login.tailscale.com/admin/dns",
    foreign_target: null, failure: state === "cleanup" ? {step:"remove",message:"Tailscale CLI unavailable"} : null,
    url: tailscale ? "https://studio.tail1234.ts.net" : "https://app.withhide.dev/", qr: state === "qr" || state === "expired" ? qr : null,
    code_expires_at_ms: state === "qr" ? now + 300_000 : state === "expired" ? now - 1_000 : null,
    phones, max_phones:4, push_mode:"app_closed", now_ms:now,
  };
}

const LOCAL: Device = { agent_scope: EMPTY_GALLERY_SCOPE, id: "local", label: null, kind: "local", state: "local", message: null, ssh_alias: null, agent_count: 0, test: null, host: { consent: "this_machine", helper_root: "~/.hide/host-helper", cli_dir: "~/.local/bin", contract: 1, bound_identity: null, granted_at_unix_ms: 1, state: "ready", message: null, platform: "macos aarch64", helper_path: null } };
const MINI: Peer = { key: "b".repeat(64), device_id: "mac-mini", name: "Mac mini", opens: "active", opened_by: "active", connected: true, path: "direct", reason: null, update: null };

/** Settings › Devices with its phones on the desktop, and beside it the phone's own screens (`state=phone-add`, `phone-list`, `phone-list-cut`, `phone-gathered`, `phone-add-hint`, `phone-devices`, `phone-missions`, `phone-missions-cut`, `phone-mission`, `phone-mission-views`, `phone-prs`, `phone-pr`, `phone-pr-agent`, `phone-pr-merge`, `phone-pr-delegate`, `phone-looks` and the `phone-look-*` viewers; `offline=1` dims the 볼 것 ones). */
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
  const own = params.get("screen") !== "other";
  const actions = useMemo(() => createActions((event) => {
    const current = useShellStore.getState().mobile;
    if (!current) return true;
    if (event.kind === "mobile_enable") useShellStore.setState({ mobile: fixture("connected") });
    if (event.kind === "mobile_show_code") useShellStore.setState({ mobile: { ...fixture("qr"), phones: current.phones } });
    if (event.kind === "mobile_hide_code") useShellStore.setState({ mobile: { ...current, qr: null, code_expires_at_ms: null } });
    return true;
  }), []);
  useLayoutEffect(() => {
    useShellStore.setState({
      connection: "live",
      mobile: fixture(state),
      // hided sends a screen of another machine no `pairing` frame.
      pairing: own ? NOTHING_UNDER_WAY : null,
      rest: { navigator: { devices: [LOCAL], peers: [MINI], guest_panes: [], focused_device_id: "local", workspaces: [] }, status: { remote: [] }, ui_state: { interface_language: language } },
    } as never);
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
    document.documentElement.style.setProperty("--interface-scale", String(scale));
    return () => { document.documentElement.style.removeProperty("--interface-scale"); };
  }, [state, language, theme, scale, own]);
  return <TooltipProvider><InterfaceLanguageBoundary />
    <div className="min-h-screen w-full bg-background text-foreground" data-gallery-scene="mobile" data-scene-ready="true">
      <div className="max-w-[calc(var(--size-settings-sheet-w)-var(--size-settings-nav-w))] px-xl py-lg">
        <DevicesTab actions={actions} />
      </div>
    </div>
  </TooltipProvider>;
}
