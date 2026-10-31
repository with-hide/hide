// A fix applied to `@xterm/addon-webgl` as the web shell is built
// (`vite.config.ts`): the addon's own code, not this repository's.
//
// The addon keeps glyphs in atlas pages and uploads each page to the GPU texture
// slot of the same index, uploading again when the page's version number moves.
// When the atlas merges four pages into one (a terminal that has drawn a few
// thousand distinct glyphs, such as colored Korean text), the pages behind them
// move down to lower indexes. A page that moved into a slot can carry the
// same version number as the page the slot last uploaded, merged pages above
// all, since each starts again from 1. The slot keeps the old page's picture, so
// every glyph on that page is drawn from the wrong place: text breaks into
// fragments of other glyphs and stays broken until the page changes again.
//
// The slot now also remembers which page it uploaded and uploads again when a
// different page sits at its index. 0.20 betas still compare the version alone.
// The addon ships minified, so a dependency patch would replace its whole
// one-line file; this rewrites the two statements and refuses a build in which
// either is not found exactly once, so an update of the addon is read again
// here instead of silently losing the fix.

const COMPARE = /this\._atlas\.pages\[(\w+)\]\.version!==this\._atlasTextures\[\1\]\.version&&/g;
const RECORD = /this\._atlasTextures\[(\w+)\]\.version=(\w+)\.pages\[\1\]\.version/g;

export const WEBGL_ADDON_MODULE = /[\\/]@xterm[\\/]addon-webgl[\\/]lib[\\/]addon-webgl\.mjs$/;

export function fixWebglAtlasSlots(code: string): string {
  const compares = [...code.matchAll(COMPARE)].length;
  const records = [...code.matchAll(RECORD)].length;
  if (compares !== 1 || records !== 1) {
    throw new Error(
      `@xterm/addon-webgl: expected one atlas slot comparison and one upload record, found ${compares} and ${records}; ` +
        "read web/src/webglAtlasSlots.ts against the new version before building",
    );
  }
  return code
    .replace(
      COMPARE,
      (_, i: string) =>
        `(this._atlas.pages[${i}].version!==this._atlasTextures[${i}].version||this._atlas.pages[${i}].canvas!==this._atlasTextures[${i}].canvas)&&`,
    )
    .replace(RECORD, (whole, i: string, atlas: string) => `${whole},this._atlasTextures[${i}].canvas=${atlas}.pages[${i}].canvas`);
}
