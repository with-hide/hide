// Add a project on invented machines (PRD device-add-project B1-B19): the
// start view on a connected Linux device, with an offline Windows PC beside
// it in the machine select (`state=start`), and the dialog's own folder list
// of that device's home with a repository, a project already added, a folder
// holding repositories and a file (`state=list`). The start view carries the
// device's projects to import, one found by two agents, one by Paseo and
// Herdr, one with a long path, and enough more to scroll under the source
// chips (B20-B23, B32). The scene answers each folder
// list and import ask itself, as hided would from the device's node; it
// proves the rendered rows, not the node's reading.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { AddProjectDialog } from "../AddProjectDialog";
import { TooltipProvider } from "../components/ui/tooltip";
import { InterfaceLanguageBoundary } from "../i18n/client";
import type { Device, DeviceHost } from "../snapshot";
import { homeFolderKey, useShellStore, type HomeListing, type ImportableProject } from "../store";
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

// Activity counted back from when the scene opens, so each row reads as hours or days ago.
const HOUR = 3_600_000;
const ago = (hours: number) => Date.now() - hours * HOUR;
const IMPORTABLE: ImportableProject[] = [
  { path: `${HOME}/api-server`, label: "~/api-server", name: "api-server", sources: ["claude", "codex"], active_at_unix_ms: ago(2) },
  { path: `${HOME}/infra`, label: "~/infra", name: "infra", sources: ["paseo", "herdr"], active_at_unix_ms: ago(26) },
  {
    path: `${HOME}/projects/clients/northwind-traders/customer-portal-redesign`,
    label: "~/projects/clients/northwind-traders/customer-portal-redesign",
    name: "customer-portal-redesign",
    sources: ["opencode", "antigravity", "herdr"],
    active_at_unix_ms: ago(80),
  },
  { path: `${HOME}/projects/billing`, label: "~/projects/billing", name: "billing", sources: ["claude"], active_at_unix_ms: ago(120) },
  { path: `${HOME}/projects/landing`, label: "~/projects/landing", name: "landing", sources: ["codex", "herdr"], active_at_unix_ms: ago(200) },
  { path: `${HOME}/scratch/etl`, label: "~/scratch/etl", name: "etl", sources: ["claude"], active_at_unix_ms: ago(400) },
  { path: `${HOME}/projects/docs-site`, label: "~/projects/docs-site", name: "docs-site", sources: ["paseo"], active_at_unix_ms: ago(600) },
];

export function AddProjectScene({ params }: { params: URLSearchParams }) {
  const theme = params.get("theme") === "light" ? "light" : "dark";
  const lang = params.get("lang") ?? "ko";
  if (!["en", "ko", "ja", "zh-CN"].includes(lang)) throw new Error(`Unknown add project language ${lang}`);
  const state = params.get("state") ?? STATES[0];
  if (!(STATES as readonly string[]).includes(state)) throw new Error(`Unknown add project state ${state}`);
  const actions = useMemo(
    () =>
      createActions((event) => {
        if (event.kind === "importable_projects") {
          const { device_id } = event.payload as { device_id: string };
          window.setTimeout(() => useShellStore.getState().applyFrame({ type: "importable_projects", payload: { device_id, projects: IMPORTABLE, reason: null } } as never));
          return true;
        }
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
