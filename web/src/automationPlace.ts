// Where the automation screen is (PRD automations B8, B11, B16) against what
// the core's section holds. An automation or run the core just answered with
// (a make, a make with its first run) waits for the section that holds it:
// the answer frame and the snapshot delta leave hided in no fixed order, so
// the answer can arrive first. Anything else the section no longer holds was
// removed, and the screen steps back from it without a word.

import type { AutomationPlace } from "./ui";
import type { AutomationsSection } from "./snapshot";

/** The place to move to, or null when `place` stands as it is. */
export function settledPlace(place: AutomationPlace, section: AutomationsSection | null): AutomationPlace | null {
  if (section === null || place.selected === null) return null;
  const automation = section.items.find((item) => item.id === place.selected);
  if (!automation) {
    if (place.arriving) return null;
    return { selected: null, run: null, form: place.form === "new" ? "new" : null, arriving: false, example: place.form === "new" ? place.example : null, doneOpen: place.doneOpen };
  }
  const runHeld = place.run === null || automation.records.some((record) => record.kind === "run" && record.number === place.run);
  if (!runHeld) return place.arriving ? null : { ...place, run: null };
  return place.arriving ? { ...place, arriving: false } : null;
}
