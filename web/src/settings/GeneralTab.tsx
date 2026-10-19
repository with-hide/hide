import type { Actions } from "../actions";
import { AppearanceGroup } from "./AppearanceGroup";

export function GeneralTab({ actions }: { actions: Actions }) {
  return <AppearanceGroup actions={actions} />;
}
