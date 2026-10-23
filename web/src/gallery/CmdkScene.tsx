// ⌘K on invented data (PRD cmdk-navigation): the production palette over a
// synthetic snapshot, no daemon. A scene is one state of the palette: what is
// in front decides the empty list, and `github` decides how the explicit
// search is answered when its row is chosen, so each state can be reached by
// typing and pressing Return exactly as in the app.

import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { Palette } from "../Palette";
import type { GithubSearch } from "../snapshot";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";
import { noteKeyboardOwner } from "../viewFocus";
import { CMDK_RESULTS, READ_AT, RICH } from "./cmdkSceneData";

export type CmdkFront = "agent" | "agent-short" | "terminal" | "none";
export type CmdkGithub = "results" | "pending" | "failed" | "empty";

export type CmdkSceneParams = { theme: "light" | "dark"; front: CmdkFront; github: CmdkGithub };

export function cmdkSceneParams(params: URLSearchParams): CmdkSceneParams {
  const front = params.get("front") ?? "agent";
  const github = params.get("github") ?? "results";
  if (front !== "agent" && front !== "agent-short" && front !== "terminal" && front !== "none") throw new Error(`Unknown ⌘K front ${front}`);
  if (github !== "results" && github !== "pending" && github !== "failed" && github !== "empty") throw new Error(`Unknown ⌘K github ${github}`);
  return { theme: params.get("theme") === "light" ? "light" : "dark", front, github };
}

const ANSWERS: Record<CmdkGithub, (requestId: string, query: string) => GithubSearch> = {
  results: (request_id, query) => ({ request_id, query, phase: "ready", results: CMDK_RESULTS, message: null }),
  pending: (request_id, query) => ({ request_id, query, phase: "working", results: [], message: null }),
  failed: (request_id, query) => ({ request_id, query, phase: "failed", results: [], message: "github_search.failed" }),
  empty: (request_id, query) => ({ request_id, query, phase: "ready", results: [], message: null }),
};

export function CmdkScene({ theme, front, github }: CmdkSceneParams) {
  const actions = useMemo(
    () =>
      createActions((event) => {
        if (event.kind === "github_search") {
          const payload = event.payload as { request_id: string; query: string };
          useShellStore.setState((state) => ({
            rest: state.rest ? { ...state.rest, issue_work: { ...state.rest.issue_work, detail: null, name: null, search: ANSWERS[github](payload.request_id, payload.query) } } : state.rest,
          }));
        }
        return true;
      }),
    [github],
  );
  useLayoutEffect(() => {
    const focused = front === "agent" ? "p-child" : front === "agent-short" ? "p-dag" : null;
    const checkout = front === "agent" || front === "terminal" ? "c-sand" : "c-main";
    // The scene is drawn live, so the last GitHub read is a few minutes before it opens.
    const read = JSON.stringify(RICH).replaceAll(String(READ_AT), String(Date.now() - 4 * 60_000));
    const base = JSON.parse(read) as typeof RICH;
    const rest = { ...base, navigator: { ...base.navigator!, focused_workspace_id: "w1", focused_checkout_id: checkout } };
    useShellStore.setState({ rest, agents: rest.navigator!.agents ?? [], connection: "live", focusedPaneId: focused });
    noteKeyboardOwner(front === "agent" || front === "agent-short" ? { kind: "agent", workspace: "w1" } : { kind: "none" });
    useUiStore.setState({ screen: front === "none" ? { kind: "main" } : { kind: "workspace" }, overlay: "search", searchOver: "none" });
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
  }, [theme, front]);
  return (
    <TooltipProvider>
      <div className="h-full bg-background p-lg font-mono text-caption text-muted-foreground" data-gallery-scene="search-palette" data-scene-front={front}>
        {front === "none" ? "Settings › Devices" : "❯ CI 결과 알려줘"}
      </div>
      <Palette actions={actions} />
    </TooltipProvider>
  );
}
