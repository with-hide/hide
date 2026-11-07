// Add a project on invented machines (PRD device-add-project B1-B19): the
// start view on a connected Linux device, with an offline Windows PC beside
// it in the machine select (`state=start`), and the dialog's own folder list
// of that device's home with a repository, a project already added, a folder
// holding repositories and a file (`state=list`). The scene answers each
// folder list ask itself, as hided would from the device's node; it proves
// the rendered rows, not the node's listing.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { AddProjectDialog } from "../AddProjectDialog";
import { TooltipProvider } from "../components/ui/tooltip";
import { InterfaceLanguageBoundary } from "../i18n/client";
import type { Device, DeviceHost } from "../snapshot";
import { homeFolderKey, useShellStore, type HomeListing } from "../store";
import { useUiStore } from "../ui";
import { EMPTY_GALLERY_SCOPE } from "./agentScopes";

const STATES = ["start", "list"] as const;
type AddProjectState = (typeof STATES)[number];

const host = (platform: string): DeviceHost => ({ consent: "granted", helper_root: "~/.hide/host-helper", cli_dir: "~/.local/bin", contract: 1, bound_identity: null, granted_at_unix_ms: 1, state: "ready", message: null, platform, helper_path: null });
const device = (id: string, over: Partial<Device>): Device => ({ agent_scope: EMPTY_GALLERY_SCOPE, id, label: null, kind: "remote", state: "ready", message: null, ssh_alias: null, agent_count: 0, test: null, ...over });

const BUILD = "build-box";
const DEVICES: Device[] = [
  device("local", { kind: "local", state: "local", host: { ...host("macos aarch64"), consent: "this_machine" } }),
  device(BUILD, { label: "build-box", ssh_alias: "build-box", host: host("linux x86_64") }),
  device("tower", { label: "tower", ssh_alias: "tower", state: "unavailable", host: { ...host("windows x86_64"), state: "unavailable" } }),
];

const HOME = "/srv/dev";
const LISTINGS: Record<string, HomeListing> = {
  "~": {
    path: HOME,
    label: "~",
    home: HOME,
    truncated: false,
    entries: [
      { name: "api-server", is_directory: true, repository: true, branch: "main" },
      { name: "infra", is_directory: true, repository: true, branch: "feature/terraform-modules" },
      { name: "projects", is_directory: true, repositories: 4 },
      { name: "scratch", is_directory: true },
      { name: "web", is_directory: true, repository: true, branch: "main" },
      { name: "notes.md", is_directory: false },
    ],
  },
};

export function AddProjectScene({ params }: { params: URLSearchParams }) {
  const theme = params.get("theme") === "light" ? "light" : "dark";
  const lang = params.get("lang") ?? "ko";
  if (!["en", "ko", "ja", "zh-CN"].includes(lang)) throw new Error(`Unknown add project language ${lang}`);
  const state = params.get("state") ?? STATES[0];
  if (!(STATES as readonly string[]).includes(state)) throw new Error(`Unknown add project state ${state}`);
  const actions = useMemo(
    () =>
      createActions((event) => {
        if (event.kind !== "home_folder_list") return true;
        const { device_id, path } = event.payload as { device_id: string; path: string };
        // Answered after the ask returns, as a frame from hided arrives.
        window.setTimeout(() =>
          useShellStore.setState((current) => {
            const seq = current.homeFolderSeq + 1;
            const listing = LISTINGS[path] ?? null;
            const answer = { device_id, request: path, listing, reason: listing ? null : "not_found", seq };
            return { homeFolderSeq: seq, homeFolders: { ...current.homeFolders, [homeFolderKey(device_id, path)]: answer } };
          }),
        );
        return true;
      }),
    [],
  );
  useLayoutEffect(() => {
    useShellStore.setState({
      connection: "live",
      rest: {
        navigator: { devices: DEVICES, focused_device_id: BUILD, workspaces: [] },
        ui_state: { interface_language: lang, workspace_registrations: [{ id: 1, device_id: BUILD, path: `${HOME}/web` }], device_add_project_folders: {} },
        status: { last_error: null },
      },
    } as never);
    useUiStore.getState().openOverlay("add_project");
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
  }, [theme, lang]);
  useLayoutEffect(() => {
    if ((state as AddProjectState) !== "list") return;
    // The list is the dialog's own step: open it the way Browse folder does.
    const timer = window.setTimeout(() => document.querySelector<HTMLElement>("[data-add-project-browse]")?.click());
    return () => window.clearTimeout(timer);
  }, [state]);
  return (
    <>
      <InterfaceLanguageBoundary />
      <TooltipProvider>
        <div className="min-h-screen w-full bg-background text-foreground" data-gallery-scene="add-project" data-scene-ready="true">
          <AddProjectDialog actions={actions} />
        </div>
      </TooltipProvider>
    </>
  );
}
