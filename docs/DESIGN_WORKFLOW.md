# Public design workflow

This guide owns changes to the public interface, tokens and component library.
[UI_BEHAVIOR.md](UI_BEHAVIOR.md) owns shipped behavior.
Original design boards, proposals, authoring tools and maintainer skills are maintained separately.
When using a Pen canvas, keep distinct areas for the design system, components, screens, review proposals and temporary feature designs.
A public checkout requires no private repository or design-tool login to build, test or contribute.

## Authorities

- `design/tokens.json` owns numeric values, including Neutral and Hide palettes in Light and Dark.
- `web/src/components/ui` owns reusable parts; `web/src/components` owns product composites.
- `web/src/gallery` renders production components with invented data for review.
- An approved visual proposal defines the intended change; review it against actual captures under the same conditions.

The token generator remains public and is the only implementation of token parsing and web generation.
Run `node scripts/gen-tokens.mjs` after changing tokens.
It generates `web/src/tokens.css` and `web/src/generated/accents.ts`; do not edit those generated files by hand.
Original boards are not an input to the public build or CI.

## Make a change

1. Read the behavior contract and inspect the production component and gallery before proposing a change.
2. Use the existing tokens, component structure and interaction patterns.
   A new token or visual pattern needs a stated reason and human approval of the proposal before implementation.
3. Keep proposals and run evidence under ignored `agents/runs/<slug>/`.
   For a feature PRD with a visual change, prepare a temporary design alongside the PRD and use the approved proposal as the implementation reference.
   Do not commit scratch boards, captures, browser profiles or review transcripts.
4. Update the owning behavior guide and relevant gallery scene with the implementation.
   Gallery names, counts, paths and content are invented fixtures, never operator data.
5. Run the static checks below and the behavior checks the change touches.
6. Review actual captures and include a representative image in the pull request, following [CONTRIBUTING.md](../CONTRIBUTING.md).

## Visual review

Compare the chosen proposal with the production gallery or an isolated running candidate under the same viewport, theme, language, text scale, content and state.
Check Light and Dark, a narrow width, long Korean text and the largest supported text size where applicable.
Inspect empty, loading, failed, selected, hover and focused states that the change can produce.
Check wrapping, overlap, sideways overflow, stable columns, keyboard focus and legibility.
A gallery proves its own components and synthetic state, not native integration or a daemon transition.
Use an isolated candidate for those claims, following [VERIFICATION.md](VERIFICATION.md) and the applicable runtime guide.
Never operate the installed app or the operator's server for QA without authorization.

Private authoring tools may automate comparisons for maintainers, but they do not replace the public behavior contracts or human visual judgment.
A missing private tool is not a reason to omit public visual evidence.
A screenshot exemption names its concrete reason in the pull request for the reviewer to judge.

## Tokens and components

Add a token with both Dark (`value`) and Light (`light`) colors when it is a color.
Use `hide` only when the Hide palette differs, and use aliases instead of restating an existing value.
Regenerate the web outputs and use the token through Tailwind classes or CSS custom properties.
Remove a token only after no public consumer needs it and maintainers have checked their separately maintained boards.
For a reusable component, extend the existing part and its gallery states before introducing a sibling implementation.
Keep product composites in `web/src/components` and smaller reusable parts in `web/src/components/ui`.

## Checks

Run `node scripts/check-design-contract.mjs` before delivery.
The public `design-contract.yml` workflow runs this same entrypoint and the contrast regression suite.

- `check-web-tokens.mjs` refuses literal colors, sizes, radii and unchosen Tailwind values in web source.
- `check-token-contrast.mjs` checks terminal text and secondary text against their backgrounds in both palettes and modes.
- `check-agent-logos.mjs` checks bundled agent marks against their public registries and asset manifest.

`node --test scripts/tests/token-contrast.test.mjs scripts/tests/design-contract.test.mjs` checks contrast policy and staged-source isolation.
The opt-in `.githooks/pre-commit` invokes `node scripts/check-design-contract.mjs --staged`.
That mode reads staged checker modules, tokens, web sources and agent registries into a temporary directory without changing the index or working tree.
Stage the checker and inputs together; an unstaged repair must never make a broken staged candidate pass.
Static checks prove their stated contracts, not rendering quality or visual approval.
