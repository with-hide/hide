// The production sidebar on the gallery's synthetic scene (PRD
// design-review-workflow D-04, B3, B4). This is the shell's own `Sidebar` and
// `createActions`, fed by `sidebarScene` instead of a daemon: a fold the
// operator clicks goes through the real action, and `applyEvent` answers it
// the way the core does. One scene fills one document, because the stores it
// seeds are the app's singletons.

import { useLayoutEffect, useMemo, useState } from "react";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { InterfaceLanguageBoundary } from "../i18n/client";
import { Sidebar } from "../sidebar";
import { TitleRow } from "../TitleRow";
import { SettingsGate } from "../SettingsSheet";
import { NOTHING_UNDER_WAY } from "../pairing";
import type { AgentScope } from "../agentScope";
import type { AgentRow, SnapshotRest } from "../snapshot";
import { useShellStore } from "../store";
import { OVERVIEW_ENTRY, useUiStore } from "../ui";
import { withMissionAsk } from "./MissionScene";
import { applyEvent, OFFLINE_DEVICE, REFERENCE_FOLDS, sidebarScene, sidebarStatusAxisScene, type SceneContent, type SceneDevices, type SceneFolds, type SceneFront } from "./sceneData";

/** What one scene document shows; every value comes from its query string. */
export type SceneParams = {
  axis?: boolean;
  state?: string;
  theme: "light" | "dark";
  /** The sidebar's width in CSS pixels, seeded as the core's `ui_state.sidebar_width`; absent, the sidebar's default. */
  width: number | null;
  /** The interface text scale the Appearance font size sets (`--interface-scale`); the sidebar follows it with the rest of the interface. */
  scale: number;
  content: SceneContent;
  /** One device, the two-device fixture, or that fixture with a busy `mini`; the rail shows in all three. */
  devices: SceneDevices;
  /** What is in front once the scene opens; a rail tile changes it. */
  front: SceneFront;
  /** `hidden` seeds the core's `ui_state.device_rail_visible` as false: the name on the top line is the device menu. */
  rail: "shown" | "hidden";
  /** The list the sidebar shows, seeded as the core's `ui_state.sidebar_lens`; `tasks-project` is the 태스크 list in 프로젝트별. */
  lens?: "projects" | "tasks" | "tasks-project";
  /** `shown` seeds the core's `ui_state.agent_logo_mode`, so the rows draw their logos. */
  logos?: "hidden" | "shown";
  /** `on`: a paired Mac-mini tells two Needs You rows of its own, under its head after this computer's (operator review 2026-11-22). */
  peers?: "off" | "on";
  /** The interface language, seeded as the core's `ui_state.interface_language`. */
  lang?: "en" | "ko";
};

export function sceneParams(params: URLSearchParams): SceneParams {
  const width = params.get("width");
  const scale = Number(params.get("scale") ?? "1");
  const content = params.get("content") ?? "reference";
  if (width !== null && !(Number(width) > 0)) throw new Error(`Scene width must be a positive number, got ${width}`);
  if (!(scale > 0)) throw new Error(`Scene scale must be a positive number, got ${params.get("scale")}`);
  if (content !== "reference" && content !== "long") throw new Error(`Unknown scene content ${content}`);
  const devices = params.get("devices") ?? "one";
  if (devices !== "one" && devices !== "two" && devices !== "busy") throw new Error(`Unknown scene devices ${devices}`);
  const front = params.get("front") ?? "local";
  if (front !== "local" && front !== "mini" && front !== "offline") throw new Error(`Unknown scene front ${front}`);
  if (devices === "one" && front !== "local") throw new Error(`Scene front ${front} needs devices=two`);
  const rail = params.get("rail") ?? "shown";
  if (rail !== "shown" && rail !== "hidden") throw new Error(`Unknown scene rail ${rail}`);
  const lens = params.get("lens") ?? "projects";
  if (lens !== "projects" && lens !== "tasks" && lens !== "tasks-project") throw new Error(`Unknown scene lens ${lens}`);
  const logos = params.get("logos") ?? "hidden";
  if (logos !== "hidden" && logos !== "shown") throw new Error(`Unknown scene logos ${logos}`);
  const peers = params.get("peers") ?? "off";
  if (peers !== "off" && peers !== "on") throw new Error(`Unknown scene peers ${peers}`);
  const lang = params.get("lang") ?? "en";
  if (lang !== "en" && lang !== "ko") throw new Error(`Unknown scene lang ${lang}`);
  return { lens, logos, peers, lang, theme: params.get("theme") === "light" ? "light" : "dark", width: width === null ? null : Number(width), scale, content, devices, front, rail, state: params.get("state") ?? "rest" };
}

export function SidebarScene({ theme, width, scale, content, devices, front, rail, state, lens = "projects", logos = "hidden", peers = "off", lang = "en", axis = false }: SceneParams) {
  const [folds, setFolds] = useState<SceneFolds>({ ...REFERENCE_FOLDS, frontDevice: front === "mini" ? "mini" : front === "offline" ? OFFLINE_DEVICE : "local" });
  const actions = useMemo(
    () =>
      createActions((event) => {
        setFolds((current) => {
          const next = applyEvent(current, event);
          if (!next) console.info(`gallery scene: ${event.kind} is not modelled here`);
          return next ?? current;
        });
      }),
    [],
  );

  // Seed the app's stores before the first paint, and again on every fold.
  const scene = useMemo(() => axis ? withOwnTurn(sidebarStatusAxisScene(content, folds, Date.now()), peers === "on") : sidebarScene(content, folds, Date.now(), devices), [content, folds, devices, axis, peers]);
  useLayoutEffect(() => {
    const sized = width === null ? scene.rest : { ...scene.rest, ui_state: { ...scene.rest.ui_state, sidebar_width: width } };
    const shown = { ...sized, ui_state: { ...sized.ui_state, device_rail_visible: rail === "shown", sidebar_lens: lens === "projects" ? "projects" as const : "tasks" as const, task_view: lens === "tasks-project" ? "project" as const : "status" as const, agent_logo_mode: logos, interface_language: lang } };
    // `mission-ask`: a mission waits on the operator with no session row to carry it, so Needs You lists the mission itself.
    // `mission-close-refused`: the same, for a mission whose nodes are all done and whose close was refused.
    // `mission-gate`: the same, for a reached review gate (PRD mission-human-gate D-23).
    const rest = state === "mission-ask" || state === "mission-close-refused" || state === "mission-gate" ? withMissionAsk(shown, "local", Date.now(), state === "mission-ask" ? "no-conductor" : state === "mission-gate" ? "gate-nocond" : "close-refused") : shown;
    const told = peers === "on" ? withPeerWork(rest, Date.now()) : rest;
    // This computer's own window, the only one that lists another computer's rows.
    useShellStore.setState({ rest: told, agents: scene.agents, connection: "live", pairing: peers === "on" ? NOTHING_UNDER_WAY : null, ...(axis ? { focusedPaneId: "a1" } : {}) });
  }, [scene, width, rail, axis, state, lens, logos, peers, lang]);

  useLayoutEffect(() => {
    useUiStore.setState({ screen: axis ? { kind: "workspace" } : { kind: "main" }, overview: { ...OVERVIEW_ENTRY, projectId: "hide" }, overviewOpen: false });
  }, [front, axis]);

  useLayoutEffect(() => {
    const root = document.documentElement;
    root.classList.toggle("dark", theme === "dark");
    root.classList.toggle("light", theme === "light");
    if (scale === 1) root.style.removeProperty("--interface-scale");
    else root.style.setProperty("--interface-scale", String(scale));
  }, [theme, scale]);

  return (
    <TooltipProvider>
      <InterfaceLanguageBoundary />
      <div className="flex h-full flex-col bg-background text-foreground" data-gallery-scene="projects-sidebar" data-scene-content={content} data-scene-devices={devices} data-scene-front={front} data-scene-rail={rail}>
        {/* The device name sits in the window's top row over the sidebar (PRD sidebar-titlebar-refresh B17). */}
        <TitleRow actions={actions} />
        <div className="flex min-h-0 flex-1">
          <Sidebar actions={actions} />
        </div>
      </div>
      <SettingsGate actions={actions} />
    </TooltipProvider>
  );
}

/** A paired Mac-mini and the two rows of its own Needs You it tells. */
function withPeerWork(rest: SnapshotRest, now: number): SnapshotRest {
  const key = "m".repeat(64);
  const rows = [
    { id: "w1:p1", group: "needs_you", title: "Claude", agent_kind: "claude", place: "docs · main", mark: "turn", status_code: "approval", changed_at_unix_ms: now - 3 * 60_000 },
    { id: "w2:p1", group: "needs_you", title: "Claude", agent_kind: "claude", place: "shop-api · main", mark: "turn", status_code: "approval", changed_at_unix_ms: now - 3 * 60_000 },
  ];
  const navigator = rest.navigator!;
  return {
    ...rest,
    navigator: {
      ...navigator,
      peers: [{ key, device_id: "mac-mini", name: "Mac-mini", opens: "active", opened_by: "active", connected: true, path: "direct" }],
      peer_work: [{ device_id: "mac-mini", key, needs_you: rows.length, done: 0, rows }],
    },
  } as SnapshotRest;
}

/** With `peers=on`, one of this computer's own tasks waits on the operator too, so its row and the paired machine's stand in one Needs You. */
function withOwnTurn(scene: { rest: SnapshotRest; agents: AgentRow[] }, on: boolean): { rest: SnapshotRest; agents: AgentRow[] } {
  if (!on) return scene;
  const agents = scene.agents.map((row) => row.pane_id === "q1" ? { ...row, group: "needs_you" as const, mark: "turn" as const, status_code: "approval" as const, demand: "approval" as const, detail: null } : row);
  const scope: AgentScope = { ...scene.rest.navigator!.agent_scope!, tasks: { cards: [{ root: "q1", state: "needs_you", members: ["q1"], checkouts: ["hide:main"], recency: "" }], needs_you: 1, recency: "" } };
  const navigator = scene.rest.navigator!;
  const rest = { ...scene.rest, navigator: { ...navigator, agents, agent_scope: scope, devices: navigator.devices!.map((device) => ({ ...device, agent_scope: scope })), workspaces: navigator.workspaces!.map((row) => ({ ...row, agent_scope: scope, checkouts: row.checkouts.map((c) => ({ ...c, agent_scope: scope })) })) } } as SnapshotRest;
  return { rest, agents };
}
