// One invented project every part that needs a checkout draws from: the gallery's synthetic scene supplies the
// wire shape (workspaces, checkouts, panes), and `docsProject` renames it so no sample word is another language.
// The scene is a store seed, so a part seeds the app's singleton store with it before its first paint.
import { REFERENCE_FOLDS, sidebarScene } from "../../gallery/sceneData";
import type { Checkout, SnapshotRest, Workspace } from "../../snapshot";
import { NOW } from "./agent";

export type DocsProject = { rest: SnapshotRest; project: Workspace; checkout: Checkout };

export function docsProject(): DocsProject {
  const base = sidebarScene("reference", REFERENCE_FOLDS, NOW);
  const project = base.rest.navigator!.workspaces!.find((row) => row.id === "hide")!;
  const checkout = project.checkouts[0]!;
  project.label = "orders-app";
  project.path = "/projects/orders-app";
  checkout.path = "/projects/orders-app";
  return { rest: base.rest, project, checkout };
}
