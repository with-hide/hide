// Production Settings › External tools with invented tools (PRD
// hide-external-control B2-B4, B11-B13, B15): `state=pending`, `connected`,
// `empty`; press a tool's row to open its agents and missions. Approve, level and disconnect
// move the invented section as the core would. No daemon and no keys.
import { useLayoutEffect, useMemo } from "react";
import { legacyAgentRow } from "../../test/legacyAgentRow";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { InterfaceLanguageBoundary } from "../i18n/client";
import { ExternalToolsTab } from "../settings/ExternalToolsTab";
import type { AccessConnection, AccessSection, AgentRow, Mission } from "../snapshot";
import { useShellStore } from "../store";
import { missionFixture } from "./MissionScene";

const MINUTE = 60_000;

function fixture(state: string): AccessSection {
  const now = Date.now();
  const busy: AccessConnection = {
    id: "gallery-conn-1", name: "hermes-ctong", level: "act", program: "/Users/me/.hermes/bin/hermes", created_ms: now - 2 * 86_400_000,
    last_used_ms: now - 3 * MINUTE, agents: [{ agent_id: "agent-41210", pane_id: "gallery:p1" }, { agent_id: "agent-41211", pane_id: "gallery:p2" }],
    agent_count: 2, missions: ["gallery-mission"], mission_count: 1,
  };
  const quiet: AccessConnection = {
    id: "gallery-conn-2", name: "nightly-cron", level: "view", program: "/usr/local/bin/nightly", created_ms: now - 9 * 86_400_000,
    last_used_ms: now - 26 * 60 * MINUTE, agents: [], agent_count: 0, missions: [], mission_count: 0,
  };
  return {
    machine: "gallery", limit: 16,
    pending: state === "pending" ? [{ id: "gallery-req-1", name: "hermes-zutongg-long-profile-name", program: "/Users/me/.hermes/profiles/zutongg/home/.local/bin/hermes-gateway", requested_ms: now - 20_000 }] : [],
    connections: state === "empty" ? [] : [busy, quiet],
  };
}

const AGENTS: AgentRow[] = [
  legacyAgentRow({ pane_id: "gallery:p1", identity_label: "로그인 버그 고치기", agent_kind: "claude", mark: "going", group: "working", status_code: "working" }),
  legacyAgentRow({ pane_id: "gallery:p2", identity_label: "릴리스 노트 초안", agent_kind: "codex", mark: "turn", status_code: "question" }),
];

export function ExternalToolsScene({ params }: { params: URLSearchParams }) {
  const state = params.get("state") ?? "pending";
  const language = params.get("lang") ?? "ko";
  const theme = params.get("theme") ?? "dark";
  const scale = params.get("scale") === "large" ? 17 / 13 : 1;
  const actions = useMemo(() => createActions((event) => {
    const current = useShellStore.getState().access;
    if (!current) return true;
    const id = (event.payload as { id?: string }).id;
    if (event.kind === "access_refuse") useShellStore.setState({ access: { ...current, pending: current.pending.filter((one) => one.id !== id) } });
    if (event.kind === "access_approve") {
      const request = current.pending.find((one) => one.id === id);
      const level = (event.payload as { level: AccessConnection["level"] }).level;
      if (request) useShellStore.setState({ access: { ...current, pending: current.pending.filter((one) => one.id !== id), connections: [...current.connections, { id: request.id, name: request.name, level, program: request.program, created_ms: Date.now(), last_used_ms: Date.now(), agents: [], agent_count: 0, missions: [], mission_count: 0 }] } });
    }
    if (event.kind === "access_level") useShellStore.setState({ access: { ...current, connections: current.connections.map((one) => one.id === id ? { ...one, level: (event.payload as { level: AccessConnection["level"] }).level } : one) } });
    if (event.kind === "access_revoke") useShellStore.setState({ access: { ...current, connections: current.connections.filter((one) => one.id !== id) } });
    return true;
  }), []);
  useLayoutEffect(() => {
    const { mission } = missionFixture("working", Date.now());
    const missions: Mission[] = [{ ...mission, id: "gallery-mission", title: "로그인 흐름 다시 만들기" }];
    useShellStore.setState({ connection: "live", access: fixture(state), agents: AGENTS, rest: { ui_state: { interface_language: language }, missions: { items: missions } } } as never);
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
    document.documentElement.style.setProperty("--interface-scale", String(scale));
    return () => { document.documentElement.style.removeProperty("--interface-scale"); };
  }, [state, language, theme, scale]);
  return <TooltipProvider><InterfaceLanguageBoundary />
    <div className="w-full max-w-[calc(var(--size-settings-sheet-w)-var(--size-settings-nav-w))] bg-background px-lg py-lg" data-gallery-scene="external-tools" data-scene-ready="true">
      <ExternalToolsTab actions={actions} />
    </div>
  </TooltipProvider>;
}
