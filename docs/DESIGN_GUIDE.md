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

`check-ui-parts.mjs` counts raw `<button>`, `<input>`, `<select>`, `<textarea>` and `role="dialog"`/`role="menu"` outside `components/ui` against `scripts/ui-parts-baseline.json`.
A file may only go down: a new raw control fails, and a file with fewer than its baseline asks you to lock the lower number in with `node scripts/check-ui-parts.mjs --update`.

## Which part

| You need | Use | Notes |
| --- | --- | --- |
| An action | `Button` | `variant`: `default` (the one primary action), `secondary`, `outline`, `ghost` (toolbar and row actions), `destructive`, `link`, `text`, `text-link`, `row-action`; `size`: `default`, `sm`, `lg`, `icon`, `icon-sm`, `icon-xs`, `inline`. The lime fill (`default`) is for the one irreversible primary action of a screen or dialog (a submit, a confirm); an action inside a card, a list or a row is `secondary`, `text` or `row-action`, and lime otherwise marks only the focus ring and a selection. |
| An action that reads as words in a row or a sentence (더 보기, 접기, 열기) | `Button variant="text" size="inline"`; `variant="text-link"` when it underlines on hover | `inline` keeps the surrounding text's size; set one with `className` (`text-caption`, `text-micro`) only to differ from it. |
| An icon-only control | `Button size="icon"`, `"icon-sm"` or `"icon-xs"` (a dense row) inside `Hint` | The hint's label is the control's accessible name. |
| An icon action inside a row that lights on hover | `Button variant="row-action" size="icon-xs"` | `ghost` would light in the row's own hover color and vanish. |
| A whole row that is one action | `RowButton` | You lay out its content; it spans the line, reads left to right and lights on hover. |
| A row that is one action but holds controls of its own | `RowOverlayButton` behind the content | Content gets `pointer-events-none relative`, each control inside `pointer-events-auto`; `aria-label` names the row's action. |
| One of a few options, all visible | `ToggleGroup type="single"` + `ToggleGroupItem` | Roving focus and arrow keys come with it; `size="sm"` for caption text, `"xs"` for micro text. |
| Panels switched by a strip | `Tabs`, `TabsList`, `TabsTrigger`, `TabsContent` | |
| An on/off setting | `Switch` | |
| Several independent choices | `Checkbox` | |
| One of a short list, shown as rows | `RadioGroup`, `RadioGroupItem` | |
| One of a long list | `Select` | |
| A text field | `Input`; `Textarea` for several lines | `Textarea bare` inside a composer whose container is the frame. |
| A range | `Slider` | |
| A menu from a button | `DropdownMenu` | Hide's action menus are `EntryDropdown`, `EntryContextMenu` and `EntryPointMenu` in `components/entry-menu.tsx`. |
| A menu on right-click | `ContextMenu` | |
| A floating panel with controls | `Popover` | |
| A modal task | `Dialog` | `DialogHeader`, `DialogBody`, `DialogFooter`. |
| Confirming an irreversible effect | `AlertDialog` | Prefer an undo (design principle 6); confirm only what cannot be undone. |
| A side or large panel | `Sheet` | |
| A searchable command or item list | `Command` | `CommandDialog` for the palette form. |
| A short label or count | `Badge` | `secondary` by default; `outline`, `default`, `destructive`. |
| A shortcut | `Kbd`, `KbdGroup` | `Keycap` is the number a modifier hold reveals on a row or tile. |
| A hint on hover or focus | `Hint` (or `Tooltip` parts) | A hint holds nothing to click. |
| A divider | `Separator` | Spacing first (design principle 8). |

## Product composites

These already draw a thing Hide shows in several places; use them instead of drawing it again.

| Thing | Composite |
| --- | --- |
| An agent's status | `StatusMark` (`status-mark.tsx`); a project's mark counts are `StatusBadge` |
| A session as a row (태스크 줄) | `SessionRowBody` (`session-row.tsx`), the marks, title line and lines under it, which the sidebar's `SidebarAgentRow`, the session record's children (`size="record"`) and `SessionRow` all draw; `SessionRow` is the pressable list item of it |
| A plan's steps | `PlanChecklist` (`plan-checklist.tsx`) |
| A letter | `letter-mark.tsx`: `LetterTag` at the head of a letter's line, `RowLetterMark` on a sidebar row, `PaneLetterButton` in a pane header |
| A mission a session conducts | `MissionChip` (`mission-chip.tsx`), reading the store; `MissionChipView` draws one from values a caller holds |
| An `@name` in a line of words | `MentionChip` (`mention-chip.tsx`), `kind` `session` or `project` |
| An agent's runtime logo | `agent-logo.tsx` |
| Time since something | `Elapsed`, `useElapsed` (`elapsed.tsx`) |
| A row's second line that must keep its leading word | `RowLineText` |
| A settings form | `Group`, `Row`, `Disclosure`, `Value`, `Status`, `Note` (`settings-rows.tsx`) |
| A pull request's mark or card | `pr-mark.tsx`, `pr-card.tsx` |
| A file or folder mark | `FileMark` |
| A device | `DeviceChip`, `device-rail.tsx` |
| What a close would take with it | `SubtreeList` |
| A state that stands in a box | `StateBox` |
| Desk: its chat, job and approval cards, composer and agent bar, `@` menu, Threads list, a thread, outputs, lane columns and rail, Settings › Desk | `components/desk/*`, drawn from props in the shape of `web/src/desk.ts`; the gallery's `scene=desk` shows every state. In Desk a session reads as `StatusMark`, agent mark and title wherever it appears; a card is unfilled with a `--border` line, `rounded-lg` and two lines, selected on `bg-accent`; nothing filled sits inside another filled box; the approval card's two answers are `Button variant="secondary"`, the composer's send a round `secondary` icon, and every other action is words (`variant="text"`, such as `답하기 ›`); Desk draws no `default` button, so lime marks focus and selection only; the lane's pin is `LanePinButton` (`components/lane-pin.tsx`), the same one the project lane draws, in the head of the column at the lane's edge just before `✕` |

## Tokens at a glance

`design/tokens.json` is the authority; `web/src/tokens.css` is generated from it.

- Text, small to large: `text-micro`, `text-caption`, `text-body`, `text-subhead`, `text-title`, `text-headline`, `text-display`.
- Spacing: `xxs`, `xs`, `sm`, `md`, `lg`, `xl`, `xxl`, `xxxl` (`gap-sm`, `px-xs`, ...).
- Radius: `rounded-xs` to `rounded-xl`.
- Text color by weight: `text-foreground`, then `text-subtle-foreground`, then `text-muted-foreground`.
- Surfaces: `bg-background`, `bg-card`, `bg-secondary`, `bg-accent` (hover), `bg-popover`.
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
