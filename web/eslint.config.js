import js from "@eslint/js";
import i18next from "eslint-plugin-i18next";
import tseslint from "typescript-eslint";
import e2e from "./eslint.e2e.mjs";

// A product-owned sentence belongs in `src/i18n/resources` (docs/LOCALIZATION.md).
// Two guards keep one from appearing anywhere else:
//  - `i18next/no-literal-string` flags text a screen draws, as JSX children and
//    the attributes a person reads or hears;
//  - `no-restricted-syntax` flags Korean, Chinese and Japanese characters in any
//    string or template of product code, which is how every sentence the shell
//    shipped outside a catalog was written, `.ts` helpers included.
// What neither sees is an English sentence returned by a `.ts` helper, which the
// review of a change to one has to catch.

const READ_ATTRIBUTES = ["aria-label", "aria-description", "aria-roledescription", "title", "placeholder", "alt", "label", "description"];

// What a screen may draw without a catalog entry: a keycap or key name, a
// product name, a symbol or a unit, and a path or address shown as an example.
const WORDS_EXCLUDE = [
  /^[^\p{L}]*$/u,
  /^(Esc|Enter|Tab|Shift|Ctrl|Alt|Cmd|Space)$/,
  /^(Claude|Codex|OpenCode|GitHub|Herdr|Hide|Tailscale|Git)$/,
  /^[…\s]*(B|KB|MB|GB|TB)$/,
  /^(~\/|https?:\/\/)/,
  /^[a-z0-9._-]+(\/[a-z0-9._-]+)*$/,
];

const CJK = "[\\u3040-\\u30ff\\u3400-\\u4dbf\\u4e00-\\u9fff\\uac00-\\ud7a3]";

export default tseslint.config(
  js.configs.recommended,
  ...tseslint.configs.recommended,
  e2e,
  {
    ignores: ["dist/**", "src/generated/**"],
  },
  {
    files: ["src/**/*.{ts,tsx}"],
    ignores: ["src/**/*.test.{ts,tsx}", "src/i18n/**", "src/gallery/**"],
    plugins: { i18next },
    rules: {
      "i18next/no-literal-string": [
        "error",
        {
          mode: "jsx-only",
          "jsx-attributes": { include: READ_ATTRIBUTES },
          "jsx-components": { exclude: ["Kbd"] },
          // `coreErrorText` takes a catalog key as its fallback sentence, as `t` does.
          callees: { exclude: ["data", "fieldLabel", "commandLabel", "sheetRows", "divider", "cn", "t", "translate", "getChildren", "coreErrorText"] },
          words: { exclude: WORDS_EXCLUDE },
        },
      ],
      "no-restricted-syntax": [
        "error",
        { selector: `Literal[value=/${CJK}/]`, message: "Product text belongs in src/i18n/resources, not in code." },
        { selector: `TemplateElement[value.raw=/${CJK}/]`, message: "Product text belongs in src/i18n/resources, not in code." },
      ],
    },
  },
);
