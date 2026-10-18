import {buildUsageHistory} from './pen-usage-history.mjs';
// Draw every `System / <Part>` sheet the gallery names (web/src/gallery/manifest.ts,
// checked by check-pen-gallery.mjs) from primitives in hide's own idiom, re-dressed
// with hide tokens and matched 1:1 to what web/src/components/ui/*.tsx renders.
//
// Each sheet is generator-owned, the way Foundations is: gen-pen.mjs rebuilds every
// part's content on every run from the document's own (already-renamed) variables,
// so a fill is always a `$--` reference that resolves per `Mode` theme. Width and
// height are the one exception (a variable-bound width/height renders at zero in the
// installed Pen toolchain), so every frame's own size is a plain number read from
// tokens.json at generation time, never a `$--size-*` reference.
//
// A part draws ONE reusable master; every state in both the Light and Dark frame is
// a `ref` of that same master id with descendant overrides, never a redrawn copy or
// a second master per theme - the master's own `$--` fills already resolve per
// theme from wherever its instance sits, which is the entire point of Pen's `Mode`
// theme axis (pencil:shadcn works the same way: one master, instanced under both a
// Light-tagged and a Dark-tagged frame). Only the ref ids differ between the two
// frames, since two instances of one master still need two distinct node ids.
//
// D-02: where Pen's bundled pencil:shadcn library
// ($(npm root -g)/@pen.dev/cli/dist/out/data/shadcn.lib.pen) has a master for a
// part, that master's own node structure is what this file builds, re-dressed with
// this library's `$--` variables, hide's control/icon sizes and no drop shadows -
// never drawn free-hand. A committed library must not depend on a node_modules path
// at render time, so the shadcn node is not imported live; instead this file's own
// `frame`/`text`/`icon` calls reproduce its composition (children, order, roles)
// by hand, which is also the schema translation shadcn's file needs: it encodes
// icons as `icon_font`, strokes as a combined object, and shadows as `effect`,
// none of which this library's schema uses. Each builder below says in its sheet's
// spec line which shadcn master it is shaped from, or that no shadcn master exists
// and it is drawn from primitives in the same idiom (Toggle Group, Slider, Kbd,
// Separator, Sheet, Sonner; Popover reuses Dropdown Menu's content-surface idiom
// since shadcn has no separate Popover master). Where hide's own web component
// (`web/src/components/ui/*.tsx`) makes a deliberate token choice shadcn's example
// does not (Tabs' list/active-trigger fill, for instance), that web choice wins:
// the instruction is that the rendering must match web/src/components/ui, not that
// shadcn's own numbers are copied verbatim.
//
// The four System masters this replaces (Icon Button, Panel Tab, Badge, Keycap) are
// still referenced by many `Component /` sheets; `foldLegacyMasters` finds each by
// its existing node id and folds it into the part sheet that plays its old role,
// unchanged beyond the variable rename gen-pen already applied, so no Component ref
// dangles. Badge's old master doubles as the new Badge part's master directly
// (same shape: optional leading icon + label, already the shadcn Badge/Default
// shape); Icon Button's old master doubles as Button's Icon/Icon Small states
// directly (same shape: one centered icon, no label, already the shadcn Icon
// Button/Default shape - a ref of Button/Default with its label hidden). Panel Tab
// and Keycap are hide composites, not shadcn parts; they no longer live in a System
// sheet at all (a System sheet holds only its shadcn part) - `componentFoldSheets`
// gives each its own `Component / Panel Tab` / `Component / Keycap` sheet instead,
// generated the same way as every other sheet here.

import {buildAgentTreeParts, PR_STATES} from './pen-agent-tree.mjs';
import {buildHideAiOrigin} from './pen-hide-ai-origin.mjs';
import {buildUsage} from './pen-usage.mjs';
import {buildSidebarChrome} from './pen-sidebar-chrome.mjs';

const UI = '$--font-ui';
const MONO = '$--font-mono';

// The node helpers every Pen generator shares; pen-screens.mjs imports them.
export function num(tokens, name) {
  const token = tokens[name];
  if (!token) throw new Error(`the Pen generator needs token ${name}, which tokens.json does not carry`);
  return token.type === 'alias' ? num(tokens, token.value) : token.value;
}

export function text(id, content, {fill = '$--foreground', size = '$--text-body', weight = '400', mono = false, width, align} = {}) {
  return {
    type: 'text', id, name: content.length > 28 ? content.slice(0, 28) : content, content, fill,
    fontFamily: mono ? MONO : UI, fontSize: size, fontWeight: weight,
    ...(width ? {textGrowth: 'fixed-width', width} : {}), ...(align ? {textAlign: align} : {}),
  };
}

export function icon(id, glyph, {size = 14, fill = '$--foreground', enabled = true} = {}) {
  return {type: 'icon', id, name: 'Glyph', enabled, width: size, height: size, icon: glyph, library: 'lucide', fill};
}

export function frame(id, name, props, children) {
  return {type: 'frame', id, name, children, ...props};
}

// The status mark (web/src/components/status-mark.tsx, docs/status-model.md,
// The mark): one 12 px slot that draws one of four shapes, the operator's turn
// as a `!` in a warning badge, in progress as a static three-quarter ring,
// to read as a 7 px dot (green only for a finished verdict), and quiet, an
// agent at rest, as a faint hollow ring smaller and thinner than the
// in-progress one (sidebar-quiet-mark-indent D-02). A row in progress that
// waits on proven work draws the same ring dashed (status-marks D-25), and a
// row of unknown state draws nothing (D-04). A master holds the shapes as
// siblings, all off; an instance turns on the one its mark draws (`markSwitch`).
export const MARKS = ['turn', 'going', 'waiting', 'done', 'unread', 'quiet', 'unknown'];

// The waiting ring: the in-progress ring's centre line (r 4.05 in its 10 px
// box, 1.9 thick) cut into five dashes and four gaps along its three quarters,
// as web/src/components/status-mark.tsx dashes it (2.2 on, 2 off over 19).
function dashedArc() {
  const r = 4.05, dash = 270 * 2.2 / 19, gap = 270 * 2 / 19;
  const at = deg => {
    const a = deg * Math.PI / 180;
    return `${(5 + r * Math.cos(a)).toFixed(3)} ${(5 + r * Math.sin(a)).toFixed(3)}`;
  };
  const parts = [];
  for (let k = 0; k < 5; k++) {
    const from = k * (dash + gap);
    parts.push(`M ${at(from)} A ${r} ${r} 0 0 1 ${at(from + dash)}`);
  }
  return parts.join(' ');
}

export function markShapes(p) {
  return [
    frame(`${p}-turn`, 'Turn', {enabled: false, width: 12, height: 12, fill: '$--warning', cornerRadius: 6, layout: 'horizontal', justifyContent: 'center', alignItems: 'center'}, [
      {...text(`${p}-turn-bang`, '!', {fill: '$--status-foreground', size: '$--text-micro', weight: '700', mono: true})},
    ]),
    {type: 'ellipse', id: `${p}-arc`, name: 'In progress', enabled: false, width: 10, height: 10, fill: '$--agent-working', innerRadius: 0.62, startAngle: 90, sweepAngle: -270},
    {type: 'path', id: `${p}-arc-wait`, name: 'Waiting', enabled: false, width: 10, height: 10, viewBox: [0, 0, 10, 10], geometry: dashedArc(), stroke: '$--agent-working', strokeWidth: 1.9},
    {type: 'ellipse', id: `${p}-dot`, name: 'Unread', enabled: false, width: 7, height: 7, fill: '$--success'},
    // r 3.4 of 12 with a 1.3 stroke at the read-status opacity, as status-mark.tsx draws it.
    {type: 'ellipse', id: `${p}-quiet`, name: 'Quiet', enabled: false, width: 6.8, height: 6.8, fill: [], stroke: '$--muted-foreground', strokeWidth: 1.3, strokeAlignment: 'center', opacity: 0.55},
  ];
}

// The overrides that draw `mark` with the shapes `markShapes(p)` made, each
// color restated so an importing document resolves it per theme; `alias` is
// the import prefix (`hideui:`) and may be a path. A master that also holds
// the sleeping moon (`moon`, the sidebar row's `sar-moon`) has it restated
// too, on only for an agent asleep (status-marks D-21).
export function markSwitch(p, mark, alias = '', {moon = false} = {}) {
  if (!MARKS.includes(mark) && !(moon && mark === 'asleep')) throw new Error(`unknown status mark ${mark}`);
  return {
    ...(moon ? {[`${alias}${p}-moon`]: {enabled: mark === 'asleep', fill: '$--muted-foreground'}} : {}),
    [`${alias}${p}-turn`]: {enabled: mark === 'turn', fill: '$--warning'},
    [`${alias}${p}-turn-bang`]: {fill: '$--status-foreground'},
    [`${alias}${p}-arc`]: {enabled: mark === 'going', fill: '$--agent-working'},
    [`${alias}${p}-arc-wait`]: {enabled: mark === 'waiting', stroke: '$--agent-working'},
    [`${alias}${p}-dot`]: {enabled: mark === 'done' || mark === 'unread', fill: mark === 'unread' ? '$--foreground' : '$--success'},
    [`${alias}${p}-quiet`]: {enabled: mark === 'quiet', stroke: '$--muted-foreground'},
  };
}

// A board names a mark; an older board's glyph and colour still name the mark
// that replaced it, so a sheet drawn before the four marks means the same row.
export function legacyMark(symbol, fill) {
  if (MARKS.includes(symbol)) return symbol;
  if (['?', '!', '▲', '×', '✕'].includes(symbol)) return 'turn';
  if (symbol === '●') return 'going';
  if (symbol === '○') return fill === '$--agent-working' ? 'going' : 'quiet';
  if (symbol === '✓') return 'done';
  if (symbol === '◐') return 'unread';
  return 'quiet';
}

// A mark slot drawn in place, with only its own shape.
export function markSlot(id, mark, {height = 12} = {}) {
  const on = {turn: `${id}-turn`, going: `${id}-arc`, waiting: `${id}-arc-wait`, done: `${id}-dot`, unread: `${id}-dot`, quiet: `${id}-quiet`}[mark];
  return frame(id, 'Status mark', {width: 12, height, layout: 'horizontal', justifyContent: 'center', alignItems: 'center'},
    markShapes(id).filter(shape => shape.id === on).map(shape => ({...shape, enabled: true, ...(mark === 'unread' ? {fill: '$--foreground'} : {})})));
}

function ref(id, masterId, name, overrides = {}, descendants) {
  return {id, type: 'ref', ref: masterId, name, ...overrides, ...(descendants ? {descendants} : {})};
}

// A state's top-level node name must be exactly the manifest's state string
// (check-pen-gallery.mjs); the instance's own id makes the cell's ids unique.
function cell(stateName, instance) {
  const key = instance.id;
  return frame(`${key}-cell`, stateName, {layout: 'vertical', gap: '$--spacing-xs', alignItems: 'start', width: 'fit_content'}, [
    text(`${key}-cap`, stateName, {size: '$--text-micro', fill: '$--muted-foreground', weight: '600'}),
    instance,
  ]);
}

// check-pen-gallery.mjs reads this frame's DIRECT children as the state list, so a
// cell can never sit inside a grouping "Row" frame; Pen's own layout has no wrap,
// so a Light or Dark frame with many states is a single wide horizontal strip
// rather than a wrapped grid.
function themeFrame(id, mode, cells) {
  return frame(id, mode, {
    theme: {Mode: mode}, layout: 'horizontal', gap: '$--spacing-lg', alignItems: 'end', padding: '$--spacing-lg',
    fill: '$--background', width: 'fit_content', cornerRadius: '$--radius-md',
  }, cells);
}

function masterCard(id, label, node) {
  return frame(id, label, {layout: 'vertical', gap: '$--spacing-xs', alignItems: 'start', width: 'fit_content'}, [
    text(`${id}-cap`, label, {size: '$--text-micro', fill: '$--muted-foreground', weight: '600'}),
    node,
  ]);
}

function buildSheet(id, name, spec, masters, lightCells, darkCells) {
  return frame(id, name, {
    layout: 'vertical', gap: '$--spacing-xl', padding: '$--spacing-xl', fill: '$--card',
    cornerRadius: '$--radius-lg', width: 'fit_content',
  }, [
    text(`${id}-title`, name.replace('System / ', ''), {size: '$--text-headline', weight: '600'}),
    text(`${id}-spec`, spec, {size: '$--text-caption', fill: '$--subtle-foreground', width: 820}),
    ...(masters.length ? [frame(`${id}-masters`, 'Masters', {layout: 'horizontal', gap: '$--spacing-xl', alignItems: 'start'}, masters)] : []),
    themeFrame(`${id}-light`, 'Light', lightCells),
    themeFrame(`${id}-dark`, 'Dark', darkCells),
  ]);
}

// -- menu idiom shared by Select, Dropdown Menu, Context Menu, Command, and the
// hide-composite menus (Component / Explorer file menu, Component / View tab menu)
// that draw their own rows: both a shadcn-shaped row (icon + single-line label, for
// Dropdown Menu/Context Menu/Command) and hide's own entry-menu.tsx row (label with
// an optional stacked disabled-reason caption, no icon) are one reusable master,
// `mnu-item-m` - a Component sheet points at the same id these gallery cells do,
// rather than a menu row staying hand-drawn text forever.

const MENU_ITEM_STATE = {default: {}, highlighted: {fill: '$--accent', text: '$--accent-foreground'}, disabled: {text: '$--muted-foreground'}, destructive: {text: '$--destructive'}};
const MENUITEM_MASTER_ID = 'mnu-item-m';
const MENUSEP_MASTER_ID = 'mnu-sep-m';

function menuItemMaster() {
  return frame(MENUITEM_MASTER_ID, 'Menu Item', {
    reusable: true, width: 'fill_container', layout: 'horizontal', alignItems: 'center', gap: '$--spacing-sm', cornerRadius: '$--radius-xs',
    padding: ['$--spacing-xs', '$--spacing-sm'], fill: '#00000000',
  }, [
    icon('mnu-item-icon', 'pencil', {size: 14, fill: '$--muted-foreground'}),
    frame('mnu-item-body', 'Body', {width: 'fill_container', layout: 'vertical', gap: 0, alignItems: 'start'}, [
      text('mnu-item-label', 'Item', {fill: '$--foreground', size: '$--text-body'}),
      text('mnu-item-reason', 'Reason', {fill: '$--muted-foreground', size: '$--text-caption', enabled: false}),
    ]),
    text('mnu-item-shortcut', '⌘K', {fill: '$--muted-foreground', size: '$--text-caption', mono: true, enabled: false}),
  ]);
}

// The icon+single-line shape Dropdown Menu, Context Menu and Command's gallery
// cells demonstrate.
function menuItem(id, label, state = 'default', glyph) {
  const s = MENU_ITEM_STATE[state];
  return ref(id, MENUITEM_MASTER_ID, 'Item', {fill: s.fill ?? '#00000000', ...(state === 'disabled' ? {opacity: 0.45} : {})}, {
    'mnu-item-icon': glyph ? {icon: glyph, fill: s.text ?? '$--muted-foreground', enabled: true} : {enabled: false},
    'mnu-item-label': {content: label, fill: s.text ?? '$--foreground'},
    'mnu-item-reason': {enabled: false},
    'mnu-item-shortcut': {enabled: false},
  });
}

// The label-with-optional-stacked-reason shape entry-menu.tsx's `EntryItems`
// actually renders: a disabled item keeps its reason visible
// under the label, per design principle 9 (a disabled state shown with its cause).
function menuItemReason(id, label, {reason = null, disabled = false, destructive = false, shortcut = null, glyph = null} = {}) {
  const textFill = destructive ? '$--destructive' : '$--foreground';
  return ref(id, MENUITEM_MASTER_ID, label, {opacity: disabled ? 0.45 : 1}, {
    'mnu-item-icon': glyph ? {icon: glyph, enabled: true, fill: destructive ? '$--destructive' : '$--muted-foreground'} : {enabled: false},
    'mnu-item-label': {content: label, fill: textFill},
    'mnu-item-reason': reason ? {content: reason, enabled: true} : {enabled: false},
    'mnu-item-shortcut': shortcut ? {content: shortcut, enabled: true} : {enabled: false},
  });
}

// Shaped from shadcn's List Divider (D24KC): a padded row holding one zero-height
// `line` node, not a filled rectangle - Pen's `line` type is the primitive shadcn's
// own menu and list content uses for a hairline rule.
function menuSepMaster() {
  return frame(MENUSEP_MASTER_ID, 'Separator', {reusable: true, width: 'fill_container', padding: ['$--spacing-xxs', 0], justifyContent: 'center'}, [
    {type: 'line', id: 'mnu-sep-line', name: 'Line', width: 'fill_container', height: 0, stroke: '$--border', strokeWidth: '$--size-hairline', strokeAlignment: 'center'},
  ]);
}

function menuSeparator(id) {
  return ref(id, MENUSEP_MASTER_ID, 'Separator');
}

// Shaped from shadcn's Dropdown (cTN8T): rounded-sm (its own 6px), a --popover
// surface with a hairline --border, no drop shadow. Its item padding stays hide's
// menu-styles.ts px-sm/py-xs (--spacing-sm/--spacing-xs) rather than shadcn's raw
// [6,12], since hide's own menu already renders that way.
function menuContent(id, width, children) {
  return frame(id, 'Content', {
    layout: 'vertical', gap: 0, padding: '$--spacing-xxs', width, cornerRadius: '$--radius-sm',
    fill: '$--popover', stroke: '$--border', strokeWidth: 1, strokeAlignment: 'inner',
  }, children);
}

// -- Button (also carries the folded Icon Button master) ---------------------------

// The five variants design/hide-screens.pen also draws (as a ref-site override
// recipe, not a redrawn control; see pen-screens.mjs). Keyed and shaped exactly
// as System / Button's own state cells below, so a variant's color is written
// once and both the gallery and the screens regenerate from it.
export const BUTTON_VARIANTS = {
  default: {overrides: {}, fg: '$--primary-foreground'},
  secondary: {overrides: {fill: '$--secondary', stroke: '$--border', strokeWidth: '$--size-hairline', strokeAlignment: 'inner'}, fg: '$--subtle-foreground'},
  outline: {overrides: {fill: '$--background', stroke: '$--border', strokeWidth: '$--size-hairline', strokeAlignment: 'inner'}, fg: '$--foreground'},
  ghost: {overrides: {fill: '#00000000'}, fg: '$--subtle-foreground'},
  destructive: {overrides: {fill: '$--destructive'}, fg: '$--destructive-foreground'},
};

function buildButton(tokens, legacy) {
  const H = num(tokens, '--size-control'), HS = num(tokens, '--size-control-sm'), HL = num(tokens, '--size-control-lg');
  const DISABLED = num(tokens, '--opacity-disabled');
  const master = frame('btn-m', 'Button', {
    reusable: true, height: H, cornerRadius: '$--radius-sm', fill: '$--primary',
    layout: 'horizontal', alignItems: 'center', justifyContent: 'center', gap: '$--spacing-xs', padding: [0, '$--spacing-md'],
  }, [icon('btn-ic', 'sparkles', {fill: '$--primary-foreground', enabled: false}), text('btn-lb', 'Button', {fill: '$--primary-foreground', weight: '500'})]);

  function states(suffix) {
    const variant = (key, name, overrides, fg) => ref(`btn-${key}-${suffix}`, master.id, name, overrides, {'btn-ic': {fill: fg}, 'btn-lb': {fill: fg}});
    const named = (key, name) => variant(key, name, BUTTON_VARIANTS[key].overrides, BUTTON_VARIANTS[key].fg);
    return [
      cell('Default', named('default', 'Default')),
      cell('Default Hover', variant('defhover', 'Default Hover', {opacity: 0.9}, '$--primary-foreground')),
      cell('Default Focus', variant('deffocus', 'Default Focus', {stroke: '$--ring', strokeWidth: 1, strokeAlignment: 'outer'}, '$--primary-foreground')),
      cell('Default Disabled', variant('defdis', 'Default Disabled', {opacity: DISABLED}, '$--primary-foreground')),
      cell('Pending', variant('pending', 'Pending', {opacity: 0.72}, '$--primary-foreground')),
      cell('Secondary', named('secondary', 'Secondary')),
      cell('Secondary Hover', variant('sechover', 'Secondary Hover', {fill: '$--accent', stroke: '$--border', strokeWidth: '$--size-hairline', strokeAlignment: 'inner'}, '$--accent-foreground')),
      cell('Outline', named('outline', 'Outline')),
      cell('Ghost', named('ghost', 'Ghost')),
      cell('Ghost Hover', variant('ghosthover', 'Ghost Hover', {fill: '$--accent'}, '$--accent-foreground')),
      cell('Destructive', named('destructive', 'Destructive')),
      cell('Destructive Hover', variant('desthover', 'Destructive Hover', {fill: '$--destructive', opacity: 0.9}, '$--destructive-foreground')),
      cell('Link', variant('link', 'Link', {fill: '#00000000'}, '$--primary')),
      cell('Small', variant('small', 'Small', {height: HS, padding: [0, '$--spacing-sm']}, '$--primary-foreground')),
      cell('Large', variant('large', 'Large', {height: HL, padding: [0, '$--spacing-lg']}, '$--primary-foreground')),
      cell('Icon', ref(`btn-icon-${suffix}`, legacy.iconButton.master.id, 'Icon', {width: H, height: H})),
      cell('Icon Small', ref(`btn-iconsm-${suffix}`, legacy.iconButton.master.id, 'Icon Small', {})),
    ];
  }
  return buildSheet('sys-button', 'System / Button', 'Shaped from shadcn Button/Default (a leading icon slot + label, centered, rounded-sm, text-body medium, gap-xs); disabled at --opacity-disabled. Icon and Icon Small reuse the folded Icon Button master below (already shadcn Icon Button/Default: a ref of Button/Default with its label hidden).',
    [masterCard('btn-master-card', 'Master', master), masterCard('btn-legacy-card', 'Legacy master (Icon Button)', legacy.iconButton.master)], states('l'), states('d'));
}

// -- Input ---------------------------------------------------------------------

function buildInput(tokens) {
  const H = num(tokens, '--size-control');
  const master = frame('inp-m', 'Input', {
    reusable: true, height: H, width: 220, cornerRadius: '$--radius-sm', fill: '$--background',
    stroke: '$--input', strokeWidth: '$--size-hairline', strokeAlignment: 'inner',
    layout: 'horizontal', alignItems: 'center', padding: [0, '$--spacing-sm'],
  }, [text('inp-t', 'Value', {fill: '$--foreground'})]);
  function states(suffix) {
    return [
      cell('Default', ref(`inp-default-${suffix}`, master.id, 'Default')),
      cell('Placeholder', ref(`inp-placeholder-${suffix}`, master.id, 'Placeholder', {}, {'inp-t': {content: 'Search branches', fill: '$--muted-foreground'}})),
      cell('Filled', ref(`inp-filled-${suffix}`, master.id, 'Filled', {}, {'inp-t': {content: 'feature/theme-reset'}})),
      cell('Focus', ref(`inp-focus-${suffix}`, master.id, 'Focus', {stroke: '$--ring', strokeWidth: 1})),
      cell('Invalid', ref(`inp-invalid-${suffix}`, master.id, 'Invalid', {stroke: '$--destructive', strokeWidth: 1})),
      cell('Disabled', ref(`inp-disabled-${suffix}`, master.id, 'Disabled', {opacity: num(tokens, '--opacity-disabled')})),
      cell('Mono', ref(`inp-mono-${suffix}`, master.id, 'Mono', {}, {'inp-t': {content: 'origin/main', fontFamily: MONO}})),
    ];
  }
  return buildSheet('sys-input', 'System / Input', 'Shaped from shadcn Input Group/Default’s Input frame: height --size-control, rounded-sm, hairline --input border. `mono` swaps the interface face for machine text.', [masterCard('inp-master-card', 'Master', master)], states('l'), states('d'));
}

// -- Select ----------------------------------------------------------------------

function buildSelect(tokens) {
  const H = num(tokens, '--size-control'), W = num(tokens, '--size-settings-control-w');
  const master = frame('sel-m', 'Select Trigger', {
    reusable: true, height: H, width: W, cornerRadius: '$--radius-sm', fill: '$--background',
    stroke: '$--input', strokeWidth: '$--size-hairline', strokeAlignment: 'inner',
    layout: 'horizontal', alignItems: 'center', justifyContent: 'space_between', padding: [0, '$--spacing-sm'],
  }, [text('sel-t', 'Value', {fill: '$--foreground'}), icon('sel-i', 'chevron-down', {size: 14, fill: '$--muted-foreground'})]);
  function states(suffix) {
    const openContent = menuContent(`sel-content-${suffix}`, W, [
      menuItem(`sel-item1-${suffix}`, 'Dark'), menuItem(`sel-item2-${suffix}`, 'Light', 'highlighted'), menuItem(`sel-item3-${suffix}`, 'System'),
    ]);
    return [
      cell('Default', ref(`sel-default-${suffix}`, master.id, 'Default')),
      cell('Placeholder', ref(`sel-placeholder-${suffix}`, master.id, 'Placeholder', {}, {'sel-t': {content: 'Choose a theme', fill: '$--muted-foreground'}})),
      cell('Focus', ref(`sel-focus-${suffix}`, master.id, 'Focus', {stroke: '$--ring', strokeWidth: 1})),
      cell('Disabled', ref(`sel-disabled-${suffix}`, master.id, 'Disabled', {opacity: num(tokens, '--opacity-disabled')})),
      cell('Open', frame(`sel-openwrap-${suffix}`, 'Open', {layout: 'vertical', gap: '$--spacing-xxs'}, [ref(`sel-opentrig-${suffix}`, master.id, 'Trigger'), openContent])),
    ];
  }
  return buildSheet('sys-select', 'System / Select', 'Shaped from shadcn Select Group/Default’s Select Trigger over Radix: same hairline --input border as Input, trigger matches Input height, its open content shares the menu idiom with Dropdown Menu and Command.', [masterCard('sel-master-card', 'Master', master)], states('l'), states('d'));
}

// -- Checkbox ----------------------------------------------------------------------

function buildCheckbox(tokens) {
  const S = num(tokens, '--size-checkbox');
  const master = frame('chk-m', 'Checkbox', {
    reusable: true, width: S, height: S, cornerRadius: '$--radius-xs', fill: '$--background',
    stroke: '$--border', strokeWidth: '$--size-hairline', strokeAlignment: 'inner',
    layout: 'horizontal', alignItems: 'center', justifyContent: 'center',
  }, [icon('chk-i', 'check', {size: 12, fill: '$--primary-foreground', enabled: false}), icon('chk-bar', 'minus', {size: 12, fill: '$--primary-foreground', enabled: false})]);
  function states(suffix) {
    const checked = {fill: '$--primary', stroke: '$--primary', strokeWidth: '$--size-hairline', strokeAlignment: 'inner'};
    return [
      cell('Unchecked', ref(`chk-off-${suffix}`, master.id, 'Unchecked')),
      cell('Checked', ref(`chk-on-${suffix}`, master.id, 'Checked', checked, {'chk-i': {enabled: true}})),
      cell('Indeterminate', ref(`chk-mixed-${suffix}`, master.id, 'Indeterminate', checked, {'chk-bar': {enabled: true}})),
      cell('Focus', ref(`chk-focus-${suffix}`, master.id, 'Focus', {stroke: '$--ring', strokeWidth: 1})),
      cell('Disabled', ref(`chk-disabled-${suffix}`, master.id, 'Disabled', {opacity: num(tokens, '--opacity-disabled')})),
      cell('Checked Disabled', ref(`chk-ondis-${suffix}`, master.id, 'Checked Disabled', {...checked, opacity: num(tokens, '--opacity-disabled')}, {'chk-i': {enabled: true}})),
    ];
  }
  return buildSheet('sys-checkbox', 'System / Checkbox', 'Shaped from shadcn Checkbox/Checked’s Checkbox frame over Radix: --size-checkbox square, rounded-xs, a --size-icon-sm check glyph, checked and indeterminate fill --primary (a minus for a group whose members are partly checked) - hide’s sizes already match the shadcn node’s own 16/4/12.', [masterCard('chk-master-card', 'Master', master)], states('l'), states('d'));
}

// -- Switch --------------------------------------------------------------------

function buildSwitch(tokens) {
  const W = num(tokens, '--size-control'), H = num(tokens, '--size-checkbox'), T = num(tokens, '--size-icon');
  const master = frame('sw-m', 'Switch', {
    reusable: true, width: W, height: H, cornerRadius: '$--radius-xl', fill: '$--input',
    layout: 'horizontal', alignItems: 'center', padding: '$--spacing-xxs',
  }, [{type: 'ellipse', id: 'sw-thumb', name: 'Thumb', width: T, height: T, fill: '$--background'}]);
  function states(suffix) {
    return [
      cell('Off', ref(`sw-off-${suffix}`, master.id, 'Off')),
      cell('On', ref(`sw-on-${suffix}`, master.id, 'On', {fill: '$--primary', justifyContent: 'end'})),
      cell('Focus', ref(`sw-focus-${suffix}`, master.id, 'Focus', {stroke: '$--ring', strokeWidth: 1, strokeAlignment: 'outer'})),
      cell('Disabled', ref(`sw-disabled-${suffix}`, master.id, 'Disabled', {opacity: num(tokens, '--opacity-disabled')})),
    ];
  }
  return buildSheet('sys-switch', 'System / Switch', 'Shaped from shadcn Switch/Checked’s Switch frame over Radix: --size-control wide, --size-checkbox tall track, an ellipse thumb (shadcn’s own shape) at --size-icon.', [masterCard('sw-master-card', 'Master', master)], states('l'), states('d'));
}

// -- Radio Group -----------------------------------------------------------------

function buildRadioGroup(tokens) {
  const S = num(tokens, '--size-checkbox'), DOT = num(tokens, '--size-icon-sm');
  // web/src/components/ui/radio-group.tsx fills its indicator with a lucide
  // CircleIcon at --size-icon-sm, and shadcn's Radio/Selected (LbK20) draws a
  // filled circle in the same role. Pen's icon glyphs render lucide's `circle` as
  // a stroke-only ring even with `fill` set, so a filled ellipse at the same size
  // is what actually renders the solid dot both of those draw.
  const master = frame('rad-m', 'Radio Item', {
    reusable: true, width: S, height: S, cornerRadius: S / 2, fill: '$--background',
    stroke: '$--input', strokeWidth: '$--size-hairline', strokeAlignment: 'inner',
    layout: 'horizontal', alignItems: 'center', justifyContent: 'center',
  }, [{type: 'ellipse', id: 'rad-dot', name: 'Indicator', width: DOT, height: DOT, fill: '$--primary', enabled: false}]);
  function states(suffix) {
    const on = ref(`rad-on-${suffix}`, master.id, 'On', {stroke: '$--primary', strokeWidth: '$--size-hairline'}, {'rad-dot': {enabled: true}});
    const off = ref(`rad-off-${suffix}`, master.id, 'Off');
    const group = frame(`rad-group-${suffix}`, 'Default', {layout: 'horizontal', gap: '$--spacing-lg', alignItems: 'center'}, [on, off]);
    return [
      cell('Default', group),
      cell('Focus', ref(`rad-focus-${suffix}`, master.id, 'Focus', {stroke: '$--ring', strokeWidth: 1})),
      cell('Disabled', ref(`rad-disabled-${suffix}`, master.id, 'Disabled', {opacity: num(tokens, '--opacity-disabled')})),
    ];
  }
  return buildSheet('sys-radio', 'System / Radio Group', 'Shaped from shadcn Radio/Selected’s Radio frame over Radix: --size-checkbox circle, --spacing-sm gap between items, a filled --size-icon-sm dot indicator matching web/src/components/ui/radio-group.tsx’s CircleIcon.', [masterCard('rad-master-card', 'Master', master)], states('l'), states('d'));
}

// -- Toggle Group ------------------------------------------------------------------

function buildToggleGroup(tokens) {
  const H = num(tokens, '--size-control-sm');
  const master = frame('tog-m', 'Toggle Group Item', {
    reusable: true, height: H, cornerRadius: '$--radius-xs', layout: 'horizontal', alignItems: 'center',
    justifyContent: 'center', padding: [0, '$--spacing-sm'],
  }, [text('tog-t', 'Split', {fill: '$--subtle-foreground'})]);
  function states(suffix) {
    const off1 = ref(`tog-off1-${suffix}`, master.id, 'Tabbed', {}, {'tog-t': {content: 'Tabbed'}});
    const on = ref(`tog-on-${suffix}`, master.id, 'Split', {fill: '$--secondary'}, {'tog-t': {content: 'Split', fill: '$--foreground'}});
    const off2 = ref(`tog-off2-${suffix}`, master.id, 'Stacked', {}, {'tog-t': {content: 'Stacked'}});
    const group = frame(`tog-group-${suffix}`, 'Default', {layout: 'horizontal', gap: '$--spacing-xxs', padding: '$--spacing-xxs', fill: '$--card', cornerRadius: '$--radius-sm'}, [off1, on, off2]);
    return [
      cell('Default', group),
      cell('Hover', ref(`tog-hover-${suffix}`, master.id, 'Hover', {}, {'tog-t': {content: 'Tabbed', fill: '$--foreground'}})),
      cell('Focus', ref(`tog-focus-${suffix}`, master.id, 'Focus', {stroke: '$--ring', strokeWidth: 1}, {'tog-t': {content: 'Tabbed'}})),
      cell('Disabled', ref(`tog-disabled-${suffix}`, master.id, 'Disabled', {opacity: num(tokens, '--opacity-disabled')}, {'tog-t': {content: 'Tabbed'}})),
    ];
  }
  return buildSheet('sys-toggle', 'System / Toggle Group', 'No Toggle Group master exists in pencil:shadcn; drawn from primitives in the same track idiom as Tabs: a --spacing-xxs padded track, the active item on --secondary.', [masterCard('tog-master-card', 'Master', master)], states('l'), states('d'));
}

// -- Slider ------------------------------------------------------------------------

function buildSlider(tokens) {
  const THUMB = num(tokens, '--size-checkbox'), TRACK = num(tokens, '--spacing-xs');
  const master = frame('sld-m', 'Slider', {reusable: true, width: 160, height: THUMB, layout: 'horizontal', alignItems: 'center'}, [
    frame('sld-track', 'Track', {width: 'fill_container', height: TRACK, cornerRadius: '$--radius-xl', fill: '$--secondary'}, [
      {type: 'rectangle', id: 'sld-range', name: 'Range', width: 96, height: TRACK, fill: '$--primary', cornerRadius: '$--radius-xl'},
    ]),
    {type: 'ellipse', id: 'sld-thumb', name: 'Thumb', x: 88, width: THUMB, height: THUMB, fill: '$--background', stroke: '$--primary', strokeWidth: '$--size-hairline'},
  ]);
  function states(suffix) {
    return [
      cell('Default', ref(`sld-default-${suffix}`, master.id, 'Default')),
      cell('Focus', ref(`sld-focus-${suffix}`, master.id, 'Focus', {}, {'sld-thumb': {stroke: '$--ring', strokeWidth: 1}})),
      cell('Disabled', ref(`sld-disabled-${suffix}`, master.id, 'Disabled', {opacity: num(tokens, '--opacity-disabled')})),
    ];
  }
  return buildSheet('sys-slider', 'System / Slider', 'No Slider master exists in pencil:shadcn; drawn from primitives: --spacing-xs track on --secondary, --primary range, --size-checkbox thumb.', [masterCard('sld-master-card', 'Master', master)], states('l'), states('d'));
}

// -- Tabs (also carries the folded Panel Tab legacy master) -----------------------

function buildTabs(tokens, legacy) {
  const H = num(tokens, '--size-control-sm');
  const master = frame('tab-m', 'Tabs Trigger', {
    reusable: true, height: H, cornerRadius: '$--radius-xs', layout: 'horizontal', alignItems: 'center',
    justifyContent: 'center', padding: [0, '$--spacing-sm'],
  }, [text('tab-t', 'History', {fill: '$--subtle-foreground'})]);
  function states(suffix) {
    const active = ref(`tab-active-${suffix}`, master.id, 'Overview', {fill: '$--secondary'}, {'tab-t': {content: 'Overview', fill: '$--foreground'}});
    const inactive = ref(`tab-inactive-${suffix}`, master.id, 'History', {}, {'tab-t': {content: 'History'}});
    const list = frame(`tab-list-${suffix}`, 'Default', {layout: 'horizontal', gap: '$--spacing-xxs', padding: '$--spacing-xxs', fill: '$--card', cornerRadius: '$--radius-sm'}, [active, inactive]);
    return [
      cell('Default', list),
      cell('Hover', ref(`tab-hover-${suffix}`, master.id, 'Hover', {}, {'tab-t': {content: 'History', fill: '$--foreground'}})),
      cell('Focus', ref(`tab-focus-${suffix}`, master.id, 'Focus', {stroke: '$--ring', strokeWidth: 1}, {'tab-t': {content: 'History'}})),
      cell('Disabled', ref(`tab-disabled-${suffix}`, master.id, 'Disabled', {opacity: num(tokens, '--opacity-disabled')}, {'tab-t': {content: 'History'}})),
    ];
  }
  return buildSheet('sys-tabs', 'System / Tabs',
    'Shaped from shadcn’s Tab Item Active/Inactive and Tabs container over Radix: a --spacing-xxs padded track. The track and active-trigger fills follow web/src/components/ui/tabs.tsx’s own choice (list on --card, active trigger on --secondary) rather than shadcn’s example (list on --secondary, active trigger on --background), since the built component is what this must match. Panel Tab (the pane-header underline tab) is a hide composite, not a shadcn part; it now has its own Component / Panel Tab sheet.',
    [masterCard('tab-master-card', 'Master', master)], states('l'), states('d'));
}

// -- Badge (reuses the folded Badge master directly) -------------------------------

// The four variants design/hide-screens.pen also draws; see BUTTON_VARIANTS above.
export const BADGE_VARIANTS = {
  default: {overrides: {fill: '$--primary', strokeWidth: 0}, fg: '$--primary-foreground'},
  secondary: {overrides: {fill: '$--secondary', strokeWidth: 0}, fg: '$--subtle-foreground'},
  destructive: {overrides: {fill: '$--destructive', strokeWidth: 0}, fg: '$--destructive-foreground'},
  outline: {overrides: {fill: '#00000000', stroke: '$--border'}, fg: '$--subtle-foreground'},
};

function buildBadge(tokens, legacy) {
  const master = legacy.badge.master, iconId = legacy.badge.iconId, labelId = legacy.badge.labelId;
  function states(suffix) {
    const variant = (key, name, overrides, textFill) => ref(`bdg-${key}-${suffix}`, master.id, name, overrides, {[iconId]: {enabled: false}, [labelId]: {content: name, fill: textFill}});
    const named = (key, name) => variant(key, name, BADGE_VARIANTS[key].overrides, BADGE_VARIANTS[key].fg);
    return [
      cell('Default', named('default', 'Default')),
      cell('Secondary', named('secondary', 'Secondary')),
      cell('Destructive', named('destructive', 'Destructive')),
      cell('Outline', named('outline', 'Outline')),
    ];
  }
  return buildSheet('sys-badge', 'System / Badge', 'shadcn Badge: --size-badge-height tall, rounded-xs, text-micro. The master is the folded former System / Badge master, unchanged beyond its variable rename.', [masterCard('bdg-master-card', 'Master', master)], states('l'), states('d'));
}

// -- Kbd (also carries the folded Keycap legacy master) ---------------------------

function buildKbd(tokens, legacy) {
  const H = num(tokens, '--size-keycap-height');
  const master = frame('kbd-m', 'Kbd', {
    reusable: true, height: H, cornerRadius: '$--radius-xs', fill: '$--secondary',
    layout: 'horizontal', alignItems: 'center', justifyContent: 'center', padding: [0, '$--spacing-xs'],
  }, [text('kbd-t', '⌘K', {fill: '$--subtle-foreground', size: '$--text-caption', weight: '500'})]);
  function states(suffix) {
    const mod = ref(`kbd-mod-${suffix}`, master.id, 'Modifier', {}, {'kbd-t': {content: '⇧'}});
    const key = ref(`kbd-key-${suffix}`, master.id, 'Key', {}, {'kbd-t': {content: 'K'}});
    return [
      cell('Default', ref(`kbd-default-${suffix}`, master.id, 'Default')),
      cell('Group', frame(`kbd-groupwrap-${suffix}`, 'Group', {layout: 'horizontal', gap: '$--spacing-xxs', alignItems: 'center'}, [mod, key])),
    ];
  }
  return buildSheet('sys-kbd', 'System / Kbd',
    'No shadcn Kbd master exists in pencil:shadcn; drawn from primitives in hide’s own idiom: --size-keycap-height tall, rounded-xs, --secondary fill, text-caption. Keycap (the divider-bordered mono keycap) is a different, older idiom; it now has its own Component / Keycap sheet.',
    [masterCard('kbd-master-card', 'Master', master)], states('l'), states('d'));
}

// -- Separator -----------------------------------------------------------------

function buildSeparator(tokens) {
  const H = num(tokens, '--size-hairline');
  function states(suffix) {
    return [
      cell('Horizontal', frame(`sep-h-${suffix}`, 'Horizontal', {width: 160, height: H, fill: '$--border'}, [])),
      cell('Vertical', frame(`sep-v-${suffix}`, 'Vertical', {width: H, height: 48, fill: '$--border'}, [])),
    ];
  }
  return buildSheet('sys-separator', 'System / Separator', 'No Separator master exists in pencil:shadcn; drawn from primitives in List Divider\u2019s (D24KC) idiom: --size-hairline on --border, either axis.', [], states('l'), states('d'));
}

// -- Dropdown Menu (also the model Context Menu shares) ----------------------------

function buildDropdownMenu(tokens) {
  const H = num(tokens, '--size-control'), W = num(tokens, '--size-settings-control-w');
  const itemMaster = menuItemMaster(), sepMaster = menuSepMaster();
  const master = frame('ddm-m', 'Trigger', {
    reusable: true, height: H, cornerRadius: '$--radius-sm', fill: '$--secondary', stroke: '$--border',
    strokeWidth: '$--size-hairline', strokeAlignment: 'inner', layout: 'horizontal', alignItems: 'center',
    justifyContent: 'center', padding: [0, '$--spacing-md'],
  }, [text('ddm-t', 'Actions', {fill: '$--subtle-foreground'})]);
  function states(suffix) {
    return [
      cell('Closed', ref(`ddm-closed-${suffix}`, master.id, 'Closed')),
      cell('Open', frame(`ddm-openwrap-${suffix}`, 'Open', {layout: 'vertical', gap: '$--spacing-xxs'}, [
        ref(`ddm-opentrig-${suffix}`, master.id, 'Trigger'),
        menuContent(`ddm-openc-${suffix}`, W, [
          menuItem(`ddm-oi0-${suffix}`, 'Rename', 'default', 'pencil'),
          menuItem(`ddm-oi1-${suffix}`, 'Duplicate', 'default', 'copy'),
          menuSeparator(`ddm-osep-${suffix}`),
          menuItem(`ddm-oi2-${suffix}`, 'Delete', 'destructive', 'trash-2'),
        ]),
      ])),
      cell('Highlighted', menuContent(`ddm-hlc-${suffix}`, W, [
        menuItem(`ddm-hi0-${suffix}`, 'Rename', 'highlighted', 'pencil'),
        menuItem(`ddm-hi1-${suffix}`, 'Duplicate', 'default', 'copy'),
      ])),
      cell('Disabled Item', menuContent(`ddm-dic-${suffix}`, W, [
        menuItem(`ddm-dii0-${suffix}`, 'Rename', 'default', 'pencil'),
        menuItem(`ddm-dii1-${suffix}`, 'Duplicate', 'disabled', 'copy'),
      ])),
      cell('Destructive Item', menuContent(`ddm-destc-${suffix}`, W, [
        menuItem(`ddm-desti0-${suffix}`, 'Rename', 'default', 'pencil'),
        menuItem(`ddm-desti1-${suffix}`, 'Delete', 'destructive', 'trash-2'),
      ])),
    ];
  }
  return buildSheet('sys-dropdown-menu', 'System / Dropdown Menu', 'Shaped from shadcn’s Dropdown (cTN8T) for the open content (rounded-sm, --popover fill, hairline --border, no drop shadow) and List Item/List Divider (2JGXl, D24KC) for each row: --radius-xs items, --accent highlight, a `line`-type separator. The trigger itself has no shadcn master and is drawn as a --secondary control. Context Menu draws the identical menu content over a point anchor. Menu Item and Separator are their own reusable masters (mnu-item-m, mnu-sep-m) - every Component sheet that draws a menu row points at them instead of hand-drawing one.', [masterCard('ddm-master-card', 'Master', master), masterCard('mnu-item-card', 'Menu Item master', itemMaster), masterCard('mnu-sep-card', 'Separator master', sepMaster)], states('l'), states('d'));
}

function buildContextMenu(tokens) {
  const W = num(tokens, '--size-settings-control-w');
  function states(suffix) {
    const surface = frame(`ctx-surf-${suffix}`, 'Target', {
      width: 220, height: 64, cornerRadius: '$--radius-md', fill: '$--card', stroke: '$--border',
      strokeWidth: '$--size-hairline', strokeAlignment: 'inner', layout: 'horizontal', alignItems: 'center', justifyContent: 'center',
    }, [text(`ctx-surf-t-${suffix}`, 'Right-click target', {fill: '$--muted-foreground', size: '$--text-caption'})]);
    return [
      cell('Open', frame(`ctx-openwrap-${suffix}`, 'Open', {layout: 'vertical', gap: '$--spacing-xxs'}, [
        surface,
        menuContent(`ctx-openc-${suffix}`, W, [menuItem(`ctx-oi0-${suffix}`, 'Copy path', 'default', 'copy'), menuItem(`ctx-oi1-${suffix}`, 'Reveal in Finder', 'default', 'folder-open')]),
      ])),
      cell('Highlighted', menuContent(`ctx-hlc-${suffix}`, W, [menuItem(`ctx-hi0-${suffix}`, 'Copy path', 'highlighted', 'copy'), menuItem(`ctx-hi1-${suffix}`, 'Reveal in Finder', 'default', 'folder-open')])),
      cell('Disabled Item', menuContent(`ctx-dic-${suffix}`, W, [menuItem(`ctx-dii0-${suffix}`, 'Copy path', 'default', 'copy'), menuItem(`ctx-dii1-${suffix}`, 'Reveal in Finder', 'disabled', 'folder-open')])),
    ];
  }
  return buildSheet('sys-context-menu', 'System / Context Menu', 'shadcn’s DropdownMenu over a controlled point anchor: the identical Dropdown/List Item content as System / Dropdown Menu, opened at the pointer or under the target for the menu key and ⇧F10.', [], states('l'), states('d'));
}

// -- Dialog / Alert Dialog / Sheet / Popover / Tooltip / Command / Sonner ---------

// Dialog and Alert Dialog's footer buttons are refs of Button's own master
// ('btn-m', built by buildButton above), the same reassembly D-21/B18 asks of
// every Component sheet: a part that contains a button points at the Button
// master with overrides rather than drawing one by hand.
function dialogButton(id, label, overrides, textFill) {
  return ref(id, 'btn-m', label, overrides, {'btn-ic': {enabled: false}, 'btn-lb': {content: label, fill: textFill}});
}

function buildDialog(tokens) {
  const W = num(tokens, '--size-worktree-dialog'), PAD = num(tokens, '--spacing-lg');
  // Shaped from shadcn's Card (pcGlv) through its Modal/Left override (oVUJY):
  // a Header slot holding a stacked Title + Subtitle, an Actions slot holding two
  // Button refs, rounded-lg, --popover fill, hairline --border, no drop shadow.
  function surface(id, withClose) {
    return frame(id, withClose ? 'With Close Button' : 'Open', {
      width: W, cornerRadius: '$--radius-lg', fill: '$--popover', stroke: '$--border', strokeWidth: '$--size-hairline',
      strokeAlignment: 'inner', layout: 'vertical', gap: '$--spacing-md',
    }, [
      frame(`${id}-hdr`, 'Header', {layout: 'vertical', gap: '$--spacing-xs', padding: ['$--spacing-lg', '$--spacing-lg', 0, '$--spacing-lg']}, [
        frame(`${id}-hdrtop`, 'Header Top', {layout: 'horizontal', justifyContent: 'space_between', alignItems: 'start'}, [
          text(`${id}-title`, 'Rename worktree', {fill: '$--foreground', size: '$--text-title', weight: '600'}),
          ...(withClose ? [icon(`${id}-x`, 'x', {size: 14, fill: '$--muted-foreground'})] : []),
        ]),
        text(`${id}-desc`, 'Choose a new name for this worktree. Panes stay attached.', {fill: '$--subtle-foreground', width: W - 2 * PAD}),
      ]),
      frame(`${id}-ftr`, 'Footer', {layout: 'horizontal', justifyContent: 'end', gap: '$--spacing-sm', padding: [0, '$--spacing-lg', '$--spacing-lg', '$--spacing-lg']}, [
        dialogButton(`${id}-cancel`, 'Cancel', {fill: '#00000000'}, '$--subtle-foreground'),
        dialogButton(`${id}-ok`, 'Rename', {}, '$--primary-foreground'),
      ]),
    ]);
  }
  function states(suffix) { return [cell('Open', surface(`dlg-open-${suffix}`, false)), cell('With Close Button', surface(`dlg-close-${suffix}`, true))]; }
  return buildSheet('sys-dialog', 'System / Dialog', 'Shaped from shadcn’s Card through its Modal/Left override: --size-worktree-dialog wide, rounded-lg, a stacked title+description header, footer actions as refs of System / Button, an overlay at --opacity-secondary behind it.', [], states('l'), states('d'));
}

function buildAlertDialog(tokens) {
  const W = num(tokens, '--size-add-device-sheet-w'), PAD = num(tokens, '--spacing-lg');
  // web/src/components/ui/alert-dialog.tsx wraps title, description and actions in
  // one --spacing-lg pad, not Card's three separately-padded slots - that choice
  // stays, but the actions are now Button refs like Dialog's.
  function surface(id, pending) {
    return frame(id, pending ? 'Pending' : 'Open', {
      width: W, cornerRadius: '$--radius-lg', fill: '$--popover', stroke: '$--border', strokeWidth: '$--size-hairline',
      strokeAlignment: 'inner', layout: 'vertical', gap: '$--spacing-md', padding: '$--spacing-lg',
    }, [
      text(`${id}-title`, 'Delete this worktree?', {fill: '$--foreground', size: '$--text-title', weight: '600'}),
      text(`${id}-desc`, 'This removes the checkout and its build cache. The branch itself is not deleted.', {fill: '$--subtle-foreground', width: W - 2 * PAD}),
      frame(`${id}-ftr`, 'Footer', {layout: 'horizontal', justifyContent: 'end', gap: '$--spacing-sm'}, [
        dialogButton(`${id}-cancel`, 'Cancel', {fill: '$--secondary'}, '$--subtle-foreground'),
        dialogButton(`${id}-del`, pending ? 'Deleting…' : 'Delete worktree', {fill: '$--destructive', opacity: pending ? 0.72 : 1}, '$--destructive-foreground'),
      ]),
    ]);
  }
  function states(suffix) { return [cell('Open', surface(`adlg-open-${suffix}`, false)), cell('Pending', surface(`adlg-pending-${suffix}`, true))]; }
  return buildSheet('sys-alert-dialog', 'System / Alert Dialog', 'Shaped from shadcn’s Card/Modal, single-padded per web/src/components/ui/alert-dialog.tsx: a confirmation for an irreversible effect. Cancel and the destructive action are refs of System / Button; the destructive action names its result.', [], states('l'), states('d'));
}

function buildSheetPart(tokens) {
  const W = num(tokens, '--size-panel-ideal');
  function panel(id, side) {
    return frame(id, side, {
      width: side === 'Right' || side === 'Left' ? 260 : 320, height: side === 'Right' || side === 'Left' ? 180 : 120,
      fill: '$--popover', stroke: '$--border', strokeWidth: '$--size-hairline', strokeAlignment: 'inner', layout: 'vertical', gap: '$--spacing-xs', padding: '$--spacing-md',
    }, [text(`${id}-title`, 'Device details', {fill: '$--foreground', size: '$--text-title', weight: '600'}), text(`${id}-desc`, `Slides in from the ${side.toLowerCase()} edge, --size-panel-ideal wide when vertical.`, {fill: '$--subtle-foreground', width: 220})]);
  }
  function states(suffix) { return [cell('Right', panel(`sheet-right-${suffix}`, 'Right')), cell('Left', panel(`sheet-left-${suffix}`, 'Left'))]; }
  return buildSheet('sys-sheet', 'System / Sheet', `No Sheet master exists in pencil:shadcn; drawn from primitives in Dialog\u2019s surface idiom: a panel on one window edge, --size-panel-ideal (${W}px) wide on the left or right, an overlay at --opacity-dimmed behind it.`, [], states('l'), states('d'));
}

function buildPopover(tokens) {
  const W = num(tokens, '--size-pr-popover');
  function states(suffix) {
    const content = frame(`pop-open-${suffix}`, 'Open', {
      width: W, cornerRadius: '$--radius-md', fill: '$--popover', stroke: '$--border', strokeWidth: '$--size-hairline',
      strokeAlignment: 'inner', layout: 'vertical', gap: '$--spacing-xs', padding: '$--spacing-sm',
    }, [text(`pop-t-${suffix}`, 'Open a pull request', {fill: '$--foreground', weight: '500'}), text(`pop-d-${suffix}`, '#118 into main · CI green', {fill: '$--subtle-foreground', size: '$--text-caption'})]);
    return [cell('Open', content)];
  }
  return buildSheet('sys-popover', 'System / Popover', `No separate Popover master exists in pencil:shadcn; its content reuses the same --popover/--border/rounded surface idiom as Dropdown Menu's content (cTN8T), at --size-pr-popover (${W}px) wide.`, [], states('l'), states('d'));
}

function buildTooltip(tokens) {
  // Shaped from shadcn's Tooltip (lxrnE): rounded-sm (its own 6px, an exact hide
  // token match), --popover fill, a hairline --border, no drop shadow, its
  // horizontal padding at --spacing-md (its own 12px, also an exact match). hide's
  // Hint balloon keeps text-caption rather than shadcn's text-title-sized label,
  // since a hint is the smallest form of a state (design principle 9).
  function balloon(id, withShortcut) {
    return frame(id, withShortcut ? 'With Shortcut' : 'Open', {
      reusable: true, cornerRadius: '$--radius-sm', fill: '$--popover', stroke: '$--border', strokeWidth: '$--size-hairline',
      strokeAlignment: 'inner', layout: 'horizontal', alignItems: 'center', gap: '$--spacing-sm', padding: ['$--spacing-xs', '$--spacing-md'],
    }, [text(`${id}-t`, 'Split terminal', {fill: '$--foreground', size: '$--text-caption', weight: '500'}), ...(withShortcut ? [text(`${id}-k`, '⌘D', {fill: '$--muted-foreground', size: '$--text-caption', mono: true})] : [])]);
  }
  function states(suffix) {
    if (suffix === 'l') return [cell('Open', balloon('tip-open-l', false)), cell('With Shortcut', balloon('tip-shortcut-l', true))];
    const darkCell = (id, state, instance) => frame(`${id}-cell`, state,
      {layout: 'vertical', gap: '$--spacing-xs', alignItems: 'start', width: 'fit_content'},
      [text(`${id}-cap`, state, {size: '$--text-micro', fill: '$--muted-foreground', weight: '600'}), instance]);
    return [darkCell('tip-open-d', 'Open', ref('Sotn7', 'tip-open-l', 'Open',
      {fill: '$--popover', stroke: '$--border'}, {'tip-open-l-t': {fill: '$--foreground'}})),
      darkCell('tip-shortcut-d', 'With Shortcut', ref('haCFg', 'tip-shortcut-l', 'With Shortcut',
        {fill: '$--popover', stroke: '$--border'}, {
          'tip-shortcut-l-t': {fill: '$--foreground'}, 'tip-shortcut-l-k': {fill: '$--muted-foreground'},
        }))];
  }
  return buildSheet('sys-tooltip', 'System / Tooltip', 'Shaped from shadcn’s Tooltip (lxrnE): rounded-sm, --popover fill, hairline --border, --spacing-md horizontal padding. Its label is the trigger’s accessible name.', [], states('l'), states('d'));
}

function buildCommand(tokens) {
  const W = num(tokens, '--size-search-sheet-w'), IH = num(tokens, '--size-control-lg');
  function inputRow(id) {
    return frame(id, 'Field', {height: IH, layout: 'horizontal', alignItems: 'center', gap: '$--spacing-sm', padding: [0, '$--spacing-md'], stroke: '$--border', strokeWidth: {bottom: '$--size-hairline'}, strokeAlignment: 'inner'}, [
      icon(`${id}-i`, 'search', {size: 14, fill: '$--muted-foreground'}), text(`${id}-t`, 'Search commands and files…', {fill: '$--muted-foreground'}),
    ]);
  }
  function surface(id, children) {
    return frame(id, 'Surface', {width: W, cornerRadius: '$--radius-lg', fill: '$--popover', stroke: '$--border', strokeWidth: '$--size-hairline', strokeAlignment: 'inner', layout: 'vertical'}, children);
  }
  function states(suffix) {
    const withList = surface(`cmd-default-${suffix}`, [inputRow(`cmd-in-${suffix}`), frame(`cmd-list-${suffix}`, 'List', {layout: 'vertical', gap: 0, padding: '$--spacing-xxs'}, [
      menuItem(`cmd-i0-${suffix}`, 'Open Settings', 'default', 'settings'), menuItem(`cmd-i1-${suffix}`, 'New Worktree', 'highlighted', 'git-branch-plus'), menuItem(`cmd-i2-${suffix}`, 'Focus Terminal', 'default', 'square-terminal'),
    ])]);
    const filtered = surface(`cmd-filtered-${suffix}`, [inputRow(`cmd-inf-${suffix}`), frame(`cmd-listf-${suffix}`, 'List', {layout: 'vertical', gap: 0, padding: '$--spacing-xxs'}, [menuItem(`cmd-fi0-${suffix}`, 'New Worktree', 'highlighted', 'git-branch-plus')])]);
    const empty = surface(`cmd-empty-${suffix}`, [inputRow(`cmd-ine-${suffix}`), frame(`cmd-emptyrow-${suffix}`, 'Empty', {padding: '$--spacing-md', layout: 'horizontal', justifyContent: 'center'}, [text(`cmd-emptyt-${suffix}`, 'No matches', {fill: '$--muted-foreground', size: '$--text-caption'})])]);
    return [cell('Default', withList), cell('Filtered', filtered), cell('Empty', empty)];
  }
  return buildSheet('sys-command', 'System / Command', `Command over cmdk has no single shadcn master; its input row is shaped from List Search Box/Default (O0rdg: a leading search icon, muted placeholder, no clear icon) and each row from List Item, the same idiom Dropdown Menu uses. --size-search-sheet-w (${W}px) wide, a --size-control-lg field.`, [], states('l'), states('d'));
}

function buildSonner(tokens) {
  function toast(id, name, description, action) {
    return frame(id, name, {
      width: 300, cornerRadius: '$--radius-md', fill: '$--popover', stroke: '$--border', strokeWidth: '$--size-hairline',
      strokeAlignment: 'inner', layout: 'vertical', gap: '$--spacing-xxs', padding: '$--spacing-md',
    }, [
      text(`${id}-t`, 'Copied to clipboard', {fill: '$--foreground'}),
      ...(description ? [text(`${id}-d`, description, {fill: '$--subtle-foreground', size: '$--text-caption', width: 260})] : []),
      ...(action ? [frame(`${id}-a`, 'Action', {layout: 'horizontal', justifyContent: 'end'}, [text(`${id}-at`, 'Undo', {fill: '$--primary', size: '$--text-caption', weight: '500'})])] : []),
    ]);
  }
  function states(suffix) {
    return [
      cell('Default', toast(`son-default-${suffix}`, 'Default')),
      cell('With Description', toast(`son-desc-${suffix}`, 'With Description', 'origin/main → pasteboard')),
      cell('With Action', toast(`son-action-${suffix}`, 'With Action', null, true)),
    ];
  }
  return buildSheet('sys-sonner', 'System / Sonner', 'No Toast/Sonner master exists in pencil:shadcn; drawn from primitives in the popover-surface idiom: a transient confirmation, never the only place a state the operator must act on lives (design 13).', [], states('l'), states('d'));
}

// -- Component / Panel Tab, Component / Keycap (the two folded legacy masters that
// are hide composites, not shadcn parts, so they no longer sit inside a System
// sheet) -------------------------------------------------------------------------

function buildLegacyComponentSheet(id, name, spec, master) {
  return frame(id, name, {
    layout: 'vertical', gap: '$--spacing-lg', padding: '$--spacing-xl', fill: '$--card',
    cornerRadius: '$--radius-lg', width: 'fit_content',
  }, [
    text(`${id}-title`, name.replace('Component / ', ''), {size: '$--text-headline', weight: '600'}),
    text(`${id}-spec`, spec, {size: '$--text-caption', fill: '$--subtle-foreground', width: 720}),
    masterCard(`${id}-master-card`, 'Master', master),
  ]);
}

export function componentFoldSheets(legacy) {
  return [
    {
      name: 'Component / Panel Tab',
      build: () => buildLegacyComponentSheet('cmp-panel-tab', 'Component / Panel Tab',
        'The pane-header underline tab: a different idiom from shadcn Tabs (System / Tabs), so it keeps its own sheet rather than folding into that part. Folded from the former System / Panel Tab master, unchanged beyond the variable rename gen-pen already applied - existing Component refs to it still resolve.',
        legacy.panelTab.master),
    },
    {
      name: 'Component / Keycap',
      build: () => buildLegacyComponentSheet('cmp-keycap', 'Component / Keycap',
        'The divider-bordered mono keycap: a different idiom from shadcn Kbd (System / Kbd), so it keeps its own sheet rather than folding into that part. Folded from the former System / Keycap master, unchanged beyond the variable rename gen-pen already applied - existing Component refs to it still resolve.',
        legacy.keycap.master),
    },
  ];
}

// -- Component / Agent graph box ------------------------------------------------
// The Agents tab's box (PRD agents-graph-view D-34, web/src/GraphView.tsx BoxView):
// a checkout's head and one row per agent. One master, with three rows that a
// state turns on, and every state a ref of it with descendant overrides, so the
// head, the chips and the row parts keep their identity. The sizes are the
// `--graph-*` tokens; width and height are numbers read at generation time.
// The lines between boxes are not part of the master: a delegation is drawn by
// the screen that lays boxes out (Screen / Project Overview).

const BOX_ROWS = [1, 2, 3];

function disabled(node) {
  return {...node, enabled: false};
}

function buildAgentGraphBox(tokens) {
  const W = num(tokens, '--graph-box-width'), HEAD = num(tokens, '--graph-head-height'), LINE = num(tokens, '--graph-head-line');
  const ROW = num(tokens, '--graph-row-height'), ROW_LINE = num(tokens, '--graph-row-line'), PAD_BOTTOM = num(tokens, '--graph-box-pad-bottom');
  const DOT = num(tokens, '--size-status-mark'), MARK = num(tokens, '--size-agent-mark'), ICON = num(tokens, '--size-icon-sm');
  const GLYPH = num(tokens, '--size-checkout-icon'), INDENT = num(tokens, '--size-lineage-indent');
  const SM = num(tokens, '--spacing-sm');
  const DIMMED = num(tokens, '--opacity-secondary');
  const HAIR = '$--size-hairline';
  const caption = (id, content, fill, mono = false) => text(id, content, {size: '$--text-caption', fill, mono});
  const grow = id => frame(id, 'Spacer', {width: 'fill_container', height: 1}, []);

  const head = frame('agb-head', 'Head', {layout: 'vertical', justifyContent: 'center', width: 'fill_container', height: HEAD, padding: [0, '$--spacing-sm']}, [
    frame('agb-l1', 'Branch', {layout: 'horizontal', gap: '$--spacing-xs', alignItems: 'center', width: 'fill_container', height: LINE}, [
      icon('agb-glyph', 'git-branch', {size: GLYPH, fill: '$--muted-foreground'}),
      text('agb-branch', 'branch-name', {mono: true}),
    ]),
    frame('agb-l2', 'Purpose', {layout: 'horizontal', alignItems: 'center', width: 'fill_container', height: LINE}, [caption('agb-purpose', 'purpose', '$--muted-foreground')]),
    frame('agb-l3', 'Chips', {layout: 'horizontal', gap: '$--spacing-sm', alignItems: 'center', width: 'fill_container', height: LINE}, [
      frame('agb-issue', 'Issue chip', {layout: 'horizontal', gap: '$--spacing-xxs', alignItems: 'center'}, [
        icon('agb-issue-g', 'circle-dot', {size: ICON, fill: '$--muted-foreground'}), caption('agb-issue-t', '#192', '$--muted-foreground', true),
      ]),
      // The PR mark (web/src/components/pr-mark.tsx): no border, the icon in the state's colour, the number in the subtle ink;
      // a change request keeps its word, red as failed checks are.
      frame('agb-pr', 'PR mark', {layout: 'horizontal', gap: '$--spacing-xs', alignItems: 'center', padding: [0, '$--spacing-xxs'], height: LINE, cornerRadius: '$--radius-xs'}, [
        frame('agb-pr-mark', 'Mark', {layout: 'horizontal', gap: '$--spacing-xxs', alignItems: 'center'}, [
          icon('agb-pr-g', 'git-pull-request', {size: ICON, fill: '$--pr-mergeable'}), caption('agb-pr-n', '221', '$--subtle-foreground', true),
        ]),
        disabled(caption('agb-pr-rv', '변경 요청', '$--pr-failed')),
      ]),
      caption('agb-dist', '↑3', '$--muted-foreground', true),
      caption('agb-files', '4 files', '$--warning', true),
      disabled(caption('agb-agents', '에이전트 1', '$--muted-foreground', true)),
      grow('agb-sp'),
      disabled(caption('agb-clean', '정리', '$--subtle-foreground')),
    ]),
  ]);

  // A row: the status mark, title and age on one line, and no provider logo, the default hidden agent logo view mode (PRD agent-mark-quiet D-01; PRD agent-logo-view-mode draws the shown mode in code only); a step in with a
  // corner arrow for a delegation inside the checkout; the chip of a
  // delegation into or from another project and the tucked badge before the
  // age; and the question as a second line, only while the agent asks.
  function rowMaster(n) {
    const p = `agb-row${n}`;
    return frame(p, `Row ${n}`, {layout: 'vertical', gap: 0, width: 'fill_container'}, [
      frame(`${p}-l1`, 'Line', {layout: 'horizontal', gap: '$--spacing-xs', alignItems: 'center', width: 'fill_container', height: ROW, padding: [0, '$--spacing-sm']}, [
        frame(`${p}-ind`, 'Indent', {width: INDENT, height: 1, enabled: false}, []),
        icon(`${p}-arrow`, 'corner-down-right', {size: ICON, fill: '$--muted-foreground', enabled: false}),
        frame(`${p}-mark`, 'Status mark', {width: MARK, height: MARK, layout: 'horizontal', justifyContent: 'center', alignItems: 'center'}, markShapes(p)),
        text(`${p}-title`, 'agent title'),
        grow(`${p}-sp`),
        frame(`${p}-cross`, 'Cross-project chip', {
          layout: 'horizontal', gap: '$--spacing-xxs', alignItems: 'center', padding: [0, '$--spacing-xs'], height: ROW_LINE, cornerRadius: '$--radius-xs',
          stroke: '$--border', strokeWidth: HAIR, strokeAlignment: 'inner', enabled: false,
        }, [
          icon(`${p}-cross-g`, 'arrow-right', {size: ICON, fill: '$--muted-foreground'}), caption(`${p}-cross-p`, 'project', '$--muted-foreground'), caption(`${p}-cross-n`, '2', '$--muted-foreground', true),
        ]),
        frame(`${p}-tucked`, 'Tucked badge', {layout: 'horizontal', gap: '$--spacing-xs', alignItems: 'center', padding: [0, '$--spacing-xs'], height: ROW_LINE, cornerRadius: '$--radius-sm', fill: '$--secondary', enabled: false}, [
          markSlot(`${p}-tk-g`, 'done'), caption(`${p}-tk-n`, '2', '$--foreground', true),
        ]),
        caption(`${p}-age`, '3m', '$--muted-foreground', true),
      ]),
      frame(`${p}-q`, 'Question', {layout: 'horizontal', alignItems: 'center', width: 'fill_container', height: ROW_LINE, padding: [0, '$--spacing-sm', 0, SM + MARK + num(tokens, '--spacing-xs')], enabled: false}, [
        caption(`${p}-q-t`, 'question', '$--warning'),
      ]),
    ]);
  }

  const master = frame('agb-m', 'Agent graph box', {
    reusable: true, layout: 'vertical', gap: 0, width: W, cornerRadius: '$--radius-md', fill: '$--card',
    stroke: '$--border', strokeWidth: HAIR, strokeAlignment: 'inner', padding: [0, 0, PAD_BOTTOM, 0],
  }, [head, rowMaster(1), ...BOX_ROWS.slice(1).map(n => ({...rowMaster(n), enabled: false}))]);

  const row = (n, {mark, title, age, line, depth = 0, tucked, cross, asking = false}) => {
    const p = `agb-row${n}`;
    return {
      ...markSwitch(p, mark),
      [`${p}-title`]: {content: title, ...(asking ? {fontWeight: '600'} : {})},
      [`${p}-age`]: {content: age},
      [`${p}-ind`]: {enabled: depth > 0},
      [`${p}-arrow`]: {enabled: depth > 0},
      [`${p}-tucked`]: tucked ? {enabled: true} : {enabled: false},
      [`${p}-tk-n`]: {content: String(tucked ?? 0)},
      [`${p}-cross`]: {enabled: Boolean(cross)},
      [`${p}-cross-g`]: {icon: cross?.direction === 'in' ? 'arrow-left' : 'arrow-right'},
      [`${p}-cross-p`]: {content: cross?.project ?? ''},
      [`${p}-cross-n`]: {enabled: (cross?.count ?? 0) > 1, content: String(cross?.count ?? 0)},
      [`${p}-q`]: line ? {enabled: true, padding: [0, '$--spacing-sm', 0, SM + MARK + depth * INDENT]} : {enabled: false},
      [`${p}-q-t`]: {content: line ?? '', ...(mark === 'turn' ? {} : {fill: '$--subtle-foreground'})},
    };
  };
  const rowsOn = (...specs) => Object.assign({}, ...BOX_ROWS.map((n, index) => specs[index] ? {[`agb-row${n}`]: {enabled: true}, ...row(n, specs[index])} : {[`agb-row${n}`]: {enabled: false}}));
  // Looked up when a state is drawn: pen-agent-tree.mjs imports this module back.
  const prGlyph = state => {
    const [, glyph, fill] = PR_STATES.find(([key]) => key === state) ?? [];
    if (!glyph) throw new Error(`unknown PR state ${state}`);
    return {icon: glyph, fill};
  };
  const headOf = ({glyph, tone, branch, purpose, issue, pr, distance, files, agents, cleanup}) => ({
    'agb-glyph': {icon: glyph, fill: tone},
    'agb-branch': {content: branch},
    'agb-purpose': {content: purpose},
    'agb-issue': {enabled: Boolean(issue)},
    'agb-issue-t': {content: issue ?? ''},
    'agb-pr': {enabled: Boolean(pr)},
    ...(pr ? {
      'agb-pr-g': prGlyph(pr.state),
      'agb-pr-n': {content: `${pr.number}`},
      'agb-pr-rv': {enabled: Boolean(pr.changes)},
    } : {}),
    'agb-dist': {enabled: Boolean(distance), content: distance ?? ''},
    'agb-files': {enabled: Boolean(files), content: `${files ?? 0} files`},
    'agb-agents': {enabled: Boolean(agents), content: `에이전트 ${agents ?? 0}`},
    'agb-clean': {enabled: Boolean(cleanup)},
    'agb-head': cleanup ? {opacity: DIMMED} : {opacity: 1},
  });

  function states(suffix) {
    const state = (key, name, overrides, descendants) => cell(name, ref(`agb-${key}-${suffix}`, master.id, name, overrides, descendants));
    return [
      state('primary', 'Primary', {}, {
        ...headOf({glyph: 'house', tone: '$--muted-foreground', branch: 'main', purpose: 'Observer · 계획과 위임', agents: 2}),
        ...rowsOn({mark: 'quiet', title: 'SIGTERM 정리 오케스트레이션', age: '20m'}, {mark: 'going', title: 'Overview 진입 흐름', age: '1m'}),
      }),
      state('asking', 'Worktree with asking row', {}, {
        ...headOf({glyph: 'git-pull-request', tone: '$--pr-failed', branch: '192-hided-sigterm-handler', purpose: '#192 SIGTERM 정리', issue: '#192', pr: {number: 221, state: 'failed', changes: true}, distance: '↑3', files: 4}),
        ...rowsOn(
          {mark: 'turn', title: 'SIGTERM 처리와 자식 정리', age: '4m', line: '기존 stdin 종료 경로도 남길까요?', asking: true},
          {mark: 'going', title: '리뷰: 종료 경로 회귀', age: '2m', depth: 1},
        ),
      }),
      state('blocked', 'Worktree with a blocked and a stopped row', {}, {
        ...headOf({glyph: 'git-branch', tone: '$--muted-foreground', branch: 'fix/ci-recovery', purpose: 'CI 복구', distance: '↑2'}),
        ...rowsOn(
          {mark: 'turn', title: 'CI 복구 시도', age: '2m', line: '디스크 여유가 부족해 검증을 못 함', asking: true},
          {mark: 'unread', title: '어댑터 정리', age: '14m', line: '일 남음 · 테스트 3개 남음'},
        ),
      }),
      state('resting', 'Resting (merged, dimmed)', {opacity: DIMMED}, {
        ...headOf({glyph: 'git-merge', tone: '$--pr-merged', branch: 'fix/checkout-capability-follow-up', purpose: '체크아웃 권한 후속', pr: {number: 216, state: 'merged'}, cleanup: true}),
        ...rowsOn({mark: 'quiet', title: '체크아웃 기능 구현 및 정리', age: '2h'}),
      }),
      state('tucked', 'Row with tucked badge', {}, {
        ...headOf({glyph: 'house', tone: '$--muted-foreground', branch: 'main', purpose: 'Observer · 계획과 위임', agents: 1}),
        ...rowsOn({mark: 'going', title: 'agent-tab-groups', age: '8m', tucked: 2}),
      }),
      // A delegation into another project (issue 718): `→ sasu 2` on the parent's row, `← hide` on a child's.
      state('cross', 'Rows with cross-project chips', {}, {
        ...headOf({glyph: 'house', tone: '$--muted-foreground', branch: 'main', purpose: 'Observer · 계획과 위임', agents: 2}),
        ...rowsOn(
          {mark: 'going', title: 'Lead', age: '1m', cross: {direction: 'out', project: 'sasu', count: 2}},
          {mark: 'quiet', title: '명세 정리', age: '3m', cross: {direction: 'in', project: 'hide', count: 1}},
        ),
      }),
      state('selected', 'Selected', {stroke: '$--primary'}, {
        ...headOf({glyph: 'git-branch', tone: '$--muted-foreground', branch: 'prd/agent-tab-groups', purpose: 'Agent tab groups', distance: '↑39 ↓17'}),
        ...rowsOn({mark: 'quiet', title: 'Agent tab groups 구현', age: '8m'}),
      }),
    ];
  }

  const id = 'cmp-agent-graph-box';
  return frame(id, 'Component / Agent graph box', {
    layout: 'vertical', gap: '$--spacing-xl', padding: '$--spacing-xl', fill: '$--card', cornerRadius: '$--radius-lg', width: 'fit_content',
  }, [
    text(`${id}-title`, 'Agent graph box', {size: '$--text-headline', weight: '600'}),
    text(`${id}-spec`, 'A checkout in the Agents graph (web/src/GraphView.tsx): a head (the kind glyph, a pull request’s mark in its state, and the mono branch, the purpose, then the issue chip, the PR mark (state icon and number, no border) with 변경 요청 in red only while a reviewer asks for changes, ↑N ↓N and the changed files; main the house and 에이전트 N; a merged box dimmed with 정리) and a row per agent (status mark, title, age, and no provider logo; a step in for a delegation inside the checkout; the chip of a delegation into or from another project; the tucked badge of folded children; the question as a second line in warning only while the agent asks). Width, head, row and padding are the --graph-* tokens. The master carries three rows that a state turns on; the lines between boxes belong to the screen that lays boxes out.', {size: '$--text-caption', fill: '$--subtle-foreground', width: 820}),
    masterCard(`${id}-master-card`, 'Master', master),
    themeFrame(`${id}-light`, 'Light', states('l')),
    themeFrame(`${id}-dark`, 'Dark', states('d')),
  ]);
}

/** The generated `Component /` sheets that are neither folded legacy masters nor hand-authored; `at` is where a new one lands until a person moves it. */
// Approved context-menu B: direct frequent actions, grouped secondary choices.
function buildActionMenus(tokens) {
  const width = num(tokens, '--size-relationship-list-max');
  const states = (suffix) => {
    const menu = (key, title, rows) => cell(title, menuContent(`action-${key}-${suffix}`, width, [
      ...(['directions', 'fork', 'copy'].includes(key) ? [] : [frame(`action-${key}-heading-${suffix}`, 'Target', {padding: ['$--spacing-xs', '$--spacing-sm']}, [text(`action-${key}-title-${suffix}`, '입력과 세션 복귀 흐름 검토', {size: '$--text-caption', fill: '$--muted-foreground'})]), menuSeparator(`action-${key}-head-${suffix}`)]),
      ...rows.map((row, index) => row === null ? menuSeparator(`action-${key}-${index}-${suffix}`) : menuItemReason(`action-${key}-${index}-${suffix}`, row[0], {glyph: row[1], ...row[2]})),
    ]));
    return [
      menu('terminal', 'Terminal', [
        ['복사', 'copy', {shortcut: '⌘C'}], ['붙여넣기', 'clipboard-paste', {shortcut: '⌘V'}], ['모두 선택', 'square-dashed'], ['찾기', 'search', {shortcut: '⌘F'}], null,
        ['오른쪽으로 분할', 'panel-right', {shortcut: '⌘D'}], ['아래로 분할', 'panel-bottom', {shortcut: '⇧⌘D'}], ['확대', 'maximize'], null,
        ['관련 세션', 'network', {shortcut: '›'}], null, ['잠자기', 'moon'], ['세션 보관', 'archive'], ['세션 분기', 'git-fork', {shortcut: '›'}], null,
        ['정보 복사', 'copy', {shortcut: '›'}], null, ['터미널 닫기', 'x'],
      ]),
      menu('tab', 'Agent tab', [['새 탭', 'plus'], ['이름 변경…', 'pencil'], null, ['분할', 'panel-right', {shortcut: '›'}], ['이동', 'arrow-right', {shortcut: '›'}], null, ['이름 복사', 'copy'], null, ['탭 닫기…', 'x']]),
      menu('view', 'File tab', [['새 탭', 'plus'], ['열어두기', 'pin'], null, ['분할', 'panel-right', {shortcut: '›'}], ['이동', 'arrow-right', {shortcut: '›'}], null, ['경로 복사', 'copy'], ['파일 트리에서 선택', 'folder-open'], ['Finder에서 보기', 'external-link'], null, ['보기 닫기', 'x']]),
      menu('session', 'Session row', [['보기', 'square-terminal'], null, ['정보 복사', 'copy', {shortcut: '›'}], null, ['탭 닫기…', 'x']]),
      menu('task', 'Live task', [['기록 보기', 'file-text', {shortcut: 'Space'}], ['터미널로 이동', 'square-terminal', {shortcut: '↩'}], ['Resolve', 'check'], null, ['세션 보관', 'archive']]),
      menu('archived', 'Archived task', [['기록 보기', 'file-text', {shortcut: 'Space'}], ['이어가기', 'play'], null, ['보관함에서 지우기…', 'trash-2', {destructive: true}]]),
      menu('unavailable', 'Shared refusal', [['세션 분기', 'git-fork', {disabled: true, reason: '세션 정보를 아직 받지 못했습니다', shortcut: '›'}]]),
      menu('directions', 'Direction submenu', [['오른쪽', 'panel-right'], ['왼쪽', 'panel-left'], ['위쪽', 'panel-top'], ['아래쪽', 'panel-bottom']]),
      menu('fork', 'Fork submenu', [['새 탭에서 분기', 'git-fork'], ['오른쪽에서 분기', 'panel-right']]),
      menu('copy', 'Copy submenu', [['세션 이름', 'copy'], ['세션 ID', 'hash', {disabled: true, reason: '세션 정보를 아직 받지 못했습니다'}], ['터미널 ID', 'hash']]),
    ];
  };
  return buildSheet('cmp-action-menus', 'Component / Action menus', 'Context menu B approved in HTML: consistent icons, one target heading, direct frequent actions, submenus for directions, related sessions, fork destinations and copied details. Close and archive remain neutral; deletion alone is destructive. A shared unavailable reason appears once on its group.', [], states('l'), states('d'));
}

export function componentSheets(tokens) {
  return [
    {name: 'Component / Action menus', build: () => buildActionMenus(tokens), at: {x: 16000, y: 8281}},
    {name: 'Component / Agent graph box', build: () => buildAgentGraphBox(tokens), at: {x: 1200, y: 8281}},
    {name: 'Component / Agent tree parts', build: () => buildAgentTreeParts(tokens), at: {x: 8800, y: 7264}},
    {name: 'Component / Hide AI origin', build: () => buildHideAiOrigin(tokens), at: {x: 11200, y: 7264}},
    {name: 'Component / Usage windows', build: () => buildUsage(tokens), at: {x: 13200, y: 6421}},
    {name: 'Component / Usage history', build: () => buildUsageHistory(tokens), at: {x: 14600, y: 6421}},
    {name: 'Component / Sidebar chrome', build: () => buildSidebarChrome(tokens), at: {x: 12000, y: 6421}},
  ];
}

// -- assembly ------------------------------------------------------------------

/** Find a node anywhere in the (already variable-renamed) document tree by id. */
export function findNode(document, id) {
  let found;
  function walk(node) {
    if (found || !node) return;
    if (node.id === id) { found = node; return; }
    for (const child of node.children ?? []) walk(child);
  }
  for (const child of document.children ?? []) walk(child);
  if (!found) throw new Error(`pen-system: no node ${id} in the document (a legacy System master was expected to still exist)`);
  return found;
}

/** The four masters System / Icon Button, Panel Tab, Badge and Keycap folded into their new homes. */
export function foldLegacyMasters(document) {
  return {
    iconButton: {master: findNode(document, 'Nyvom')},
    panelTab: {master: findNode(document, 'ywjtY')},
    badge: {master: findNode(document, 'eHAjc'), iconId: 'xXuNa', labelId: 'n8L5dm'},
    keycap: {master: findNode(document, 'Ec2W1')},
  };
}

/** The old top-level sheets folded away entirely; System / Badge is replaced in place by name. */
export const RETIRED_SHEETS = ['System / Icon Button', 'System / Panel Tab', 'System / Keycap'];

export function systemSheets(tokens, legacy) {
  return [
    {name: 'System / Button', build: () => buildButton(tokens, legacy)},
    {name: 'System / Input', build: () => buildInput(tokens)},
    {name: 'System / Select', build: () => buildSelect(tokens)},
    {name: 'System / Checkbox', build: () => buildCheckbox(tokens)},
    {name: 'System / Switch', build: () => buildSwitch(tokens)},
    {name: 'System / Radio Group', build: () => buildRadioGroup(tokens)},
    {name: 'System / Toggle Group', build: () => buildToggleGroup(tokens)},
    {name: 'System / Slider', build: () => buildSlider(tokens)},
    {name: 'System / Tabs', build: () => buildTabs(tokens, legacy)},
    {name: 'System / Badge', build: () => buildBadge(tokens, legacy)},
    {name: 'System / Kbd', build: () => buildKbd(tokens, legacy)},
    {name: 'System / Separator', build: () => buildSeparator(tokens)},
    {name: 'System / Dropdown Menu', build: () => buildDropdownMenu(tokens)},
    {name: 'System / Context Menu', build: () => buildContextMenu(tokens)},
    {name: 'System / Dialog', build: () => buildDialog(tokens)},
    {name: 'System / Alert Dialog', build: () => buildAlertDialog(tokens)},
    {name: 'System / Sheet', build: () => buildSheetPart(tokens)},
    {name: 'System / Popover', build: () => buildPopover(tokens)},
    {name: 'System / Tooltip', build: () => buildTooltip(tokens)},
    {name: 'System / Command', build: () => buildCommand(tokens)},
    {name: 'System / Sonner', build: () => buildSonner(tokens)},
  ];
}
