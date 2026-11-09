// Settings › Labs: the features still being tried, each switched on here and
// off by default. Desk is the first; its page is its own component, drawn here
// so Desk's room and its settings stay one module.

import type { Actions } from "../actions";
import { DeskSettingsPage } from "../DeskScreen";

export function LabsTab({ actions }: { actions: Actions }) {
  return <DeskSettingsPage actions={actions} />;
}
