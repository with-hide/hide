import type { Actions } from "../actions";
import { HerdrImportGroup } from "../HerdrImport";
import { AppearanceGroup } from "./AppearanceGroup";

export function GeneralTab({ actions }: { actions: Actions }) {
  return (
    <>
      <AppearanceGroup actions={actions} />
      <HerdrImportGroup actions={actions} />
    </>
  );
}
