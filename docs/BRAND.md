# Hide brand

## Core idea

hide is a multi-agent IDE that helps one person keep several coding efforts in view without watching every terminal.
It preserves the context of each task and brings forward the questions, approvals, and results that need a person's attention.

**Brand line:** Run more agents. Hold less.
**Korean line:** 여러 에이전트의 흐름은 한눈에, 집중은 필요한 곳에만.

The product should make three things easy:

1. See which work is moving and where it belongs.
2. Notice a change that needs a decision or a review.
3. Return to that work without rebuilding its context.

These are product promises, not claims that hide completes or verifies an agent's work on the operator's behalf.
The actual attention groups and read rules are owned by [the status model](status-model.md).

## Mark

The approved mark is the owl face in [hide-mark.png](../design/brand/hide-mark.png).
Its two eyes suggest parallel work, while the connected lime shape brings them into one field of view.
The broad ivory face and open blue-gray space keep the expression calm and legible.
It is a symbol of awareness, not a character that speaks for the agents or a live status indicator.

Use the lowercase `hide` wordmark in text beside the mark when the product name is needed.
The mark itself contains no lettering.

| Role | Reference color | Use |
| --- | --- | --- |
| Quiet field | `#B7C9CC` | The mark's pale blue-gray background |
| Face | `#F4F4F6` | The owl's broad ivory silhouette |
| Shared view | `#B9FF66` | The connected eye shape and restrained brand emphasis |
| Focus | `#101112` | The two eyes and dark text on the light field |

These values describe the artwork; they do not replace the web shell's functional tokens or status colors.
The app's default Hide color theme carries the brand into the shell instead: its Dark is the landing page's Owl Field night, a blue-green near `#0D191B`, and its Light is white tinted with a little of the quiet field `#B7C9CC`, with a terminal palette tuned to each; the values live in `design/tokens.json` (`hide`), and the Neutral color theme keeps the grey chrome.
In Settings the Hide choice is marked by that quiet field `#B7C9CC` and Neutral by a grey (`--swatch-hide`, `--swatch-neutral`), the same in Light and Dark.
Keep the image's proportions, lower-right crop, eye spacing, and full square background.
Do not add glow, gradients, expressions, accessories, or a different color to signal agent state.
At small sizes, use the full mark rather than extracting either eye.

Inside the app the mark names Hide AI as the one who started an agent, never an agent's state: it stands beside the status mark, in the place of a provider logo, on a row that stands alone while logos are hidden, and as a badge on the provider's logo while they are shown (`web/src/AgentMark.tsx`, `web/src/assets/hide-mark.png`, a 64-pixel derivative of the full square mark).
The macOS app icon at `desktop/resources/hide-icon-1024.png` is the mark inside a rounded 896-pixel tile with a 64-pixel transparent inset on a 1024-pixel canvas.
`desktop/resources/hide.icns` packages that icon for the app.
`design/brand/hide-icon-blinking.svg` is that app icon drawn as vectors and set moving in a three-second loop: the face rises six pixels and settles, the eyes look left and then right with the lime shape and beak following a little, and the eyes blink twice along the face's tilt.
It was drawn in LottieFiles Creator over a trace of `hide-icon-1024.png` and exported as SVG with SMIL animation, so it plays inside an `<img>` with no script, GitHub's README included.
The site header and the repository README show it; it is the mark at ease, so it never stands for loading, progress or an agent's state, and inside the app it appears only where a PRD puts it.
The mobile web icons in `web/public/m/` are size derivatives of the full square mark.

## The owl character

The owl is hide's brand character: the mark's owl drawn whole, with a body, wings and a pose, for illustration.
It gives the site and other outward material warmth that the calm interface does not try to carry.
The character is not the mark: the header, the app icon, favicons and anywhere the product is named use the mark above.

The character keeps the mark's palette and features: the broad ivory face, the dark eyes inside the connected lime shape, and a round, soft silhouette.
A pose may hold one small prop that belongs to its scene, such as a laptop for devices or a conveyor for the Factory, and one owl appears per scene.
It may be gently cute, but stays calm: no glow, no gradients on the owl, no speech bubbles, and no third-party agent mark drawn into the art.

The character is a symbol of awareness, like the mark.
It never speaks for an agent, never stands in for a status mark, and never signals live state; a scene shows the owl at ease or at work, not reacting to what an agent did.
Copy beside it describes keeping work in view, never watching people or agents.
Inside the app the character appears only where a PRD puts it, because the shell's states belong to its status marks (design principle 13).

The current poses are in `site/assets/owls/`.
Each takes part in one clear action that belongs to what it is doing, and under it the owl rises and settles on its own breath and blinks while its eyes are open:

| Pose | Where | Scene | Motion |
| --- | --- | --- | --- |
| `owl-peek` | around the hero window | `owl-agents` traced into vector parts, peeking out from behind the window's edges | pops out at an edge, glances at the work there and ducks back, the same loop every time |
| `owl-ai` | beside the attention pillar | reading an open book | reads along the lines of each page, lifts the corner with its wing and turns the page, its head and body following the page across |
| `owl-agents` | beside the hand-off pillar | at ease among four terminals | the terminals clear and type in again one after another, clockwise, and the owl turns its head and leans toward each in turn |
| `owl-devices` | beside the core pillar | waving beside a laptop, the always-on core and a phone | a light runs along the dots from the laptop through the core to the phone and back, each device flashing as it arrives, and the owl's head follows it |
| `owl-factory` | in the Software Factory section | in a hard hat beside a conveyor | the conveyor steps one box on (the last box rolls off, the next gets its check, a new box is set down) and the owl turns to watch the box go |
| `owl-rest` | at the close | asleep in the crescent moon | breathes in and out, and a small Z drifts up on each breath out |

A pose keeps work in view rather than reacting to it: the face keeps its expression whatever a pane shows, the owl follows its own prop on a fixed schedule, and nothing in a loop answers what an agent did.
Every moving pose has a `-still.svg` frame that is complete on its own and is what shows off screen, in a hidden tab and under reduced motion; a section pose's loop starts on that frame.
The poses were made with the mark as the style reference, so a new pose starts from the mark again rather than from an edited copy of another pose, and it is reviewed against this section before it ships.
The SVGs are traced from those generated poses into separate flat shapes, with a moving prop redrawn as plain geometry and the owl's ears, face, eyes, wings and body kept as separate shapes so they can move.

## Third-party agent marks

The first-run agent choice, the Agents tab and Hide AI's agent selects and Add menu draw an agent's mark on a fixed light plate (`--logo-plate`), so a dark-filled mark stays legible in both themes (`web/src/components/agent-logo.tsx`).
`AgentMark` (`web/src/AgentMark.tsx`) draws the same logo at the compact badge size for each of the seven supported agents on a pane's own header and in the places that are about the provider itself; a sidebar or Sessions row and an Agent tab draw it only while the operator's view mode shows logos or an ⌥ hold peeks at them (`AgentRowMark`, PRD agent-logo-view-mode), and every other list row or result draws no logo; every row names its agent in its hint and accessible name (PRD agent-mark-quiet), and a plain shell or a kind Hide does not support keeps the neutral `>_`; `agentMarkOf` in `web/src/agentLogos.ts` is the one place that picks the logo, from the adapter's `logo_id`.
There the plate is only for a mark that would vanish on the dark theme: the manifest entry of a mark drawn dark on a transparent ground (Grok, Cursor) says `"plate": true`, and the others sit on the screen as published, since a plate behind every mark would turn the dark theme into a row of white tiles.
`web/src/assets/agents/manifest.json` is the only list: a bundled file names its source URL, licence, any modification and the date it was taken, and an agent without an official source mark is a monogram with the reason written beside it.
`scripts/check-agent-logos.mjs` (run by `check-design-contract`) fails a bundled file the manifest does not list, an adapter with no entry, an unknown entry and an entry without its source and licence.
A retired agent's logo may remain for stored sessions only when its manifest entry says `history_only: true` and names the retained reader in its note; an active adapter or provider cannot use that classification.
A mark can outlive its agent's row in the kit while Hide AI still offers that agent: Gemini CLI's stays for the Hide AI provider.
Marks are shown as the vendor published them: never recoloured, redrawn or approximated (design principle 10), and a mark whose source cannot be confirmed becomes a monogram rather than a guess.
A mark is bundled in the format the vendor publishes, an SVG or, where it publishes none, its PNG (`gemini-cli.png`), named for its agent's id so `web/src/agentLogos.ts` maps it without a list of its own.
An SVG that recolours itself under `prefers-color-scheme` takes the scheme from the app's theme inside an `<img>`, not from the plate behind it, so a dark-theme variant turns light on the light plate; where the vendor offers a fixed-colour variant that one is bundled instead, with the reason in its manifest `note` (Pi).
Showing a mark names the product it belongs to and implies no endorsement; trademark questions go to the vendor's own brand page named in the manifest.

## Voice and visual rhythm

Be calm, direct, and specific about what changed and what the operator can do.
Show summaries and relationships before raw activity, and let actionable changes carry the emphasis.
Keep layouts spacious and stable; use one clear accent at a time instead of decorating every working agent.
Avoid surveillance language, noisy urgency, and promises that an agent's output is correct before a person reviews it.

The logo signals this approach, while the interface continues to use its established design tokens and semantic status rules.

## Product decoration

The connecting desktop status page uses the original `desktop/static/owl-roam.json` with one locally bundled, pinned Lottie light player and a static SVG fallback.
Its lowercase hide wordmark and brand line sit inside the flight path, above a thin indeterminate bar and the host's localized connecting sentence.
Ready navigation never waits for a loop, and failure hides the decoration and preserves the existing recovery actions.
The player and bar pause for reduced motion, a hidden document or an offscreen scene, and the document destroys its player and listeners on exit.
The reduced-motion and failure still rests beside the brand so it cannot cover the words indefinitely.
The player is MIT licensed; `desktop/static/owl-motion-LICENSE.md` ships beside the status page.

The wizard uses `owl-agents`, `owl-devices` and `owl-ai` for its existing three steps.
The ready Home without projects uses `owl-ai`; Factory uses one `owl-factory` in its first-visit line or empty state, without opening the unreleased Factory.
`web/src/components/brand-owl.tsx` switches the canonical SVG animation to its matching still while motion is reduced or the image is hidden or offscreen, and owns and removes its listeners and intersection observer.
A failed decoration records a structured diagnostic and leaves the screen's actions available.
No decoration represents an agent's live state.

`design/assets/owls/*.png` are transparent raster derivatives of those vector stills for Pen's image fills, with the same colors and proportions.
Regenerate them with `rsvg-convert` at 288 square for the four web poses and 960 by 540 for the startup still.
These are design artwork, not verification captures.
