import type { TabStripSizes } from "./areaLayout";

/** The tab strip's sizes from their design tokens; a token that does not resolve is a broken build, not a strip of empty tabs. */
export function readTabStripSizes(): TabStripSizes {
  const read = (name: string) => {
    const value = Number.parseFloat(getComputedStyle(document.documentElement).getPropertyValue(name)) || 0;
    if (!(value > 0)) throw new Error(`design token ${name} did not resolve to a pixel size`);
    return value;
  };
  return {
    preferred: read("--size-tab-preferred"),
    titleMin: read("--size-tab-title-min"),
    icon: read("--size-tab-icon-identity"),
    control: read("--size-control-sm"),
  };
}
