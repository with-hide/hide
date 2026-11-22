// Production Settings › 봇 and the device rail's bot tiles with invented bots
// (PRD hide-external-control B2-B4, B11-B13, D-50): `state=pending`,
// `connected`, `invites` (bots made here that wait for their AI, one with its
// code run out), `empty`, and `rail` for the rail alone. Making a bot, a new
// code, approve, mode, cancel and disconnect move the invented section as the
// core would, with an invented code. No daemon and no keys.
import { useLayoutEffect, useMemo } from "react";
import { configureAccessCalls, receiveAccessResult, type AccessCall } from "../accessCall";
import { createActions } from "../actions";
import type { BotModeKind } from "../bots";
import { BotTileView, RailTileView } from "../components/device-rail";
import { TooltipProvider } from "../components/ui/tooltip";
import { InterfaceLanguageBoundary } from "../i18n/client";
import { NOTHING_UNDER_WAY } from "../pairing";
import { ExternalToolsTab } from "../settings/ExternalToolsTab";
import type { AccessConnection, AccessInvite, AccessMode, AccessSection } from "../snapshot";
import { useShellStore } from "../store";

const MINUTE = 60_000;

const OWN: AccessMode = { kind: "own" };

function connection(id: string, name: string, mode: AccessMode, agents: number, missions: number, lastUsedMinutes: number): AccessConnection {
  const now = Date.now();
  return { id, name, mode, program: `/usr/local/bin/${name}`, created_ms: now - 2 * 86_400_000, last_used_ms: now - lastUsedMinutes * MINUTE, agent_count: agents, mission_count: missions };
}

const CODE_MINUTES = 10;
const GALLERY_CODE = "K7Q4-MZ2P-X3RD";

function invite(id: string, name: string, mode: AccessMode, madeMs: number): AccessInvite {
  return { id, name, mode, created_ms: madeMs, expires_ms: madeMs + CODE_MINUTES * MINUTE };
}

/** The core's answer to Settings' call, on the invented section: a made bot with an invented code. */
function answerCall(call: AccessCall): { ok: true; value: { id: string; name: string; mode: AccessMode; code: string; expires_ms: number } } | { ok: false; code: string } {
  const current = useShellStore.getState().access;
  if (!current) return { ok: false, code: "access_unavailable" };
  const now = Date.now();
  if (call.call === "invite") {
    const taken = [...current.connections, ...current.invites, ...current.pending].some((one) => one.name.toLowerCase() === call.name.toLowerCase());
    if (taken) return { ok: false, code: "access_name_taken" };
    const made = invite(`gallery-inv-${now}`, call.name, call.mode, now);
    useShellStore.setState({ access: { ...current, invites: [...current.invites, made] } });
    return { ok: true, value: { ...made, code: GALLERY_CODE } };
  }
  const found = current.invites.find((one) => one.id === call.id);
  if (!found) return { ok: false, code: "access_invite_gone" };
  if (call.call === "renew") {
    const renewed = { ...found, created_ms: now, expires_ms: now + CODE_MINUTES * MINUTE };
    useShellStore.setState({ access: { ...current, invites: current.invites.map((one) => (one.id === found.id ? renewed : one)) } });
    return { ok: true, value: { ...renewed, code: GALLERY_CODE } };
  }
  return found.expires_ms > now ? { ok: true, value: { ...found, code: GALLERY_CODE } } : { ok: false, code: "access_invite_expired" };
}

function fixture(state: string): AccessSection {
  const now = Date.now();
  return {
    machine: "gallery", limit: 16,
    pending: state === "pending" ? [{ id: "gallery-req-1", name: "orca-bot", program: "/Applications/Orca.app/Contents/MacOS/orca", requested_ms: now - 20_000 }] : [],
    invites: state === "invites" ? [invite("gallery-inv-1", "hermes", OWN, now - MINUTE), invite("gallery-inv-2", "nightly-report", OWN, now - 12 * MINUTE)] : [],
    connections: state === "empty" ? [] : [
      connection("gallery-conn-1", "hermes-ctong", { kind: "all" }, 4, 1, 3),
      connection("gallery-conn-2", "nightly", OWN, 0, 0, 26 * 60),
      connection("gallery-conn-3", "slack-relay", OWN, 2, 0, 0),
    ],
  };
}

/** The rail's bots, after This Mac and a short line: bots in their own space only, one waiting on two answers. */
const RAIL: { name: string; turn: number }[] = [
  { name: "nightly", turn: 2 },
  { name: "slack-relay", turn: 0 },
  { name: "orca", turn: 1 },
];

function Rail({ selected }: { selected: string | null }) {
  return (
    <nav aria-label="rail" data-gallery-rail="true" className="flex w-(--size-rail) shrink-0 flex-col items-center gap-md self-stretch border-r border-border bg-sidebar py-md">
      <RailTileView tile={{ id: "gallery-mac", label: "This Mac", icon: "local" }} selected={selected === null} counts={{ needs_you: 2, done: 0 }} connected onSelect={() => undefined} />
      <span aria-hidden="true" className="h-(--size-hairline) w-(--size-icon) bg-border" />
      {RAIL.map((bot) => (
        <BotTileView key={bot.name} name={bot.name} turn={bot.turn} connection={bot.name} selected={bot.name === selected} onSelect={() => undefined} />
      ))}
    </nav>
  );
}

export function ExternalToolsScene({ params }: { params: URLSearchParams }) {
  const state = params.get("state") ?? "pending";
  const language = params.get("lang") ?? "ko";
  const theme = params.get("theme") ?? "dark";
  const scale = params.get("scale") === "large" ? 17 / 13 : 1;
  const actions = useMemo(() => createActions((event) => {
    const current = useShellStore.getState().access;
    if (!current) return true;
    const payload = event.payload as { id?: string; mode?: { kind: BotModeKind } };
    const wire = (kind: BotModeKind): AccessMode => ({ kind });
    const id = payload.id;
    if (event.kind === "access_refuse") useShellStore.setState({ access: { ...current, pending: current.pending.filter((one) => one.id !== id) } });
    if (event.kind === "access_approve" && payload.mode) {
      const request = current.pending.find((one) => one.id === id);
      if (request) useShellStore.setState({ access: { ...current, pending: current.pending.filter((one) => one.id !== id), connections: [...current.connections, { ...connection(request.id, request.name, wire(payload.mode.kind), 0, 0, 0), program: request.program }] } });
    }
    const mode = payload.mode ? wire(payload.mode.kind) : null;
    if (event.kind === "access_mode" && mode) useShellStore.setState({ access: { ...current, connections: current.connections.map((one) => one.id === id ? { ...one, mode } : one) } });
    if (event.kind === "access_revoke") useShellStore.setState({ access: { ...current, connections: current.connections.filter((one) => one.id !== id) } });
    if (event.kind === "access_invite_cancel") useShellStore.setState({ access: { ...current, invites: current.invites.filter((one) => one.id !== id) } });
    return true;
  }), []);
  useLayoutEffect(() => {
    const before = configureAccessCalls((event) => {
      const { request_id, call } = event.payload as { request_id: string; call: AccessCall };
      queueMicrotask(() => receiveAccessResult({ request_id, ...answerCall(call) }));
      return true;
    });
    return () => void configureAccessCalls(before);
  }, []);
  useLayoutEffect(() => {
    // This machine's own window, which makes, approves and changes bots; another machine's view lists them only.
    useShellStore.setState({ connection: "live", access: fixture(state), pairing: NOTHING_UNDER_WAY, rest: { ui_state: { interface_language: language }, navigator: { devices: [{ id: "local", kind: "local", label: null, machine_name: "studio-mac" }] } } } as never);
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
    document.documentElement.style.setProperty("--interface-scale", String(scale));
    return () => { document.documentElement.style.removeProperty("--interface-scale"); };
  }, [state, language, theme, scale]);
  return <TooltipProvider><InterfaceLanguageBoundary />
    <div className="flex bg-background" data-gallery-scene="external-tools" data-scene-ready="true">
      <Rail selected={state === "rail" ? "nightly" : null} />
      {state === "rail" ? null : (
        <div className="w-full max-w-[calc(var(--size-settings-sheet-w)-var(--size-settings-nav-w))] px-lg py-lg">
          <ExternalToolsTab actions={actions} />
        </div>
      )}
    </div>
  </TooltipProvider>;
}
