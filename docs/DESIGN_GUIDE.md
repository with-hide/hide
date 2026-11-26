# Design guide

Read this before changing anything a user looks at in `web/src` or a value in `design/tokens.json`.
It says which part to build a screen from and the few rules that keep screens alike.
[UI_BEHAVIOR.md](UI_BEHAVIOR.md) owns what each screen does; [DESIGN_WORKFLOW.md](DESIGN_WORKFLOW.md) owns how a change is proposed, reviewed and delivered.
A project hook refuses the first edit of a web source or the tokens in an agent session until this file has been read in that session (`scripts/hooks/design-guide-first.sh`).

## Three rules

1. Build from a part in `web/src/components/ui`, and from a product composite in `web/src/components` when one already draws the thing.
   When no part fits, extend the part (a variant, a size, a slot) with its gallery states in the same pull request; never restyle a raw `<button>`, `<input>` or layer inside a screen.
2. Reach every color, size, radius and text size through a token (`check-web-tokens.mjs` refuses the rest).
   Use a general token (`--size-control`, `--size-icon`, the spacing scale) before adding a feature-specific `--size-*`; a new token needs a reason the existing ones cannot carry.
3. Open every layer (menu, dialog, popover, sheet, select) through its part.
   The parts register with the shell's one Escape owner and hand keyboard focus back when they close (`components/ui/layer.ts`); a hand-made layer leaves Escape to the terminal under it or loses focus.
   A surface no part can draw, such as the Start panel's floating palette, registers itself with `useEscapeLayer` and hands focus back as the parts do; that is the exception, and it stays in the baseline.
   A surface whose own press handler stops `mousedown`, such as the terminal (`PaneView`), marks itself with `useOutsidePressSurface` (`components/ui/layer.ts`): a popover closes on the click and reads a press whose `mousedown` was stopped as taken elsewhere, so without the mark a popover over it stays open.

`check-ui-parts.mjs` counts raw `<button>`, `<input>`, `<select>`, `<textarea>` and `role="dialog"`/`role="menu"` outside `components/ui` against `scripts/ui-parts-baseline.json`.
A file may only go down: a new raw control fails, and a file with fewer than its baseline asks you to lock the lower number in with `node scripts/check-ui-parts.mjs --update`.

## Which part

| You need | Use | Notes |
| --- | --- | --- |
| An action | `Button` | `variant`: `default` (the one primary action), `secondary`, `outline`, `ghost` (toolbar and row actions), `destructive`, `link`, `text`, `text-link`, `row-action`, `tint` (the accent's light fill with the accent as its words, for the primary action of a small surface that is not a screen or a dialog, such as a corner notice's `지금 업데이트`), `ask` (the warning amber fill, the primary action of a card on the ask surface such as Desk's approval card's `한 번 허용`); `size`: `default`, `sm`, `lg`, `icon`, `icon-sm`, `icon-xs`, `inline`. The lime fill (`default`) is for the one irreversible primary action of a screen or dialog (a submit, a confirm); an action inside a card, a list or a row is `secondary`, `text` or `row-action`, and lime otherwise marks only the focus ring and a selection. |
| An action that reads as words in a row or a sentence (더 보기, 접기, 열기) | `Button variant="text" size="inline"`; `variant="text-link"` when it underlines on hover | `inline` keeps the surrounding text's size; set one with `className` (`text-caption`, `text-micro`) only to differ from it. |
| An icon-only control | `Button size="icon"`, `"icon-sm"` or `"icon-xs"` (a dense row) inside `Hint` | The hint's label is the control's accessible name. |
| An icon action inside a row that lights on hover | `Button variant="row-action" size="icon-xs"` | `ghost` would light in the row's own hover color and vanish. |
| A whole row that is one action | `RowButton` | You lay out its content; it spans the line, reads left to right and lights on hover. |
| A row that is one action but holds controls of its own | `RowOverlayButton` behind the content | Content gets `pointer-events-none relative`, each control inside `pointer-events-auto`; `aria-label` names the row's action. |
| One of a few options, all visible | `ToggleGroup type="single"` + `ToggleGroupItem` | Roving focus and arrow keys come with it; `size="sm"` for caption text, `"xs"` for micro text. `variant="pill"` on the group and each item draws loose rounded chips that wrap, for a filter over a list whose options vary in number (Add a project's sources). |
| Panels switched by a strip | `Tabs`, `TabsList`, `TabsTrigger`, `TabsContent` | |
| An on/off setting | `Switch` | |
| Several independent choices | `Checkbox` | |
| One of a short list, shown as rows | `RadioGroup`, `RadioGroupItem` | |
| One of a long list | `Select` | |
| A text field | `Input`; `Textarea` for several lines | `Textarea bare` inside a composer whose container is the frame. |
| A range | `Slider` | |
| A menu from a button | `DropdownMenu` | Hide's action menus are `EntryDropdown`, `EntryContextMenu` and `EntryPointMenu` in `components/entry-menu.tsx`. A short choice among a few (two or three rows that each carry a line of description) uses `DropdownMenuRadioItem mark="check"`: a green ✓ and a lit row for the chosen one, where the default `dot` marks a long list's choice quietly (the gallery's Dropdown Menu › Radio Check). |
| A menu on right-click | `ContextMenu` | |
| A floating panel with controls | `Popover` | |
| A modal task | `Dialog` | `DialogHeader`, `DialogBody`, `DialogFooter`. |
| Confirming an irreversible effect | `AlertDialog` | Prefer an undo (design principle 6); confirm only what cannot be undone. |
| Guarding a destructive choice that can be undone, in a row of choices | The same `Button`, pressed twice | The first press turns it into its confirm word (`제외 확인`) in the `destructive` variant in place, a press elsewhere puts it back, and only the second press sends; no layer opens (`AttentionChoices`, `components/mission-attention.tsx`). |
| A side or large panel | `Sheet` | |
| A searchable command or item list | `Command` | `CommandDialog` for the palette form. |
| A short label or count | `Badge` | `secondary` by default; `outline`, `default`, `destructive`, and `warning` for a mark that asks for a look. |
| A shortcut | `Kbd`, `KbdGroup` | An outline in `--keycap-line` with no fill, its letters muted. `Keycap` is the number a modifier hold reveals on a row or tile. |
| A hint on hover or focus | `Hint` (or `Tooltip` parts) | A hint holds nothing to click. |
| A notice the operator can act on | `Toast` inside the window's one `ToastStack` (`App.tsx`) | `level="info"` leaves on its own after `TOAST_INFO_MS` when given `timeout`; `level="important"` draws the warning border and mark and stays; a shell notice goes through `useUiStore`'s `showNotice` rather than a Toast of its own ([UI_BEHAVIOR.md](UI_BEHAVIOR.md#notices)). |
| A divider | `Separator` | Spacing first (design principle 8). |

## Product composites

These already draw a thing Hide shows in several places; use them instead of drawing it again.

| Thing | Composite |
| --- | --- |
| An agent's status | `StatusMark` (`status-mark.tsx`); a project's mark counts are `StatusBadge` |
| A session as a row (태스크 줄) | `SessionRowBody` (`session-row.tsx`), the marks, title line and lines under it, which the sidebar's `SidebarAgentRow`, the session record's children (`size="record"`) and `SessionRow` all draw; `SessionRow` is the pressable list item of it |
| A plan's steps | `PlanChecklist` (`plan-checklist.tsx`) |
| A map page, a mission's or a plan's | `MapPage`, `MapSplit` (the map with the column beside it), and in that column `MapPanel` with `MapPanelHead` for the picked card (`map-page.tsx`); what a step's card, row and panel say comes from `plans.ts` (`stepStatusWord`, `stepLeft`) |
| A state pill's colour | `PILL_TONE` and `STATE_PILL_TONE` (`mission-tone.tsx`) |
| A letter | `letter-mark.tsx`: `LetterTag` at the head of a letter's line, `RowLetterMark` on a sidebar row, `PaneLetterButton` in a pane header |
| A mission a session conducts | `MissionChip` (`mission-chip.tsx`), reading the store; `MissionChipView` draws one from values a caller holds; Desk draws a mission in two lines (`DeskMissionChip` for a card's, `MissionChipOf` for an answer's and the head of the session beside Desk), and on one line where an answer's table cell or list item names it (`MissionChipOf lines={1}`), the sidebar's Conductor row keeping its own one-line chip |
| An `@name` in a line of words | `MentionChip` (`mention-chip.tsx`), `kind` `session` or `project`; `lead` (a project's ⌂) replaces `@` on the same colour; Desk draws a project through `DeskMentionChip` in its messages, threads and the composer's chips, and a session there as `SessionChip` |
| A session named in a Desk line (a reply, a letter, a thread, the operator's message, a composer chip), with its card on hover | `SessionChip` (`components/desk/session-chip.tsx`), in the session colour of `MentionChip`, fed by `SessionChipProvider`; `SessionCard` is the card alone |
| An agent's runtime logo | `agent-logo.tsx` |
| Time since something | `Elapsed`, `useElapsed` (`elapsed.tsx`) |
| A row's second line that must keep its leading word | `RowLineText` |
| A settings form | `Group`, `Row`, `Disclosure`, `Value`, `Status`, `Note` (`settings-rows.tsx`); `Status` tone `live` is a connection that is up (`● 연결됨`), `ok` a check that passed |
| A pull request's mark or card | `pr-mark.tsx`, `pr-card.tsx` |
| A file or folder mark | `FileMark` |
| A device | `DeviceChip`, `device-rail.tsx`; the chip's `variant="inline"` puts its icon and name in a caption line without a border or padding, and `variant="remote"` fills it in the other-device colour (`--device-remote`) for the mission map's card and panel |
| What a close would take with it | `SubtreeList` |
| A state that stands in a box | `StateBox` |
| The children chip (`≡ ● 2 · • 3 ⌄`) | `TreeChipFace` (`tree-chip.tsx`), which the desktop pane header's button and the phone's chip on a list row and a detail's state strip all draw at their own sizes |
| Desk: its chat, job and approval cards, composer and agent bar, `@` menu, the session beside it and its head, the floating tab panel, outputs, Settings › Labs › Desk | `components/desk/*`, drawn from props in the shape of `web/src/desk.ts`; the gallery's `scene=desk` shows every state. In Desk a session reads as `StatusMark`, agent mark and title wherever it appears; a card is unfilled with a `--border` line, `rounded-lg` and two lines, selected on `bg-accent`; nothing filled sits inside another filled box; the approval card's two answers are `Button variant="secondary"`, the composer's send a round `secondary` icon, and every other action is words (`variant="text"`, such as `답하기 ›`); Desk draws no `default` button of its own, so lime marks focus and selection only (the question card is the phone's `AskCardView` as it is); the session's head reuses the pane head's parts (`AncestorPath`, `PaneHeadChildren`, `PaneWorkChip`, `PanePrChip`), and the tab panel reuses the Explorer, the file view, the mission map and Desk's output viewer, floating on `shadow-overlay` with `rounded-l-lg` over a `bg-scrim` dim |

## The phone

The phone app (`web/src/mobile`) builds its screens from the same parts and tokens, and from the shared layout parts in `web/src/mobile/layout.tsx`; a screen composes them and never draws its own copy.

| You need | Use | Notes |
| --- | --- | --- |
| A list screen's head (에이전트, 미션, 자동화) | `ScreenHead` | The big `text-headline` title, one line under it, and its action at the right. |
| A page's head (an agent, a mission, an automation, a look) | `PageHead` | Back, the marks that lead the title, the title with what never shrinks after it on its line (`titleEnd`, an agent's chips) and one line under it, an action at the right, and what the page keeps under its head (tabs, cells, buttons) as children. The lead is one title line tall, so the marks stay on the first line of a wrapped title. |
| A row of a list | `TwoLineRow` inside `RowButton`, or behind `RowOverlayButton` when the row holds a control | The lead column (a status mark), the title with what never shrinks after it, the time at the right; the second line starts under the title (`ROW_INSET`). Rows are separated by spacing, not dividers, except a page's run history. |
| The state of what a page is about | `StatusStrip` | The state's word in its colour, one line of what it does now and how long ago it changed; drawn only from fields the Mac sends. |
| What an agent does now, at the foot of its conversation | `ActivityLine` | `생각 중 · 4초`, its time counted on the phone. |
| A field a reply is written in | `Composer` | One rounded line with a round `secondary` send; each screen passes its own name, limit and test marks. |
| An agent's mark and logo, title, project | `AgentLead`, `AgentTitle`, `PlaceParts` (`web/src/mobile/parts.tsx`) | `PlaceParts` shows the project alone when the Mac sends `project`, else the place. |
| A chip (children, a mission, a pull request) | `PhoneChip` (`web/src/mobile/parts.tsx`) with the desktop's face inside (`TreeChipFace`, `MissionChipFace`, the PR mark) | The desktop chip's shape at the phone's text size; its border shows only while a child asks the operator. A press wraps it in a touch target. |

- Every press is at least `--size-touch-target` (44 px) tall; a row that is one press spans the screen's width.
- A row's less frequent action, such as removing a device, is its menu (`ContextMenu`), which a held finger opens as a right-click does, never a second button on the row; the click that ends the long press does not press the row.
- A hover fill lights only where a pointer can hover: the shared `hover` variant (`web/src/index.css`) sits in `@media (hover: hover)`, so a tap on a touch screen leaves no filled box behind.
- A device is drawn as the device picker draws it (`web/src/mobile/HostList.tsx`): its icon in a small rounded `bg-secondary` tile, its name on one line, its state's dot and word, and its 내 차례 and 작업 중 counts as two small mono `Badge`s at the right, the first in `text-warning`.
- The heads take the top safe area with `phone-safe-top`, and what sits at the bottom of a screen takes `phone-safe-bottom`; nothing else pads for the notch or the home bar.
- Check a phone screen at 393 by 852 in Light and Dark, with a long Korean title and with `--interface-scale` raised, as the compare captures for a phone pull request do.

## Tokens at a glance

`design/tokens.json` is the authority; `web/src/tokens.css` is generated from it.

- Text, small to large: `text-micro`, `text-caption`, `text-body`, `text-subhead`, `text-title`, `text-headline`, `text-display`.
- Spacing: `xxs`, `xs`, `sm`, `md`, `lg`, `xl`, `xxl`, `xxxl` (`gap-sm`, `px-xs`, ...).
- Radius: `rounded-xs` to `rounded-xl`.
- Text color by weight: `text-foreground`, then `text-subtle-foreground`, then `text-muted-foreground`.
- Surfaces: `bg-background`, `bg-card`, `bg-popover`, `bg-popover-nested`, `bg-dialog` are opaque; `bg-secondary`, `bg-accent` (hover) and `bg-code-block` are see-through fills over them.
- Every neutral color is a step of the generated palette (`--gray-*`, `--slate-*` for the Hide theme, and one step each of amber, red, green, blue and violet for status), named by its OKLCH lightness; a semantic token names its step under `palette` in `design/tokens.json`, and the generator refuses a recorded hex that its step does not draw. A new color picks a step; a new step is added only when no existing one fits, and screens use the semantic token, never the step.
- A see-through fill is a `fill` token: per mode, a palette step and its alpha (`{"step": "--gray-100", "alpha": 0.06}` in Dark, `--gray-0` in Light), written as `rgba()` of the step's sRGB (the phone's iOS 15 WebView has no `color-mix()`). It takes no Hide value, because the surface under it carries the theme. `--secondary` (a secondary button, a Badge, a picked picker, a segmented track, inline code), `--accent` (a hovered or picked row, a menu item), `--hairline` (the edge of a filled control, a Badge, a picker, a code block), `--keycap-line` (a `Kbd`'s outline), `--code-block` (a fenced code block) and `--segment-on` (a segmented control's picked item: white 16% in Dark, opaque white with `shadow-raised` in Light, since a see-through white over a Light track reads as the track) are fills; a screen never writes its own `foreground/6` or `rgb()` tint for one, nor an opacity on a fill (`bg-accent/40` is 2%, which no one sees).
- Because fills see through, they stack: a filled control's hover lays `--accent` over its `--secondary` with the `fill-hovered` utility (`hover:fill-hovered`), never `hover:bg-accent`, which would swap in a fainter fill; and a surface that has to hide what is under it, such as a drag ghost, a badge over an icon or a fade over a row's text, is opaque (`bg-popover`, `bg-raised`, `from-sidebar`), never a fill. A hover is never `hover:bg-popover`: the popover step is a floating surface's, not a lighter grey.
- Opaque surfaces climb one step at a time: `bg-background`, `bg-sidebar`, `bg-card`. `scripts/check-token-contrast.mjs` checks the minimum distance between neighbouring layers and from `--border` to the surfaces it sits on, and checks each fill composited over every surface it can sit on (`--background`, `--sidebar`, `--card`, `--muted`, `--raised`, `--input`, `--popover`, `--popover-nested`, `--dialog`, and a picked row in each accent choice), plus the hover over a filled control, the hairline over its fill and the picked segment over its track, and a floating surface on the page and a nested one on its modal, in Neutral and Hide, Dark and Light.
- A floating surface (a menu, the `@` and `/` menus, a select or picker, a popover or hover card, a tooltip, a toast, the ⌃⇥ and ⌥⇥ switchers) draws on `bg-popover`, the ⌘K palette's step, the same as a modal's `bg-dialog`; its edge is `shadow-overlay`'s hairline ring and the modal's shadow, never a `border-border` line. Its highlighted row is `--accent`, its separator `bg-hairline` and its keycaps outlines, as anywhere else.
- A menu stands one step above the surface it opened on: a modal (`Dialog`, `AlertDialog`, `Sheet`) wraps its content in `ModalSurface` (`components/ui/floating-surface.tsx`), and a floating part opened inside it adds `useFloatingSurface()`, which sets `--popover` to `--popover-nested` for that surface. React context crosses the portal a floating part renders through; a CSS variable set on the modal would not. A modal drawn without `Dialog`, `AlertDialog` or `Sheet`, such as the Start panel, wraps itself in `ModalSurface` and carries `data-modal-surface`, which an editor's tooltip, rendered inside the editor rather than through a portal, reads for the same step (`index.css`). `scripts/check-web-tokens.mjs` refuses a `role="dialog"` on `bg-dialog` in a file without both. A secondary button and a hovered row inside a modal need nothing of their own, since a fill sits one step above whatever it is laid on. Light draws both white and tells them apart by shadow.
- In Light the fills sit close together and shadows carry the rest: a small object that floats over its surface, such as a graph node, draws on `bg-raised` (white in Light) with `shadow-raised` rather than on a gray fill.
- Elevation has three levels and no others: `shadow-raised` for a control that sits on its surface, `shadow-overlay` for a popover, menu, select, tooltip or toast (in Dark the modal's hairline and shadow, since a floating surface shares the modal's step), and `shadow-modal` for a dialog, alert or sheet. A modal dims what is behind it with `bg-scrim`, never with an opacity on `bg-background`. Tailwind's own `shadow-sm`/`shadow-md`/`shadow-lg` are removed.
- Dimming and disabled use the opacity tokens (`opacity-(--opacity-disabled)`, `--opacity-dimmed`), never a raw number.

## Screen rules

- A state reaches the screen only where the operator would act on it; a failure they cannot act on goes to the log (design principle 13).
- Encode state with marks, color and position before sentences; a box or border has to carry information spacing cannot.
- Draw every state the screen can be in (empty, loading, failed, stale) in its smallest form, a mark or a dimmed row before a banner.
- Check long Korean text, a narrow width, Light and Dark, and the largest text size.
- Copy uses plain words the operator already knows, in Korean and English (`web/src/i18n`).

## Before delivery

1. Add or update the gallery state of any part you changed (`web/src/gallery`, served at `/gallery` by `pnpm --dir web dev`); the shared parts above are in its `scene=shared-parts` and Desk's parts in `scene=desk`.
2. Run `node scripts/check-design-contract.mjs`; it runs the token, contrast, logo and part-count checks.
3. Put a capture of the changed screen in the pull request, following [DESIGN_WORKFLOW.md](DESIGN_WORKFLOW.md#visual-review).
