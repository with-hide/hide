// A module-resolution hook that lets node read a web source file as it is: `web/src/*.ts` imports its neighbours
// without an extension (`./laneTools`), which a bundler resolves and node does not. Node strips the types itself
// (22.18 and later, and 24), so with this hook the shortcut registry is imported as it stands, with no build and
// no second copy. Register it before importing a web file: `register("./ts-extensionless.mjs", import.meta.url)`.
export async function resolve(specifier, context, next) {
  if (/^\.\.?\//.test(specifier) && !/\.[a-z]+$/.test(specifier)) return next(`${specifier}.ts`, context);
  return next(specifier, context);
}
