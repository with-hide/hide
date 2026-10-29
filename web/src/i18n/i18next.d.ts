import "i18next";
import type { english } from "./catalogs";

// Each message is typed by its {{name}} placeholders only, never by its whole
// English sentence: i18next parses the text of every message once per
// translator type it meets, so whole sentences made the typecheck grow with
// the catalog (4m26s of a 5-minute CI step). A call still has to pass each
// placeholder, as text or a number, and nothing else by those names.
type Placeholders<Text extends string> = Text extends `${string}{{${infer Name}}}${infer Rest}`
  ? `${string}{{${Name}}}${Placeholders<Rest>}`
  : string;

declare module "i18next" {
  interface CustomTypeOptions {
    defaultNS: "translation";
    keySeparator: false;
    nsSeparator: false;
    returnNull: false;
    returnObjects: false;
    strictKeyChecks: true;
    resources: { translation: { [Key in keyof typeof english]: Placeholders<(typeof english)[Key]> } };
  }
}
