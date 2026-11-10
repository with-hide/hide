// The View area of the right lane: a file, a diff and a browser page as tabs, the file in front. It is the
// product's own `ViewAreas` over one invented layout; no document or page is read from anywhere.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../../actions";
import type { EditorDocumentSnapshot, EditorTabSnapshot, ViewDisplaySnapshot } from "../../snapshot";
import { useShellStore } from "../../store";
import { useUiStore } from "../../ui";
import { ViewAreas } from "../../ViewAreas";
import { docsProject } from "../fixtures/project";
import { PartFrame } from "../frame";

const SOURCE = `export async function listOrders(page: number) {
  const response = await fetch(\`/v2/orders?page=\${page}\`);
  if (!response.ok) throw new Error("orders request failed");
  return response.json();
}
`;

function fixture() {
  const { rest, project, checkout } = docsProject();
  const root = checkout.path;
  const tab: EditorTabSnapshot = { id: "docs-file", device_id: project.device_id, workspace_id: project.id, checkout_id: checkout.id, path: `${root}/api/orders.ts`, label: "orders.ts", kind: "file", diff_committed: null, markdown_live: false, wrap: false, dirty: false, preview: false };
  const base = { committed: null, preview: false, state: "open", reason: null } as const;
  const displays: ViewDisplaySnapshot[] = [
    { ...base, id: "d1", tab_id: tab.id, path: tab.path, label: tab.label, kind: "file" },
    { ...base, id: "d2", tab_id: null, path: `${root}/api/orders.ts`, label: "orders.ts", kind: "diff", committed: false },
    { ...base, id: "d3", tab_id: null, path: "", label: "Orders preview", kind: "browser", url: "http://localhost:5173/orders", title: "Orders preview" },
  ];
  const document: EditorDocumentSnapshot = { path: tab.path, language: "typescript", document_kind: "text", contents_utf8: SOURCE, revision: "sha256:0", dirty: false, readonly_reason: null, conflict: null, save: null };
  rest.navigator!.focused_workspace_id = project.id;
  rest.navigator!.focused_checkout_id = checkout.id;
  rest.workspace_view = {
    device_id: "local", path: root, open: true, documents: true, tool: null, pinned: true, document_width: null, detail_width: null, explorer_width: null, panel_width: null, lane_called: 0, lane_calls: 0, lane_called_side: "view",
    layout: { root: { area: { id: "a1", active: "d1", displays } }, active_area: "a1", limits: { areas: 4, depth: 3, displays: 32 }, display_count: displays.length },
  };
  return { rest, tab, document };
}

export default function ViewSlotPart() {
  const { rest, tab, document } = useMemo(fixture, []);
  const actions = useMemo(() => createActions(() => true), []);
  useLayoutEffect(() => {
    useShellStore.setState({ rest, agents: [], connection: "live", editor: { tabs: [tab], active_tab_id: tab.id }, documents: { [tab.id]: document } });
    useUiStore.setState({ screen: { kind: "workspace" }, stackFront: "view" });
  }, [rest, tab, document]);
  return (
    <PartFrame part="view-slot" height={320}>
      <div className="flex h-full min-h-0 min-w-0 rounded-md border border-border bg-card"><ViewAreas actions={actions} /></div>
    </PartFrame>
  );
}
