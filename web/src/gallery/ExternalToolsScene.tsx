// Production Settings › 봇 and the device rail's bot tiles with invented bots
// (PRD hide-external-control B2-B4, B11-B13, D-50): `state=pending`,
// `connected`, `empty`, and `rail` for the rail alone. Approve, mode and
// disconnect move the invented section as the core would. No daemon and no
// keys.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import type { BotModeKind } from "../bots";
import { BotTileView, RailTileView } from "../components/device-rail";
import { TooltipProvider } from "../components/ui/tooltip";
import { InterfaceLanguageBoundary } from "../i18n/client";
import { ExternalToolsTab } from "../settings/ExternalToolsTab";
import type { AccessConnection, AccessMode, AccessSection } from "../snapshot";
import { useShellStore } from "../store";

const MINUTE = 60_000;

/** A bot in its own space, as the core's wire names one today. */
const OWN: AccessMode = { kind: "workspace", workspace_id: "gallery-own" };

function connection(id: string, name: string, mode: AccessMode, agents: number, missions: number, lastUsedMinutes: number): AccessConnection {
  const now = Date.now();
  return { id, name, mode, program: `/usr/local/bin/${name}`, created_ms: now - 2 * 86_400_000, last_used_ms: now - lastUsedMinutes * MINUTE, agent_count: agents, mission_count: missions };
}

function fixture(state: string): AccessSection {
  const now = Date.now();
  return {
    machine: "gallery", limit: 16,
    pending: state === "pending" ? [{ id: "gallery-req-1", name: "orca-bot", program: "/Applications/Orca.app/Contents/MacOS/orca", requested_ms: now - 20_000 }] : [],
    connections: state === "empty" ? [] : [
      connection("gallery-conn-1", "hermes-ctong", { kind: "all" }, 4, 1, 3),
      connection("gallery-conn-2", "nightly", OWN, 0, 0, 26 * 60),
      connection("gallery-conn-3", "slack-relay", OWN, 2, 0, 0),
    ],
  };
}

/** The rail's bots, after This Mac and a short line: both modes, one waiting on two answers. */
const RAIL: { name: string; turn: number; mode: BotModeKind }[] = [
  { name: "hermes-ctong", turn: 2, mode: "all" },
  { name: "nightly", turn: 0, mode: "own" },
  { name: "slack-relay", turn: 0, mode: "own" },
  { name: "cursor-agent", turn: 0, mode: "all" },
  { name: "orca", turn: 1, mode: "own" },
];

function Rail({ selected }: { selected: string | null }) {
  return (
    <nav aria-label="rail" data-gallery-rail="true" className="flex w-(--size-rail) shrink-0 flex-col items-center gap-md self-stretch border-r border-border bg-sidebar py-md">
      <RailTileView tile={{ id: "gallery-mac", label: "This Mac", icon: "local", core: false }} selected={selected === null} counts={{ needs_you: 2, done: 0 }} connected onSelect={() => undefined} />
      <span aria-hidden="true" className="h-(--size-hairline) w-(--size-icon) bg-border" />
      {RAIL.map((bot) => (
        <BotTileView key={bot.name} name={bot.name} turn={bot.turn} connection={bot.name} mode={bot.mode} selected={bot.name === selected} onSelect={() => undefined} />
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
    const wire = (kind: BotModeKind): AccessMode => (kind === "all" ? { kind: "all" } : OWN);
    const id = payload.id;
    if (event.kind === "access_refuse") useShellStore.setState({ access: { ...current, pending: current.pending.filter((one) => one.id !== id) } });
    if (event.kind === "access_approve" && payload.mode) {
      const request = current.pending.find((one) => one.id === id);
      if (request) useShellStore.setState({ access: { ...current, pending: current.pending.filter((one) => one.id !== id), connections: [...current.connections, { ...connection(request.id, request.name, wire(payload.mode.kind), 0, 0, 0), program: request.program }] } });
    }
    const mode = payload.mode ? wire(payload.mode.kind) : null;
    if (event.kind === "access_mode" && mode) useShellStore.setState({ access: { ...current, connections: current.connections.map((one) => one.id === id ? { ...one, mode } : one) } });
    if (event.kind === "access_revoke") useShellStore.setState({ access: { ...current, connections: current.connections.filter((one) => one.id !== id) } });
    return true;
  }), []);
  useLayoutEffect(() => {
    useShellStore.setState({ connection: "live", access: fixture(state), rest: { ui_state: { interface_language: language } } } as never);
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
    document.documentElement.style.setProperty("--interface-scale", String(scale));
    return () => { document.documentElement.style.removeProperty("--interface-scale"); };
  }, [state, language, theme, scale]);
  return <TooltipProvider><InterfaceLanguageBoundary />
    <div className="flex bg-background" data-gallery-scene="external-tools" data-scene-ready="true">
      <Rail selected={state === "rail" ? "hermes-ctong" : null} />
      {state === "rail" ? null : (
        <div className="w-full max-w-[calc(var(--size-settings-sheet-w)-var(--size-settings-nav-w))] px-lg py-lg">
          <ExternalToolsTab actions={actions} />
        </div>
      )}
    </div>
  </TooltipProvider>;
}
