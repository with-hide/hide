// Draw every `Screen / <Area>` sheet the web shell renders today into
// design/hide-screens.pen (PRD web-design-system-reset B19, D-18, D-19). A Screen
// sheet is a layout proposal, not a part library: it imports design/hide-ui.lib.pen
// as a library (`imports: {hideui: './hide-ui.lib.pen'}`) and draws its content as
// refs of that library's `System /` and `Component /` masters, the same way a
// Component sheet is built from System masters - never a hand-redrawn control.
//
// A confirmed Pen toolchain limit shapes every helper below: a cross-library ref
// resolves its OWN internal `$--token` references using hide-ui.lib.pen's own
// default (Light) value, regardless of any `theme: {Mode: 'Dark'}` tag anywhere in
// the importing document - the Mode axis does not cross an `imports` boundary for a
// master's untouched properties. Two things DO resolve correctly per theme: a bare
// local `$--token` reference on a node this document itself authors (proven with a
// minimal repro), and an explicit property override placed AT the cross-library ref
// site using this document's own local `$--token` (also proven, and the descendant
// override KEY needs the alias prefix too, e.g. `hideui:btn-lb`, not just its ref
// target - undocumented in the Pen CLI's own docs). So this document carries its own
// full copy of the token variables (`readLocalVariables` below, read through the
// same pen-tokens.mjs pipeline gen-pen.mjs uses, so it can never drift from
// tokens.json), and `themedOverrides` restates a ref's colors by walking the ACTUAL
// master node in the live design/hide-ui.lib.pen (via pen-tokens.mjs's loadCanvas)
// and copying every `$--token` name it finds on the master's own top level and on
// every descendant, at any depth, into a local (non-aliased) override of the exact
// same name - never a hand-typed color recipe, so a master's default colors changing
// in the library is a change this generator picks up on its next run, not a second
// place to edit. Button and Badge additionally carry real design decisions per
// variant (which token names "destructive" or "secondary" mean), which is why those
// two import their variant tables from pen-system.mjs (`BUTTON_VARIANTS`,
// `BADGE_VARIANTS`) rather than recomputing them from a single default state.
//
// Nested same-library refs carry their master's defaults and instance patches.
// The generator and checker share this effective color walk, addressed by each
// full instance path, so separate instances never share an override by accident.

import {read as readTokenPlan, loadCanvas, CANVAS} from './pen-tokens.mjs';
import {BUTTON_VARIANTS, BADGE_VARIANTS, frame, icon, legacyMark, markSlot, markSwitch, num, text} from './pen-system.mjs';
import {coreMoveRows} from './pen-screens-core-move.mjs';
import {diskCleanupRows} from './pen-screens-disk.mjs';
import {factoryRows} from './pen-screens-factory.mjs';
import {laneBoards} from './pen-screens-lane.mjs';
import {onboardingBoards} from './pen-screens-onboarding.mjs';
import {settingsBoards, settingsParts} from './pen-screens-settings.mjs';
import {projectSettingsScreens} from './pen-screens-project-settings.mjs';
import {sessionScreens} from './pen-screens-sessions.mjs';
import {overviewScreens} from './pen-screens-overview.mjs';
import {AGENT_TREE_MASTERS, PR_STATES} from './pen-agent-tree.mjs';

const LOCAL_TOKEN = /^\$--[A-Za-z0-9_-]+$/;
const THEMED_PROPS = ['fill', 'stroke'];

let libraryRoot;
let libraryDocumentCache;
let disabledOpacity;

/** Set once by screenSheets(); every themedOverrides() call reads design/hide-ui.lib.pen through this. */
function setLibraryRoot(root, tokens) {
  libraryRoot = root;
  libraryDocumentCache = undefined;
  // A variable-bound opacity renders invisible in this toolchain (the same
  // proven limit AGENTS.md and pen-system.mjs's Button Disabled state already
  // work around for width/height): materialize the number once, here, rather
  // than pass the `$--opacity-disabled` token string at a disabled state's ref site.
  disabledOpacity = num(tokens, '--opacity-disabled');
}

function libraryDocument() {
  if (!libraryRoot) throw new Error('pen-screens: themedOverrides() was called before screenSheets() set the library root');
  if (!libraryDocumentCache) libraryDocumentCache = loadCanvas(libraryRoot).document;
  return libraryDocumentCache;
}

function findMaster(node, id) {
  if (typeof id !== 'string' || !id) return null;
  if (node && typeof node === 'object') {
    if (node.id === id) return node;
    for (const child of node.children ?? []) {
      const found = findMaster(child, id);
      if (found) return found;
    }
  }
  return null;
}

/**
 * Every `$--token` fill/stroke design/hide-ui.lib.pen's own `masterId` node
 * carries, restated as a local (non-aliased) override: `{top, descendants}`,
 * where `top` goes on the ref's own property overrides and `descendants` goes
 * on the ref's `descendants` map (xref() adds the alias prefix). A literal
 * color (not a `$--token` string) is left alone - it is already theme-independent.
 */
export function themedOverrides(masterId, document = libraryDocument()) {
  const master = findMaster({children: document.children}, masterId);
  if (!master) throw new Error(`pen-screens needs master ${masterId}, which ${CANVAS} no longer carries`);
  const top = {};
  const descendants = {};
  const colors = (node, address) => {
    const props = {};
    for (const prop of THEMED_PROPS) if (LOCAL_TOKEN.test(node[prop])) props[prop] = node[prop];
    if (!address.length) Object.assign(top, props);
    else if (Object.keys(props).length) descendants[address.join('/')] = props;
  };
  const patches = (address, scopes) => {
    const props = {};
    for (const {prefix, entries} of scopes) {
      if (!prefix.every((part, i) => address[i] === part)) continue;
      const key = address.slice(prefix.length).join('/');
      if (Object.hasOwn(entries, key)) Object.assign(props, entries[key]);
    }
    return props;
  };
  const walk = (node, prefix, scopes, activeMasters, isRoot = false) => {
    const address = isRoot ? prefix : [...prefix, node.id];
    const effective = {...node, ...patches(address, scopes)};
    if (effective.type === 'ref') {
      let resolved = effective;
      let nestedScopes = scopes;
      const active = new Set(activeMasters);
      while (resolved.type === 'ref') {
        const target = findMaster(document, resolved.ref);
        if (!target) throw new Error(`Component instance ${address.join('/')} has no target ${JSON.stringify(resolved.ref)}`);
        if (active.has(target.id)) throw new Error(`Component instance ${address.join('/')} cycles to active master ${target.id}`);
        active.add(target.id);
        // Inner master patches are defaults; enclosing instances apply last.
        nestedScopes = [{prefix: address, entries: resolved.descendants ?? {}}, ...nestedScopes];
        const {id: _id, type: _type, ref: _ref, children: _children, descendants: _descendants, ...overrides} = resolved;
        resolved = {...target, ...overrides};
      }
      colors(resolved, address);
      for (const child of resolved.children ?? []) walk(child, address, nestedScopes, active);
    } else {
      colors(effective, address);
      for (const child of effective.children ?? []) walk(child, prefix, scopes, activeMasters);
    }
  };
  walk(master, [], [], new Set([masterId]), true);
  return {top, descendants};
}

/** xref(), with every themed color themedOverrides(masterId) finds restated first; `overrides`/`descendants` win on a shared key. */
export function themedXref(id, masterId, name, overrides = {}, descendants = {}) {
  const auto = themedOverrides(masterId);
  const mergedDescendants = {...auto.descendants};
  for (const [key, props] of Object.entries(descendants)) mergedDescendants[key] = {...(mergedDescendants[key] ?? {}), ...props};
  return xref(id, masterId, name, {...auto.top, ...overrides}, mergedDescendants);
}

// design/hide-ui.lib.pen carries a few variables of its own that tokens.json never
// generates (a font family is a library choice, not a color/spacing/radius token);
// this document's text() helper needs them too, so they are read live from the
// library itself rather than duplicated as a literal, keeping the same
// never-drifts guarantee pen-tokens.mjs gives the generated set.
const LIBRARY_AUTHORED_VARIABLES = ['--font-ui', '--font-mono', '--color-selected-wash', '--color-selected', '--diff-added-wash', '--diff-removed-wash'];

export const ALIAS = 'hideui';
export const LIBRARY_PATH = './hide-ui.lib.pen';

// A cross-library ref: the target and every descendant-override key need the
// `hideui:` alias prefix (the key, not just the ref target - confirmed empirically,
// undocumented in DESIGN_WORKFLOW.md). `overrides` are plain top-level property
// overrides on the ref itself (fill, stroke, width...); `descendants` is a plain
// {rawId: props} map this function prefixes for you.
function xref(id, masterId, name, overrides = {}, descendants) {
  const prefixed = descendants ? Object.fromEntries(Object.entries(descendants).map(([key, value]) => [key.split('/').map(part => `${ALIAS}:${part}`).join('/'), value])) : undefined;
  return {id, type: 'ref', ref: `${ALIAS}:${masterId}`, name, ...overrides, ...(prefixed ? {descendants: prefixed} : {})};
}

function themeFrame(id, name, mode, props, children) {
  return frame(id, name, {theme: {Mode: mode}, ...props}, children);
}

function screenSheet(id, name, spec, lightBuild, darkBuild) {
  return frame(id, name, {layout: 'vertical', gap: '$--spacing-xl', padding: '$--spacing-xl', fill: '#EDEDEE', width: 'fit_content'}, [
    text(`${id}-title`, name.replace('Screen / ', ''), {size: '$--text-headline', weight: '600'}),
    text(`${id}-spec`, spec, {size: '$--text-caption', fill: '$--muted-foreground', width: 960}),
    themeFrame(`${id}-light`, 'Light', 'Light', {layout: 'horizontal', gap: '$--spacing-lg', alignItems: 'start', padding: '$--spacing-lg', fill: '$--background', cornerRadius: '$--radius-md', width: 'fit_content'}, lightBuild('l')),
    themeFrame(`${id}-dark`, 'Dark', 'Dark', {layout: 'horizontal', gap: '$--spacing-lg', alignItems: 'start', padding: '$--spacing-lg', fill: '$--background', cornerRadius: '$--radius-md', width: 'fit_content'}, darkBuild('d')),
  ]);
}

function sessionDesign(tokens) {
  return sessionScreens(tokens, {frame, text, icon, themedXref, screenButton});
}

// -- simple, frequently-reused controls: themedXref restates every default color --

function screenButton(id, label, {variant = 'default', height, width, icon: glyph} = {}) {
  const v = BUTTON_VARIANTS[variant];
  return themedXref(id, 'btn-m', label, {...v.overrides, ...(height ? {height} : {}), ...(width ? {width} : {})}, {
    'btn-ic': glyph ? {icon: glyph, fill: v.fg, enabled: true} : {enabled: false},
    'btn-lb': {content: label, fill: v.fg},
  });
}

function screenIconButton(id, glyph, {size} = {}) {
  return themedXref(id, 'Nyvom', 'Icon', size ? {width: size, height: size} : {}, {ZIZFR: {icon: glyph, fill: '$--muted-foreground'}});
}

function screenInput(id, {content, placeholder, mono, width} = {}) {
  return themedXref(id, 'inp-m', 'Input', width ? {width} : {}, {
    'inp-t': content !== undefined ? {content, fill: '$--foreground', ...(mono ? {fontFamily: '$--font-mono'} : {})} : {content: placeholder, fill: '$--muted-foreground'},
  });
}

function screenSelect(id, {content, placeholder, width} = {}) {
  return themedXref(id, 'sel-m', 'Select', width ? {width} : {}, {
    'sel-t': content !== undefined ? {content, fill: '$--foreground'} : {content: placeholder, fill: '$--muted-foreground'},
  });
}

// A horizontal tab strip: `items` is an array of labels, `activeIndex` the selected one.
function tabRefs(id, items, activeIndex) {
  return items.map((label, index) => themedXref(`${id}-${index}`, 'tab-m', label,
    index === activeIndex ? {fill: '$--secondary'} : {},
    {'tab-t': {content: label, fill: index === activeIndex ? '$--foreground' : '$--subtle-foreground'}}));
}

function screenTabs(id, items, activeIndex) {
  return frame(id, 'Tabs', {layout: 'horizontal', gap: '$--spacing-xxs', padding: '$--spacing-xxs', fill: '$--card', cornerRadius: '$--radius-sm'}, tabRefs(id, items, activeIndex));
}

// A radio-styled swatch: System / Radio Group's master with its indicator dot
// disabled, since web's accent swatch is the whole circle in the accent color
// with a ring rather than a filled dot ([&_svg]:hidden on RadioGroupItem).
function screenSwatch(id, colorToken, selected) {
  return themedXref(id, 'rad-m', colorToken, {fill: colorToken, stroke: selected ? '$--foreground' : '$--border', strokeWidth: selected ? 2 : '$--size-hairline'}, {'rad-dot': {enabled: false}});
}

// System / Radio Group's own idiom (a dot inside the circle when selected),
// for a plain radio choice rather than an accent swatch.
function screenRadioItem(id, label, selected) {
  return frame(id, 'Radio', {layout: 'horizontal', gap: '$--spacing-xs', alignItems: 'center'}, [
    themedXref(`${id}-dot`, 'rad-m', label, {}, {'rad-dot': {enabled: selected}}),
    text(`${id}-label`, label, {weight: '500'}),
  ]);
}

const MENU_ITEM_STATE = {default: {}, highlighted: {fill: '$--accent', text: '$--accent-foreground'}, disabled: {text: '$--muted-foreground'}, destructive: {text: '$--destructive'}};

function screenMenuItem(id, label, {state = 'default', glyph, reason, reasonWidth = 200, shortcut} = {}) {
  const s = MENU_ITEM_STATE[state];
  return themedXref(id, 'mnu-item-m', label, {...(s.fill ? {fill: s.fill} : {}), ...(state === 'disabled' ? {opacity: disabledOpacity} : {})}, {
    'mnu-item-icon': glyph ? {icon: glyph, fill: s.text ?? '$--muted-foreground', enabled: true} : {enabled: false},
    'mnu-item-label': {content: label, fill: s.text ?? '$--foreground'},
    'mnu-item-reason': reason ? {content: reason, enabled: true, textGrowth: 'fixed-width', width: reasonWidth} : {enabled: false},
    'mnu-item-shortcut': shortcut ? {content: shortcut, enabled: true} : {enabled: false},
  });
}

function screenMenuSeparator(id) {
  return themedXref(id, 'mnu-sep-m', 'Separator');
}

function screenMenuContent(id, width, children) {
  return frame(id, 'Content', {
    layout: 'vertical', gap: 0, padding: '$--spacing-xxs', width, cornerRadius: '$--radius-sm',
    fill: '$--popover', stroke: '$--border', strokeWidth: '$--size-hairline', strokeAlignment: 'inner',
  }, children);
}

// A dialog/alert-dialog surface, matching System / Dialog's own composition
// (Card/Modal shaped header + footer actions as Button refs). The description
// is a path in mono unless `prose`; `titleGlyph` leads the title.
function screenDialogSurface(id, {width, title, titleGlyph, description, prose = false, body, actions}) {
  const heading = text(`${id}-title`, title, {fill: '$--foreground', size: '$--text-title', weight: '600'});
  return frame(id, 'Surface', {
    width, cornerRadius: '$--radius-lg', fill: '$--popover', stroke: '$--border', strokeWidth: '$--size-hairline',
    strokeAlignment: 'inner', layout: 'vertical', gap: '$--spacing-md',
  }, [
    frame(`${id}-hdr`, 'Header', {layout: 'vertical', gap: '$--spacing-xs', padding: ['$--spacing-lg', '$--spacing-lg', 0, '$--spacing-lg']}, [
      titleGlyph ? frame(`${id}-titlerow`, 'Title', {layout: 'horizontal', gap: '$--spacing-xs', alignItems: 'center'}, [icon(`${id}-titleg`, titleGlyph, {size: 14, fill: '$--muted-foreground'}), heading]) : heading,
      ...(description ? [text(`${id}-desc`, description, prose ? {fill: '$--subtle-foreground', size: '$--text-body', width: width - 2 * 16} : {fill: '$--subtle-foreground', size: '$--text-caption', mono: true, width: width - 2 * 16})] : []),
    ]),
    ...(body ? [frame(`${id}-body`, 'Body', {layout: 'vertical', gap: '$--spacing-md', padding: [0, '$--spacing-lg']}, body)] : []),
    // DialogFooter ends its actions at the surface's right edge.
    frame(`${id}-ftr`, 'Footer', {layout: 'horizontal', justifyContent: 'end', alignItems: 'center', gap: '$--spacing-sm', width: 'fill_container', padding: [0, '$--spacing-lg', '$--spacing-lg', '$--spacing-lg']}, actions),
  ]);
}

// A DialogBody field: a caption label above its control, the settings-rows.tsx
// idiom this dialog form also uses (a label the control sits directly under).
function screenField(id, label, control) {
  return frame(id, 'Field', {layout: 'vertical', gap: '$--spacing-xxs'}, [
    text(`${id}-label`, label, {size: '$--text-caption', weight: '500', fill: '$--subtle-foreground'}),
    control,
  ]);
}

// -- the status mark --------------------------------------------------------------
// status-mark.tsx draws one of four marks in one 12 px slot, the in-progress ring
// solid or dashed (docs/status-model.md, The mark). A board names a mark (`turn`,
// `going`, `waiting`, `done`, `unread`, `quiet`);
// an older board's glyph and colour still name the mark that replaced it, so a
// sheet drawn before the four marks keeps meaning the same row.
// The overrides that turn on a master's mark (Sidebar agent row, tree row);
// a ring whose line leads with 대기 waits, so it is drawn dashed (D-25).
function markOverrides(prefix, symbol, fill, line) {
  const mark = legacyMark(symbol, fill);
  return markSwitch(prefix, mark === 'going' && line?.startsWith('대기') ? 'waiting' : mark, '', {moon: prefix === 'sar'});
}

// The same mark for a row this document draws itself rather than from a master.
function screenStatusMark(tokens, id, symbol, fill) {
  return markSlot(id, legacyMark(symbol, fill));
}

// -- composite Component masters: themedXref restates every internal default color,
// so only content (and any state that changes what the master would not restate on
// its own, like the status dot's color) needs to be named at each call site. -------

// The master's own title text (IxbXq) is `textGrowth: 'fixed-width', width:
// 'fill_container'`, which wraps across several lines once this ref's own
// width is overridden away from the master's narrower native context (Pen
// has no text-truncation property at all - `fixed-width` always wraps,
// `fixed-width-height` only clips without an ellipsis - so `auto` growth,
// the same no-textGrowth idiom Project row's own title (JkPyX) already uses,
// is the one way to keep a title and a path each on their own single line;
// a real ellipsis is not something this toolchain can draw).
function screenWorkspaceRow(id, {title, role, width = 260}) {
  return themedXref(id, 'pPtY6', title, {width}, {IxbXq: {content: title, textGrowth: 'auto'}, JzcqS: {content: role, textGrowth: 'auto'}});
}

function screenSectionHeader(id, {label, count, detail, width = 260}) {
  return themedXref(id, 'O79KF', label, {width}, {VOd6D: {content: label}, rVsKB: {content: count}, g7fzt: {content: detail}});
}

function screenLineRow(id, {label, meta, status, width = 260}) {
  return themedXref(id, 'jJJqB', label, {width}, {hJDwP: {content: label}, xT4J9: {content: meta}, e44ufH: {content: status}});
}

// sidebar-agent-row.tsx's SidebarAgentRow, from the library's Sidebar agent row:
// line one is the marks, the title, the chips, the badge, the elapsed time and a
// parent's fold; a request or news has its own line and Agents adds the context
// line. `inset` is where the marks start. Every row keeps the fold slot, so its
// time ends on the column every sidebar row's time ends on; `fold` is `folded`
// on a parent whose chevron shows at rest, and `unfolded` or null leaves the
// slot empty at rest. A list with nothing to fold (an Overview card) passes
// `none`, and the row has no fold slot at all.
function screenSidebarAgentRow(id, {title, symbol = '●', color = '$--agent-working', age, line, lineFill = '$--warning', place, device, branch, badge, fold = null, bright = false, selected = false, selectedFill = '$--color-selected', inset = 4, width = 268}) {
  return themedXref(id, 'sidebar-agent-row', title, {width, padding: ['$--spacing-xs', '$--spacing-xs', '$--spacing-xs', inset], ...(selected ? {fill: selectedFill} : {})}, {
    ...markOverrides('sar', symbol, color, line),
    // 12/400 in every state: attention and selection brighten the title, never thicken it.
    'sar-title': {content: title, ...(bright || selected ? {fill: '$--foreground'} : {})},
    'sar-elapsed': {content: age},
    'sar-line': line ? {enabled: true, content: line, fill: lineFill} : {enabled: false},
    'sar-place': place ? {enabled: true, content: place} : {enabled: false},
    'sar-device': device ? {enabled: true} : {enabled: false},
    'sar-device-label': {content: device ?? ''},
    'sar-branch': branch ? {enabled: true} : {enabled: false},
    'sar-branch-label': {content: branch ?? ''},
    'sar-badge': badge ? {enabled: true} : {enabled: false},
    'sar-badge-label': {content: badge ?? ''},
    'sar-fold': fold === 'none' ? {enabled: false} : {opacity: fold === 'folded' ? 1 : 0},
    'sar-chevron': {icon: fold === 'unfolded' ? 'chevron-down' : 'chevron-right'},
  });
}

// agent-tree's tree row (PRD agent-hierarchy-screens D-38), from the library's
// Agent tree parts row master: the rails of each level above, the elbow from
// the parent's chevron lane, the chevron (open, closed, or none), the marks,
// the title, the own PR mark, a folded parent's descendant mark and the age;
// a child on another machine adds its device line. `rails` says, per level
// above the elbow, whether an ancestor's later sibling keeps a rail running.
const PR_TREE = Object.fromEntries(PR_STATES.map(([state, glyph, fill]) => [state, [glyph, fill]]));
function screenTreeRow(id, {title, symbol = '○', color = '$--muted-foreground', age, line, depth = 0, last = false, rail = false, chevron = null, pr, mark, device, offline = false, more = false, bright = false, selected = false, statusAxis = false, inset = 0, width = 268}) {
  const descendants = {
    ...markOverrides('ath-row', symbol, color, line),
    'ath-row-title': {content: title, ...(bright || selected ? {fill: '$--foreground'} : {})},
    'ath-row-age': {content: age},
    'ath-row-line': line ? {enabled: true, content: line, fill: '$--subtle-foreground'} : {enabled: false},
    // A second line makes the row 16 taller, and the rails that pass it with it.
    'ath-row-rail': {enabled: depth > 1, ...(line ? {height: 44} : {})},
    'ath-row-rail-v': {enabled: rail, ...(line ? {height: 44} : {})},
    'ath-row-elbow': {enabled: depth > 0, ...(line ? {height: 44} : {})},
    'ath-row-elbow-v': {height: last ? 13 : line ? 44 : 28},
    'ath-row-chev': chevron ? {enabled: true, icon: chevron === 'open' ? 'chevron-down' : 'chevron-right'} : {enabled: false},
    'ath-row-pr': pr ? {enabled: true} : {enabled: false},
    ...(pr ? {'ath-row-pr/ath-pr-g': {icon: PR_TREE[pr.state][0], fill: PR_TREE[pr.state][1]}, 'ath-row-pr/ath-pr-n': {content: `${pr.number}`}} : {}),
    'ath-row-desc': mark ? {enabled: true} : {enabled: false},
    ...(mark ? {
      'ath-row-desc/ath-mark-n': {content: String(mark.count), fill: '$--warning'},
    } : {}),
    'ath-row-device': device ? {enabled: true} : {enabled: false},
    'ath-row-device-t': {content: device ? `${device}${offline ? ' · 연결 안 됨' : ''}` : ''},
    ...(statusAxis ? {
      'ath-row-lane': {layoutPosition: 'absolute', x: depth === 0 ? width - 20 : inset + 18 * (depth - 1), y: 0, width: 12, enabled: Boolean(chevron)},
      'ath-row-body': {padding: [4, 24, 4, 0]},
    } : {}),
    // "N개 더" is a tree line too: its chevron opens the rest in place, with no mark.
    ...(more ? {'ath-row-mark': {enabled: false}} : {}),
  };
  return themedXref(id, 'ath-row', title, {width, padding: [0, 0, 0, inset], ...(selected ? {fill: '$--secondary'} : {}), ...(offline ? {opacity: 0.5} : {})}, descendants);
}

// view-tab's own master carries a themed top-level fill (the card behind an
// inactive tab); the active tab is a different token the master does not
// default to, so it is the one property named explicitly, every time (never
// only when active), so an inactive tab is never left un-restated either.
function screenViewTab(id, {title, active}) {
  return themedXref(id, 'view-tab', title, {fill: active ? '$--background' : '$--card'}, {'view-tab-title': {content: title}});
}

// The approved R1 remote-device treatment: the existing Badge shape, a server
// glyph, and the real device display name. It is shared by folded lineage
// summaries, the child list, and pane-header child chips.
function screenDeviceChip(id, name) {
  return frame(id, 'Device chip', {layout: 'horizontal', gap: '$--spacing-xxs', padding: [0, '$--spacing-xs'], height: 16, alignItems: 'center', cornerRadius: '$--radius-xs', stroke: '$--border', strokeWidth: '$--size-hairline', strokeAlignment: 'inner'}, [
    icon(`${id}-g`, 'server', {size: 10, fill: '$--muted-foreground'}),
    text(`${id}-t`, name, {size: '$--text-micro', fill: '$--subtle-foreground', weight: '500'}),
  ]);
}

// The weekly usage chip at the sidebar's foot (web/src/components/usage.tsx):
// the master draws Claude Code's mark, so only another provider's mark is named.
function screenUsageChip(id, {provider, value}) {
  const mark = provider === 'claude' ? {} : {'usage-chip-mark': {fill: {type: 'image', enabled: true, url: `../web/src/assets/agent-${provider}.png`, mode: 'fit'}}};
  return themedXref(id, 'usage-chip', `Usage ${provider}`, {}, {...mark, 'usage-chip-percent': {content: value}});
}

// The single sidebar App.tsx/sidebar.tsx always shows beside a
// screen's own content; every sheet that draws a whole screen (Main,
// Workspace) includes it so a reader sees the whole thing, not just its
// own feature in isolation. It is --size-sidebar-ideal wide, as sidebar.tsx
// draws it: the Search row on top, then the Overview row, marked while
// `overview` is the screen beside it (session-first-ui B1-B10), and a footer of
// the usage chips, Help and the Settings gear.
function screenSidebar(tokens, id, suffix, agents, {overview = false} = {}) {
  return frame(`${id}-${suffix}`, 'Sidebar', {width: num(tokens, '--size-sidebar-ideal'), layout: 'vertical', gap: '$--spacing-md', fill: '$--sidebar', padding: '$--spacing-md', cornerRadius: '$--radius-md'}, [
    // The Search row opens the list; there is no Home row (PRD sidebar-titlebar-refresh D-06, D-08).
    themedXref(`${id}-search-${suffix}`, 'sbc-search', 'Search row', {width: 'fill_container', padding: 0}),
    frame(`${id}-overview-${suffix}`, 'Overview', {
      width: 'fill_container', height: num(tokens, '--size-project-row'), layout: 'horizontal', gap: '$--spacing-sm', alignItems: 'center', padding: [0, '$--spacing-sm'], cornerRadius: '$--radius-sm',
      ...(overview ? {fill: '$--color-selected'} : {}),
    }, [
      icon(`${id}-overviewi-${suffix}`, 'layout-dashboard', {size: num(tokens, '--size-checkout-icon'), fill: '$--subtle-foreground'}),
      text(`${id}-overviewt-${suffix}`, 'Overview', {size: '$--text-subhead', weight: '600'}),
      frame(`${id}-overviewgap-${suffix}`, 'Spacer', {width: 'fill_container', height: 1}, []),
      text(`${id}-overviewn-${suffix}`, '내 차례 2 · ⌘⇧O', {fill: '$--muted-foreground'}),
    ]),
    text(`${id}-seen-${suffix}`, 'Projects · Recent activity', {size: '$--text-micro', fill: '$--muted-foreground', weight: '500'}),
    // Root rows keep direct children behind their shared badge.
    ...agents.flatMap((agent, index) => [
      screenSidebarAgentRow(`${id}-agent${index}-${suffix}`, {
        title: agent.title, symbol: agent.symbol ?? '●', color: agent.statusColor ?? '$--agent-working', age: agent.age ?? '3m', place: agent.place ?? 'hide › main',
        device: agent.device, badge: agent.badge, fold: 'none',
        width: num(tokens, '--size-sidebar-ideal') - 2 * num(tokens, '--spacing-md'),
      }),
    ]),
    frame(`${id}-footer-${suffix}`, 'Footer', {width: 'fill_container', gap: '$--spacing-xs', alignItems: 'center'}, [
      themedXref(`${id}-device-${suffix}`, 'Nyvom', 'Devices', {width: 20, height: 20}, {ZIZFR: {icon: 'laptop', fill: '$--muted-foreground'}}),
      frame(`${id}-footergap-${suffix}`, 'Gap', {width: 'fill_container', height: 1}, []),
      screenUsageChip(`${id}-usage0-${suffix}`, {provider: 'claude', value: '62%'}),
      screenUsageChip(`${id}-usage1-${suffix}`, {provider: 'codex', value: '59%'}),
      screenIconButton(`${id}-help-${suffix}`, 'circle-help', {size: 20}),
      screenIconButton(`${id}-settings-${suffix}`, 'settings', {size: 20}),
    ]),
  ]);
}

// -- Screen / Overview and Screen / Main --------------------------------------------

function overviewDesign(tokens) {
  return overviewScreens(tokens, {frame, text, icon, screenButton, screenIconButton, screenInput});
}

const OVERVIEW_SPEC = 'web/src/Overview.tsx, MainScreen.tsx, BacklogTab.tsx, ProjectsTab.tsx, OverviewRecord.tsx and QuickAdd.tsx (PRD overview-device-tasks D-01..D-38, B1-B66; sidebar-task-lens D-20, B38, B41): the Overview of one device, as the layer over a Workspace (1440 × 960 at most) or the page beside the sidebar with no Workspace in front. The head is the title, the device switch (each device as its rail tile and name, the rail\'s front device marked rail, a Needs You dot on a tile, a device that does not answer dimmed with 연결 안 됨) and Esc. The tabs are 백로그 and 프로젝트 with their counts, then 사용량; there is no 태스크 tab, since a device\'s tasks are the sidebar\'s 태스크 list. The chips, the project chip and the search follow on the right. 백로그 is one section per project with its source chip, its open and started counts (진행 중 N is a count, not a link), the input that says where Enter sends the title, a row being made, a row that failed with its reason, 다시 시도 and 지우기, and the open items with the missed-work flag, labels and age; an item opens its panel with ▷ 시작, GitHub, 어디서 왔나 and 되돌리기, and at Home a session\'s record opens in the same panel with Workspace 열기 and lane에서 보기. 프로젝트 is the device\'s disk line (free and total, the projects\' sum, what a cleanup frees) over one row per project: what its tasks are doing now, its backlog, its disk bar with the share a cleanup frees, N merged → 정리 and the last activity, a row being measured dimmed, and the quiet projects folded. The device that does not answer says why with 다시 연결. ⌘⇧M is one input over any screen: the project and why it was picked, the title, and where it goes, then 만드는 중, a failure that keeps the text, and the made line with 보기. Names and numbers are invented example content.';

function buildOverview(tokens) {
  const design = overviewDesign(tokens);
  return screenSheet('screen-overview', 'Screen / Overview', OVERVIEW_SPEC, design.boards, design.boards);
}

const MAIN_SPEC = 'web/src/MainScreen.tsx and Overview.tsx: with no Workspace in front the Overview is the page beside the sidebar, whose Overview row is marked, opened on 백로그 (PRD overview-device-tasks D-02, D-30; sidebar-task-lens D-20); Screen / Overview draws its tabs and states, and the Task lens sheets draw the sidebar\'s 태스크 list that replaced the Sessions panel.';

function buildMain(tokens) {
  const design = overviewDesign(tokens);
  const build = suffix => [
    screenSidebar(tokens, 'main-sidebar', suffix, [
      {title: '카드 상태 시트 설계', symbol: '?', statusColor: '$--warning', place: 'hide › main'},
      {title: '조용한 순찰 기능 개발', symbol: '○', statusColor: '$--muted-foreground', place: 'sasu › mailbox-decouple', device: 'mini'},
    ], {overview: true}),
    frame(`main-views-${suffix}`, 'Views', {layout: 'vertical', gap: '$--spacing-xl'}, [design.page(suffix)]),
  ];
  return screenSheet('screen-main', 'Screen / Main', MAIN_SPEC, build, build);
}

// -- Screen / Workspace ---------------------------------------------------------

// The shared window template and explicit state differences are decoded from the
// reviewed Pen proposal. Every window keeps its real exported node ID. Master refs
// and local token overrides stay linked to the project library; no raster assets.
// Changes to this design should update this scoped factory and its Pen sheet together.
function buildWorkspace(tokens) {
  const windowTemplate = {
    "type": "frame",
    "id": "TUoDW",
    "name": "Workspace / Light / full1116",
    "clip": true,
    "width": 1456,
    "height": 900,
    "fill": "$--background",
    "layout": "none",
    "children": [
      {
        "type": "frame",
        "id": "vT0el",
        "x": 0,
        "y": 0,
        "name": "Native window chrome - comparison context",
        "width": 1456,
        "height": 28,
        "fill": "$--secondary",
        "stroke": "$--border",
        "strokeWidth": {
          "bottom": "$--size-hairline"
        },
        "strokeAlignment": "inner",
        "layout": "none",
        "children": [
          {
            "type": "ellipse",
            "id": "mDiyL",
            "x": 8,
            "y": 8,
            "name": "Inactive window control 1",
            "opacity": 0.5,
            "fill": "$--muted-foreground",
            "width": 12,
            "height": 12
          },
          {
            "type": "ellipse",
            "id": "sQ9MO",
            "x": 28,
            "y": 8,
            "name": "Inactive window control 2",
            "opacity": 0.5,
            "fill": "$--muted-foreground",
            "width": 12,
            "height": 12
          },
          {
            "type": "ellipse",
            "id": "rxHFj",
            "x": 48,
            "y": 8,
            "name": "Inactive window control 3",
            "opacity": 0.5,
            "fill": "$--muted-foreground",
            "width": 12,
            "height": 12
          },
          {
            "type": "text",
            "id": "JNfcl",
            "x": 714,
            "y": 5,
            "name": "Window title",
            "fill": "$--muted-foreground",
            "content": "hide",
            "fontFamily": "$--font-ui",
            "fontSize": "$--text-body",
            "fontWeight": "600"
          }
        ]
      },
      {
        "type": "frame",
        "id": "j82QG",
        "x": 0,
        "y": 28,
        "name": "Device rail",
        "width": 48,
        "height": 872,
        "fill": "$--sidebar",
        "stroke": "$--border",
        "strokeWidth": {
          "right": 1
        },
        "strokeAlignment": "inner",
        "layout": "vertical",
        "gap": 12,
        "padding": [
          8,
          4
        ],
        "children": [
          {
            "id": "NFivf",
            "type": "ref",
            "ref": "hideui:Nyvom",
            "name": "This Mac",
            "width": 40,
            "height": 40,
            "stroke": "$--foreground",
            "strokeWidth": 2,
            "cornerRadius": "$--radius-lg",
            "descendants": {
              "hideui:ZIZFR": {
                "fill": "$--subtle-foreground",
                "icon": "laptop"
              }
            }
          },
          {
            "id": "XHA5O",
            "type": "ref",
            "ref": "hideui:Nyvom",
            "name": "Add device",
            "width": 40,
            "height": 32,
            "stroke": "$--border",
            "strokeWidth": 1,
            "cornerRadius": "$--radius-lg",
            "descendants": {
              "hideui:ZIZFR": {
                "fill": "$--subtle-foreground",
                "icon": "plus"
              }
            }
          }
        ]
      },
      {
        "type": "frame",
        "id": "E6s0TD",
        "x": 48,
        "y": 28,
        "name": "Projects sidebar",
        "width": 292,
        "height": 872,
        "fill": "$--sidebar",
        "stroke": "$--border",
        "strokeWidth": {
          "right": 1
        },
        "strokeAlignment": "inner",
        "layout": "none",
        "children": [
          {
            "type": "frame",
            "id": "mFUBL",
            "x": 0,
            "y": 0,
            "name": "This Mac header",
            "width": 292,
            "height": 32,
            "stroke": "$--border",
            "strokeWidth": {
              "bottom": "$--size-hairline"
            },
            "strokeAlignment": "inner",
            "gap": 8,
            "padding": [
              0,
              12
            ],
            "alignItems": "center",
            "children": [
              {
                "type": "text",
                "id": "di8S5",
                "name": "Device title",
                "fill": "$--foreground",
                "textGrowth": "fixed-width",
                "width": "fill_container",
                "content": "This Mac",
                "fontFamily": "$--font-ui",
                "fontSize": "$--text-title",
                "fontWeight": "600"
              },
              {
                "id": "Rfg8S",
                "type": "ref",
                "ref": "hideui:Nyvom",
                "name": "Add project",
                "descendants": {
                  "hideui:ZIZFR": {
                    "fill": "$--subtle-foreground",
                    "icon": "plus"
                  }
                }
              },
              {
                "id": "Y02KdW",
                "type": "ref",
                "ref": "hideui:Nyvom",
                "name": "Search",
                "descendants": {
                  "hideui:ZIZFR": {
                    "fill": "$--subtle-foreground",
                    "icon": "search"
                  }
                }
              }
            ]
          },
          {
            "type": "frame",
            "id": "jAdn6",
            "x": 0,
            "y": 32,
            "name": "Sidebar tab strip",
            "width": 292,
            "height": 32,
            "stroke": "$--border",
            "strokeWidth": {
              "bottom": "$--size-hairline"
            },
            "strokeAlignment": "inner",
            "gap": 8,
            "padding": [
              0,
              12
            ],
            "alignItems": "center",
            "children": [
              {
                "type": "text",
                "id": "OYRm7",
                "name": "Selected tab",
                "fill": "$--foreground",
                "content": "Projects",
                "fontFamily": "$--font-ui",
                "fontSize": "$--text-body",
                "fontWeight": "normal"
              },
              {
                "type": "text",
                "id": "KGCa7",
                "name": "Other tab",
                "fill": "$--muted-foreground",
                "content": "Agents",
                "fontFamily": "$--font-ui",
                "fontSize": "$--text-body",
                "fontWeight": "normal"
              }
            ]
          },
          {
            "type": "frame",
            "id": "sv6IB",
            "x": 4,
            "y": 68,
            "name": "Home",
            "width": 284,
            "height": 36,
            "gap": 8,
            "padding": [
              0,
              8
            ],
            "alignItems": "center",
            "children": [
              {
                "type": "icon",
                "id": "h2d0u",
                "name": "Home icon",
                "width": 14,
                "height": 14,
                "icon": "house",
                "library": "lucide",
                "fill": "$--subtle-foreground"
              },
              {
                "type": "text",
                "id": "pv35D",
                "name": "Home title",
                "fill": "$--foreground",
                "textGrowth": "fixed-width",
                "width": "fill_container",
                "content": "Home",
                "fontFamily": "$--font-ui",
                "fontSize": "$--text-title",
                "fontWeight": "600"
              },
              {
                "type": "text",
                "id": "NDnyJ",
                "name": "Project count",
                "fill": "$--muted-foreground",
                "content": "0 projects",
                "fontFamily": "$--font-ui",
                "fontSize": "$--text-body",
                "fontWeight": "normal"
              }
            ]
          },
          {
            "type": "text",
            "id": "c4b0sQ",
            "x": 12,
            "y": 110,
            "name": "Recent activity",
            "fill": "$--muted-foreground",
            "content": "Projects · Recent activity · 1",
            "fontFamily": "$--font-ui",
            "fontSize": "$--text-micro",
            "fontWeight": "600"
          },
          {
            "type": "frame",
            "id": "HObxT",
            "x": 4,
            "y": 124,
            "name": "Selected checkout",
            "width": 284,
            "height": 36,
            "fill": "$--secondary",
            "cornerRadius": "$--radius-sm",
            "gap": 8,
            "padding": [
              0,
              8
            ],
            "alignItems": "center",
            "children": [
              {
                "type": "icon",
                "id": "qQH68",
                "name": "Checkout folder",
                "width": 14,
                "height": 14,
                "icon": "folder",
                "library": "lucide",
                "fill": "$--subtle-foreground"
              },
              {
                "type": "text",
                "id": "swzTQ",
                "name": "Checkout label",
                "fill": "$--foreground",
                "content": "fixture",
                "fontFamily": "$--font-ui",
                "fontSize": "$--text-body",
                "fontWeight": "600"
              }
            ]
          },
          {
            "type": "frame",
            "id": "F0TmV",
            "x": 0,
            "y": 840,
            "name": "Sidebar footer",
            "width": 292,
            "height": 32,
            "stroke": "$--border",
            "strokeWidth": {
              "top": 1
            },
            "strokeAlignment": "inner",
            "gap": 8,
            "padding": [
              0,
              12
            ],
            "justifyContent": "end",
            "alignItems": "center",
            "children": [
              {
                "id": "VnRz8",
                "type": "ref",
                "ref": "hideui:Nyvom",
                "name": "Background usage",
                "descendants": {
                  "hideui:ZIZFR": {
                    "fill": "$--subtle-foreground",
                    "icon": "activity"
                  }
                }
              },
              {
                "id": "ydsLS",
                "type": "ref",
                "ref": "hideui:Nyvom",
                "name": "Settings",
                "descendants": {
                  "hideui:ZIZFR": {
                    "fill": "$--subtle-foreground",
                    "icon": "settings"
                  }
                }
              }
            ]
          }
        ]
      },
      {
        "id": "W2zeQP",
        "type": "ref",
        "ref": "hideui:VMZTz",
        "name": "Shared Workspace toolbar",
        "fill": "$--sidebar",
        "stroke": "$--border",
        "x": 340,
        "y": 28,
        "width": 1116,
        "height": 32,
        "descendants": {
          "hideui:bkh81": {
            "fill": "$--subtle-foreground",
            "content": "Home  /  fixture  /  main"
          },
          "hideui:Kjfje/hideui:side-panel-tools-toggle/hideui:ZIZFR": {
            "fill": "$--subtle-foreground"
          }
        }
      }
    ]
  };
  const sheet = {
    "type": "frame",
    "id": "screen-workspace",
    "x": 0,
    "y": 32635,
    "name": "Screen / Workspace",
    "width": 2020,
    "fill": "#EDEDEE",
    "layout": "vertical",
    "gap": "$--spacing-xl",
    "padding": "$--spacing-xl",
    "children": [
      {
        "type": "text",
        "id": "wlf1z",
        "name": "Workspace title",
        "fill": "$--foreground",
        "content": "Workspace",
        "fontFamily": "$--font-ui",
        "fontSize": "$--text-headline",
        "fontWeight": "600"
      },
      {
        "type": "text",
        "id": "p47Nai",
        "name": "Workspace contract",
        "fill": "$--muted-foreground",
        "textGrowth": "fixed-width",
        "width": 1940,
        "content": "PRD right-lane: Agent Views fill the body and one right lane holds the four tools, Memory, PRs, Files and Changes, chosen on the 48 px rail at the window's right edge. Unpinned, the lane opens over the agents and the terminals keep their width; a press on a terminal or Escape puts it away. Pinned, it docks beside the agents behind an 8 px divider, and a window that cannot leave the agents 480 px opens it over them instead (D-07). The lane is 520 px by default; the file tree is 240 px, stays on until it is turned off and widens the lane rather than the document; with no document open the tree fills the lane. Expand gives the lane the whole body until it is pressed again or Escape. The window's top row carries the path, Overview and Open server beside the traffic lights (PRD sidebar-titlebar-refresh D-15, D-16). Rail, lane header, pin, tree button, document header and dividers are refs of Component / Workspace columns.",
        "fontFamily": "$--font-ui",
        "fontSize": "$--text-body",
        "fontWeight": "normal"
      },
      {
        "type": "frame",
        "id": "g0S6az",
        "name": "Light",
        "theme": {
          "Mode": "Light"
        },
        "width": 1972,
        "fill": "$--background",
        "layout": "vertical",
        "gap": 16,
        "padding": 16,
        "children": [
          {
            "type": "text",
            "id": "i8NtD",
            "name": "Theme",
            "fill": "$--foreground",
            "content": "Light",
            "fontFamily": "$--font-ui",
            "fontSize": "$--text-headline",
            "fontWeight": "600"
          }
        ]
      },
      {
        "type": "frame",
        "id": "dkzAZ",
        "name": "Dark",
        "theme": {
          "Mode": "Dark"
        },
        "width": 1972,
        "fill": "$--background",
        "layout": "vertical",
        "gap": 16,
        "padding": 16,
        "children": [
          {
            "type": "text",
            "id": "RCFCu",
            "name": "Theme",
            "fill": "$--foreground",
            "content": "Dark",
            "fontFamily": "$--font-ui",
            "fontSize": "$--text-headline",
            "fontWeight": "600"
          }
        ]
      }
    ]
  };
  function nodes(node, result = []) {
    result.push(node);
    for (const child of node.children ?? []) nodes(child, result);
    return result;
  }
  // PRD sidebar-titlebar-refresh D-15 to D-17: macOS's title bar and the toolbar
  // row are one top row. Over the rail and the sidebar it names the device a step
  // after the traffic lights; over the center the toolbar carries the path without
  // Home and the Workspace actions; the rail, the sidebar (Search row first, no Home
  // row) and the body start at its foot.
  const TOP = num(tokens, '--size-tab-strip');
  const OLD_TOP = 28;
  const OLD_BODY = 60;
  const toolbarColors = themedOverrides('VMZTz').descendants;
  const aliased = map => Object.fromEntries(Object.entries(map).map(([key, value]) => [key.split('/').map(part => `${ALIAS}:${part}`).join('/'), value]));
  function topRow(window) {
    const named = (parent, name) => {
      const node = parent.children.find(child => child.name === name);
      if (!node) throw new Error(`Workspace window ${window.id} lacks ${name}`);
      return node;
    };
    const chrome = named(window, 'Native window chrome - comparison context');
    const rail = named(window, 'Device rail');
    const sidebar = named(window, 'Projects sidebar');
    const toolbar = named(window, 'Shared Workspace toolbar');
    const [light0, light1, light2, title] = chrome.children;
    Object.assign(chrome, {name: 'Window top row: device', width: sidebar.x + sidebar.width, height: TOP, fill: '$--sidebar', strokeWidth: {bottom: '$--size-hairline', right: '$--size-hairline'}});
    // The sidebar's switch opens the row a step after the lights, and the device name follows it (TitleRow.tsx).
    const switchX = num(tokens, '--size-traffic-light-inset') + num(tokens, '--spacing-xs');
    const control = num(tokens, '--size-control-sm');
    const glyph = num(tokens, '--size-icon');
    chrome.children = [
      {...light0, x: 13, y: 10}, {...light1, x: 33, y: 10}, {...light2, x: 53, y: 10},
      {...icon(`${chrome.id}-switch`, 'panel-left', {size: glyph, fill: '$--subtle-foreground'}), name: 'Sidebar switch', x: switchX + (control - glyph) / 2, y: (TOP - glyph) / 2},
      {...title, name: 'Device', content: 'This Mac', x: switchX + control + num(tokens, '--spacing-xs'), y: 8, fill: '$--foreground', fontSize: '$--text-subhead'},
    ];
    for (const column of [rail, sidebar]) Object.assign(column, {y: TOP, height: column.height - (TOP - OLD_TOP)});
    const header = named(sidebar, 'This Mac header');
    const tabs = named(sidebar, 'Sidebar tab strip');
    const search = themedXref(header.id, 'sbc-search', 'Search row', {x: 0, y: 0, width: sidebar.width});
    const searchH = num(tokens, '--spacing-sm') + num(tokens, '--size-sidebar-search') + num(tokens, '--spacing-xs');
    Object.assign(tabs, {name: 'Overview row', y: searchH, children: [
      icon(`${tabs.id}-g`, 'layout-dashboard', {size: num(tokens, '--size-icon'), fill: '$--subtle-foreground'}),
      {...tabs.children[0], name: 'Overview', content: 'Overview', fill: '$--subtle-foreground'},
    ]});
    sidebar.children = sidebar.children.filter(child => child.name !== 'Home').map(child => child === header ? search : child);
    Object.assign(named(sidebar, 'Recent activity'), {y: searchH + TOP + num(tokens, '--spacing-md'), fontWeight: '500'});
    named(sidebar, 'Selected checkout').y = searchH + TOP + num(tokens, '--spacing-md') + num(tokens, '--size-sidebar-section-header') - num(tokens, '--spacing-sm');
    named(sidebar, 'Sidebar footer').y = sidebar.height - TOP;
    const breadcrumb = `${ALIAS}:bkh81`;
    const descendants = {...aliased(toolbarColors), ...toolbar.descendants};
    if (descendants[breadcrumb]?.content) descendants[breadcrumb] = {...descendants[breadcrumb], content: descendants[breadcrumb].content.replace(/^Home\s+\/\s+/, '')};
    Object.assign(toolbar, {name: 'Top row: Workspace path and icons', y: 0, height: TOP, descendants});
    for (const child of window.children) {
      if (![chrome, rail, sidebar, toolbar].includes(child) && child.y >= OLD_BODY) child.y -= OLD_BODY - TOP;
    }
  }
  // The window every lane board starts from: the template's ids, no columns of its own.
  const template = structuredClone(windowTemplate);
  topRow(template);
  for (const node of nodes(template)) {
    if (node.type !== 'ref') continue;
    if (!node.ref.startsWith(`${ALIAS}:`)) throw new Error(`Workspace reference ${node.ref} must use ${ALIAS}`);
    const masterId = node.ref.slice(ALIAS.length + 1);
    const master = findMaster({children: libraryDocument().children}, masterId);
    if (!master?.reusable) throw new Error(`Workspace needs reusable master ${masterId}`);
  }
  const result = structuredClone(sheet);
  for (const [name, suffix] of [['Light', 'l'], ['Dark', 'd']]) {
    const section = result.children.find(node => node.name === name);
    if (!section) throw new Error(`Workspace lacks ${name} section`);
    section.children.push(...laneBoards(tokens, {themedXref, template, alias: ALIAS}, suffix), sessionDesign(tokens).headerBoard(suffix), sessionDesign(tokens).edgeBoard(suffix), sessionDesign(tokens).taskBoard(suffix));
  }
  for (const state of buildWorkspaceSupplemental()) {
    const parent = nodes(result).find(node => node.id === state.parent);
    if (!parent) throw new Error(`Workspace supplemental parent ${state.parent} is missing`);
    parent.children.push(...state.children);
  }
  // Frozen screen states keep their own overrides, while inherited master
  // colors need local tokens for the importing document's current theme.
  for (const node of nodes(result)) {
    if (node.type !== 'ref') continue;
    if (!node.ref.startsWith(`${ALIAS}:`)) throw new Error(`Workspace reference ${node.ref} must use ${ALIAS}`);
    const auto = themedOverrides(node.ref.slice(ALIAS.length + 1));
    for (const [prop, value] of Object.entries(auto.top)) if (!Object.hasOwn(node, prop)) node[prop] = value;
    for (const [key, props] of Object.entries(auto.descendants)) {
      const address = key.split('/').map(part => `${ALIAS}:${part}`).join('/');
      const patch = node.descendants?.[address] ?? {};
      const missing = Object.entries(props).filter(([prop]) => !Object.hasOwn(patch, prop));
      if (missing.length) (node.descendants ??= {})[address] = {...patch, ...Object.fromEntries(missing)};
    }
  }
  return result;
}

// Retained valid review targets are separate from the column condition matrix.
// Their actual Pen IDs and Korean selection ownership are preserved.
function buildWorkspaceSupplemental() {
  return [
    {"mode":"Light","parent":"g0S6az","children":[{"type":"frame","id":"ws-focus-l","name":"One keyboard area, other selections readable","width":960,"height":440,"fill":"$--background","children":[{"type":"frame","id":"ws-focus-l-1","name":"Keyboard owner","width":"fill_container","height":"fill_container","layout":"vertical","children":[{"type":"frame","id":"ws-focus-l-1-bar","name":"Tab bar","width":"fill_container","height":32,"fill":"$--background","children":[{"id":"ws-focus-l-1-tab1","type":"ref","ref":"hideui:view-tab","name":"Selected Korean tab","fill":"$--background","stroke":"$--primary","strokeWidth":{"bottom":"$--size-tab-indicator"},"strokeAlignment":"inner","descendants":{"hideui:view-tab-mark":{"fill":"$--file-document"},"hideui:view-tab-title":{"fill":"$--foreground","content":"한글 노트.md"},"hideui:view-tab-strike":{"fill":"$--muted-foreground"},"hideui:view-tab-dirty":{"fill":"$--warning"},"hideui:view-tab-close":{"enabled":true}}},{"id":"ws-focus-l-1-tab2","type":"ref","ref":"hideui:view-tab","name":"검증 결과.md","fill":"$--card","descendants":{"hideui:view-tab-mark":{"fill":"$--file-document"},"hideui:view-tab-title":{"fill":"$--subtle-foreground","content":"검증 결과.md"},"hideui:view-tab-strike":{"fill":"$--muted-foreground"},"hideui:view-tab-dirty":{"fill":"$--warning"}}}]},{"type":"frame","id":"ws-focus-l-1-body","name":"Readable Korean content","width":"fill_container","height":"fill_container","stroke":"$--primary","strokeWidth":2,"strokeAlignment":"inner","fill":"$--background","layout":"vertical","padding":"$--spacing-sm","children":[{"type":"text","id":"ws-focus-l-1-text","name":"# 한글 노트\n\n현재 입력을 받는 영역만 강조합니다","fill":"$--foreground","textGrowth":"fixed-width","width":"fill_container","content":"# 한글 노트\n\n현재 입력을 받는 영역만 강조합니다.\n다른 영역의 원래 선택과 내용은 읽을 수 있습니다.\n\nfixture % echo 한글 확인\n한글 확인","fontFamily":"$--font-mono","fontSize":"$--text-caption","fontWeight":"normal"}]}]},{"type":"frame","id":"ws-focus-divider-l","name":"Area divider","width":"fit_content(0)","height":"fill_container","fill":"$--border"},{"type":"frame","id":"ws-focus-l-2","name":"Other area, retained selection","width":"fill_container","height":"fill_container","layout":"vertical","children":[{"type":"frame","id":"ws-focus-l-2-bar","name":"Tab bar","width":"fill_container","height":32,"fill":"$--card","children":[{"id":"ws-focus-l-2-tab1","type":"ref","ref":"hideui:view-tab","name":"Selected Korean tab","fill":"$--secondary","descendants":{"hideui:view-tab-mark":{"fill":"$--file-document"},"hideui:view-tab-title":{"fill":"$--foreground","content":"한글 노트.md"},"hideui:view-tab-strike":{"fill":"$--muted-foreground"},"hideui:view-tab-dirty":{"fill":"$--warning"},"hideui:view-tab-close":{"enabled":true}}},{"id":"ws-focus-l-2-tab2","type":"ref","ref":"hideui:view-tab","name":"검증 결과.md","fill":"$--card","descendants":{"hideui:view-tab-mark":{"fill":"$--file-document"},"hideui:view-tab-title":{"fill":"$--subtle-foreground","content":"검증 결과.md"},"hideui:view-tab-strike":{"fill":"$--muted-foreground"},"hideui:view-tab-dirty":{"fill":"$--warning"}}}]},{"type":"frame","id":"ws-focus-l-2-body","name":"Readable Korean content","width":"fill_container","height":"fill_container","fill":"$--background","layout":"vertical","padding":"$--spacing-sm","children":[{"type":"text","id":"ws-focus-l-2-text","name":"# 한글 노트\n\n현재 입력을 받는 영역만 강조합니다","fill":"$--foreground","textGrowth":"fixed-width","width":"fill_container","content":"# 한글 노트\n\n현재 입력을 받는 영역만 강조합니다.\n다른 영역의 원래 선택과 내용은 읽을 수 있습니다.\n\nfixture % echo 한글 확인\n한글 확인","fontFamily":"$--font-mono","fontSize":"$--text-caption","fontWeight":"normal"}]}]}]},{"type":"frame","id":"ewWtp","name":"Multiple running servers; the right lane closed","clip":true,"width":1440,"height":900,"fill":"$--background","layout":"none","children":[{"type":"frame","id":"NlkIg","x":0,"y":0,"name":"Native window chrome - comparison context","width":1440,"height":28,"fill":"$--secondary","stroke":"$--border","strokeWidth":{"bottom":"$--size-hairline"},"strokeAlignment":"inner","layout":"none","children":[{"type":"ellipse","id":"j9wRLv","x":8,"y":8,"name":"Inactive window control 1","opacity":0.5,"fill":"$--muted-foreground","width":12,"height":12},{"type":"ellipse","id":"PbX47","x":28,"y":8,"name":"Inactive window control 2","opacity":0.5,"fill":"$--muted-foreground","width":12,"height":12},{"type":"ellipse","id":"slAYW","x":48,"y":8,"name":"Inactive window control 3","opacity":0.5,"fill":"$--muted-foreground","width":12,"height":12},{"type":"text","id":"PXL5q","x":706,"y":5,"name":"Window title","fill":"$--muted-foreground","content":"hide","fontFamily":"$--font-ui","fontSize":"$--text-body","fontWeight":"600"}]},{"type":"frame","id":"lE5kU","x":0,"y":28,"name":"Device rail","width":48,"height":872,"fill":"$--sidebar","stroke":"$--border","strokeWidth":{"right":1},"strokeAlignment":"inner","layout":"vertical","gap":12,"padding":[8,4],"children":[{"id":"tQ1fK","type":"ref","ref":"hideui:Nyvom","name":"This Mac","width":40,"height":40,"stroke":"$--foreground","strokeWidth":2,"cornerRadius":"$--radius-lg","descendants":{"hideui:ZIZFR":{"fill":"$--subtle-foreground","icon":"laptop"}}},{"id":"O9kV0","type":"ref","ref":"hideui:Nyvom","name":"Add device","width":40,"height":32,"stroke":"$--border","strokeWidth":1,"cornerRadius":"$--radius-lg","descendants":{"hideui:ZIZFR":{"fill":"$--subtle-foreground","icon":"plus"}}}]},{"type":"frame","id":"WXmI0","x":48,"y":28,"name":"Projects sidebar","width":292,"height":872,"fill":"$--sidebar","stroke":"$--border","strokeWidth":{"right":1},"strokeAlignment":"inner","layout":"none","children":[{"type":"frame","id":"e1gTd","x":0,"y":0,"name":"This Mac header","width":292,"height":32,"stroke":"$--border","strokeWidth":{"bottom":"$--size-hairline"},"strokeAlignment":"inner","gap":8,"padding":[0,12],"alignItems":"center","children":[{"type":"text","id":"YhV3w","name":"Device title","fill":"$--foreground","textGrowth":"fixed-width","width":"fill_container","content":"This Mac","fontFamily":"$--font-ui","fontSize":"$--text-title","fontWeight":"600"},{"id":"hjnnU","type":"ref","ref":"hideui:Nyvom","name":"Add project","descendants":{"hideui:ZIZFR":{"fill":"$--subtle-foreground","icon":"plus"}}},{"id":"lr1rE","type":"ref","ref":"hideui:Nyvom","name":"Search","descendants":{"hideui:ZIZFR":{"fill":"$--subtle-foreground","icon":"search"}}}]},{"type":"frame","id":"x47WeE","x":0,"y":32,"name":"Sidebar tab strip","width":292,"height":32,"stroke":"$--border","strokeWidth":{"bottom":"$--size-hairline"},"strokeAlignment":"inner","gap":8,"padding":[0,12],"alignItems":"center","children":[{"type":"text","id":"zVAh1","name":"Selected tab","fill":"$--foreground","content":"Projects","fontFamily":"$--font-ui","fontSize":"$--text-body","fontWeight":"normal"},{"type":"text","id":"aBZ23","name":"Other tab","fill":"$--muted-foreground","content":"Agents","fontFamily":"$--font-ui","fontSize":"$--text-body","fontWeight":"normal"}]},{"type":"frame","id":"e1Y8X","x":4,"y":68,"name":"Home","width":284,"height":36,"gap":8,"padding":[0,8],"alignItems":"center","children":[{"type":"icon","id":"z9Zjo","name":"Home icon","width":14,"height":14,"icon":"house","library":"lucide","fill":"$--subtle-foreground"},{"type":"text","id":"x0S7j","name":"Home title","fill":"$--foreground","textGrowth":"fixed-width","width":"fill_container","content":"Home","fontFamily":"$--font-ui","fontSize":"$--text-title","fontWeight":"600"},{"type":"text","id":"vGB7F","name":"Project count","fill":"$--muted-foreground","content":"0 projects","fontFamily":"$--font-ui","fontSize":"$--text-body","fontWeight":"normal"}]},{"type":"text","id":"iWAse","x":12,"y":110,"name":"Recent activity","fill":"$--muted-foreground","content":"Projects · Recent activity · 1","fontFamily":"$--font-ui","fontSize":"$--text-micro","fontWeight":"600"},{"type":"frame","id":"vhwXo","x":4,"y":124,"name":"Selected checkout","width":284,"height":36,"fill":"$--secondary","cornerRadius":"$--radius-sm","gap":8,"padding":[0,8],"alignItems":"center","children":[{"type":"icon","id":"klozz","name":"Checkout folder","width":14,"height":14,"icon":"folder","library":"lucide","fill":"$--subtle-foreground"},{"type":"text","id":"vmjI1","name":"Checkout label","fill":"$--foreground","content":"fixture","fontFamily":"$--font-ui","fontSize":"$--text-body","fontWeight":"600"}]},{"type":"frame","id":"D2gM4I","x":0,"y":840,"name":"Sidebar footer","width":292,"height":32,"stroke":"$--border","strokeWidth":{"top":1},"strokeAlignment":"inner","gap":8,"padding":[0,12],"justifyContent":"end","alignItems":"center","children":[{"id":"pDsO7","type":"ref","ref":"hideui:Nyvom","name":"Background usage","descendants":{"hideui:ZIZFR":{"fill":"$--subtle-foreground","icon":"activity"}}},{"id":"M9bxgS","type":"ref","ref":"hideui:Nyvom","name":"Settings","descendants":{"hideui:ZIZFR":{"fill":"$--subtle-foreground","icon":"settings"}}}]}]},{"id":"KabPT","type":"ref","ref":"hideui:VMZTz","name":"Shared Workspace toolbar","fill":"$--sidebar","stroke":"$--border","x":340,"y":28,"width":1100,"height":32,"descendants":{"hideui:bkh81":{"fill":"$--subtle-foreground","content":"Home  /  fixture  /  main"},"hideui:Kjfje/hideui:side-panel-tools-toggle/hideui:ZIZFR":{"fill":"$--subtle-foreground"}}},{"id":"H4VH2","type":"ref","ref":"hideui:rnWpl","name":"Agent column","fill":"$--background","x":340,"y":60,"width":1052,"height":840,"descendants":{"hideui:m3nqv":{"fill":"$--card","stroke":"$--border"},"hideui:Yf5g6":{"fill":"$--secondary"},"hideui:Yf5g6/hideui:view-tab-mark":{"fill":"$--subtle-foreground"},"hideui:Yf5g6/hideui:view-tab-title":{"fill":"$--foreground"},"hideui:Yf5g6/hideui:view-tab-strike":{"fill":"$--muted-foreground"},"hideui:Yf5g6/hideui:view-tab-dirty":{"fill":"$--warning"},"hideui:Yf5g6/hideui:view-tab-close/hideui:ZIZFR":{"fill":"$--subtle-foreground"},"hideui:ujfpg/hideui:ZIZFR":{"fill":"$--subtle-foreground"},"hideui:GYdsi/hideui:ZIZFR":{"fill":"$--subtle-foreground"},"hideui:jEy0G":{"fill":"$--background","width":1100},"hideui:blLvs":{"fill":"$--secondary","stroke":"$--border"},"hideui:W2jHfs":{"fill":"$--subtle-foreground"},"hideui:v57l6H":{"fill":"$--foreground"},"hideui:CG4wC":{"fill":"$--muted-foreground"},"hideui:K8ikC/hideui:ZIZFR":{"fill":"$--subtle-foreground"},"hideui:a63n6t/hideui:ZIZFR":{"fill":"$--subtle-foreground"},"hideui:jPwvc":{"fill":"$--background"},"hideui:PNaQy":{"fill":"$--foreground"},"hideui:aTsYy":{"fill":"$--background","stroke":"$--border","enabled":false,"width":0,"height":0},"hideui:lFQa0":{"fill":"$--card","stroke":"$--border"},"hideui:ljIwg":{"fill":"$--subtle-foreground"},"hideui:e7uYn":{"fill":"$--foreground"},"hideui:uJYZa":{"fill":"$--muted-foreground"},"hideui:x9QjtJ/hideui:ZIZFR":{"fill":"$--subtle-foreground"},"hideui:E0Mk6/hideui:ZIZFR":{"fill":"$--subtle-foreground"},"hideui:M6Mnyn":{"fill":"$--background"},"hideui:QSc1K":{"fill":"$--foreground"}}},{"id":"ewWtp-rail","type":"ref","ref":"hideui:lane-rail","name":"Tool rail","x":1392,"y":60,"height":840},{"type":"frame","id":"E3BX0","x":1112,"y":60,"name":"Running servers picker","width":320,"fill":"$--popover","cornerRadius":"$--radius-md","stroke":"$--border","strokeWidth":1,"strokeAlignment":"inner","layout":"vertical","gap":"$--spacing-xs","padding":"$--spacing-sm","children":[{"type":"text","id":"ILw1e","name":"Running servers","fill":"$--popover-foreground","content":"Running servers","fontFamily":"$--font-ui","fontSize":"$--text-caption","fontWeight":"500"},{"id":"u8ij3q","type":"ref","ref":"hideui:btn-m","name":"127.0.0.1:3000","width":"fill_container","height":28,"justifyContent":"start","gap":"$--spacing-xs","padding":[0,"$--spacing-md"],"fill":"$--accent","stroke":"$--ring","strokeWidth":1,"strokeAlignment":"outer","descendants":{"hideui:btn-ic":{"enabled":true,"icon":"globe","fill":"$--popover-foreground"},"hideui:btn-lb":{"content":"127.0.0.1:3000","fontFamily":"$--font-mono","fontSize":"$--text-caption","fill":"$--popover-foreground"}}},{"id":"BOKUB","type":"ref","ref":"hideui:btn-m","name":"[::1]:5173","width":"fill_container","height":28,"justifyContent":"start","gap":"$--spacing-xs","padding":[0,"$--spacing-md"],"fill":[],"stroke":[],"strokeWidth":0,"strokeAlignment":"outer","descendants":{"hideui:btn-ic":{"enabled":true,"icon":"globe","fill":"$--popover-foreground"},"hideui:btn-lb":{"content":"[::1]:5173","fontFamily":"$--font-mono","fontSize":"$--text-caption","fill":"$--popover-foreground"}}}]}]}]},
    {"mode":"Dark","parent":"dkzAZ","children":[{"type":"frame","id":"ws-focus-d","name":"One keyboard area, other selections readable","width":960,"height":440,"fill":"$--background","children":[{"type":"frame","id":"ws-focus-d-1","name":"Keyboard owner","width":"fill_container","height":"fill_container","layout":"vertical","children":[{"type":"frame","id":"ws-focus-d-1-bar","name":"Tab bar","width":"fill_container","height":32,"fill":"$--background","children":[{"id":"ws-focus-d-1-tab1","type":"ref","ref":"hideui:view-tab","name":"Selected Korean tab","fill":"$--background","stroke":"$--primary","strokeWidth":{"bottom":"$--size-tab-indicator"},"strokeAlignment":"inner","descendants":{"hideui:view-tab-mark":{"fill":"$--file-document"},"hideui:view-tab-title":{"fill":"$--foreground","content":"한글 노트.md"},"hideui:view-tab-strike":{"fill":"$--muted-foreground"},"hideui:view-tab-dirty":{"fill":"$--warning"},"hideui:view-tab-close":{"enabled":true}}},{"id":"ws-focus-d-1-tab2","type":"ref","ref":"hideui:view-tab","name":"검증 결과.md","fill":"$--card","descendants":{"hideui:view-tab-mark":{"fill":"$--file-document"},"hideui:view-tab-title":{"fill":"$--subtle-foreground","content":"검증 결과.md"},"hideui:view-tab-strike":{"fill":"$--muted-foreground"},"hideui:view-tab-dirty":{"fill":"$--warning"}}}]},{"type":"frame","id":"ws-focus-d-1-body","name":"Readable Korean content","width":"fill_container","height":"fill_container","stroke":"$--primary","strokeWidth":2,"strokeAlignment":"inner","fill":"$--background","layout":"vertical","padding":"$--spacing-sm","children":[{"type":"text","id":"ws-focus-d-1-text","name":"# 한글 노트\n\n현재 입력을 받는 영역만 강조합니다","fill":"$--foreground","textGrowth":"fixed-width","width":"fill_container","content":"# 한글 노트\n\n현재 입력을 받는 영역만 강조합니다.\n다른 영역의 원래 선택과 내용은 읽을 수 있습니다.\n\nfixture % echo 한글 확인\n한글 확인","fontFamily":"$--font-mono","fontSize":"$--text-caption","fontWeight":"normal"}]}]},{"type":"frame","id":"ws-focus-divider-d","name":"Area divider","width":"fit_content(0)","height":"fill_container","fill":"$--border"},{"type":"frame","id":"ws-focus-d-2","name":"Other area, retained selection","width":"fill_container","height":"fill_container","layout":"vertical","children":[{"type":"frame","id":"ws-focus-d-2-bar","name":"Tab bar","width":"fill_container","height":32,"fill":"$--card","children":[{"id":"ws-focus-d-2-tab1","type":"ref","ref":"hideui:view-tab","name":"Selected Korean tab","fill":"$--secondary","descendants":{"hideui:view-tab-mark":{"fill":"$--file-document"},"hideui:view-tab-title":{"fill":"$--foreground","content":"한글 노트.md"},"hideui:view-tab-strike":{"fill":"$--muted-foreground"},"hideui:view-tab-dirty":{"fill":"$--warning"},"hideui:view-tab-close":{"enabled":true}}},{"id":"ws-focus-d-2-tab2","type":"ref","ref":"hideui:view-tab","name":"검증 결과.md","fill":"$--card","descendants":{"hideui:view-tab-mark":{"fill":"$--file-document"},"hideui:view-tab-title":{"fill":"$--subtle-foreground","content":"검증 결과.md"},"hideui:view-tab-strike":{"fill":"$--muted-foreground"},"hideui:view-tab-dirty":{"fill":"$--warning"}}}]},{"type":"frame","id":"ws-focus-d-2-body","name":"Readable Korean content","width":"fill_container","height":"fill_container","fill":"$--background","layout":"vertical","padding":"$--spacing-sm","children":[{"type":"text","id":"ws-focus-d-2-text","name":"# 한글 노트\n\n현재 입력을 받는 영역만 강조합니다","fill":"$--foreground","textGrowth":"fixed-width","width":"fill_container","content":"# 한글 노트\n\n현재 입력을 받는 영역만 강조합니다.\n다른 영역의 원래 선택과 내용은 읽을 수 있습니다.\n\nfixture % echo 한글 확인\n한글 확인","fontFamily":"$--font-mono","fontSize":"$--text-caption","fontWeight":"normal"}]}]}]},{"type":"frame","id":"es3RE","name":"Multiple running servers; the right lane closed","clip":true,"width":1440,"height":900,"fill":"$--background","layout":"none","children":[{"type":"frame","id":"aTb9G","x":0,"y":0,"name":"Native window chrome - comparison context","width":1440,"height":28,"fill":"$--secondary","stroke":"$--border","strokeWidth":{"bottom":"$--size-hairline"},"strokeAlignment":"inner","layout":"none","children":[{"type":"ellipse","id":"OXSdD","x":8,"y":8,"name":"Inactive window control 1","opacity":0.5,"fill":"$--muted-foreground","width":12,"height":12},{"type":"ellipse","id":"PJTbo","x":28,"y":8,"name":"Inactive window control 2","opacity":0.5,"fill":"$--muted-foreground","width":12,"height":12},{"type":"ellipse","id":"FO6Yr","x":48,"y":8,"name":"Inactive window control 3","opacity":0.5,"fill":"$--muted-foreground","width":12,"height":12},{"type":"text","id":"XgiMy","x":706,"y":5,"name":"Window title","fill":"$--muted-foreground","content":"hide","fontFamily":"$--font-ui","fontSize":"$--text-body","fontWeight":"600"}]},{"type":"frame","id":"uwCzX","x":0,"y":28,"name":"Device rail","width":48,"height":872,"fill":"$--sidebar","stroke":"$--border","strokeWidth":{"right":1},"strokeAlignment":"inner","layout":"vertical","gap":12,"padding":[8,4],"children":[{"id":"m3Yw2","type":"ref","ref":"hideui:Nyvom","name":"This Mac","width":40,"height":40,"stroke":"$--foreground","strokeWidth":2,"cornerRadius":"$--radius-lg","descendants":{"hideui:ZIZFR":{"fill":"$--subtle-foreground","icon":"laptop"}}},{"id":"FDTjR","type":"ref","ref":"hideui:Nyvom","name":"Add device","width":40,"height":32,"stroke":"$--border","strokeWidth":1,"cornerRadius":"$--radius-lg","descendants":{"hideui:ZIZFR":{"fill":"$--subtle-foreground","icon":"plus"}}}]},{"type":"frame","id":"VJuAf","x":48,"y":28,"name":"Projects sidebar","width":292,"height":872,"fill":"$--sidebar","stroke":"$--border","strokeWidth":{"right":1},"strokeAlignment":"inner","layout":"none","children":[{"type":"frame","id":"ad4af","x":0,"y":0,"name":"This Mac header","width":292,"height":32,"stroke":"$--border","strokeWidth":{"bottom":"$--size-hairline"},"strokeAlignment":"inner","gap":8,"padding":[0,12],"alignItems":"center","children":[{"type":"text","id":"OA1kQ","name":"Device title","fill":"$--foreground","textGrowth":"fixed-width","width":"fill_container","content":"This Mac","fontFamily":"$--font-ui","fontSize":"$--text-title","fontWeight":"600"},{"id":"k09v3","type":"ref","ref":"hideui:Nyvom","name":"Add project","descendants":{"hideui:ZIZFR":{"fill":"$--subtle-foreground","icon":"plus"}}},{"id":"CY50L","type":"ref","ref":"hideui:Nyvom","name":"Search","descendants":{"hideui:ZIZFR":{"fill":"$--subtle-foreground","icon":"search"}}}]},{"type":"frame","id":"USpqd","x":0,"y":32,"name":"Sidebar tab strip","width":292,"height":32,"stroke":"$--border","strokeWidth":{"bottom":"$--size-hairline"},"strokeAlignment":"inner","gap":8,"padding":[0,12],"alignItems":"center","children":[{"type":"text","id":"W0Y3y","name":"Selected tab","fill":"$--foreground","content":"Projects","fontFamily":"$--font-ui","fontSize":"$--text-body","fontWeight":"normal"},{"type":"text","id":"ukkoQ","name":"Other tab","fill":"$--muted-foreground","content":"Agents","fontFamily":"$--font-ui","fontSize":"$--text-body","fontWeight":"normal"}]},{"type":"frame","id":"T6Nm2","x":4,"y":68,"name":"Home","width":284,"height":36,"gap":8,"padding":[0,8],"alignItems":"center","children":[{"type":"icon","id":"V84dHa","name":"Home icon","width":14,"height":14,"icon":"house","library":"lucide","fill":"$--subtle-foreground"},{"type":"text","id":"F2k1Um","name":"Home title","fill":"$--foreground","textGrowth":"fixed-width","width":"fill_container","content":"Home","fontFamily":"$--font-ui","fontSize":"$--text-title","fontWeight":"600"},{"type":"text","id":"OrXCs","name":"Project count","fill":"$--muted-foreground","content":"0 projects","fontFamily":"$--font-ui","fontSize":"$--text-body","fontWeight":"normal"}]},{"type":"text","id":"vWmVO","x":12,"y":110,"name":"Recent activity","fill":"$--muted-foreground","content":"Projects · Recent activity · 1","fontFamily":"$--font-ui","fontSize":"$--text-micro","fontWeight":"600"},{"type":"frame","id":"RBYXs","x":4,"y":124,"name":"Selected checkout","width":284,"height":36,"fill":"$--secondary","cornerRadius":"$--radius-sm","gap":8,"padding":[0,8],"alignItems":"center","children":[{"type":"icon","id":"tPXqp","name":"Checkout folder","width":14,"height":14,"icon":"folder","library":"lucide","fill":"$--subtle-foreground"},{"type":"text","id":"aYWyU","name":"Checkout label","fill":"$--foreground","content":"fixture","fontFamily":"$--font-ui","fontSize":"$--text-body","fontWeight":"600"}]},{"type":"frame","id":"QuMHI","x":0,"y":840,"name":"Sidebar footer","width":292,"height":32,"stroke":"$--border","strokeWidth":{"top":1},"strokeAlignment":"inner","gap":8,"padding":[0,12],"justifyContent":"end","alignItems":"center","children":[{"id":"xaaDR","type":"ref","ref":"hideui:Nyvom","name":"Background usage","descendants":{"hideui:ZIZFR":{"fill":"$--subtle-foreground","icon":"activity"}}},{"id":"M3iGzm","type":"ref","ref":"hideui:Nyvom","name":"Settings","descendants":{"hideui:ZIZFR":{"fill":"$--subtle-foreground","icon":"settings"}}}]}]},{"id":"Uw8N7","type":"ref","ref":"hideui:VMZTz","name":"Shared Workspace toolbar","fill":"$--sidebar","stroke":"$--border","x":340,"y":28,"width":1100,"height":32,"descendants":{"hideui:bkh81":{"fill":"$--subtle-foreground","content":"Home  /  fixture  /  main"},"hideui:Kjfje/hideui:side-panel-tools-toggle/hideui:ZIZFR":{"fill":"$--subtle-foreground"}}},{"id":"ejdE4","type":"ref","ref":"hideui:rnWpl","name":"Agent column","fill":"$--background","x":340,"y":60,"width":1052,"height":840,"descendants":{"hideui:m3nqv":{"fill":"$--card","stroke":"$--border"},"hideui:Yf5g6":{"fill":"$--secondary"},"hideui:Yf5g6/hideui:view-tab-mark":{"fill":"$--subtle-foreground"},"hideui:Yf5g6/hideui:view-tab-title":{"fill":"$--foreground"},"hideui:Yf5g6/hideui:view-tab-strike":{"fill":"$--muted-foreground"},"hideui:Yf5g6/hideui:view-tab-dirty":{"fill":"$--warning"},"hideui:Yf5g6/hideui:view-tab-close/hideui:ZIZFR":{"fill":"$--subtle-foreground"},"hideui:ujfpg/hideui:ZIZFR":{"fill":"$--subtle-foreground"},"hideui:GYdsi/hideui:ZIZFR":{"fill":"$--subtle-foreground"},"hideui:jEy0G":{"fill":"$--background","width":1100},"hideui:blLvs":{"fill":"$--secondary","stroke":"$--border"},"hideui:W2jHfs":{"fill":"$--subtle-foreground"},"hideui:v57l6H":{"fill":"$--foreground"},"hideui:CG4wC":{"fill":"$--muted-foreground"},"hideui:K8ikC/hideui:ZIZFR":{"fill":"$--subtle-foreground"},"hideui:a63n6t/hideui:ZIZFR":{"fill":"$--subtle-foreground"},"hideui:jPwvc":{"fill":"$--background"},"hideui:PNaQy":{"fill":"$--foreground"},"hideui:aTsYy":{"fill":"$--background","stroke":"$--border","enabled":false,"width":0,"height":0},"hideui:lFQa0":{"fill":"$--card","stroke":"$--border"},"hideui:ljIwg":{"fill":"$--subtle-foreground"},"hideui:e7uYn":{"fill":"$--foreground"},"hideui:uJYZa":{"fill":"$--muted-foreground"},"hideui:x9QjtJ/hideui:ZIZFR":{"fill":"$--subtle-foreground"},"hideui:E0Mk6/hideui:ZIZFR":{"fill":"$--subtle-foreground"},"hideui:M6Mnyn":{"fill":"$--background"},"hideui:QSc1K":{"fill":"$--foreground"}}},{"id":"es3RE-rail","type":"ref","ref":"hideui:lane-rail","name":"Tool rail","x":1392,"y":60,"height":840},{"type":"frame","id":"Y0b444","x":1112,"y":60,"name":"Running servers picker","width":320,"fill":"$--popover","cornerRadius":"$--radius-md","stroke":"$--border","strokeWidth":1,"strokeAlignment":"inner","layout":"vertical","gap":"$--spacing-xs","padding":"$--spacing-sm","children":[{"type":"text","id":"eydR1","name":"Running servers","fill":"$--popover-foreground","content":"Running servers","fontFamily":"$--font-ui","fontSize":"$--text-caption","fontWeight":"500"},{"id":"UX8VT","type":"ref","ref":"hideui:btn-m","name":"127.0.0.1:3000","width":"fill_container","height":28,"justifyContent":"start","gap":"$--spacing-xs","padding":[0,"$--spacing-md"],"fill":"$--accent","stroke":"$--ring","strokeWidth":1,"strokeAlignment":"outer","descendants":{"hideui:btn-ic":{"enabled":true,"icon":"globe","fill":"$--popover-foreground"},"hideui:btn-lb":{"content":"127.0.0.1:3000","fontFamily":"$--font-mono","fontSize":"$--text-caption","fill":"$--popover-foreground"}}},{"id":"zn5Ax","type":"ref","ref":"hideui:btn-m","name":"[::1]:5173","width":"fill_container","height":28,"justifyContent":"start","gap":"$--spacing-xs","padding":[0,"$--spacing-md"],"fill":[],"stroke":[],"strokeWidth":0,"strokeAlignment":"outer","descendants":{"hideui:btn-ic":{"enabled":true,"icon":"globe","fill":"$--popover-foreground"},"hideui:btn-lb":{"content":"[::1]:5173","fontFamily":"$--font-mono","fontSize":"$--text-caption","fill":"$--popover-foreground"}}}]}]}]}
  ];
}

// -- Screen / Settings ----------------------------------------------------------

const SETTINGS_SPEC = 'web/src/SettingsSheet.tsx (a Dialog), settings.ts SETTINGS_GROUPS, settings-rows.tsx Group/Row and the pages under web/src/settings/ (PRD settings-left-nav B1-B15): the ⌘, sheet, 880 wide, with a 200 list on its left in place of the tab strip, App (General, Notifications, Usage, Devices, Shortcuts) and This Mac (Agents, Hide AI, Mobile, GitHub), About at the list\u2019s foot, and the chosen page on its right under its title alone, without a description line. General holds Appearance alone; Notifications holds This Mac\u2019s desktop notifications and the Phone push; GitHub and About are pages of their own; Agents keeps its machine switch inside the page. Copy is the shipped catalog; values are invented.';

// Each board is a captioned section in one column, built by pen-screens-settings.mjs.
function buildSettings(tokens, root) {
  const boards = settingsBoards(tokens, root, {themedXref, screenIconButton, screenSwatch});
  const build = suffix => [frame(`set-column-${suffix}`, 'Boards', {layout: 'vertical', gap: '$--spacing-xxl'},
    boards.map(board => frame(`${board.id}-${suffix}`, board.name, {layout: 'vertical', gap: '$--spacing-md'}, [
      text(`${board.id}-${suffix}-title`, board.name, {size: '$--text-title', weight: '600'}),
      text(`${board.id}-${suffix}-spec`, board.spec, {size: '$--text-caption', fill: '$--muted-foreground', width: 1100}),
      ...board.build(suffix),
    ])))];
  return screenSheet('screen-settings', 'Screen / Settings', SETTINGS_SPEC, build, build);
}

// -- Screen / Palette ------------------------------------------------------------

// The ⌘K palette (PRD cmdk-navigation) and the ⌘P file palette on the same
// shell: the sidebar's Search row that opens ⌘K, the query row with its Esc
// keycap, two-line rows with the agent's status mark and ↵ on the selected row,
// and for ⌘K a detail beside the list. Rows are authored here on local tokens, as the
// Overview board authors its agent rows, since no library master draws a
// palette row; the keycaps are System / Kbd refs. An agent's row leads with its
// status mark and draws no provider logo (PRD agent-mark-quiet D-09).
function buildPalette(tokens) {
  const W = num(tokens, '--size-search-sheet-w');
  const SIDEBAR = num(tokens, '--size-sidebar-ideal');
  const MARK = num(tokens, '--size-agent-badge-compact');
  const ROW = W - 2 * num(tokens, '--spacing-xxs');
  function build(suffix) {
    const id = (name) => `pal-${name}-${suffix}`;
    const kbd = (key, label) => themedXref(id(key), 'kbd-m', label, {}, {'kbd-t': {content: label}});
    // An agent's result leads with its status mark in the icon slot, and no provider logo (PRD agent-mark-quiet D-09).
    const mark = (key, symbol, fill) => frame(id(key), 'Status mark slot', {width: MARK, height: MARK, layout: 'horizontal', alignItems: 'center', justifyContent: 'center'}, [screenStatusMark(tokens, `${id(key)}-s`, symbol, fill)]);
    const glyph = (key, name) => frame(id(key), 'Icon', {width: MARK, height: MARK, layout: 'horizontal', alignItems: 'center', justifyContent: 'center'}, [
      icon(`${id(key)}-i`, name, {size: num(tokens, '--size-icon'), fill: '$--muted-foreground'}),
    ]);
    function row(key, {lead, title, detail, mono = false, selected = false, unavailable = false}) {
      return frame(id(key), selected ? 'Row · selected' : 'Row', {
        layout: 'horizontal', gap: '$--spacing-sm', alignItems: 'center', width: ROW, padding: ['$--spacing-xs', '$--spacing-sm'],
        cornerRadius: '$--radius-xs', ...(selected ? {fill: '$--accent'} : {}), ...(unavailable ? {opacity: disabledOpacity} : {}),
      }, [
        lead,
        frame(`${id(key)}-body`, 'Body', {layout: 'vertical', gap: 0, width: 'fill_container'}, [
          text(`${id(key)}-title`, title, {weight: '500', fill: unavailable ? '$--muted-foreground' : '$--foreground'}),
          ...(detail ? [text(`${id(key)}-detail`, detail, {size: '$--text-caption', fill: '$--muted-foreground', mono})] : []),
        ]),
        ...(selected ? [text(`${id(key)}-enter`, '↵', {size: '$--text-caption', fill: '$--muted-foreground'})] : []),
      ]);
    }
    const heading = (key, label) => frame(id(key), 'Group heading', {padding: ['$--spacing-xs', '$--spacing-sm'], width: ROW}, [
      text(`${id(key)}-t`, label, {size: '$--text-caption', weight: '500', fill: '$--muted-foreground'}),
    ]);
    function surface(key, {query, placeholder, list}) {
      return frame(id(key), 'Palette', {width: W, cornerRadius: '$--radius-lg', fill: '$--popover', stroke: '$--border', strokeWidth: '$--size-hairline', strokeAlignment: 'inner', layout: 'vertical', clip: true}, [
        frame(`${id(key)}-in`, 'Query', {height: num(tokens, '--size-control-lg'), width: W, layout: 'horizontal', alignItems: 'center', gap: '$--spacing-sm', padding: [0, '$--spacing-md'], stroke: '$--border', strokeWidth: {bottom: num(tokens, '--size-hairline')}}, [
          icon(`${id(key)}-ini`, 'search', {size: num(tokens, '--size-icon'), fill: '$--muted-foreground'}),
          frame(`${id(key)}-inf`, 'Field', {width: 'fill_container'}, [
            query ? text(`${id(key)}-inq`, query) : text(`${id(key)}-inp`, placeholder, {fill: '$--muted-foreground'}),
          ]),
          kbd(`${key}-esc`, 'Esc'),
        ]),
        frame(`${id(key)}-list`, 'List', {layout: 'vertical', gap: 0, padding: '$--spacing-xxs'}, list),
      ]);
    }
    const empty = (key, words) => frame(id(key), 'Empty', {padding: ['$--spacing-sm', '$--spacing-md']}, [
      text(`${id(key)}-t`, words, {size: '$--text-caption', fill: '$--muted-foreground'}),
    ]);
    const labelled = (key, label, node) => frame(id(key), label, {layout: 'vertical', gap: '$--spacing-xs'}, [
      text(`${id(key)}-cap`, label, {size: '$--text-micro', weight: '600', fill: '$--muted-foreground'}),
      node,
    ]);

    // The sidebar's Search row under the window's top row reads Search and the
    // bound chord, and opens ⌘K (PRD sidebar-titlebar-refresh D-08).
    const sidebar = frame(id('side'), 'Sidebar', {width: SIDEBAR, layout: 'vertical', gap: '$--spacing-xs', fill: '$--sidebar', padding: [0, 0, '$--spacing-md', 0], cornerRadius: '$--radius-md'}, [
      themedXref(id('side-search'), 'sbc-search', 'Search row', {width: SIDEBAR}),
      frame(id('side-sectwrap'), 'Section', {padding: ['$--spacing-sm', '$--spacing-md', 0, '$--spacing-md']}, [
        text(id('side-sect'), 'NEEDS YOU · 1', {size: '$--text-micro', fill: '$--muted-foreground'}),
      ]),
      frame(id('side-a0'), 'Agent row', {layout: 'horizontal', gap: '$--spacing-xs', alignItems: 'center', padding: ['$--spacing-xs', '$--spacing-md']}, [
        screenStatusMark(tokens, id('side-a0s'), '?', '$--warning'),
        text(id('side-a0t'), 'mailbox 원격 에이전트 구현', {weight: '500'}),
      ]),
    ]);

    // ⌘K's own wide layout (PRD cmdk-navigation D-01): the list at the shared
    // palette width and the highlighted row's detail beside it.
    const WIDE = Math.round(W * 1.5);
    const DETAIL = WIDE - W;
    const pill = (key, label, fill = '$--secondary', color = '$--subtle-foreground') => frame(id(key), 'Pill', {padding: [0, '$--spacing-sm'], cornerRadius: '$--radius-sm', fill}, [
      text(`${id(key)}-t`, label, {size: '$--text-caption', fill: color}),
    ]);
    const status = (key, label, color) => text(id(key), label, {size: '$--text-caption', fill: color});
    function wide(key, {query, placeholder, list, detail, footer}) {
      const hasBody = list !== null;
      return frame(id(key), 'Palette · wide', {width: WIDE, cornerRadius: '$--radius-lg', fill: '$--popover', stroke: '$--border', strokeWidth: '$--size-hairline', strokeAlignment: 'inner', layout: 'vertical', clip: true}, [
        frame(`${id(key)}-in`, 'Query', {height: num(tokens, '--size-control-lg'), width: WIDE, layout: 'horizontal', alignItems: 'center', gap: '$--spacing-sm', padding: [0, '$--spacing-md'], ...(hasBody ? {stroke: '$--border', strokeWidth: {bottom: num(tokens, '--size-hairline')}} : {})}, [
          icon(`${id(key)}-ini`, 'search', {size: num(tokens, '--size-icon'), fill: '$--muted-foreground'}),
          frame(`${id(key)}-inf`, 'Field', {width: 'fill_container'}, [
            query ? text(`${id(key)}-inq`, query) : text(`${id(key)}-inp`, placeholder, {fill: '$--muted-foreground'}),
          ]),
          kbd(`${key}-esc`, 'Esc'),
        ]),
        ...(hasBody ? [
          frame(`${id(key)}-body`, 'Body', {layout: 'horizontal', width: WIDE}, [
            frame(`${id(key)}-list`, 'List', {layout: 'vertical', gap: 0, padding: '$--spacing-xxs', width: W, stroke: '$--border', strokeWidth: {right: num(tokens, '--size-hairline')}}, list),
            frame(`${id(key)}-detail`, 'Detail', {layout: 'vertical', gap: '$--spacing-sm', padding: '$--spacing-md', width: DETAIL}, detail),
          ]),
          frame(`${id(key)}-foot`, 'Footer', {layout: 'horizontal', gap: '$--spacing-md', alignItems: 'center', width: WIDE, padding: ['$--spacing-xxs', '$--spacing-md'], stroke: '$--border', strokeWidth: {top: num(tokens, '--size-hairline')}}, footer ?? [
            text(`${id(key)}-f1`, '↑ ↓ 이동', {size: '$--text-caption', fill: '$--muted-foreground'}),
            text(`${id(key)}-f2`, '↵ 열기', {size: '$--text-caption', fill: '$--muted-foreground'}),
            text(`${id(key)}-f3`, 'Esc 닫기', {size: '$--text-caption', fill: '$--muted-foreground'}),
          ]),
        ] : []),
      ]);
    }
    const detailLines = (key, kind, title, pills, facts, related) => [
      text(`${id(key)}-k`, kind, {size: '$--text-caption', weight: '600', fill: '$--muted-foreground'}),
      text(`${id(key)}-t`, title, {weight: '600'}),
      frame(`${id(key)}-p`, 'Pills', {layout: 'horizontal', gap: '$--spacing-xs'}, pills),
      ...facts.map(([label, value], i) => frame(`${id(key)}-fact${i}`, 'Fact', {layout: 'horizontal', gap: '$--spacing-md'}, [
        text(`${id(key)}-fl${i}`, label, {size: '$--text-caption', fill: '$--muted-foreground'}),
        text(`${id(key)}-fv${i}`, value, {size: '$--text-caption'}),
      ])),
      ...(related ? [text(`${id(key)}-rh`, '관계', {size: '$--text-caption', weight: '600', fill: '$--muted-foreground'}), ...related.map(([label, depth, bold], i) => frame(`${id(key)}-rel${i}`, 'Relation', {padding: [0, 0, 0, depth * 12]}, [
        text(`${id(key)}-rl${i}`, label, {size: '$--text-caption', weight: bold ? '600' : '400', fill: bold ? '$--foreground' : '$--subtle-foreground'}),
      ]))] : []),
    ];

    // Empty query in front of an agent: the Overview's grouping, issues on
    // top, the checkout's group with its pull request and lineage, a parent
    // elsewhere as one `↑ 부모` line.
    const relations = wide('related', {placeholder: '이름이나 #번호를 입력하세요', list: [
      heading('g-related', 'Related'),
      row('q0', {lead: glyph('q0i', 'circle-dot'), title: '#273 mailbox 쓰기 명령이 sandbox 거부를 internal로 숨김', detail: 'Issue · hide', selected: true}),
      row('q1', {lead: glyph('q1i', 'git-branch'), title: 'fix/mailbox-sandbox-letters', detail: 'hide'}),
      row('q2', {lead: glyph('q2i', 'git-pull-request'), title: '#275 Surface mailbox sandbox refusals', detail: 'PR · Open · fix/mailbox-sandbox-letters'}),
      row('q3', {lead: mark('q3m', '✓', '$--success'), title: 'mailbox 쓰기 명령 sandbox 오류 해결', detail: 'hide › fix/mailbox-sandbox-letters · Done'}),
      row('q4', {lead: mark('q4m', '●', '$--agent-working'), title: '↑ 부모 codex workspace-write 원인 조사', detail: 'hide › main · Working'}),
    ], detail: detailLines('rel-d', 'Issue', 'mailbox 쓰기 명령이 sandbox 거부를 internal로 숨김', [pill('rel-p1', 'Open', '$--secondary', '$--success'), pill('rel-p2', '#273')], [['프로젝트', 'hide'], ['맡은 곳', 'fix/mailbox-sandbox-letters'], ['닫는 PR', '#275'], ['읽음', '4분 전 읽음']], [['#273 mailbox 쓰기 명령이 sandbox 거부를…', 0, true], ['fix/mailbox-sandbox-letters', 0, false], ['#275 Surface mailbox sandbox refusals', 1, false]])});

    const typed = wide('typed', {query: 'sand', list: [
      heading('g-issues', 'Issues'),
      row('t0', {lead: glyph('t0i', 'circle-dot'), title: '#273 mailbox 쓰기 명령이 sandbox 거부를 internal로 숨김', detail: 'Issue · hide'}),
      heading('g-agents', 'Agents'),
      row('t1', {lead: mark('t1m', '✓', '$--success'), title: 'mailbox 쓰기 명령 sandbox 오류 해결', detail: 'hide › fix/mailbox-sandbox-letters · Done', selected: true}),
      heading('g-checkouts', 'Checkouts'),
      row('t2', {lead: glyph('t2i', 'git-branch'), title: 'hide / fix/mailbox-sandbox-letters', detail: '~/projects/hide.worktrees/sandbox', mono: true}),
      frame(id('t-gh'), 'GitHub row', {layout: 'horizontal', gap: '$--spacing-sm', alignItems: 'center', width: ROW, padding: ['$--spacing-xs', '$--spacing-sm']}, [
        glyph('t-ghi', 'search'),
        text(id('t-ght'), 'GitHub에서 "sand" 검색', {weight: '500'}),
      ]),
    ], detail: detailLines('typed-d', 'Agent · Claude', 'mailbox 쓰기 명령 sandbox 오류 해결', [pill('typed-p1', 'Done', '$--secondary', '$--success'), pill('typed-p2', 'This Mac')], [['checkout', 'hide › fix/mailbox-sandbox-letters']], null)});

    const number = wide('number', {query: '#275', list: [
      heading('g-prs', 'Pull requests'),
      row('n0', {lead: glyph('n0i', 'git-pull-request'), title: '#275 Surface mailbox sandbox refusals', detail: 'PR · Open · fix/mailbox-sandbox-letters', selected: true}),
    ], detail: detailLines('number-d', 'Pull request', 'Surface mailbox sandbox refusals', [pill('number-p1', 'Open', '$--secondary', '$--success'), pill('number-p2', 'CI 진행 중', '$--secondary', '$--warning'), pill('number-p3', '#275')], [['Review', '리뷰 필요'], ['브랜치', 'fix/mailbox-sandbox-letters'], ['닫는 이슈', '#273']], null)});

    const collapsed = wide('collapsed', {placeholder: '이름이나 #번호를 입력하세요', list: null});

    const ghRow = (key, label, lead) => frame(id(key), 'GitHub row', {layout: 'horizontal', gap: '$--spacing-sm', alignItems: 'center', width: ROW, padding: ['$--spacing-xs', '$--spacing-sm'], fill: '$--accent', cornerRadius: '$--radius-xs'}, [
      lead,
      text(id(`${key}-t`), label, {weight: '500', fill: '$--foreground'}),
    ]);
    const ghDetail = (key) => [
      text(`${id(key)}-k`, 'GitHub', {size: '$--text-caption', weight: '600', fill: '$--muted-foreground'}),
      text(`${id(key)}-t`, '"quota wall"', {weight: '600'}),
      text(`${id(key)}-d`, '이 Mac의 GitHub 프로젝트 저장소에서 PR과 이슈를 한 번 검색합니다. 입력하는 동안에는 GitHub를 부르지 않습니다.', {size: '$--text-caption', fill: '$--muted-foreground', width: DETAIL - 2 * num(tokens, '--spacing-md')}),
    ];
    const ghWorking = wide('gh-working', {query: 'quota wall', list: [
      empty('gh-working-e', '일치하는 항목 없음'),
      ghRow('gh-working-r', 'GitHub에서 "quota wall" 검색', icon(`${id('gh-working-ri')}`, 'loader-circle', {size: num(tokens, '--size-icon-sm'), fill: '$--muted-foreground'})),
    ], detail: ghDetail('gh-working-d')});
    const ghResults = wide('gh-results', {query: 'quota wall', list: [
      heading('g-github', 'GitHub'),
      row('gr0', {lead: glyph('gr0i', 'git-pull-request'), title: '#118 Close stale sandbox watches', detail: 'PR · acme/hide · Merged', selected: true}),
      row('gr1', {lead: glyph('gr1i', 'circle-dot'), title: '#96 mailbox sandbox 거부 로그가 비어 있음', detail: 'Issue · acme/hide · Closed'}),
    ], detail: detailLines('gh-results-d', 'Pull request · GitHub', 'Close stale sandbox watches', [pill('gh-results-p1', 'Merged'), pill('gh-results-p2', '#118'), pill('gh-results-p3', 'acme/hide')], [], null)});
    const ghFailed = wide('gh-failed', {query: 'quota wall', list: [
      empty('gh-failed-e', '일치하는 항목 없음'),
      ghRow('gh-failed-r', 'GitHub 검색 실패 · 다시 시도', icon(`${id('gh-failed-ri')}`, 'triangle-alert', {size: num(tokens, '--size-icon'), fill: '$--warning'})),
    ], detail: ghDetail('gh-failed-d')});

    const noMatch = surface('nomatch', {query: 'zzz', list: [empty('nomatch-e', '일치하는 항목 없음')]});
    const files = surface('files', {placeholder: 'Search files by name', list: [
      row('f0', {lead: glyph('f0i', 'file-text'), title: 'docs/한글 노트.md', selected: true}),
      row('f1', {lead: glyph('f1i', 'file-code'), title: 'scripts/pen-screens.mjs'}),
    ]});
    const filesCell = frame(id('files-wrap'), 'Files', {layout: 'vertical', gap: '$--spacing-xs'}, [
      files,
      text(id('files-hint'), '⌘↵ 옆에 열기', {size: '$--text-caption', fill: '$--muted-foreground'}),
    ]);

    return [
      labelled('side-cell', 'Sidebar · the Search row opens ⌘K', sidebar),
      frame(id('wide-states'), 'Wide states', {layout: 'vertical', gap: '$--spacing-lg'}, [
        labelled('related-cell', '⌘K · empty, in front of an agent: relations', relations),
        labelled('typed-cell', '⌘K · typed', typed),
        labelled('number-cell', '⌘K · #number', number),
        labelled('collapsed-cell', '⌘K · nothing in front: the input alone', collapsed),
        labelled('ghw-cell', '⌘K · GitHub searching', ghWorking),
        labelled('ghr-cell', '⌘K · GitHub results', ghResults),
        labelled('ghf-cell', '⌘K · GitHub failed', ghFailed),
      ]),
      frame(id('states'), 'States', {layout: 'vertical', gap: '$--spacing-lg'}, [
        labelled('nomatch-cell', '⌘K · narrow window or no detail: no match', noMatch),
        labelled('files-cell', '⌘P · same shell, ⌘↵ opens beside', filesCell),
      ]),
    ];
  }
  return screenSheet('screen-palette', 'Screen / Palette', 'web/src/SearchPalette.tsx and web/src/Palette.tsx over Command/CommandDialog (PRD cmdk-navigation): the sidebar’s Search row with its ⌘K chord, ⌘K’s wide list-and-detail layout (empty in front of an agent drawn as the Overview groups it, typed results by group, #number, the input alone, GitHub searching, results and failed) and the ⌘P file palette whose ⌘↵ opens beside.', build, build);
}

// -- Screen / Dialogs and Sheets -------------------------------------------------

function buildDialogs(tokens) {
  const H = num(tokens, '--size-control');
  function build(suffix) {
    // WorkspaceDialogs.tsx's worktree form: Branch (Input), Base (Select),
    // "Start in the new pane" (a plain radio choice, Terminal only/Claude/Codex),
    // Purpose (optional, Input) - not a bare title, the fields are the dialog.
    const newWorktree = screenDialogSurface(`dlg-neww-${suffix}`, {
      width: 380, title: 'New worktree in sasu-web-design-system-reset', description: '~/projects/sasu/worktrees/web-design-system-reset',
      body: [
        screenField(`dlg-neww-branch-${suffix}`, 'Branch', screenInput(`dlg-neww-branchv-${suffix}`, {placeholder: 'feature/name', mono: true, width: 348})),
        screenField(`dlg-neww-base-${suffix}`, 'Base', screenSelect(`dlg-neww-basev-${suffix}`, {content: 'main', width: 348})),
        screenField(`dlg-neww-agent-${suffix}`, 'Start in the new pane', frame(`dlg-neww-agentrow-${suffix}`, 'Row', {layout: 'horizontal', gap: '$--spacing-md'}, [
          screenRadioItem(`dlg-neww-agent0-${suffix}`, 'Terminal only', true),
          screenRadioItem(`dlg-neww-agent1-${suffix}`, 'Claude', false),
          screenRadioItem(`dlg-neww-agent2-${suffix}`, 'Codex', false),
        ])),
        screenField(`dlg-neww-purpose-${suffix}`, 'Purpose (optional)', screenInput(`dlg-neww-purposev-${suffix}`, {placeholder: '', width: 348})),
      ],
      actions: [screenButton(`dlg-neww-cancel-${suffix}`, 'Cancel', {variant: 'ghost', height: H}), screenButton(`dlg-neww-ok-${suffix}`, 'Create worktree', {height: H})],
    });
    const deleteWorktree = screenDialogSurface(`dlg-delw-${suffix}`, {
      width: 380, title: 'Delete worktree prd/checkout-row-d?', description: '~/projects/sasu/worktrees/checkout-row-d',
      actions: [screenButton(`dlg-delw-cancel-${suffix}`, 'Keep worktree', {variant: 'secondary', height: H}), screenButton(`dlg-delw-ok-${suffix}`, 'Delete worktree', {variant: 'destructive', height: H})],
    });
    const removeProject = screenDialogSurface(`dlg-remp-${suffix}`, {
      width: 380, title: 'Remove project sasu-web-design-system-reset?', description: '~/projects/sasu/worktrees/web-design-system-reset',
      actions: [screenButton(`dlg-remp-cancel-${suffix}`, 'Keep project', {variant: 'secondary', height: H}), screenButton(`dlg-remp-ok-${suffix}`, 'Close 3 panes and remove', {variant: 'destructive', height: H})],
    });
    return [
      frame(`dlg-row1-${suffix}`, 'Row 1', {layout: 'horizontal', gap: '$--spacing-lg'}, [newWorktree, deleteWorktree, removeProject]),
    ];
  }
  function build2(suffix) {
    return [frame(`dlg-row2-${suffix}`, 'Row 2', {layout: 'horizontal', gap: '$--spacing-lg'}, [
      screenDialogSurface(`dlg-purp2-${suffix}`, {width: 380, title: 'Purpose', description: 'What this workspace is for', actions: [screenButton(`dlg-purp2-cancel-${suffix}`, 'Cancel', {variant: 'ghost', height: H}), screenButton(`dlg-purp2-ok-${suffix}`, 'Save', {height: H})]}),
      screenDialogSurface(`dlg-draft2-${suffix}`, {width: 380, title: 'Unsaved drafts', actions: [screenButton(`dlg-draft2-discard-${suffix}`, 'Discard drafts', {variant: 'destructive', height: H}), screenButton(`dlg-draft2-ok-${suffix}`, 'Keep and continue', {height: H})]}),
      screenDialogSurface(`dlg-neww3-${suffix}`, {width: 380, title: 'New workspace', description: '~/…', actions: [screenButton(`dlg-neww3-cancel-${suffix}`, 'Cancel', {variant: 'ghost', height: H}), screenButton(`dlg-neww3-ok-${suffix}`, 'Add workspace', {height: H})]}),
      screenDialogSurface(`dlg-short2-${suffix}`, {width: 260, title: 'Keyboard shortcuts', actions: [screenButton(`dlg-short2-close-${suffix}`, 'Close', {variant: 'ghost', height: H})]}),
    ])];
  }
  // IssueDialogs.tsx (the issue-first Overview): New issue and Start from an
  // issue, each --size-worktree-dialog wide with a body-size label above each
  // control and its note at the label's right (Field), the submit button's
  // ⌘↵ keycap beside it.
  const W = num(tokens, '--size-worktree-dialog');
  const inner = W - 2 * num(tokens, '--spacing-lg');
  const LINE = 18;
  function issueField(id, label, control, aside = []) {
    return frame(id, 'Field', {layout: 'vertical', gap: '$--spacing-xxs', width: inner}, [
      frame(`${id}-lr`, 'Label', {layout: 'horizontal', gap: '$--spacing-xxs', alignItems: 'center', width: inner}, [
        text(`${id}-l`, label, {fill: '$--subtle-foreground'}),
        ...(aside.length ? [frame(`${id}-sp`, 'Spacer', {width: 'fill_container', height: 1}, []), ...aside] : []),
      ]),
      control,
    ]);
  }
  // The text field's look over `rows` rows; IssueDialogs.tsx draws it on a
  // plain textarea, so it is authored here on local tokens.
  function textArea(id, content, rows) {
    const padX = num(tokens, '--spacing-sm'), padY = num(tokens, '--spacing-xs');
    return frame(id, 'Text area', {layout: 'vertical', width: inner, height: rows * LINE + 2 * padY, padding: [padY, padX], cornerRadius: '$--radius-sm', fill: '$--background', stroke: '$--input', strokeWidth: '$--size-hairline', strokeAlignment: 'inner', clip: true}, [
      text(`${id}-t`, content, {width: inner - 2 * padX}),
    ]);
  }
  function checkbox(id, label, checked) {
    return frame(id, label, {layout: 'horizontal', gap: '$--spacing-xs', alignItems: 'center'}, [
      themedXref(`${id}-box`, 'chk-m', label, checked ? {fill: '$--primary', stroke: '$--primary'} : {}, {'chk-i': {enabled: checked}}),
      text(`${id}-t`, label),
    ]);
  }
  function submit(id, label) {
    return frame(id, label, {layout: 'horizontal', gap: '$--spacing-xs', alignItems: 'center'}, [
      screenButton(`${id}-b`, label, {height: H}),
      themedXref(`${id}-k`, 'kbd-m', '⌘↵', {}, {'kbd-t': {content: '⌘↵'}}),
    ]);
  }
  function newIssue(id, {where, title, body, start}) {
    return screenDialogSurface(id, {
      width: W, title: '새 이슈',
      body: [
        issueField(`${id}-f-where`, '어디에', screenSelect(`${id}-f-wherev`, {content: where, width: inner})),
        issueField(`${id}-f-title`, '제목', screenInput(`${id}-f-titlev`, {content: title, width: inner})),
        issueField(`${id}-f-body`, '내용', textArea(`${id}-f-bodyv`, body, 4)),
        checkbox(`${id}-f-start`, '만들고 바로 시작', start),
      ],
      actions: [screenButton(`${id}-cancel`, '취소', {variant: 'secondary', height: H}), submit(`${id}-ok`, start ? '만들고 시작…' : '이슈 만들기')],
    });
  }
  function startIssue(id) {
    const agents = frame(`${id}-agentrow`, 'Agents', {layout: 'horizontal', gap: '$--spacing-md', alignItems: 'center', height: H}, [
      screenRadioItem(`${id}-agent0`, '터미널만', false),
      screenRadioItem(`${id}-agent1`, 'Claude', true),
      screenRadioItem(`${id}-agent2`, 'Codex', false),
    ]);
    return screenDialogSurface(id, {
      width: W, title: '#192 작업 시작', titleGlyph: 'circle-dot', prose: true,
      description: 'hided has no SIGTERM handler: AI children die with the stdin pipe',
      body: [
        issueField(`${id}-name`, '워크트리 · 브랜치', screenInput(`${id}-namev`, {content: '192-hided-sigterm-handler', mono: true, width: inner}), [
          icon(`${id}-name-ai`, 'sparkles', {size: 12, fill: '$--primary'}),
          text(`${id}-name-note`, 'AI 지음 · 고칠 수 있음', {size: '$--text-caption', fill: '$--muted-foreground'}),
        ]),
        frame(`${id}-grid`, 'Base and agent', {layout: 'horizontal', gap: '$--spacing-md', alignItems: 'end', width: inner}, [
          frame(`${id}-base`, 'Field', {layout: 'vertical', gap: '$--spacing-xxs', width: 'fill_container'}, [
            text(`${id}-base-l`, '기준', {fill: '$--subtle-foreground'}),
            screenSelect(`${id}-basev`, {content: 'main', width: 180}),
          ]),
          frame(`${id}-agent`, 'Field', {layout: 'vertical', gap: '$--spacing-xxs'}, [
            text(`${id}-agent-l`, '에이전트', {fill: '$--subtle-foreground'}),
            agents,
          ]),
        ]),
        issueField(`${id}-prompt`, '첫 지시', textArea(`${id}-promptv`, 'Issue #192를 해결해줘: hided has no SIGTERM handler: AI children die with the stdin pipe\n\n완료되면 이 이슈를 닫는 PR을 열어줘 (PR 본문에 Closes #192).', 5), [
          text(`${id}-prompt-note`, '이슈 본문에서 채움 · 고칠 수 있음', {size: '$--text-caption', fill: '$--muted-foreground'}),
        ]),
      ],
      actions: [screenButton(`${id}-cancel`, '취소', {variant: 'secondary', height: H}), submit(`${id}-ok`, '시작')],
    });
  }
  function build3(suffix) {
    return [frame(`dlg-row3-${suffix}`, 'Row 3 · issues', {layout: 'horizontal', gap: '$--spacing-lg', alignItems: 'start'}, [
      newIssue(`dlg-issue-${suffix}`, {where: 'hide · GitHub modakbul-gongbang/hide', title: 'Overview: Tasks board as the default tab', body: '이슈 우선 흐름: 백로그 → 진행 중 → 리뷰 → 완료.\n카드는 이슈, 에이전트, PR 순서로 읽는다.', start: false}),
      newIssue(`dlg-issuego-${suffix}`, {where: 'creator · Local', title: '릴스 6탄 대본 초안', body: '인터뷰 녹취에서 핵심 세 문장을 뽑는다.', start: true}),
      startIssue(`dlg-start-${suffix}`),
    ])];
  }
  function full(suffix) {
    return [frame(`dlg-rows-${suffix}`, 'Rows', {layout: 'vertical', gap: '$--spacing-xl'}, [...build(suffix), ...build2(suffix), ...build3(suffix)])];
  }
  return screenSheet('screen-dialogs', 'Screen / Dialogs and Sheets', 'web/src/WorkspaceDialogs.tsx, NewWorkspace.tsx, DraftRecovery.tsx, ShortcutSheet.tsx, IssueDialogs.tsx: every Dialog/AlertDialog surface the shell opens, shaped from System / Dialog and System / Alert Dialog with Button refs for every action. The third row is the issue-first Overview’s: New issue (어디에, 제목, 내용 and 만들고 바로 시작, unchecked and then checked, when the button reads 만들고 시작…) and Start from an issue (#192 작업 시작 over the issue’s title, the worktree name the AI wrote and the operator can edit, 기준, the agent, and 첫 지시 filled from the issue).', full, full);
}

// -- Screen / Menus and Overlays --------------------------------------------------

function buildMenus(tokens) {
  const FG = '$--foreground', SUBTLE = '$--subtle-foreground', MUTED = '$--muted-foreground';
  const XS = num(tokens, '--spacing-xs'), SM = num(tokens, '--spacing-sm'), MD = num(tokens, '--spacing-md');
  const at = (node, x, y) => ({...node, x: Math.round(x), y: Math.round(y)});
  const spacer = id => frame(id, 'Spacer', {width: 'fill_container', height: 1}, []);

  // The ⌘N start panel (StartPanel), placed as the ⌘K palette is: the same width,
  // --size-settings-sheet-window-inset from the window top, --popover with a
  // hairline and a shadow. On --popover a --border hairline vanishes in Dark, so
  // its rule, the three dropdowns and the menus stand on --secondary.
  const ASK_W = num(tokens, '--size-search-sheet-w');
  const ASK_TOP = 16;
  const FIELD_H = 88, BAR_H = 40, DD_Y = 7, DD_H = 26;
  const SHADOW = {type: 'shadow', shadowType: 'outer', offset: {x: 0, y: 12}, blur: 32, spread: 0, color: '#00000073'};
  const DD = {target: {x: MD, w: 164}, kind: {x: MD + 164 + SM, w: 112}, model: {x: MD + 164 + SM + 112 + SM, w: 132}};
  const PROMPT = '블로그 초안을 두 가지 톤으로 다시 써서 비교해줘';
  const providerArt = (id, provider, size = 14) => frame(id, `${provider} mark`, {width: size, height: size, fill: {type: 'image', enabled: true, url: `../web/src/assets/agent-${provider}.png`, mode: 'fit'}}, []);
  function dropdown(id, {lead, label, width, open = false}) {
    return frame(id, label, {width, height: DD_H, layout: 'horizontal', gap: 6, alignItems: 'center', padding: [0, SM], cornerRadius: '$--radius-sm', fill: '$--secondary', ...(open ? {stroke: MUTED, strokeWidth: 1, strokeAlignment: 'inner'} : {})}, [
      lead, text(`${id}-t`, label, {size: '$--text-caption'}), spacer(`${id}-sp`), icon(`${id}-c`, open ? 'chevron-up' : 'chevron-down', {size: 12, fill: MUTED}),
    ]);
  }
  function startPanel(p, {open = null} = {}) {
    return frame(`${p}-ask`, '⌘N', {width: ASK_W, layout: 'vertical', fill: '$--popover', cornerRadius: '$--radius-lg', stroke: '$--border', strokeWidth: 1, strokeAlignment: 'inner', clip: true, effect: SHADOW}, [
      frame(`${p}-field`, 'Prompt', {width: ASK_W, height: FIELD_H, padding: [MD, num(tokens, '--spacing-lg')], layout: 'vertical'}, [
        text(`${p}-q`, PROMPT, {size: '$--text-subhead'}),
      ]),
      frame(`${p}-bar`, 'Bar', {width: ASK_W, height: BAR_H, layout: 'none', fill: '$--popover'}, [
        at(frame(`${p}-barrule`, 'Rule', {width: ASK_W, height: 1, fill: '$--secondary'}, []), 0, 0),
        at(dropdown(`${p}-dt`, {lead: icon(`${p}-dtg`, 'folder-git-2', {size: 12, fill: SUBTLE}), label: 'hide · main', width: DD.target.w, open: open === 'target'}), DD.target.x, DD_Y),
        at(dropdown(`${p}-dk`, {lead: providerArt(`${p}-dkg`, 'claude'), label: 'Claude', width: DD.kind.w, open: open === 'kind'}), DD.kind.x, DD_Y),
        at(dropdown(`${p}-dm`, {lead: icon(`${p}-dmg`, 'cpu', {size: 12, fill: SUBTLE}), label: 'opus', width: DD.model.w, open: open === 'model'}), DD.model.x, DD_Y),
        at(themedXref(`${p}-kbd`, 'kbd-m', '⏎', {}, {'kbd-t': {content: '⏎'}}), ASK_W - MD - 56 - SM - 22, 10),
        at(screenButton(`${p}-go`, '시작', {height: DD_H, width: 56}), ASK_W - MD - 56, DD_Y),
      ]),
    ]);
  }
  // A menu row: the lead, the label and the check on the row's right edge; a
  // device that is not connected keeps its rows, dimmed, with 연결 안 됨 under.
  function menuRow(id, {lead, label, checked = false, highlighted = false, detail, width}) {
    return frame(id, label, {width, height: detail ? 40 : 28, layout: 'horizontal', gap: SM, alignItems: 'center', padding: [0, SM], cornerRadius: '$--radius-xs', ...(highlighted ? {fill: '$--accent'} : {})}, [
      lead,
      detail
        ? frame(`${id}-b`, 'Body', {layout: 'vertical', gap: 0}, [text(`${id}-t`, label, {size: '$--text-body', fill: MUTED}), text(`${id}-d`, detail, {size: '$--text-caption', fill: MUTED})])
        : text(`${id}-t`, label, {size: '$--text-body'}),
      spacer(`${id}-sp`),
      ...(checked ? [icon(`${id}-ck`, 'check', {size: 14, fill: FG})] : []),
    ]);
  }
  const menuSep = (id, width) => frame(id, 'Separator', {width, height: 9, layout: 'vertical', justifyContent: 'center'}, [frame(`${id}-l`, 'Line', {width, height: 1, fill: '$--secondary'}, [])]);
  const menuBox = (id, width, rows) => frame(id, 'Menu', {width, layout: 'vertical', padding: XS, fill: '$--popover', cornerRadius: '$--radius-sm', stroke: '$--secondary', strokeWidth: 1, strokeAlignment: 'inner', effect: SHADOW}, rows);
  const lead = (id, glyph) => frame(id, 'Lead', {width: 16, height: 16, layout: 'horizontal', justifyContent: 'center', alignItems: 'center'}, [icon(`${id}-i`, glyph, {size: 14, fill: MUTED})]);
  const MENU_W = 236, ROW_W = MENU_W - 2 * XS;
  // The target follows what is in front (here hide main); Home stays on top
  // so going back to it is one pick, then this device's checkouts, then every
  // other device's Home and checkouts.
  const targetMenu = p => menuBox(`${p}-mt`, MENU_W, [
    menuRow(`${p}-mt0`, {lead: lead(`${p}-mt0g`, 'house'), label: 'Home', width: ROW_W}),
    menuSep(`${p}-mts0`, ROW_W),
    menuRow(`${p}-mt1`, {lead: lead(`${p}-mt1g`, 'folder-git-2'), label: 'hide · main', checked: true, highlighted: true, width: ROW_W}),
    menuRow(`${p}-mt2`, {lead: lead(`${p}-mt2g`, 'folder-git-2'), label: 'oh-my-principle · main', width: ROW_W}),
    menuRow(`${p}-mt3`, {lead: lead(`${p}-mt3g`, 'folder-git-2'), label: 'sasu · main', width: ROW_W}),
    menuRow(`${p}-mt4`, {lead: lead(`${p}-mt4g`, 'folder-git-2'), label: 'demo · main', width: ROW_W}),
    menuSep(`${p}-mts`, ROW_W),
    menuRow(`${p}-mt5`, {lead: lead(`${p}-mt5g`, 'server'), label: 'mini · Home', width: ROW_W}),
    menuRow(`${p}-mt6`, {lead: lead(`${p}-mt6g`, 'server'), label: 'build-box · Home', detail: '연결 안 됨', width: ROW_W}),
  ]);
  const kindMenu = p => menuBox(`${p}-mk`, 180, [
    menuRow(`${p}-mk0`, {lead: providerArt(`${p}-mk0g`, 'claude', 16), label: 'Claude', checked: true, highlighted: true, width: 172}),
    menuRow(`${p}-mk1`, {lead: providerArt(`${p}-mk1g`, 'codex', 16), label: 'Codex', width: 172}),
  ]);
  const CLAUDE_MODELS = ['haiku', 'sonnet', 'opus', 'fable'];
  const CODEX_MODELS = ['gpt-6-astra', 'gpt-6-sol', 'gpt-6-luna', 'gpt-5.6-sol', 'gpt-5.6-terra', 'gpt-5.6-luna', 'gpt-5.5'];
  const modelMenu = (p, models, pick, w = 180) => menuBox(`${p}-mm`, w, models.map((m, i) => menuRow(`${p}-mm${i}`, {lead: lead(`${p}-mm${i}g`, 'cpu'), label: m, checked: m === pick, highlighted: m === pick, width: w - 2 * XS})));
  function startCut(p, {open, menu}) {
    const menuX = DD[open].x;
    const menuH = {target: 8 + 28 * 5 + 9 * 2 + 28 + 40, kind: 8 + 2 * 28, model: 8 + 4 * 28}[open];
    return frame(`${p}-cut`, 'Cut', {layout: 'none', width: ASK_W + 48, height: 24 + FIELD_H + BAR_H + 4 + menuH + 24, fill: '$--background', cornerRadius: '$--radius-md', clip: true}, [
      at(startPanel(p, {open}), 24, ASK_TOP),
      at(menu, 24 + menuX, ASK_TOP + FIELD_H + DD_Y + DD_H + 4),
    ]);
  }
  const captioned = (id, label, node) => frame(id, label, {layout: 'vertical', gap: '$--spacing-sm', alignItems: 'start'}, [text(`${id}-l`, label, {size: '$--text-caption', weight: '600', fill: MUTED}), node]);
  // The panel at rest with the three dropdowns closed, then each dropdown open.
  function startPanels(suffix) {
    const p = key => `mn-ask-${key}-${suffix}`;
    return frame(`mn-ask-${suffix}`, '⌘N start panel', {layout: 'vertical', gap: '$--spacing-lg'}, [
      captioned(p('rest'), '⌘N · 창 위에 뜬 판, 글을 적은 상태, 드롭다운 셋 닫힘', frame(p('restbox'), 'Rest', {layout: 'vertical', padding: 24, fill: '$--background', cornerRadius: '$--radius-md'}, [startPanel(p('r'))])),
      frame(p('row1'), 'Kind and model menus', {layout: 'horizontal', gap: '$--spacing-lg', alignItems: 'start'}, [
        captioned(p('k'), '종류 메뉴', startCut(p('kc'), {open: 'kind', menu: kindMenu(p('kc'))})),
        captioned(p('m'), '모델 메뉴 · Claude', startCut(p('mc'), {open: 'model', menu: modelMenu(p('mc'), CLAUDE_MODELS, 'opus')})),
      ]),
      frame(p('row2'), 'Target and Codex menus', {layout: 'horizontal', gap: '$--spacing-lg', alignItems: 'start'}, [
        captioned(p('t'), '대상 메뉴', startCut(p('tc'), {open: 'target', menu: targetMenu(p('tc'))})),
        captioned(p('x'), 'Codex를 고르면 모델 목록이 바뀐다', frame(p('xw'), 'Codex models', {layout: 'vertical', padding: 16, fill: '$--background', cornerRadius: '$--radius-md'}, [modelMenu(p('xm'), CODEX_MODELS, 'gpt-6-astra', 200)])),
      ]),
    ]);
  }

  function build(suffix) {
    // workspaceManage.ts projectMenu() in a browser tab, which has no Finder
    // (PRD sidebar-context-menus D-02, D-07); Remove project carries no
    // destructive style in the real menu either. The desktop app's menu, with
    // Reveal in Finder, is on Screen / Projects Sidebar.
    const rowMenu = screenMenuContent(`mn-row-${suffix}`, 220, [
      screenMenuItem(`mn-row1-${suffix}`, 'New worktree…'),
      screenMenuItem(`mn-row2-${suffix}`, 'New tab in main', {shortcut: '⌥T'}),
      screenMenuSeparator(`mn-rowsep1-${suffix}`),
      screenMenuItem(`mn-row3-${suffix}`, 'Copy path'),
      screenMenuSeparator(`mn-rowsep2-${suffix}`),
      screenMenuItem(`mn-row4-${suffix}`, 'Pin'),
      screenMenuItem(`mn-row5-${suffix}`, 'Remove project…'),
    ]);
    // ExplorerTree.tsx's row-context branch (a file row, not the empty-area
    // branch that offers New File/New Folder instead): Open to the side (with
    // its width-reason, since this Explorer is too narrow to split), Rename,
    // then Move to Trash separated and destructive.
    const explorerCtx = screenMenuContent(`mn-ctx-${suffix}`, 280, [
      screenMenuItem(`mn-ctx0-${suffix}`, 'Open to the side', {state: 'disabled', reason: 'This view area is too narrow to open a second view beside it.', reasonWidth: 240}),
      screenMenuItem(`mn-ctx1-${suffix}`, 'Rename'),
      screenMenuSeparator(`mn-ctxsep-${suffix}`),
      screenMenuItem(`mn-ctx2-${suffix}`, 'Move to Trash', {state: 'destructive'}),
    ]);
    // The one-device window's footer button: the device it is, and the way to add another.
    const deviceMenu = screenMenuContent(`mn-dev-${suffix}`, 200, [
      screenMenuItem(`mn-dev0-${suffix}`, 'This Mac · 이 기기', {glyph: 'laptop', shortcut: '✓'}),
      screenMenuSeparator(`mn-devsep-${suffix}`),
      screenMenuItem(`mn-dev1-${suffix}`, '기기 추가…', {glyph: 'plus', state: 'highlighted'}),
    ]);
    // ExplorerTree.tsx puts the Git status in the Explorer's root row, so its
    // answer never moves a row (issue 570): a spinner while the first answer is on
    // its way, the outline Badge in the warning color, whose tooltip says why,
    // for a failure the operator can repair, and nothing for a current answer
    // or a folder that is not a repository.
    const gitRow = (key, mark) => frame(`mn-git${key}-${suffix}`, 'Explorer root row', {layout: 'horizontal', alignItems: 'center', gap: '$--spacing-xs', width: 360}, [
      text(`mn-git${key}t-${suffix}`, 'demo', {size: '$--text-caption', weight: '600', fill: '$--muted-foreground', width: 'fill_container'}),
      ...(mark ? [mark] : []),
      icon(`mn-git${key}r-${suffix}`, 'refresh-cw', {size: 12, fill: '$--muted-foreground'}),
    ]);
    const notice = frame(`mn-notice-${suffix}`, 'Explorer Git status', {width: 360, layout: 'vertical', gap: '$--spacing-sm'}, [
      gitRow('l', icon(`mn-gitls-${suffix}`, 'loader-circle', {size: 12, fill: '$--muted-foreground'})),
      gitRow('u', themedXref(`mn-gitub-${suffix}`, 'eHAjc', '!', {...BADGE_VARIANTS.outline.overrides, stroke: '$--warning'}, {xXuNa: {enabled: false}, n8L5dm: {content: '!', fill: '$--warning'}})),
      gitRow('n', null),
    ]);
    return [frame(`mn-wrap-${suffix}`, 'Wrap', {layout: 'vertical', gap: '$--spacing-lg'}, [
      frame(`mn-row-a-${suffix}`, 'Row', {layout: 'horizontal', gap: '$--spacing-lg', alignItems: 'start'}, [rowMenu, explorerCtx, deviceMenu]),
      notice,
      startPanels(suffix),
    ])];
  }
  return screenSheet('screen-menus', 'Screen / Menus and Overlays', 'entry-menu.tsx EntryContextMenu, the sidebar footer’s device button menu (This Mac, 기기 추가…), the Explorer root row’s Git status marks and the ⌘N start panel (PRD home-device-rail D-18..D-20): overlays shown anchored in their real screen context rather than the abstract System gallery. The panel floats over the window like ⌘K with one line to write and three dropdowns, target, agent kind and model, and 시작 with ⏎. The target defaults to what is in front and its menu lists Home first, then the front device’s checkouts, then every other device’s Home, an unreachable one dimmed with 연결 안 됨; the kind menu is Claude and Codex, and the model menu follows the kind (Claude: haiku, sonnet, opus, fable; Codex: the list the CLI reports). The kind and each kind’s model are remembered; the target is not.', build, build);
}

// -- Screen / Projects Sidebar -------------------------------------------------

// The sidebar's Projects tab (sidebar.tsx, projects.ts), drawn from the library's
// Project row, Checkout row, Sidebar agent row and Inactive Fold Row. The left
// reads: a project's glyph at the row's inset, a checkout's glyph one lineage step
// in, and an opened checkout's agent marks under the checkout name. Every line
// ends the same way: its time or status badge, then its fold slot, so names never
// move and every time, badge and chevron shares one column; a folded chevron is
// drawn, an unfolded one waits for the pointer. A row's menu is a right-click,
// with nothing drawn for it. An opened checkout and its agents share a group fill.
function buildProjectsSidebar(tokens) {
  const width = num(tokens, '--size-sidebar-ideal');
  const xs = num(tokens, '--spacing-xs');
  const sm = num(tokens, '--spacing-sm');
  // The list's inner width; the 240px cutout redraws the same rows at its own (`atRow`).
  let row = width - 2 * sm;
  const indent = num(tokens, '--size-lineage-indent');
  const glyphColumn = 2 * xs + num(tokens, '--size-agent-mark');
  const nameColumn = glyphColumn + num(tokens, '--size-checkout-icon') + xs;
  const markSlot = num(tokens, '--size-agent-mark');
  const STATUS = {working: '$--agent-working', asking: '$--warning', done: '$--success', seen: '$--muted-foreground'};
  const SYMBOL = {working: '●', asking: '?', done: '✓', seen: '○'};
  const KIND = {
    overview: {icon: 'layout-dashboard', fill: '$--subtle-foreground'},
    primary: {icon: 'house', fill: '$--subtle-foreground'},
    branch: {icon: 'git-branch', fill: '$--subtle-foreground'},
    folder: {icon: 'folder', fill: '$--subtle-foreground'},
    mergeable: {icon: PR_TREE.mergeable[0], fill: PR_TREE.mergeable[1]},
    draft: {icon: PR_TREE.draft[0], fill: PR_TREE.draft[1]},
    missing: {icon: 'git-branch', fill: '$--destructive'},
  };

  // The status badge (status-badge.tsx): one part per mark the agents' rows
  // draw, in the order whose move is next, zero parts left out; a quiet agent
  // counts in none, and no counted agent, no badge (status-marks B19). A board
  // may still count by the older states, which add up into the marks they became.
  function fourMarks(marks) {
    const of = key => marks?.[key] ?? 0;
    return {turn: of('turn') + of('error') + of('approval') + of('question'), going: of('going') + of('working'), done: of('done'), unread: of('unread') + of('stopped')};
  }
  function status(prefix, marks) {
    const counts = fourMarks(marks);
    const parts = ['turn', 'going', 'done', 'unread'];
    const counted = parts.filter(part => counts[part] > 0);
    if (!counted.length) return {[`${prefix}-status`]: {enabled: false}};
    return Object.fromEntries([
      [`${prefix}-status`, {enabled: true}],
      ...parts.map(part => [`${prefix}-${part}`, {enabled: counted.includes(part)}]),
      ...counted.map(part => [`${prefix}-${part}-count`, {content: String(counts[part])}]),
    ]);
  }

  // A project's badge is its checkouts' added up, and it stays while they are open.
  // The name is 600 while an awake agent there draws a mark and 400 when
  // its agents only rest or sleep or it has none (PRD sidebar-titlebar-refresh D-02);
  // a project or one-line folder whose agents all sleep is folded behind one moon
  // cell (D-03), and under the pointer + and ⋯ stand over the badge slot (D-04).
  const marked = marks => Object.values(fourMarks(marks)).some(count => count > 0);
  function projectRow(id, {name, marks, expanded, selected = false, glyph = 'folder-git-2', asleep, hover = false, menu = false}) {
    const working = marked(marks);
    const actions = hover || menu;
    return themedXref(id, 'qdhY0', name, {width: row, height: num(tokens, '--size-project-row'), ...(selected ? {fill: '$--color-selected'} : actions ? {fill: '$--accent'} : {})}, {
      iBYjj: {icon: expanded ? 'chevron-down' : 'chevron-right'},
      // An unfolded project's chevron waits for the pointer in its slot.
      wgtfo: {opacity: expanded && !actions ? 0 : 1},
      mIzlX: {icon: glyph},
      JkPyX: {content: name, fontWeight: working ? '600' : '400'},
      ...status('project', marks),
      ...(asleep && !expanded ? {'project-status': {enabled: false}, 'project-asleep': {enabled: !actions}, 'project-asleep-age': {content: asleep}} : {}),
      ...(actions ? {'project-status': {enabled: false}, 'project-actions': {enabled: true}} : {}),
      ...(menu ? {'project-action-more': {fill: '$--secondary'}} : {}),
    });
  }

  // Line one is the name, 12/400 with its path prefix muted (500 while its
  // Workspace is in front), the badge while the agent rows are folded (always,
  // on a folder, whose badge is its project's) and the fold slot; line two is
  // the purpose (after the parent it was raised from, if any) and the
  // last-commit age, drawn only when one of them exists. A row without either
  // is one line with its age there, and a missing folder has none.
  // Pen draws no ellipsis, so a long name or purpose is written already cut the
  // way the row truncates it.
  function checkoutRow(id, {name, kind = 'branch', age, purpose, raisedFrom, marks, expanded = false, selected = false, hovered = false}) {
    const k = KIND[kind];
    const project = kind === 'folder';
    const agents = Boolean(marks);
    const secondLine = Boolean(purpose || raisedFrom);
    const slash = name.indexOf('/');
    const [prefix, rest] = project || slash <= 0 ? ['', name] : [name.slice(0, slash + 1), name.slice(slash + 1)];
    // Line one starts with the master's mark slot and a gap before the glyph.
    const leading = (project ? sm : glyphColumn) - markSlot - sm;
    const inner = row - xs - leading;
    return themedXref(id, 'DLP27', name, {
      width: row, height: num(tokens, secondLine ? '--size-checkout-row-detailed' : project ? '--size-project-row' : '--size-checkout-row'),
      padding: [0, xs, 0, leading],
      ...(selected ? {fill: '$--secondary'} : hovered ? {fill: '$--accent'} : {}),
    }, {
      QetL0: {width: inner},
      VtSRn: {icon: k.icon, fill: k.fill},
      'checkout-name-prefix': prefix ? {content: prefix, enabled: true} : {enabled: false},
      // A plain folder's name is the project's: 600 while an agent there works, else 400 (D-02).
      q91d7: project ? {content: name, fontSize: '$--text-subhead', fontWeight: marked(marks) ? '600' : '400'} : {content: rest, fontWeight: selected ? '500' : 'normal'},
      TS5x9: {enabled: kind === 'missing'},
      ...status('checkout', project || !expanded ? marks : null),
      TuosT: !secondLine && age ? {content: age, enabled: true} : {enabled: false},
      // The fold keeps its slot on a row with no agents, so every age lines up.
      CgZj3: {icon: expanded ? 'chevron-down' : 'chevron-right'},
      IXwZI: {opacity: agents && !expanded ? 1 : 0},
      o6XLUj: {enabled: secondLine, width: inner},
      yfur4: {width: markSlot + sm},
      D37Vr: raisedFrom || purpose ? {content: raisedFrom ? `↰ ${raisedFrom}에서${purpose ? ` · ${purpose}` : ''}` : purpose, enabled: true} : {enabled: false},
      'line2-age': secondLine && age ? {content: age, enabled: true} : {enabled: false},
    });
  }

  // sidebar.tsx's FolderRowView: the project's name and badge on line one, in
  // the checkout row's columns, and the purpose on line two; a folder has no commit age.
  function folderRow(id, options) {
    return checkoutRow(id, {...options, kind: 'folder'});
  }

  // An opened checkout's agent row: its marks under the checkout name, one
  // lineage step further per level, the context already said by the rows above.
  function agentRow(id, {status, depth = 0, ...options}) {
    return screenSidebarAgentRow(id, {fold: 'none', symbol: SYMBOL[status], color: STATUS[status], inset: nameColumn + depth * indent, width: row, ...options});
  }

  // An opened checkout's tree row: the chevron lane sits in the checkout glyph's
  // column, so the marks start under the checkout name, one indent per level.
  const TREE_STATUS = {working: ['●', '$--agent-working'], waiting: ['○', '$--agent-working'], asking: ['?', '$--warning'], approval: ['!', '$--warning'], done: ['✓', '$--success'], seen: ['○', '$--muted-foreground']};
  // A tree row is the 20px line and its 4px padding, and a child on another
  // device adds that device's chip line; the segment's guide is as tall as its rows.
  const treeHeights = new Map();
  function tree(id, {status, ...options}) {
    const [symbol, color] = status ? TREE_STATUS[status] : ['', '$--muted-foreground'];
    const node = screenTreeRow(id, {symbol, color, inset: nameColumn, width: row, statusAxis: true, ...options});
    treeHeights.set(node, num(tokens, '--size-sidebar-line') + 2 * xs + (options.device ? 16 : 0) + (options.line ? 16 : 0));
    return node;
  }

  // A tab is a segment of its checkout's agent list, not a level. A tab of two
  // or more agents draws a 1px guide centered under the checkout's kind glyph
  // spanning the segment's rows, and opens with one quiet 22px line inside the
  // guide, the tab glyph on the status-mark column and its name (the manual
  // one, else `Tab N`) on the title column (sidebar.tsx's TabSegmentHead);
  // `name` is set exactly for such a tab. A single-agent tab draws no guide;
  // the gap between segments is the tab boundary.
  function segment(id, {name = null, rows}) {
    const tabLine = num(tokens, '--size-sidebar-line') + num(tokens, '--spacing-xxs');
    const glyphSize = num(tokens, '--size-icon-sm');
    const mark = num(tokens, '--size-agent-mark');
    const guideX = glyphColumn + (num(tokens, '--size-checkout-icon') - 1) / 2;
    const markX = nameColumn + (mark - glyphSize) / 2;
    const head = name === null ? [] : [frame(`${id}-line`, 'Tab line', {width: row, height: tabLine, layout: 'horizontal', alignItems: 'center', gap: nameColumn + mark + xs - markX - glyphSize, padding: [0, xs, 0, markX]}, [
      icon(`${id}-glyph`, 'panels-top-left', {size: glyphSize, fill: '$--subtle-foreground'}),
      text(`${id}-name`, name, {size: '$--text-caption', fill: '$--subtle-foreground', width: 'fill_container'}),
    ])];
    const total = (name === null ? 0 : tabLine) + rows.reduce((sum, node) => sum + treeHeights.get(node), 0);
    const guide = name === null ? [] : [frame(`${id}-guide`, 'Tab guide', {layoutPosition: 'absolute', x: guideX, y: xs, width: 1, height: total - 2 * xs, fill: '$--lineage-rail'}, [])];
    return frame(id, 'Tab segment', {width: row, layout: 'vertical'}, [...head, ...rows, ...guide]);
  }

  // An opened checkout and its unboxed tab segments, 8px apart: the segments hang
  // 2px under the checkout row, and the group's end carries the same 8px before
  // the next row (sidebar.tsx's OpenAgentRows).
  function group(id, rows) {
    const [checkout, ...segments] = rows;
    return frame(id, 'Opened checkout', {width: row, layout: 'vertical', gap: '$--spacing-xxs', padding: [0, 0, sm, 0]}, [
      checkout,
      frame(`${id}-segments`, 'Tab segments', {width: row, layout: 'vertical', gap: '$--spacing-sm'}, segments),
    ]);
  }

  function fold(id, label, level) {
    return themedXref(id, 'inactive-fold-row', label, {width: row, padding: [0, xs, 0, level === 'project' ? sm : glyphColumn]}, {'inactive-fold-label': {content: label}});
  }

  // Component / Sidebar chrome's section header, 10/500 on its baseline in a 26
  // row: Pinned leads with the pin, Projects ends in Add project, and a header stuck
  // to the list's top wears its hairline (PRD sidebar-titlebar-refresh D-05).
  function section(id, label, {pin = false, add = false, stuck = false} = {}) {
    return themedXref(id, 'sbc-section', label, {width: row, ...(stuck ? {stroke: '$--border', strokeWidth: {bottom: 1}, strokeAlignment: 'inner'} : {})}, {
      'sbc-section-pin': {enabled: pin},
      'sbc-section-label': {content: label},
      'sbc-section-add': {enabled: add},
    });
  }

  // -- the device rail and the header line (PRD home-device-rail D-09..D-14, quick device-rail-slack) --
  const railWidth = num(tokens, '--size-rail');
  const WIN_H = 720;
  const TILE = 32, RING = 2, RAIL_TOP = 12;
  const MARK = num(tokens, '--size-rail-mark'), BADGE = num(tokens, '--size-rail-badge'), BADGE_TEXT = num(tokens, '--size-rail-badge-text'), CUT = 2;
  const FG = '$--foreground', SUBTLE = '$--subtle-foreground', MUTED = '$--muted-foreground';
  const at = (node, x, y) => ({...node, x: Math.round(x), y: Math.round(y)});
  const spacer = id => frame(id, 'Spacer', {width: 'fill_container', height: 1}, []);
  const DEVICES = {
    mac: {glyph: 'laptop', label: 'This Mac'},
    mini: {monogram: 'M', label: 'mini'},
    build: {monogram: 'Bb', label: 'build-box'},
    add: {glyph: 'plus', label: '기기 추가'},
  };
  // The state fills the Agents tab's counts wear as text.
  const BADGE_FILL = {needs_you: '$--warning', done: '$--success', working: '$--agent-working'};

  // A mark notched into a tile's corner: the mark on a ring of the rail's own fill.
  function notched(id, name, w, h, inner, children = []) {
    return frame(id, name, {width: w + 2 * CUT, height: h + 2 * CUT, cornerRadius: (h + 2 * CUT) / 2, fill: '$--sidebar', layout: 'horizontal', justifyContent: 'center', alignItems: 'center'}, [
      frame(`${id}-m`, name, {width: w, height: h, cornerRadius: h / 2, layout: 'horizontal', justifyContent: 'center', alignItems: 'center', ...inner}, children),
    ]);
  }

  // A tile is a 32 square with This Mac's laptop or a device's monogram and no
  // name under it; the selected one is ringed 2 off its edge. One mark notched into
  // the top-right shows the most urgent state: the Needs You count (9+ from ten),
  // else a dot for unseen Done. Working has no mark, and an unreachable device dims
  // its glyph and wears a x at the bottom-right. The add tile's edge is dashed in the app; Pen has no
  // dash, so it is drawn solid.
  function railTile(id, key, {selected = false, off = false, needs = 0, done = false} = {}) {
    const d = DEVICES[key];
    const box = TILE + 4 * RING, x0 = (railWidth - TILE) / 2, y0 = 2 * RING;
    const glyphFill = key === 'add' ? MUTED : selected ? FG : SUBTLE;
    const drawn = d.monogram ? text(`${id}-g`, d.monogram, {size: '$--text-body', weight: '600', fill: glyphFill}) : icon(`${id}-g`, d.glyph, {size: key === 'add' ? 14 : 20, fill: glyphFill});
    const glyph = off ? {...drawn, opacity: num(tokens, '--opacity-dimmed')} : drawn;
    const pill = needs >= 10 ? BADGE + 2 : BADGE;
    return frame(id, d.label, {layout: 'none', width: railWidth, height: box}, [
      ...(selected ? [at(frame(`${id}-ring`, 'Selected', {width: box, height: box, cornerRadius: 12, stroke: FG, strokeWidth: RING, strokeAlignment: 'inner'}, []), x0 - 2 * RING, y0 - 2 * RING)] : []),
      at(frame(`${id}-t`, 'Tile', {
        width: TILE, height: TILE, cornerRadius: '$--radius-md', layout: 'horizontal', justifyContent: 'center', alignItems: 'center',
        ...(key === 'add' ? {stroke: '$--border', strokeWidth: 1, strokeAlignment: 'inner'} : {fill: '$--secondary'}),
      }, [glyph]), x0, y0),
      ...(done && needs === 0 && !off ? [at(notched(`${id}-done`, 'Done', MARK, MARK, {fill: BADGE_FILL.done}), x0 + TILE + 2 - MARK - CUT, y0 - 2 - CUT)] : []),
      ...(needs > 0 && !off ? [at(notched(`${id}-needs`, 'Needs You', pill, BADGE, {fill: BADGE_FILL.needs_you}, [
        text(`${id}-nt`, needs >= 10 ? '9+' : String(needs), {size: BADGE_TEXT, weight: '600', fill: '$--status-foreground'}),
      ]), x0 + TILE + 4 - pill - CUT, y0 - 4 - CUT)] : []),
      ...(off ? [at(notched(`${id}-x`, 'Not connected', BADGE, BADGE, {fill: '$--card'}, [
        text(`${id}-xt`, '×', {size: BADGE_TEXT, fill: MUTED}),
      ]), x0 + TILE + 4 - BADGE - CUT, y0 + TILE + 4 - BADGE - CUT)] : []),
    ]);
  }

  // The sidebar's full-height left column: This Mac and the registered devices in
  // order, and + directly under the last one, 44 apart as in the app (each frame
  // carries the ring's 4 above and below the tile, so the gap is 4, not 12); the sidebar beside it follows the
  // selected tile. `tiles` overrides the three-device set.
  function deviceRail(p, selected, {miniOff = false, tiles} = {}) {
    const set = tiles ?? [
      ['mac', {selected: selected === 'mac', needs: 2, done: true}],
      ['mini', {selected: selected === 'mini', needs: miniOff ? 0 : 12, off: miniOff}],
      ['build', {selected: selected === 'build', off: true}],
    ];
    return frame(`${p}-rail`, 'Device rail', {width: railWidth, height: WIN_H, layout: 'vertical', gap: '$--spacing-xs', padding: [RAIL_TOP - 2 * RING, 0, 4, 0], fill: '$--sidebar', stroke: '$--border', strokeWidth: {right: 1}, strokeAlignment: 'inner'}, [
      ...set.map(([key, options], i) => railTile(`${p}-rt${i}`, key, options)),
      railTile(`${p}-radd`, 'add'),
    ]);
  }

  // The window's top row over the rail and the sidebar (TitleRow.tsx, PRD
  // sidebar-titlebar-refresh D-15): the traffic lights, the sidebar's switch a
  // step after their space, then the device in front; with the rail hidden the
  // name carries a chevron and opens the device menu. Its right edge is the
  // sidebar's, so the divider runs up.
  const TOP = num(tokens, '--size-tab-strip');
  function topRow(p, {name, tag, menu = false, rail = true}) {
    const total = (rail ? railWidth : 0) + width;
    const switchX = num(tokens, '--size-traffic-light-inset') + xs;
    const control = num(tokens, '--size-control-sm');
    const glyph = num(tokens, '--size-icon');
    const nameX = switchX + control + xs;
    const light = (i, x) => ({type: 'ellipse', id: `${p}-tl${i}`, name: 'Traffic light', x, y: 10, width: 12, height: 12, fill: MUTED, opacity: 0.5});
    return frame(`${p}-top`, 'Window top row', {width: total, height: TOP, layout: 'none', fill: '$--sidebar', stroke: '$--border', strokeWidth: {bottom: 1, right: 1}, strokeAlignment: 'inner'}, [
      light(0, 13), light(1, 33), light(2, 53),
      at({...icon(`${p}-sw`, 'panel-left', {size: glyph, fill: SUBTLE}), name: 'Sidebar switch'}, switchX + (control - glyph) / 2, (TOP - glyph) / 2),
      at(frame(`${p}-tn`, 'Device', {layout: 'horizontal', gap: sm, alignItems: 'center', height: TOP}, [
        text(`${p}-tn-t`, name, {size: '$--text-subhead', weight: '600'}),
        ...(tag ? [text(`${p}-tn-tag`, tag, {size: '$--text-caption', fill: MUTED})] : []),
        ...(menu ? [icon(`${p}-tn-c`, 'chevron-down', {size: 12, fill: MUTED})] : []),
      ]), nameX, 0),
    ]);
  }

  // The Search row the sidebar opens with, under the top row (D-08).
  const searchRow = p => themedXref(`${p}-search`, 'sbc-search', 'Search row', {width});

  const ruleLine = id => frame(id, 'Rule', {width, height: 1, fill: '$--border'}, []);

  function sharedOverviewRow(p, count = 0) {
    return frame(`${p}-overview`, 'Overview', {width, height: num(tokens, '--size-tab-strip'), layout: 'horizontal', gap: sm, padding: [0, '$--spacing-md'], alignItems: 'center'}, [
      icon(`${p}-overview-g`, 'layout-dashboard', {size: 16, fill: SUBTLE}),
      text(`${p}-overview-t`, 'Overview', {size: '$--text-body'}), spacer(`${p}-overview-space`),
      ...(count ? [text(`${p}-overview-n`, `내 차례 ${count}`, {size: '$--text-caption', fill: '$--warning'})] : []),
      text(`${p}-overview-k`, '⌘⇧O', {size: '$--text-micro', fill: MUTED, mono: true}),
    ]);
  }

  // Usage chips, then Help (?) and the Settings gear (D-11).
  function footer(p, {helpOpen = false} = {}) {
    return frame(`${p}-ftr`, 'Footer', {width, layout: 'vertical'}, [
      ruleLine(`${p}-fr`),
      frame(`${p}-fb`, 'Footer row', {width, layout: 'horizontal', padding: [sm, '$--spacing-md'], gap: xs, alignItems: 'center'}, [
        screenUsageChip(`${p}-u0`, {provider: 'claude', value: '62%'}),
        screenUsageChip(`${p}-u1`, {provider: 'codex', value: '59%'}),
        spacer(`${p}-fsp`),
        helpOpen
          ? themedXref(`${p}-help`, 'Nyvom', 'Help', {width: 20, height: 20, fill: '$--secondary'}, {ZIZFR: {icon: 'circle-help', fill: '$--foreground'}})
          : screenIconButton(`${p}-help`, 'circle-help', {size: 20}),
        screenIconButton(`${p}-gear`, 'settings', {size: 20}),
      ]),
    ]);
  }

  // Agent rows the device sidebars share: title, mark, and the context line the
  // Needs You and Agents lists carry; a remote row wears the device chip.
  const AGENTS = {
    deploy: {title: '배포 전 확인', status: 'asking', age: '30s', line: '승인  e2e 테스트 돌리던 중', place: 'hide › main', bright: true},
    blog: {title: '블로그 초안 정리', status: 'asking', age: '2m', line: '질문 · 톤을 이대로 갈까요?', place: 'Home', bright: true},
    research: {title: '두 프로젝트 비교 조사', status: 'seen', age: '14m', place: 'Home'},
    ci: {title: 'CI 러너 샤드 정리', status: 'done', age: '6m', place: 'hide › ci-shards', bright: true},
    readable: {title: '사이드바 가독성 개선', status: 'working', age: '1m', place: 'hide › main'},
    principle: {title: '원칙 문서 정리', status: 'working', age: '3m', place: 'oh-my-principle › main'},
    sasu: {title: '판정 로그 확인', status: 'seen', age: '1h', place: 'sasu › main'},
    batch: {title: '배치 감시', status: 'asking', age: '5m', line: 'PR #252 머지할까요?', place: 'Home', bright: true},
    build: {title: '릴리스 빌드 확인', status: 'working', age: '1m', place: 'hide › main'},
  };
  // A raised group: its heading with the whole count, its most recent rows up to
  // the cap, and a More fold for the rest.
  const raised = (p, title, list, total = list.length) => [
    section(`${p}-sn`, `${title} · ${total}`),
    ...list.map(([key, extra], i) => agentRow(`${p}-n${i}`, {...AGENTS[key], inset: sm, ...extra})),
    ...(total > list.length ? [fold(`${p}-more`, `More ${total - list.length}`, 'project')] : []),
  ];

  // One device's sidebar column, under the window's top row: the Search row and the
  // Overview row, then the Needs You and Done groups and the projects (no Home row,
  // D-06); a fixed `height` is the window's, and the list takes what is left.
  function deviceSidebar(key, s, {needs, done, rows = [], height, helpOpen = false}) {
    const p = `psb-${key}`;
    const id = `${p}-${s}`;
    return frame(`${p}-${s}`, 'Sidebar', {width, ...(height ? {height} : {}), layout: 'vertical', fill: '$--sidebar', clip: true}, [
      searchRow(id), sharedOverviewRow(id, needs?.total ?? 0), ruleLine(`${id}-r1`),
      ...(needs ? [frame(`${id}-needs`, 'Needs You', {width, layout: 'vertical', padding: [0, xs, xs, xs]}, raised(needs.p, 'Needs You', needs.rows, needs.total))] : []),
      ...(done ? [frame(`${id}-done`, 'Done', {width, layout: 'vertical', padding: [0, xs, xs, xs]}, raised(done.p, 'Done', done.rows, done.total))] : []),
      frame(`${id}-list`, 'Projects list', {width, ...(height ? {height: 'fill_container'} : {}), layout: 'vertical', padding: [0, sm], clip: Boolean(height)}, rows),
      footer(id, {helpOpen}),
    ]);
  }

  // The window's left part: the top row over the rail and the sidebar beside it.
  const windowPart = (p, top, body) => frame(`${p}-window`, 'Top row, rail and sidebar', {layout: 'vertical', gap: 0, alignItems: 'start'}, [top, body]);

  function build(s) {
    const id = key => `psb-${key}-${s}`;
    const mark = {question: 1, working: 2, done: 1};
    // The rest state the review target names: This Mac in front, its Needs You and
    // Done groups over Pinned and Projects; working projects at 600, a resting one at
    // 400, sleeping ones folded behind the moon cell, then No agents and Inactive
    // projects folded (PRD sidebar-titlebar-refresh D-01 to D-06).
    const rest = deviceSidebar('sidebar', s, {
      needs: {p: `psb-nu-${s}`, rows: [['deploy'], ['blog']], total: 2}, done: {p: `psb-dn-${s}`, rows: [['ci']], total: 1},
      rows: [
        section(`psb-sec-pin-${s}`, 'Pinned · 1', {pin: true}),
        folderRow(`psb-p-notes-${s}`, {name: 'team-notes', marks: {idle: 1}, purpose: '회의록 요약 정리'}),
        section(`psb-sec-recent-${s}`, 'Projects · Recent activity · 5', {add: true}),
        projectRow(`psb-p-herdr-${s}`, {name: 'hide', marks: {question: 3, working: 5, done: 1, idle: 1}, expanded: true}),
        // main opened (PRD agent-hierarchy-screens D-12, D-38, B35): one line, its agents as a tree.
        group(`psb-g-main-${s}`, [
          checkoutRow(`psb-c2-${s}`, {name: 'main', kind: 'primary', age: 'now', marks: {question: 2, working: 4, idle: 1}, expanded: true}),
          segment(`psb-seg-main-${s}`, {name: 'Tab 1', rows: [
            tree(`psb-t0-${s}`, {title: '사이드바 가독성 개선', status: 'approval', age: '12m', chevron: 'open', pr: {state: 'mergeable', number: 180}, bright: true}),
            tree(`psb-t1-${s}`, {title: 'P8 읽기 전용 준비', status: 'waiting', age: '19h', line: '대기 · 자식 1명 작업 중', depth: 1, chevron: 'open', pr: {state: 'failed', number: 214}}),
            tree(`psb-t2-${s}`, {title: '테스트 작성', status: 'approval', age: '12m', depth: 2, rail: true}),
            tree(`psb-t3-${s}`, {title: '문서 정리', status: 'done', age: '40m', depth: 2, rail: true, last: true}),
            tree(`psb-t4-${s}`, {title: 'P7 Pi·omp 재우기', status: 'asking', age: '1h', depth: 1, pr: {state: 'pending', number: 838}}),
            tree(`psb-t5-${s}`, {title: '미전달 편지 승격 해제', status: 'seen', age: '5h', depth: 1, device: 'Mac mini'}),
            tree(`psb-t6-${s}`, {title: '3개 더', status: null, age: '', depth: 1, last: true, chevron: 'closed', more: true}),
            tree(`psb-t7-${s}`, {title: 'Mac mini 연동 5단계', status: 'waiting', age: '2m', line: '대기 · 자식 1명 작업 중', chevron: 'closed'}),
          ]}),
          segment(`psb-seg-solo-${s}`, {rows: [tree(`psb-t8-${s}`, {title: '에이전트 패널 표시 규칙', status: 'working', age: '38s'})]}),
        ]),
        checkoutRow(`psb-c3-${s}`, {name: 'quick/155-browser-display', age: '40m', marks: {question: 1}, raisedFrom: 'main', purpose: '#155 browser display (WebCon…'}),
        checkoutRow(`psb-c4-${s}`, {name: 'quick/154-search-palette', kind: 'draft', age: '1h', marks: {done: 1}, raisedFrom: 'main', purpose: '#154 ⌘K search palette UI'}),
        checkoutRow(`psb-c1-${s}`, {name: 'electron-shortcut-bindings', kind: 'mergeable', age: '2h', marks: {working: 1}, purpose: 'Electron desktop host for the we…'}),
        checkoutRow(`psb-c5-${s}`, {name: 'design/workspace-ux-prop…', age: '5h', purpose: 'Workspace UX 제안과 상태 소유…'}),
        fold(`psb-no-agents-${s}`, 'No agents 1', 'checkout'),
        fold(`psb-cleanup-${s}`, 'Cleanup 1', 'checkout'),
        fold(`psb-f-herdr-${s}`, 'Inactive 14', 'checkout'),
        projectRow(`psb-p-sasu-${s}`, {name: 'sasu', marks: {working: 1, done: 1, idle: 1}, expanded: false}),
        projectRow(`psb-p-codex-${s}`, {name: 'codex-daemon-apply-spec', glyph: 'folder', asleep: '4d'}),
        projectRow(`psb-p-sticky-${s}`, {name: 'sticky-notes', asleep: '14d'}),
        fold(`psb-f-none-${s}`, 'No agents 3', 'project'),
        fold(`psb-f-proj-${s}`, 'Inactive projects 3', 'project'),
      ],
    });

    // A remote device in front: its own Projects, tagged Remote in the top row; its
    // twelve Needs You draw the five most recent and fold the rest.
    const remoteNeeds = [
      ['batch', {place: 'Home'}],
      ['batch', {title: '릴리스 노트 검토', age: '7m', line: '이 문구로 확정할까요?', place: 'hide › main'}],
      ['batch', {title: '프런트모스트 창 고정', age: '12m', line: '접근성 권한을 요청할까요?', place: 'hide › quick/246-frontmost'}],
      ['batch', {title: '판정 로그 재실행', age: '20m', line: '실패한 3건을 다시 돌릴까요?', place: 'sasu › main'}],
      ['batch', {title: '디스크 정리', age: '31m', line: '캐시 12 GB를 지울까요?', place: 'Home'}],
    ];
    const remote = deviceSidebar('remote', s, {
      height: WIN_H, needs: {p: `psb-rm-${s}`, rows: remoteNeeds, total: 12},
      rows: [
        section(`psb-rm-sp-${s}`, 'Projects · Recent activity · 2', {add: true}),
        projectRow(`psb-rm-p0-${s}`, {name: 'hide', marks: {working: 1}, expanded: true}),
        checkoutRow(`psb-rm-p0m-${s}`, {name: 'main', kind: 'primary', age: '1m', purpose: '릴리스 빌드 확인', marks: {working: 1}, selected: true}),
        checkoutRow(`psb-rm-p0w-${s}`, {name: 'quick/246-frontmost', kind: 'mergeable', age: '3h', purpose: '#246 frontmost 창 고정'}),
        projectRow(`psb-rm-p1-${s}`, {name: 'sasu'}),
      ],
    });

    // A device that is not connected: its name, the state, one action; the last tree is not drawn.
    const offBody = frame(`psb-off-body-${s}`, 'Not connected', {width, height: 'fill_container', layout: 'vertical', gap: sm, alignItems: 'center', padding: [120, '$--spacing-md', 0, '$--spacing-md']}, [
      icon(`psb-off-g-${s}`, 'server', {size: 20, fill: MUTED}),
      text(`psb-off-n-${s}`, 'mini', {size: '$--text-subhead', weight: '600'}),
      text(`psb-off-s-${s}`, '연결 안 됨', {size: '$--text-caption', fill: MUTED}),
      screenButton(`psb-off-b-${s}`, '다시 연결', {variant: 'secondary', height: num(tokens, '--size-control-sm'), icon: 'refresh-cw'}),
    ]);
    const off = frame(`psb-off-${s}`, 'Sidebar', {width, height: WIN_H, layout: 'vertical', fill: '$--sidebar', clip: true}, [
      searchRow(`psb-off-${s}`), ruleLine(`psb-off-r0-${s}`), offBody, footer(`psb-off-${s}`),
    ]);

    // One device: the rail still shows with This Mac alone and its unseen Done dot.
    const one = deviceSidebar('one', s, {
      height: WIN_H,
      rows: [
        section(`psb-one-sp-${s}`, 'Projects · Recent activity · 4', {add: true}),
        projectRow(`psb-one-p0-${s}`, {name: 'hide', marks: mark, expanded: true}),
        checkoutRow(`psb-one-p0m-${s}`, {name: 'main', kind: 'primary', age: 'now', purpose: '사이드바 가독성 개선', marks: {question: 1, working: 1}, selected: true}),
        checkoutRow(`psb-one-p0w-${s}`, {name: 'feat/home-device-rail', kind: 'mergeable', age: '12m', purpose: 'Home · 기기 레일', marks: {working: 1}}),
        projectRow(`psb-one-p1-${s}`, {name: 'oh-my-principle', marks: {working: 1}}),
        projectRow(`psb-one-p2-${s}`, {name: 'sasu', marks: {idle: 1}}),
      ],
    });

    // The rail hidden: the top row's name is the device menu, open under it.
    const hiddenMenu = frame(id('hidden-menu'), 'Device menu', {layout: 'vertical', gap: 0, padding: [TOP, 0, 0, 0]}, [
      screenMenuContent(id('hidden-menu-content'), 220, [
        screenMenuItem(`psb-hm-0-${s}`, 'This Mac'), screenMenuItem(`psb-hm-1-${s}`, 'mini'), screenMenuItem(`psb-hm-2-${s}`, 'build-box · 연결 안 됨'),
        screenMenuSeparator(`psb-hm-s-${s}`),
        screenMenuItem(`psb-hm-3-${s}`, '기기 추가…'), screenMenuItem(`psb-hm-4-${s}`, '레일 표시'),
      ]),
    ]);
    const hidden = deviceSidebar('hidden', s, {
      height: WIN_H,
      rows: [
        section(`psb-hid-sp-${s}`, 'Projects · Recent activity · 2', {add: true}),
        projectRow(`psb-hid-p0-${s}`, {name: 'hide', marks: mark}),
        projectRow(`psb-hid-p1-${s}`, {name: 'sasu', marks: {idle: 1}}),
      ],
    });

    const labeled = (key, label, children) => frame(id(`cap-${key}`), label, {layout: 'vertical', gap: '$--spacing-sm', alignItems: 'start'}, [
      text(`${id(`cap-${key}`)}-l`, label, {size: '$--text-caption', weight: '600', fill: MUTED}), ...children,
    ]);
    const paired = (key, selected, sidebar, options) => frame(id(`pair-${key}`), 'Rail and sidebar', {layout: 'horizontal', gap: 0, alignItems: 'start'}, [deviceRail(id(key), selected, options), sidebar]);
    return [frame(id('wrap'), 'Wrap', {layout: 'vertical', gap: '$--spacing-xl'}, [
      frame(id('row-rest'), 'This Mac in front', {layout: 'horizontal', gap: '$--spacing-lg', alignItems: 'start'}, [
        labeled('rest', 'This Mac in front · the top row, Search, Needs You, Done, Pinned, Projects, No agents', [windowPart(id('rest'), topRow(id('rest'), {name: 'This Mac'}), paired('rest', 'mac', rest))]),
        frame(id('row-parts'), 'Rows and headers', {layout: 'vertical', gap: '$--spacing-lg', alignItems: 'start'}, [rowStates(s), stuckState(s), helpState(s)]),
        hoverState(s), menuStates(s),
      ]),
      frame(id('row-states'), 'Rail states', {layout: 'horizontal', gap: '$--spacing-xl', alignItems: 'start'}, [
        labeled('remote', 'mini in front · 9+ pill, Needs You past its cap', [windowPart(id('remote'), topRow(id('remote'), {name: 'mini', tag: 'Remote'}), paired('remote', 'mini', remote))]),
        labeled('off', 'mini not connected · dimmed glyph and x, no mark', [windowPart(id('off'), topRow(id('off'), {name: 'mini', tag: 'Remote'}), paired('off', 'mini', off, {miniOff: true}))]),
        labeled('one', 'One device · the rail shows with This Mac alone', [windowPart(id('one'), topRow(id('one'), {name: 'This Mac'}), frame(id('pair-one'), 'Rail and sidebar', {layout: 'horizontal', gap: 0, alignItems: 'start'}, [deviceRail(id('one'), 'mac', {tiles: [['mac', {selected: true, done: true}]]}), one]))]),
        labeled('hidden', 'Rail hidden · the top row’s name is the device menu', [frame(id('pair-hidden'), 'Sidebar and menu', {layout: 'horizontal', gap: '$--spacing-md', alignItems: 'start'}, [windowPart(id('hidden'), topRow(id('hidden'), {name: 'This Mac', menu: true, rail: false}), hidden), hiddenMenu])]),
      ]),
    ])];
  }

  // The project row's states in the list's own width (PRD sidebar-titlebar-refresh
  // D-02 to D-04): the weight, the moon cell folded and opened, and + ⋯ over the
  // badge under the pointer, which keep the name's width.
  function rowStates(s) {
    const cut = (key, label, rows) => frame(`psb-rs-${key}-${s}`, label, {layout: 'vertical', gap: '$--spacing-xs', alignItems: 'start'}, [
      text(`psb-rs-${key}-l-${s}`, label, {size: '$--text-caption', weight: '600', fill: MUTED}),
      frame(`psb-rs-${key}-list-${s}`, 'Projects list, cut', {width, layout: 'vertical', padding: [xs, xs], fill: '$--sidebar', cornerRadius: '$--radius-sm'}, rows),
    ]);
    return frame(`psb-rs-${s}`, 'Project row states', {layout: 'vertical', gap: '$--spacing-md', alignItems: 'start'}, [
      cut('weight', 'Working 600 · resting 400', [
        projectRow(`psb-rs-w0-${s}`, {name: 'sasu', marks: {working: 1, idle: 1}}),
        projectRow(`psb-rs-w1-${s}`, {name: 'oh-my-principle', marks: {idle: 2}}),
      ]),
      cut('asleep', 'All asleep · folded with the moon, then opened', [
        projectRow(`psb-rs-a0-${s}`, {name: 'sticky-notes', asleep: '14d'}),
        projectRow(`psb-rs-a1-${s}`, {name: 'sticky-notes', asleep: '14d', expanded: true}),
        checkoutRow(`psb-rs-a1m-${s}`, {name: 'main', kind: 'primary', age: '14d'}),
      ]),
      cut('hover', 'Under the pointer · + ⋯ over the badge or the moon', [
        projectRow(`psb-rs-h0-${s}`, {name: 'hide', marks: {question: 3, working: 5}, hover: true}),
        projectRow(`psb-rs-h1-${s}`, {name: 'codex-daemon-apply-spec', glyph: 'folder', asleep: '4d', hover: true}),
      ]),
      cut('none', 'No agents opened · its projects in the usual rows', [
        fold(`psb-rs-n-${s}`, 'No agents 3', 'project'),
        projectRow(`psb-rs-n0-${s}`, {name: 'investment'}),
        projectRow(`psb-rs-n1-${s}`, {name: 'landing-page'}),
        projectRow(`psb-rs-n2-${s}`, {name: 'modakbul-site'}),
      ]),
    ]);
  }

  // The list scrolled: Projects stuck to its top with its hairline, rows passing under it (D-05).
  function stuckState(s) {
    return frame(`psb-stuck-${s}`, 'Header stuck while scrolled', {layout: 'vertical', gap: '$--spacing-xs', alignItems: 'start'}, [
      text(`psb-stuck-l-${s}`, 'Scrolled · Projects stuck to the top with its hairline', {size: '$--text-caption', weight: '600', fill: MUTED}),
      frame(`psb-stuck-list-${s}`, 'Projects list, scrolled', {width, height: 120, layout: 'vertical', padding: [0, xs], fill: '$--sidebar', cornerRadius: '$--radius-sm', clip: true}, [
        section(`psb-stuck-h-${s}`, 'Projects · Recent activity · 5', {add: true, stuck: true}),
        checkoutRow(`psb-stuck-c0-${s}`, {name: 'design/workspace-ux-prop…', age: '5h', purpose: 'Workspace UX 제안과 상태 소유…'}),
        projectRow(`psb-stuck-p0-${s}`, {name: 'sasu', marks: {working: 1, done: 1, idle: 1}}),
      ]),
    ]);
  }

  // The footer's ? pressed with the Help menu above it (D-12).
  function helpState(s) {
    const I = (key, label, options) => screenMenuItem(`psb-help-${key}-${s}`, label, options);
    return frame(`psb-help-${s}`, 'Help menu above the footer', {layout: 'vertical', gap: '$--spacing-xs', alignItems: 'end'}, [
      text(`psb-help-l-${s}`, 'Help · the menu above the footer', {size: '$--text-caption', weight: '600', fill: MUTED}),
      screenMenuContent(`psb-help-menu-${s}`, 220, [
        I('0', 'Keyboard shortcuts', {shortcut: '⌘/'}), I('1', 'Copy diagnostics'), I('2', 'Report an issue'),
        screenMenuSeparator(`psb-help-s-${s}`),
        frame(`psb-help-v-${s}`, 'Version', {width: 'fill_container', padding: [xs, sm]}, [text(`psb-help-vt-${s}`, 'hide 0.4.0', {size: '$--text-caption', fill: MUTED, mono: true})]),
      ]),
      frame(`psb-help-f-${s}`, 'Footer', {width, fill: '$--sidebar', cornerRadius: '$--radius-sm'}, [footer(`psb-help-${s}`, {helpOpen: true})]),
    ]);
  }

  // A row's right-click menu, one per row kind (PRD sidebar-context-menus
  // D-02..D-04, D-10): a cut of the list around the row on the hover wash,
  // and the menu (Component / Menu parts) opened under the pointer. The items,
  // their order and separators are workspaceManage.ts's projectMenu,
  // checkoutMenu and agentMenu as the desktop app draws them.
  function menuStates(s) {
    const I = (id, label, options) => screenMenuItem(`${id}-${s}`, label, options);
    const S = id => screenMenuSeparator(`${id}-${s}`);
    const cut = (id, rows) => frame(`psb-m-${id}-list-${s}`, 'Projects list, cut', {width, layout: 'vertical', padding: [xs, xs], fill: '$--sidebar', cornerRadius: '$--radius-sm', clip: true}, rows);
    const opened = (id, title, rows, menuWidth, items) => frame(`psb-m-${id}-${s}`, title, {layout: 'vertical', gap: 0}, [
      cut(id, rows),
      frame(`psb-m-${id}-at-${s}`, 'Menu under the pointer', {layout: 'horizontal', padding: [0, 0, 0, 3 * num(tokens, '--spacing-lg')]}, [screenMenuContent(`psb-m-${id}-menu-${s}`, menuWidth, items)]),
    ]);
    return frame(`psb-menus-${s}`, 'Row menus', {layout: 'vertical', gap: '$--spacing-lg'}, [
      opened('proj', 'Project row menu, from ⋯ or a right-click', [
        projectRow(`psb-m-proj-row-${s}`, {name: 'hide', marks: {question: 3, working: 5, done: 1, idle: 1}, expanded: true, menu: true}),
      ], 220, [
        I('psb-m-proj-1', 'New worktree…'),
        I('psb-m-proj-2', 'New tab in main', {shortcut: '⌘T'}),
        S('psb-m-proj-s1'),
        I('psb-m-proj-3', 'Reveal in Finder'),
        I('psb-m-proj-4', 'Copy path'),
        S('psb-m-proj-s2'),
        I('psb-m-proj-5', 'Pin'),
        I('psb-m-proj-6', 'Remove project…'),
      ]),
      opened('co', 'Checkout row menu', [
        checkoutRow(`psb-m-co-row-${s}`, {name: 'electron-shortcut-bindings', kind: 'mergeable', age: '2h', marks: {working: 1}, purpose: 'Electron desktop host for the we…', hovered: true}),
      ], 240, [
        I('psb-m-co-0', 'Open'),
        I('psb-m-co-1', 'New tab here', {shortcut: '⌘T'}),
        I('psb-m-co-2', 'Open pull request #149'),
        S('psb-m-co-s1'),
        I('psb-m-co-3', 'Set purpose…'),
        I('psb-m-co-4', 'Set as default checkout'),
        I('psb-m-co-5', 'Copy branch name'),
        I('psb-m-co-6', 'Copy path'),
        I('psb-m-co-9', 'Open in VS Code'),
        I('psb-m-co-7', 'Reveal in Finder'),
        S('psb-m-co-s2'),
        I('psb-m-co-8', 'Delete worktree…', {state: 'destructive'}),
      ]),
      // A plain folder's menu: the project's items, Open in VS Code before the reveal
      // since the folder is the checkout, then the checkout's own Open and Set purpose.
      opened('fo', 'Folder row menu', [
        projectRow(`psb-m-fo-row-${s}`, {name: 'team-notes', glyph: 'folder', marks: {idle: 1}, menu: true}),
      ], 240, [
        I('psb-m-fo-0', 'New worktree…', {state: 'disabled', reason: 'This project is not a Git repository.'}),
        I('psb-m-fo-1', 'New tab in main', {shortcut: '⌘T'}),
        S('psb-m-fo-s1'),
        I('psb-m-fo-2', 'Open in VS Code'),
        I('psb-m-fo-3', 'Reveal in Finder'),
        I('psb-m-fo-4', 'Copy path'),
        S('psb-m-fo-s2'),
        I('psb-m-fo-5', 'Unpin'),
        I('psb-m-fo-6', 'Remove project…'),
        S('psb-m-fo-s3'),
        I('psb-m-fo-7', 'Open'),
        I('psb-m-fo-8', 'Set purpose…'),
      ]),
      opened('ag', 'Agent row menu', [
        agentRow(`psb-m-ag-row-${s}`, {title: '배포 전 확인', status: 'asking', age: '30s', line: '프로덕션 배포 전에 변경 내용을 확인해…', bright: true}),
      ], 220, [
        I('psb-m-ag-0', 'Show', {shortcut: '⌥3'}),
        S('psb-m-ag-s1'),
        I('psb-m-ag-1', 'Copy title'),
        I('psb-m-ag-2', 'Copy session id'),
        S('psb-m-ag-s2'),
        I('psb-m-ag-3', 'Close tab…'),
      ]),
    ]);
  }

  // The pull-request row under the pointer with its card beside it (PRD
  // checkout-pr-glyph-card D-03, D-05): a cut of the list around the row, the
  // row on the hover wash, and Component / PR hover card to the right at the
  // row's top, the way the tooltip places it. Every text on the card is the
  // row's own facts; the card's width is the master's.
  function hoverState(s) {
    const rowH = num(tokens, '--size-checkout-row-detailed');
    const projectH = num(tokens, '--size-project-row');
    const plainH = num(tokens, '--size-checkout-row');
    const gap = num(tokens, '--spacing-sm');
    const cardW = num(tokens, '--size-pr-popover');
    const rows = [
      projectRow(`psb-h-p-${s}`, {name: 'hide', marks: {question: 3, working: 5, done: 1, idle: 1}, expanded: true}),
      checkoutRow(`psb-h-c3-${s}`, {name: 'quick/155-browser-display', age: '40m', marks: {question: 1}, purpose: '#155 browser display (WebCon…'}),
      checkoutRow(`psb-h-c4-${s}`, {name: 'quick/154-search-palette', kind: 'draft', age: '1h', marks: {done: 1}, purpose: '#154 ⌘K search palette UI'}),
      checkoutRow(`psb-h-c1-${s}`, {name: 'electron-shortcut-bindings', kind: 'mergeable', age: '2h', marks: {working: 1}, purpose: 'Electron desktop host for the we…', hovered: true}),
      checkoutRow(`psb-h-c5-${s}`, {name: 'design/workspace-ux-prop…', age: '5h', purpose: 'Workspace UX 제안과 상태 소유…'}),
    ];
    // The hovered row's top: the project row, the Overview row and two detailed rows above it.
    const rowTop = projectH + plainH + 2 * rowH;
    const listH = 2 * xs + projectH + plainH + 4 * rowH;
    const list = frame(`psb-h-list-${s}`, 'Projects list, cut', {width, layout: 'vertical', padding: [xs, xs], fill: '$--sidebar', cornerRadius: '$--radius-sm', clip: true}, rows);
    const card = themedXref(`psb-h-card-${s}`, 'pr-card', 'PR hover card', {x: width + gap, y: xs + rowTop}, {
      'pr-card-badge-label': {content: 'Open'},
      'pr-card-number': {content: '#149'},
      'pr-card-title': {content: 'Electron desktop host for the web shell'},
      'pr-card-row-review': {enabled: false},
      'pr-card-checks': {content: 'Passing'},
      'pr-card-branch': {content: 'electron-shortcut-bindings'},
      'pr-card-agents-turn': {enabled: false},
      'pr-card-agents-going-count': {content: '1'},
      'pr-card-commit': {content: '2h ago'},
      'pr-card-path': {content: '~/projects/hide.worktrees/electron-shortcut-bindings'},
    });
    return frame(`psb-hover-${s}`, 'Checkout row under the pointer, with its card', {width: width + gap + cardW, height: listH}, [{...list, x: 0, y: 0}, card]);
  }
  // The approved 240px cutout: the production rows at the narrowest width, a named
  // tab, a one-agent tab (no line) and an unnamed tab of two (its line reads Tab 3).
  function atRow(inner, draw) {
    const saved = row;
    row = inner;
    try { return draw(); } finally { row = saved; }
  }
  function statusAxis(s) {
    return atRow(240 - 2 * sm, () => frame(`psb-axis-${s}`, 'Approved tab segments · 240 px', {width: 240, layout: 'vertical', padding: [sm, sm, 0, sm], fill: '$--sidebar', clip: true}, [group(`psb-axis-g-${s}`, [
      checkoutRow(`psb-axis-checkout-${s}`, {name: 'main', kind: 'primary', age: '8m', marks: {working: 4, idle: 1}, expanded: true}),
      segment(`psb-axis-a-${s}`, {name: '아주 긴 한국어 탭 이름과 작업…', rows: [
        tree(`psb-axis-one-${s}`, {title: '탭 경계·여백 정리', status: 'working', age: '1m', selected: true}),
        tree(`psb-axis-two-${s}`, {title: '왼쪽 pane 검토', status: 'working', age: '8m'}),
      ]}),
      segment(`psb-axis-single-${s}`, {rows: [tree(`psb-axis-three-${s}`, {title: '세션 기록 동기화', status: 'working', age: '2s'})]}),
      segment(`psb-axis-b-${s}`, {name: 'Tab 3', rows: [
        tree(`psb-axis-four-${s}`, {title: 'PR 질문 표시 개선', status: 'working', age: '6m'}),
        tree(`psb-axis-five-${s}`, {title: '탐색 개선', status: 'seen', age: '11m'}),
      ]}),
    ])]));
  }
  const approvedBuild = suffix => [...build(suffix), statusAxis(suffix)];
  return screenSheet('screen-projects-sidebar', 'Screen / Projects Sidebar', 'sidebar.tsx and core agent_state (session-first-ui B1-B10; agent-hierarchy-screens D-12, D-37 to D-42): one sidebar follows the selected device, under the window’s top row that names it after the traffic lights (PRD sidebar-titlebar-refresh D-15): the Search row, the Overview row, bounded Needs You and Done groups and default-open project checkouts, with no Home row (D-06). Pinned leads with the pin and Projects ends in Add project; both headers stick to the list’s top with a hairline (D-05). A project’s name is 600 while an agent there works and 400 otherwise, a project whose agents all sleep is folded behind a moon cell with its age, projects with no agents fold behind No agents N (D-01 to D-03), and under the pointer + and ⋯ stand over the badge (D-04). The footer adds Help before the gear (D-11, D-12), and checkout and folder menus carry Open in VS Code before the reveal (D-13). A Needs You row’s second line is the verb and what to do. Roots and orphans head an opened checkout; a root the operator opened draws its children and grandchildren as one-line tree rows with elbow rails and a chevron lane, five siblings and then N개 더, each row with its own PR icon and, folded, its descendant mark; an opened checkout is one line. Every root stands on one status axis right of the checkout glyph whatever tab holds it: a hairline guide under that glyph spans each tab’s rows with a gap between tabs, and a tab of two or more agents opens with one quiet line inside the guide, the tab glyph on the status-mark column and its manual name or Tab N on the title column. A plain folder’s glyph and name stand on the project row’s columns. A grandchild’s chevron opens its tree popover, and the popover’s last line opens the Overview Agents graph. Clean worktrees without agents start in No agents; closed agent worktrees and missing paths start in Cleanup. Explicit checkout folds persist and retain Needs You rows. Project and checkout menus, glyphs, age columns, the device rail and usage footer retain their existing behavior.', approvedBuild, approvedBuild);
}

// -- Screen / Mobile ---------------------------------------------------------------

// The mobile companion (PRD mobile-companion D-08, D-12): Settings > Mobile on the
// desktop, blocked on a failing check and ready with its QR, and the phone app's
// pairing, list, detail, unreachable and empty states and its push banner. Drawn
// as the operator-approved board drew it: plain nodes on this file's local tokens,
// because no library master draws a phone screen, so every size is a number this
// builder owns. The machine name and tailnet are placeholders, and the QR is a
// deterministic pattern, never a real code.
function buildMobile(tokens, root) {
  const settingsSheetParts = settingsParts(tokens, root, {themedXref, screenIconButton, screenSwatch});
  const XS = num(tokens, '--spacing-xs'), SM = num(tokens, '--spacing-sm'), MD = num(tokens, '--spacing-md');
  const LG = num(tokens, '--spacing-lg'), XL = num(tokens, '--spacing-xl');
  const SP = {xs: '$--spacing-xs', sm: '$--spacing-sm', md: '$--spacing-md', lg: '$--spacing-lg', xl: '$--spacing-xl'};
  const FG = '$--foreground', MUTED = '$--muted-foreground', SUBTLE = '$--subtle-foreground';
  const WARN = '$--warning', OK = '$--success', WORK = '$--agent-working', BAD = '$--destructive';
  const PILL = 999;
  const PHONE_W = 390, PHONE_H = 800, STATUS_H = 44, HOME_H = 24;
  const CONTENT_W = PHONE_W - 2 * LG;
  const SET_W = num(tokens, '--size-settings-sheet-w');
  const MACHINE = 'my-mac';
  const URL = `https://${MACHINE}.tailnet-name.ts.net`;

  // Pen has no ellipsis: cut a string to a measured width.
  function measure(s, size, mono = false) {
    let w = 0;
    for (const ch of s) {
      const c = ch.codePointAt(0);
      if ((c >= 0xac00 && c <= 0xd7a3) || (c >= 0x3130 && c <= 0x318f)) w += size * 0.93;
      else if (mono) w += size * 0.6;
      else if (ch === ' ') w += size * 0.28;
      else if (/[A-Z#@%MW]/.test(ch)) w += size * 0.68;
      else if (/[il.,:;'|!]/.test(ch)) w += size * 0.28;
      else w += size * 0.55;
    }
    return w;
  }
  function fit(s, max, size, mono = false) {
    if (!s || measure(s, size, mono) <= max) return s;
    let cut = s;
    while (cut.length && measure(`${cut}…`, size, mono) > max) cut = cut.slice(0, -1);
    return `${cut.trimEnd()}…`;
  }

  // -- atoms --
  const spacer = id => frame(id, 'Spacer', {width: 'fill_container', height: 1}, []);
  const rule = (id, width) => ({type: 'line', id, name: 'Rule', width, height: 0, stroke: '$--border', strokeWidth: '$--size-hairline', strokeAlignment: 'center'});
  const STATUS = {working: ['●', WORK], done: ['✓', OK], seen: ['○', SUBTLE], question: ['?', WARN], approval: ['!', WARN], error: ['×', BAD]};
  // The phone draws the desktop row's mark (status-marks B22).
  function mark(id, status) {
    const [glyph, fill] = STATUS[status];
    return markSlot(id, legacyMark(glyph, fill));
  }
  function provider(id, kind, size = 14) {
    return frame(id, 'Provider', {width: size, height: size, cornerRadius: 3, fill: {type: 'image', enabled: true, url: `../web/src/assets/agent-${kind}.png`, mode: 'fit'}}, []);
  }
  function chip(id, label, {glyph, fill = '$--secondary', color = MUTED} = {}) {
    return frame(id, 'Chip', {layout: 'horizontal', gap: SP.xs, alignItems: 'center', padding: [1, 6], fill, cornerRadius: PILL, height: 18}, [
      ...(glyph ? [icon(`${id}-i`, glyph, {size: 11, fill: color})] : []),
      text(`${id}-t`, label, {size: '$--text-caption', fill: color}),
    ]);
  }
  function button(id, label, {primary = false, width, height = 40, size = 15} = {}) {
    return frame(id, label, {layout: 'horizontal', alignItems: 'center', justifyContent: 'center', height, ...(width ? {width} : {padding: [0, SP.lg]}), fill: primary ? '$--primary' : '$--secondary', cornerRadius: '$--radius-lg'}, [
      text(`${id}-t`, label, {size, weight: '600', fill: primary ? '$--primary-foreground' : FG}),
    ]);
  }

  // -- phone chrome --
  function phone(id, name, body) {
    return frame(id, name, {layout: 'vertical', width: PHONE_W, height: PHONE_H, fill: '$--background', cornerRadius: 40, clip: true, stroke: '$--border', strokeWidth: '$--size-hairline', strokeAlignment: 'inner'}, [
      frame(`${id}-sb`, 'Status bar', {layout: 'horizontal', width: PHONE_W, height: STATUS_H, padding: [0, SP.xl], alignItems: 'center'}, [
        text(`${id}-sb-t`, '9:41', {size: 15, weight: '600', fill: FG}),
        spacer(`${id}-sb-s`),
        text(`${id}-sb-r`, '●●● ▲ ▮', {size: '$--text-caption', fill: FG}),
      ]),
      frame(`${id}-body`, 'Body', {layout: 'vertical', width: PHONE_W, height: PHONE_H - STATUS_H - HOME_H, clip: true}, body),
      frame(`${id}-hb`, 'Home', {width: PHONE_W, height: HOME_H, alignItems: 'center', justifyContent: 'center'}, [
        frame(`${id}-hb-b`, 'Bar', {width: 134, height: 5, cornerRadius: PILL, fill: FG, opacity: 0.6}, []),
      ]),
    ]);
  }
  function header(id, {title, sub, back = null, connected = true, trailing = null}) {
    const titleRow = frame(`${id}-r`, 'Title row', {layout: 'horizontal', gap: SP.sm, alignItems: 'center', width: CONTENT_W, height: 32}, [
      ...(back ? [frame(`${id}-bk`, 'Back', {layout: 'horizontal', gap: '$--spacing-xxs', alignItems: 'center', height: 32}, [icon(`${id}-bi`, 'arrow-left', {size: 18, fill: FG}), text(`${id}-bt`, back, {size: 15, fill: FG})])] : []),
      ...(back ? [] : [text(`${id}-t`, title, {size: 22, weight: '600', fill: FG})]),
      spacer(`${id}-s`),
      ...(trailing ? [trailing] : []),
      ...(back ? [] : [frame(`${id}-dot`, 'Connection', {width: 8, height: 8, cornerRadius: PILL, fill: connected ? OK : MUTED}, [])]),
    ]);
    const subRow = sub ? [frame(`${id}-sub`, 'Sub', {layout: 'horizontal', gap: SP.xs, alignItems: 'center', width: CONTENT_W, height: 18}, [
      icon(`${id}-si`, 'laptop', {size: 12, fill: MUTED}),
      text(`${id}-st`, sub, {size: '$--text-body', fill: MUTED}),
    ])] : [];
    return frame(id, 'Header', {layout: 'vertical', gap: '$--spacing-xxs', width: PHONE_W, padding: [SP.sm, SP.lg, SP.md, SP.lg]}, [titleRow, ...subRow]);
  }

  // -- phone: list --
  // The list's + opens the start sheet (PRD home-device-rail B42).
  function startButton(id) {
    return frame(id, 'New agent', {width: 32, height: 32, cornerRadius: PILL, fill: '$--secondary', alignItems: 'center', justifyContent: 'center'}, [icon(`${id}-i`, 'plus', {size: 18, fill: FG})]);
  }
  const INSET = 16 + XS + 14 + XS;
  // A list row has no provider logo in the default hidden agent logo view mode (PRD agent-mark-quiet B12; the shown mode is drawn in code only): its lines start under the title, after the status mark alone. The detail head keeps the logo and INSET.
  const LIST_INSET = 16 + XS;
  function sectionLabel(id, label, count) {
    return frame(id, label, {layout: 'horizontal', gap: SP.sm, alignItems: 'center', width: PHONE_W, height: 32, padding: [SP.md, SP.lg, 2, SP.lg]}, [
      text(`${id}-t`, label, {size: '$--text-body', weight: '500', fill: MUTED}),
      text(`${id}-c`, String(count), {size: '$--text-body', fill: MUTED, mono: true}),
    ]);
  }
  function agentRow(id, {title, status, project, branch, machine = null, age, request = null, news = null, dim = false}) {
    const attention = status === 'question' || status === 'approval' || status === 'error';
    const titleW = CONTENT_W - 16 - XS - SM - 36 - SM - 16;
    const line2 = request ?? news ?? null;
    return frame(id, title, {layout: 'vertical', gap: 3, width: PHONE_W, height: line2 ? 82 : 64, padding: [SP.sm, SP.lg, SP.sm, SP.lg]}, [
      frame(`${id}-l1`, 'Line 1', {layout: 'horizontal', gap: SP.xs, alignItems: 'center', width: CONTENT_W, height: 22}, [
        mark(`${id}-m`, status),
        frame(`${id}-tw`, 'Title', {width: titleW, height: 22, alignItems: 'center', clip: true, padding: [0, 0, 0, SP.xs]}, [
          text(`${id}-t`, fit(title, titleW, 15), {size: 15, weight: attention ? '600' : dim ? '400' : '500', fill: dim ? MUTED : attention ? FG : SUBTLE}),
        ]),
        spacer(`${id}-s`),
        text(`${id}-a`, age, {size: '$--text-body', fill: MUTED, mono: true}),
        icon(`${id}-c`, 'chevron-right', {size: 16, fill: MUTED}),
      ]),
      frame(`${id}-l2`, 'Line 2', {layout: 'horizontal', gap: SP.sm, alignItems: 'center', width: CONTENT_W, height: 18, padding: [0, 0, 0, LIST_INSET]}, [
        text(`${id}-pb`, fit(`${project} · ${branch}`, CONTENT_W - 120, 12), {size: '$--text-body', fill: MUTED}),
        ...(machine ? [chip(`${id}-mc`, machine, {glyph: 'server'})] : []),
      ]),
      ...(line2 ? [frame(`${id}-l3`, 'Line 3', {layout: 'horizontal', width: CONTENT_W, height: 18, padding: [0, 0, 0, LIST_INSET], alignItems: 'center'}, [
        text(`${id}-rq`, fit(line2, CONTENT_W - 40, 13), {size: '$--text-subhead', fill: request ? WARN : FG}),
      ])] : []),
      spacer(`${id}-sp`),
      rule(`${id}-rule`, CONTENT_W),
    ]);
  }
  const AGENTS = {
    needs: [
      {title: '솔루션 6 구현', status: 'approval', project: 'hide', branch: 'prd/mobile-companion', age: '2m', request: 'Bash(cargo test -p hided) 실행을 허용할까요?'},
      {title: 'mailbox 플러그인 구현', status: 'question', kind: 'codex', project: 'hide', branch: 'prd/mailbox-plugin', machine: 'mini', age: '9m', request: '워크스페이스 이름을 어떤 걸로 할까요?'},
    ],
    done: [
      {title: '사이드바 행 클릭으로 펼치기', status: 'done', project: 'hide', branch: 'prd/sidebar-row-click-unfold', age: '14m', news: 'PR #186 열림 · CI 통과'},
    ],
    working: [
      {title: '리뷰 반영', status: 'working', project: 'hide', branch: 'fix/hook-probe', age: '5m'},
      {title: '세션 초기화 작업 대기', status: 'working', kind: 'codex', project: 'contong', branch: 'main', machine: 'mini', age: '21m'},
      {title: 'Agent tab group 시각화 검토', status: 'working', project: 'hide', branch: 'prd/agent-tab-groups', age: '38m'},
    ],
    seen: [
      {title: '코덱스 데이터 구조 조사', status: 'seen', project: 'hide', branch: 'main', age: '1h', dim: true},
      {title: 'tab-names 구현 상태 점검', status: 'seen', project: 'hide', branch: 'main', age: '2h', dim: true},
      {title: '체크아웃 capability 구현', status: 'seen', project: 'hide', branch: 'prd/checkout-capability', age: '3h', dim: true},
    ],
  };
  function listBody(id, {unreachable = false, empty = false} = {}) {
    const groups = empty ? [] : [
      sectionLabel(`${id}-g1`, '내 차례', AGENTS.needs.length), ...AGENTS.needs.map((a, i) => agentRow(`${id}-n${i}`, a)),
      sectionLabel(`${id}-g2`, '읽을 것', AGENTS.done.length), ...AGENTS.done.map((a, i) => agentRow(`${id}-d${i}`, a)),
      sectionLabel(`${id}-g3`, '작업 중', AGENTS.working.length), ...AGENTS.working.map((a, i) => agentRow(`${id}-w${i}`, a)),
      sectionLabel(`${id}-g4`, '확인함', AGENTS.seen.length), ...AGENTS.seen.map((a, i) => agentRow(`${id}-s${i}`, a)),
    ];
    const banner = unreachable ? [frame(`${id}-un`, 'Unreachable', {layout: 'horizontal', gap: SP.sm, alignItems: 'center', width: PHONE_W, padding: [SP.sm, SP.lg, SP.sm, SP.lg], fill: '$--muted'}, [
      icon(`${id}-ui`, 'loader-circle', {size: 14, fill: MUTED}),
      text(`${id}-ut`, '연결 안 됨 · 맥의 hide가 꺼져 있거나 폰의 Tailscale이 꺼져 있어요. 다시 시도 중', {size: '$--text-body', fill: MUTED, width: CONTENT_W - 22}),
    ])] : [];
    const emptyLine = empty ? [frame(`${id}-em`, 'Empty', {width: PHONE_W, padding: [SP.xl, SP.lg]}, [text(`${id}-et`, '실행 중인 에이전트가 없어요', {size: '$--text-title', fill: MUTED})])] : [];
    return [
      header(`${id}-h`, {title: 'hide', sub: `${MACHINE} · 폰 1대 더 연결됨`, connected: !unreachable, trailing: startButton(`${id}-new`)}),
      ...banner,
      frame(`${id}-list`, 'List', {layout: 'vertical', width: PHONE_W, opacity: unreachable ? 0.45 : 1}, [...groups, ...emptyLine]),
    ];
  }

  // -- phone: detail --
  const SCROLLBACK = [
    ['● 페어링 코드는 hided 상태 파일에 0600으로 두고, 코드 교환은 /ws의', FG],
    ['  첫 프레임으로 받겠습니다. 먼저 테스트를 돌립니다.', FG],
    ['', MUTED],
    ['● Bash(cargo test -p hided)', SUBTLE],
    ['  ⎿  Running…', MUTED],
    ['', MUTED],
    ['$ cargo test -p hided', SUBTLE],
    ['   Compiling hided v0.1.0 (/Users/example/projects/hide/hided)', MUTED],
    ['    Finished test [unoptimized] target(s) in 41.2s', MUTED],
    ['test server::tests::token_compare_needs_the_whole_token ... ok', MUTED],
    ['test pane_auth::tests::sweep_drops_unregistered ... ok', MUTED],
    ['test pane_auth::tests::phone_credential_refuses_file_open ... ok', MUTED],
    ['test mobile::tests::pairing_code_is_single_use ... ok', MUTED],
    ['test mobile::tests::pairing_code_expires_after_five_minutes ... ok', MUTED],
    ['test result: ok. 128 passed; 0 failed', OK],
    ['', MUTED],
    ['● hided의 페어링 코드 검사를 구현했습니다. 다음으로 7일 미접속', FG],
    ['  해지 스윕을 붙이겠습니다.', FG],
    ['', MUTED],
    ['╭─────────────────────────────────────────────╮', MUTED],
    ['│ Bash command                                │', FG],
    ['│   cargo test -p hided -- --include-ignored  │', FG],
    ['│                                             │', MUTED],
    ['│ Do you want to proceed?                     │', FG],
    ['│ ❯ 1. Yes                                    │', WARN],
    ['│   2. Yes, and don\'t ask again this session  │', FG],
    ['│   3. No, and tell Claude what to do         │', FG],
    ['╰─────────────────────────────────────────────╯', MUTED],
  ];
  function scrollback(id, height) {
    return frame(id, 'Scrollback', {layout: 'vertical', gap: 0, width: PHONE_W, height, padding: [SP.sm, SP.lg, SP.sm, SP.lg], fill: '$--card', clip: true, alignItems: 'start', justifyContent: 'end'}, SCROLLBACK.map(([s, fill], i) =>
      frame(`${id}-l${i}`, 'Line', {width: CONTENT_W, height: 18, alignItems: 'center', clip: true}, [text(`${id}-t${i}`, s === '' ? ' ' : fit(s, CONTENT_W, 11, true), {size: '$--text-caption', fill, mono: true})])));
  }
  function quickKeys(id) {
    const keys = ['⏎', 'Esc', '↑', '↓', '^C'];
    const w = (CONTENT_W - 4 * SM) / keys.length;
    return frame(id, 'Quick keys', {layout: 'horizontal', gap: SP.sm, width: PHONE_W, padding: [SP.sm, SP.lg, 0, SP.lg]}, keys.map((k, i) =>
      frame(`${id}-k${i}`, k, {width: w, height: 34, fill: '$--secondary', cornerRadius: '$--radius-md', alignItems: 'center', justifyContent: 'center'}, [text(`${id}-kt${i}`, k, {size: '$--text-title', weight: '600', fill: FG, mono: true})])));
  }
  function replyBar(id) {
    return frame(id, 'Reply bar', {layout: 'horizontal', gap: SP.sm, alignItems: 'center', width: PHONE_W, padding: [SP.sm, SP.lg, SP.md, SP.lg]}, [
      frame(`${id}-in`, 'Input', {layout: 'horizontal', alignItems: 'center', width: CONTENT_W - SM - 76, height: 40, padding: [0, SP.md], fill: '$--secondary', cornerRadius: 12}, [
        text(`${id}-ph`, '답장…', {size: 15, fill: MUTED}),
      ]),
      button(`${id}-send`, '보내기', {primary: true, width: 76}),
    ]);
  }
  function detailBody(id, a) {
    const headH = 32 + 2 + 18 + SM + MD;
    const bodyH = PHONE_H - STATUS_H - HOME_H;
    const sbH = bodyH - headH - 22 - (SM + 34) - (SM + 40 + MD);
    return [
      header(`${id}-h`, {back: '목록', title: a.title, trailing: text(`${id}-age`, a.age, {size: '$--text-body', fill: MUTED, mono: true})}),
      frame(`${id}-ar`, 'Agent', {layout: 'vertical', gap: '$--spacing-xxs', width: PHONE_W, padding: [0, SP.lg, SP.sm, SP.lg]}, [
        frame(`${id}-ar1`, 'Line 1', {layout: 'horizontal', gap: SP.xs, alignItems: 'center', width: CONTENT_W, height: 22}, [
          mark(`${id}-m`, a.status), provider(`${id}-p`, a.kind ?? 'claude'),
          text(`${id}-t`, a.title, {size: 17, weight: '600', fill: FG}),
        ]),
        frame(`${id}-ar2`, 'Line 2', {layout: 'horizontal', gap: SP.sm, alignItems: 'center', width: CONTENT_W, height: 18, padding: [0, 0, 0, INSET]}, [
          text(`${id}-pb`, `${a.project} · ${a.branch}`, {size: '$--text-body', fill: MUTED}),
          ...(a.machine ? [chip(`${id}-mc`, a.machine, {glyph: 'server'})] : []),
        ]),
      ]),
      scrollback(`${id}-sb`, sbH),
      quickKeys(`${id}-qk`),
      replyBar(`${id}-rb`),
    ];
  }

  // -- phone: pairing --
  function pairBody(id) {
    return [
      frame(`${id}-c`, 'Pairing', {layout: 'vertical', gap: SP.lg, alignItems: 'center', width: PHONE_W, height: PHONE_H - STATUS_H - HOME_H, padding: [120, SP.lg, 0, SP.lg]}, [
        frame(`${id}-logo`, 'Logo', {width: 64, height: 64, cornerRadius: '$--radius-xl', clip: true, fill: {type: 'image', enabled: true, url: '../web/public/m/icon-192.png', mode: 'fit'}}, []),
        text(`${id}-t`, `${MACHINE}과 연결`, {size: 22, weight: '600', fill: FG}),
        text(`${id}-d`, '이 폰에서 hide의 에이전트를 보고, 기다리는 에이전트에 답할 수 있어요.', {size: '$--text-title', fill: MUTED, width: CONTENT_W - 32, align: 'center'}),
        frame(`${id}-sp`, 'Gap', {height: LG, width: 1}, []),
        button(`${id}-ok`, '연결', {primary: true, width: CONTENT_W, height: 48, size: 16}),
        text(`${id}-n`, '코드는 5분 안에 만료돼요. 만료되면 맥에서 QR을 다시 여세요.', {size: '$--text-body', fill: MUTED, width: CONTENT_W - 32, align: 'center'}),
      ]),
    ];
  }

  // -- phone: start sheet --
  // The bottom sheet over the dimmed list: what to do, then target, kind and model
  // as one field each (kind and model start on the desktop's remembered values,
  // the target on This Mac · Home), and 시작.
  function startField(id, label, value, leadNode) {
    return frame(id, label, {layout: 'horizontal', gap: SP.sm, alignItems: 'center', width: CONTENT_W, height: 44, padding: [0, SP.md], fill: '$--secondary', cornerRadius: 12}, [
      text(`${id}-l`, label, {size: '$--text-body', fill: MUTED, width: 40}),
      ...(leadNode ? [leadNode] : []),
      text(`${id}-v`, value, {size: 15, fill: FG}),
      spacer(`${id}-s`),
      icon(`${id}-c`, 'chevron-down', {size: 16, fill: MUTED}),
    ]);
  }
  function startBody(id) {
    const bodyH = PHONE_H - STATUS_H - HOME_H;
    const sheetH = 380;
    const sheet = frame(`${id}-sheet`, 'Start sheet', {layout: 'vertical', gap: SP.md, width: PHONE_W, height: sheetH, padding: [SP.sm, SP.lg, SP.lg, SP.lg], fill: '$--popover', cornerRadius: [24, 24, 0, 0], stroke: '$--border', strokeWidth: '$--size-hairline', strokeAlignment: 'inner'}, [
      frame(`${id}-grab`, 'Grab', {width: CONTENT_W, height: 12, alignItems: 'center', justifyContent: 'center'}, [frame(`${id}-grabb`, 'Bar', {width: 36, height: 4, cornerRadius: PILL, fill: MUTED, opacity: 0.6}, [])]),
      frame(`${id}-box`, 'Text box', {layout: 'vertical', width: CONTENT_W, height: 96, padding: SP.md, fill: '$--secondary', cornerRadius: 12}, [
        text(`${id}-q`, '블로그 초안을 두 가지 톤으로 다시 써서 비교해줘', {size: 15, fill: FG, width: CONTENT_W - 2 * MD}),
      ]),
      startField(`${id}-target`, '대상', 'This Mac · Home', icon(`${id}-tg`, 'house', {size: 14, fill: MUTED})),
      startField(`${id}-kind`, '종류', 'Claude', provider(`${id}-kg`, 'claude', 16)),
      startField(`${id}-model`, '모델', 'opus', icon(`${id}-mg`, 'cpu', {size: 14, fill: MUTED})),
      button(`${id}-go`, '시작', {primary: true, width: CONTENT_W, height: 48, size: 16}),
    ]);
    return [frame(`${id}-stack`, 'List under the sheet', {layout: 'none', width: PHONE_W, height: bodyH, clip: true}, [
      {...frame(`${id}-dim`, 'Dimmed list', {layout: 'vertical', width: PHONE_W, height: bodyH, opacity: 0.4}, listBody(`${id}-l`)), x: 0, y: 0},
      {...sheet, x: 0, y: bodyH - sheetH},
    ])];
  }

  // -- push banner --
  function pushBanner(id) {
    const W = 360;
    return frame(id, 'Push banner', {layout: 'horizontal', gap: SP.md, alignItems: 'center', width: W, padding: [SP.md, SP.lg], fill: '$--popover', cornerRadius: 22, stroke: '$--border', strokeWidth: '$--size-hairline', strokeAlignment: 'inner'}, [
      frame(`${id}-ic`, 'App icon', {width: 38, height: 38, cornerRadius: 9, fill: '$--primary', alignItems: 'center', justifyContent: 'center'}, [text(`${id}-it`, 'h', {size: 22, weight: '700', fill: '$--primary-foreground'})]),
      frame(`${id}-tx`, 'Text', {layout: 'vertical', gap: 1, width: W - 2 * LG - 38 - MD}, [
        frame(`${id}-r1`, 'Row', {layout: 'horizontal', width: 'fill_container', alignItems: 'center'}, [text(`${id}-app`, 'hide', {size: '$--text-subhead', weight: '600', fill: FG}), spacer(`${id}-s`), text(`${id}-when`, '지금', {size: '$--text-body', fill: MUTED})]),
        text(`${id}-ti`, '솔루션 6 구현', {size: '$--text-title', weight: '600', fill: FG}),
        text(`${id}-bo`, '내 차례 · hide', {size: '$--text-subhead', fill: SUBTLE}),
      ]),
    ]);
  }

  // -- Settings > Mobile, the page right of Settings' list (PRD settings-left-nav) --
  const ROW_W = SET_W - settingsSheetParts.NAV_W - 2 * settingsSheetParts.PAD_X;
  // Deterministic pseudo-QR: three finder patterns and a seeded module field, drawn
  // as rows of cells. Black on white in both themes, as a camera needs it.
  function qr(id, size = 168) {
    const n = 25, cell = Math.floor((size - 16) / n), pad = (size - cell * n) / 2;
    const finder = (r, c) => (r >= 0 && r < 7 && c >= 0 && c < 7) && (r === 0 || r === 6 || c === 0 || c === 6 || (r >= 2 && r <= 4 && c >= 2 && c <= 4));
    let seed = 7;
    const rnd = () => { seed = (seed * 1103515245 + 12345) & 0x7fffffff; return seed / 0x7fffffff; };
    const rows = [];
    for (let r = 0; r < n; r++) {
      const cells = [];
      for (let c = 0; c < n; c++) {
        let on;
        if (r < 7 && c < 7) on = finder(r, c);
        else if (r < 7 && c >= n - 7) on = finder(r, c - (n - 7));
        else if (r >= n - 7 && c < 7) on = finder(r - (n - 7), c);
        else if ((r < 8 && c < 8) || (r < 8 && c >= n - 8) || (r >= n - 8 && c < 8)) on = false;
        else on = rnd() < 0.45;
        cells.push(frame(`${id}-c${r}-${c}`, 'm', {width: cell, height: cell, fill: on ? '#101112' : '#FFFFFF'}, []));
      }
      rows.push(frame(`${id}-r${r}`, 'row', {layout: 'horizontal', gap: 0, width: cell * n, height: cell}, cells));
    }
    return frame(id, 'QR', {layout: 'vertical', gap: 0, width: size, height: size, padding: pad, fill: '#FFFFFF', cornerRadius: '$--radius-md'}, rows);
  }
  function groupTitle(id, label) {
    return frame(id, label, {width: ROW_W, padding: [SP.lg, 0, SP.xs, 0]}, [text(`${id}-t`, label, {size: '$--text-caption', weight: '500', fill: MUTED})]);
  }
  function row(id, {label, detail = null, trailing = null, leading = null, tone = FG}) {
    return frame(`${id}-w`, label, {layout: 'vertical', gap: 0, width: ROW_W}, [
      frame(id, label, {layout: 'horizontal', gap: SP.md, alignItems: 'center', width: ROW_W, padding: [SP.sm, 0]}, [
        ...(leading ? [leading] : []),
        frame(`${id}-tx`, 'Text', {layout: 'vertical', gap: '$--spacing-xxs', width: 'fill_container'}, [
          text(`${id}-l`, label, {size: '$--text-body', fill: tone}),
          ...(detail ? [text(`${id}-d`, detail, {size: '$--text-caption', fill: MUTED, width: ROW_W - 160})] : []),
        ]),
        ...(trailing ? [trailing] : []),
      ]),
      rule(`${id}-rule`, ROW_W),
    ]);
  }
  function toggle(id, on) {
    return frame(id, 'Switch', {width: 32, height: 18, cornerRadius: PILL, fill: on ? '$--primary' : '$--secondary', padding: 2, layout: 'horizontal', justifyContent: on ? 'end' : 'start', alignItems: 'center'}, [
      frame(`${id}-k`, 'Knob', {width: 14, height: 14, cornerRadius: PILL, fill: on ? '$--primary-foreground' : MUTED}, []),
    ]);
  }
  function linkButton(id, label, danger = false) {
    return frame(id, label, {padding: [4, 10], cornerRadius: '$--radius-sm', fill: '$--secondary'}, [text(`${id}-t`, label, {size: '$--text-body', fill: danger ? BAD : FG})]);
  }
  function checkItem(id, label, state, action = null) {
    const glyph = state === 'ok' ? 'check' : state === 'fail' ? 'triangle-alert' : 'circle-alert';
    const fill = state === 'ok' ? OK : state === 'fail' ? WARN : MUTED;
    const trailing = action ? frame(`${id}-a`, 'Action', {layout: 'horizontal', gap: SP.xs, alignItems: 'center'}, [text(`${id}-at`, action, {size: '$--text-body', fill: '$--primary'}), icon(`${id}-ai`, 'external-link', {size: 12, fill: '$--primary'})]) : null;
    const detail = state === 'fail' ? 'Tailscale 관리 콘솔 › DNS에서 MagicDNS와 HTTPS Certificates를 켜세요. 켜면 여기서 바로 이어집니다.' : null;
    return row(id, {label, tone: state === 'todo' ? MUTED : FG, leading: icon(`${id}-i`, glyph, {size: 14, fill}), trailing, detail});
  }
  function settings(id, {passing}) {
    const content = [
      groupTitle(`${id}-g0`, '모바일'),
      row(`${id}-on`, {label: '폰에서 hide 열기', detail: '맥의 Tailscale로 이 hide를 tailnet 안에서만 엽니다. hide가 tailscale serve를 켜고, 끄면 자기가 만든 항목만 지웁니다.', trailing: toggle(`${id}-sw`, true)}),
      checkItem(`${id}-c1`, '맥에 Tailscale 설치됨', 'ok'),
      checkItem(`${id}-c2`, `Tailscale에 로그인됨 · ${MACHINE}`, 'ok'),
      checkItem(`${id}-c3`, passing ? 'tailnet에 MagicDNS와 HTTPS 켜짐' : 'tailnet에 HTTPS가 꺼져 있어요', passing ? 'ok' : 'fail', passing ? null : '관리 콘솔 열기'),
      checkItem(`${id}-c4`, '폰에도 Tailscale 앱을 설치하고 같은 계정으로 로그인', passing ? 'ok' : 'todo'),
    ];
    const qrBlock = passing ? [
      frame(`${id}-qrw`, 'QR row', {layout: 'horizontal', gap: SP.xl, alignItems: 'center', width: ROW_W, padding: [SP.lg, 0]}, [
        qr(`${id}-qr`),
        frame(`${id}-qt`, 'QR text', {layout: 'vertical', gap: SP.sm, width: ROW_W - 168 - XL}, [
          text(`${id}-q1`, '폰 카메라로 찍으세요', {size: '$--text-title', weight: '600', fill: FG}),
          text(`${id}-q2`, '열리는 페이지에서 연결을 누르고, 공유 › 홈 화면에 추가로 앱처럼 두세요.', {size: '$--text-body', fill: MUTED, width: ROW_W - 168 - XL}),
          text(`${id}-q3`, URL, {size: '$--text-caption', fill: SUBTLE, mono: true}),
          frame(`${id}-qx`, 'Expiry', {layout: 'horizontal', gap: SP.sm, alignItems: 'center'}, [text(`${id}-q4`, '코드는 4:38 후 만료', {size: '$--text-caption', fill: MUTED, mono: true}), linkButton(`${id}-qn`, '새 코드')]),
        ]),
      ]),
    ] : [
      frame(`${id}-noqr`, 'No QR', {layout: 'horizontal', gap: SP.sm, alignItems: 'center', width: ROW_W, padding: [SP.lg, 0]}, [
        icon(`${id}-nqi`, 'circle-alert', {size: 14, fill: MUTED}),
        text(`${id}-nqt`, '위 항목이 모두 통과하면 QR이 여기 나타납니다.', {size: '$--text-body', fill: MUTED}),
      ]),
    ];
    const phones = passing ? [
      groupTitle(`${id}-g1`, '연결된 폰 · 2 / 4'),
      row(`${id}-p1`, {label: 'iPhone 15 Pro', detail: '방금 · 알림 받는 중', trailing: linkButton(`${id}-p1r`, '해지', true)}),
      row(`${id}-p2`, {label: 'iPad', detail: '3일 전 · 4일 뒤 자동 해지', trailing: linkButton(`${id}-p2r`, '해지', true)}),
    ] : [];
    // The push mode is on Settings > Notifications (PRD settings-left-nav B7).
    return {...settingsSheetParts.settingsFrame(id, {page: 'mobile', lang: 'ko', height: passing ? 760 : 520, body: () => [...content, ...qrBlock, ...phones]}), name: passing ? 'Settings · Mobile · ready' : 'Settings · Mobile · blocked'};
  }

  // -- sheet --
  function labeled(id, label, note, node) {
    return frame(id, label, {layout: 'vertical', gap: SP.sm, alignItems: 'start'}, [
      text(`${id}-l`, label, {size: '$--text-title', weight: '600', fill: FG}),
      text(`${id}-n`, note, {size: '$--text-caption', fill: MUTED, width: node.width ?? PHONE_W}),
      node,
    ]);
  }
  function build(s) {
    const p = key => `mob-${key}-${s}`;
    const needs0 = AGENTS.needs[0];
    const rowFrame = (key, name, children) => frame(p(key), name, {layout: 'horizontal', gap: SP.xl, alignItems: 'start'}, children);
    return [frame(p('wrap'), 'Wrap', {layout: 'vertical', gap: SP.xl}, [
      rowFrame('row-settings', 'Settings', [
        labeled(p('cap-blocked'), '설정 › Mobile · 3단계에서 막힘', '막힌 단계만 강조하고 정확한 행동 하나를 붙입니다. QR은 안 나옵니다.', settings(p('set-blocked'), {passing: false})),
        labeled(p('cap-ready'), '설정 › Mobile · 모두 통과', 'QR + ts.net 주소 + 만료 카운트다운, 연결된 폰 목록(해지, 자동 해지 예고), 푸시 3단.', settings(p('set-ready'), {passing: true})),
      ]),
      rowFrame('row-phone', 'Phone', [
        labeled(p('cap-pair'), '① 페어링 확인', 'QR을 찍으면 Safari에 열리는 첫 화면. 연결을 누르면 코드가 교환되고 목록으로.', phone(p('pair'), 'Phone · Pairing', pairBody(p('pair-b')))),
        labeled(p('cap-list'), '② 목록', '내 차례 / 읽을 것 / 작업 중 / 확인함, 사이드바와 같은 이름과 네 표시. 요청은 warning, 소식은 bright, 확인한 건 dim. SSH 머신은 칩.', phone(p('list'), 'Phone · List', listBody(p('list-b')))),
        labeled(p('cap-detail'), '③ 상세', '읽기 전용 스크롤백(위로 당기면 더), 퀵키 5개, 한 줄 답장. 권한 프롬프트는 ↑↓⏎로 답합니다.', phone(p('detail'), 'Phone · Detail', detailBody(p('detail-b'), needs0))),
      ]),
      rowFrame('row-states', 'Phone states', [
        labeled(p('cap-unreach'), '④ 연결 안 됨', '한 줄로 두 원인을 말하고 자동 재시도. 목록은 마지막 상태로 흐리게.', phone(p('unreach'), 'Phone · Unreachable', listBody(p('unreach-b'), {unreachable: true}))),
        labeled(p('cap-empty'), '⑤ 비어 있음', '에이전트가 없을 때의 가장 작은 형태.', phone(p('empty'), 'Phone · Empty', listBody(p('empty-b'), {empty: true}))),
        labeled(p('cap-push'), '⑥ 푸시 배너', '루트 에이전트당 하나, 작업명 + 상태 + 프로젝트. 탭하면 ③으로.', pushBanner(p('push'))),
        labeled(p('cap-start'), '⑦ 시키기', '목록의 +가 여는 시트. 글, 대상(기본 This Mac · Home), 종류, 모델. 종류와 모델은 데스크톱의 마지막 값으로 미리 골라져 있고 시작하면 그 에이전트 상세로 간다.', phone(p('start'), 'Phone · Start', startBody(p('start-b')))),
      ]),
    ])];
  }
  return screenSheet('screen-mobile', 'Screen / Mobile', 'web/src/MobileTab.tsx, mobile.ts, and the phone app under web/src/mobile/ (entry web/mobile.html), PRD mobile-companion D-08 and D-12: Settings > Mobile after Devices, blocked with only the failing check lit and one action beside it and no QR, then ready with every check passing, the QR, the ts.net address, the code countdown with 새 코드, the connected phones (2 / 4) with 해지 and the automatic-removal notice, and the three push modes. The phone app: the pairing confirm the QR opens, the list in four groups (내 확인 대기, 끝, 진행 중, 확인함) on the desktop row rules at phone sizes, the detail with its read-only scrollback, five quick keys and one-line reply, the unreachable state (one line naming both causes over the dimmed last list), the empty state, the push banner that opens the detail, and the start sheet the list’s + opens (PRD home-device-rail D-24): a text box, target (default This Mac · Home), kind, model and 시작 over the dimmed list. The machine name and tailnet are placeholders and the QR is never a real code.', build, build);
}

// -- assembly ---------------------------------------------------------------------

// -- Screen / Disk Cleanup -------------------------------------------------------

const DISK_CLEANUP_SPEC = 'web/src/DiskCleanupSheet.tsx, diskCleanup.ts (PRD disk-layers B1-B28, worded from the shipped app; its way in is the Overview’s Projects tab, drawn in Screen / Overview, PRD overview-device-tasks B51): the sheet as a checkout x layer table (빌드 캐시, 의존성, 워크트리, 기타 read-only, 합계) with the checkbox for every unit picked at once (cell, row, column, top-left, the fold of small checkouts, all limited to the rows the filter shows; a worktree pick turns its row’s caches into the disabled Checked state, a partly picked group draws the Checkbox’s Indeterminate). The states the table carries follow: measuring, not measured, in use with its reason beside the name, usage unreadable, another cleanup running, no row for the filter. The table has no subtitle, no second header line and no legend, and its footer says only `N칸 · X`. The confirm step exists only when a worktree is picked and names the folder; the result is one line of the volume’s free space before and after over its per-cell rows. Sizes and names are invented mock content.';

function buildDiskCleanup(tokens) {
  const build = suffix => diskCleanupRows(tokens, {themedXref, screenButton, screenDialogSurface}, suffix);
  return screenSheet('screen-disk-cleanup', 'Screen / Disk Cleanup', DISK_CLEANUP_SPEC, build, build);
}

const FACTORY_SPEC = 'web/src/factory/ (FactoryScreen.tsx, MyTurn.tsx, FactoryBoard.tsx, FactoryGraph.tsx, FactoryCard.tsx, TaskPage.tsx, CreateSheet.tsx), web/src/components/sidebar-header.tsx (PRD software-factory-ui D-03, D-18; the board and the card from PRD factory-board-cards D-06..D-20; the 라인, 결정 필요 and Task page from PRD factory-human-loop D-09, D-31..D-35, D-43, B20..B33, approved ahead of the code): the Factory place in the window, drawn from one invented example data set so the 결정 필요 count, the 라인 tab number and the sidebar badge agree. The header is Factory, the project filter, the pause, + Factory 만들기 and 비서에게 묻기, then the tabs 라인 · 보드 · 그래프 · 설정 with the Factory\'s state at the row\'s end: the last GitHub read, dimmed once it is old, main 깨짐 ringed in red and the daily AI limit; there is no flow bar. 라인 opens with 결정 필요, only what the person moves, each item one shape: a sentence, what it stopped, two or three choices with their results and the recommended one, 다른 답, the default and its deadline, or one button for a to-do with a copyable command where one is needed, and the evidence folded at its foot. Under it one table row per Task (title, issue and PR, the four steps 접수 · 작업 · 검증 · 머지 with the current one named, one sentence of what it does now, elapsed time, how many answers Factory AI gave), the person\'s turn alone in the warning band, finished work dimmed, and 후속 후보 folded at the foot. A second frame draws every 결정 필요 kind at once: a question, a risk-path merge, a stop at the verification limit, Hide AI off, a to-do after recovery ran out, a command for the person and the GitHub login, with 권한 기다림 on the row it holds. The Task page reads top to bottom: title and state sentence, the track with who decided where, summary, the criteria checklist (충족, 미충족, 판단 불가), decisions split 내가 정한 것 / AI가 정한 것 with 다른 답, the activity timeline with four-part reports, the original issue rendered, and this Task\'s 후속 후보. The states are the 라인 with no Task (the label line), the first read (skeleton rows) and a Task page that could not be read (다시 읽기). 보드 has four lanes by movement with 멈춤 split into 나를 기다림 and 다른 걸 기다림; a card picks its size from its own width (small below 240, wide from 420), and 같은 카드, 세 크기 lays every kind out at the three sizes. 그래프 lays the Tasks out as small cards in layers with arrows, none for a dependency a longer path implies. The create sheet asks for the project, the verification and the merge mode; the Observer frames draw the settings in each mode, all projects, the worker pick and the cards the Observer changes. Every name, title and number is invented mock content.';

function buildFactory(tokens) {
  const build = suffix => factoryRows(tokens, {themedXref, screenButton, screenSelect, screenIconButton, screenDialogSurface, screenRadioItem, screenMenuItem, screenMenuSeparator, screenMenuContent}, suffix);
  return screenSheet('screen-factory', 'Screen / Factory', FACTORY_SPEC, build, build);
}

const CORE_MOVE_SPEC = 'web/src/settings/DevicesTab.tsx, DeviceRow.tsx, CoreMoveDialog.tsx, web/src/components/device-rail.tsx, web/src/badge.tsx, web/src/connection.ts, web/src/coreMove.ts (PRD core-host-node-move B2-B5, B10, B16, W1-W4, D-06; the design the operator chose in three review rounds): the core marker and the move\'s entry on today\'s Devices rows, the move as one dialog over Settings in five states (checks failed, confirm, moving, failed, done), the rail with the crown notched into the core machine\'s tile in both machines\' windows, and the window states outside Settings: the strip while the core moves, while this machine updates its core and while the core cannot be reached, the desktop host\'s status page for an app older than the core, and the strip after the operator ended the link here, which follows W3 and was not in the approved bundle. The copy is the approved bundle\'s; every name and number is invented example content.';

function buildCoreMove(tokens, root) {
  const build = suffix => coreMoveRows(tokens, root, {themedXref, screenIconButton, screenSwatch}, suffix);
  return screenSheet('screen-core-move', 'Screen / Core move', CORE_MOVE_SPEC, build, build);
}

export function readLocalVariables(root) {
  const {expected} = readTokenPlan(root);
  const {document} = loadCanvas(root);
  const libraryAuthored = {};
  for (const name of LIBRARY_AUTHORED_VARIABLES) {
    if (!(name in document.variables)) throw new Error(`pen-screens needs library-authored variable ${name}, which ${CANVAS} no longer carries`);
    libraryAuthored[name] = document.variables[name];
  }
  return {...Object.fromEntries(expected), ...libraryAuthored};
}

// -- Screen / Onboarding ---------------------------------------------------------

const ONBOARDING_SPEC = 'web/src/FirstRunWizard.tsx, NotificationControls.tsx, firstRun.ts, StartPanel.tsx, MainScreen.tsx (EmptyHomeView), sidebar.tsx, settings/NotificationsTab.tsx, ScreenIntro.tsx and desktop/static/status.css (PRD first-run-onboarding D-20, D-27, B1-B52): the first run in three steps as a dialog over the dimmed shell, with the three-segment bar and "1 of 3" on top. Step 1 is the seven agents in the kit order as tiles that say whether each is ready, signed out, checking or not installed, the What changes disclosure that lists each file Hide writes once with the agents that share it, the Hide AI line beside Continue, and its failed, no-agent and reopened states. Step 2 is the notification line on each system and in each answer macOS can give, with the one press that answers it and Also on your phone. Step 3 is Add a project inside the wizard. Then the start panel it closes into (with Not signed in beside an agent that is), the empty Home in its two states, the sidebar whose only Add project is the + on the Projects label, Settings > Notifications on macOS, on Windows and Linux and with no notification service, the one-time line on Factory, the startup and failure screen, and the longest copy (Korean at the largest text, Light). Paths, projects and the build line are invented.';

// Each board is a captioned section in one column. Its themed frame keeps the
// id and geometry the approved bundle's node has (agents/runs/first-run-onboarding
// design baseline, `<board>-light` / `<board>-dark`), so a design review exports
// the same node from this file; the ids inside are prefixed to stay unique.
function buildOnboarding(tokens, root) {
  const boards = onboardingBoards(tokens, root, {themedXref, screenButton, screenIconButton, screenSelect, screenSwatch, screenTabs});
  const prefix = node => ({...node, id: `onb-${node.id}`, ...(node.children ? {children: node.children.map(prefix)} : {})});
  const build = suffix => {
    const mode = suffix === 'l' ? 'Light' : 'Dark';
    return [frame(`onb-column-${suffix}`, 'Boards', {layout: 'vertical', gap: '$--spacing-xxl'},
      boards.filter(board => board.frames[suffix]).map(board => frame(`onb-${board.id}-${suffix}`, board.name, {layout: 'vertical', gap: '$--spacing-md'}, [
        text(`onb-${board.id}-${suffix}-title`, board.name, {size: '$--text-title', weight: '600'}),
        text(`onb-${board.id}-${suffix}-spec`, board.spec, {size: '$--text-caption', fill: '$--muted-foreground', width: 1100}),
        frame(`${board.id}-${mode.toLowerCase()}`, mode, {theme: {Mode: mode}, layout: 'vertical', gap: '$--spacing-lg', padding: '$--spacing-xl', fill: '$--background', cornerRadius: '$--radius-md', width: 'fit_content'}, board.frames[suffix].map(prefix)),
      ])))];
  };
  return screenSheet('screen-onboarding', 'Screen / Onboarding', ONBOARDING_SPEC, build, build);
}

// Sheets of screens the product no longer has (PRD overview-device-tasks D-16,
// D-20; the Work tool, sidebar-task-lens D-16, D-17): gen-screens removes them
// from design/hide-screens.pen.
export const RETIRED_SHEETS = [
  'Screen / Session record',
  'Screen / Project Overview', 'Screen / Project Sessions',
  'Screen / Work A. Window', 'Screen / Work B. PR card states', 'Screen / Work C. Tools column', 'Screen / Work D. Group rules',
  'Screen / Work E. Merge and update', 'Screen / Work F. Lens and card states', 'Screen / Work G. Elsewhere',
];

export function screenSheets(tokens, root) {
  setLibraryRoot(root, tokens);
  return [
    {name: 'Screen / Overview', build: () => buildOverview(tokens)},
    {name: 'Screen / Main', build: () => buildMain(tokens)},
    {name: 'Screen / Workspace', build: () => buildWorkspace(tokens)},
    {name: 'Screen / Settings', build: () => buildSettings(tokens, root)},
    {name: 'Screen / Project Settings', build: () => projectSettingsScreens(tokens, settingsParts(tokens, root, {themedXref, screenIconButton, screenSwatch}), {screenSheet, screenIconButton, screenInput})},
    {name: 'Screen / Usage', build: () => {
      const build = suffix => [themedXref(`screen-usage-popover-${suffix}`, 'usage-popover-light', 'Usage popover'),
        themedXref(`screen-usage-week-${suffix}`, 'usage-history-popover-week-light', 'Seven-day chart'),
        frame(`screen-usage-overview-${suffix}`, 'Overview Usage', {width: 1160, layout: 'vertical', padding: '$--spacing-lg', gap: '$--spacing-lg', fill: '$--background'}, [
          text(`screen-usage-title-${suffix}`, '개요 · 백로그   프로젝트   사용량', {size: '$--text-subhead', weight: '600'}),
          themedXref(`screen-usage-year-${suffix}`, 'usage-history-year-light', 'Date and agent records'),
          text(`screen-usage-limits-${suffix}`, '› 계정 사용 한도', {size: '$--text-caption', fill: '$--muted-foreground'}),
        ])];
      return screenSheet('screen-usage', 'Screen / Usage', 'web/src/components/usage.tsx: compact cost-first summary, 7-day bars / 30-day contribution grid switch and date/agent records. Values are invented examples; Settings / Usage owns the shared chip choices.', build, build);
    }},
    {name: 'Screen / Palette', build: () => buildPalette(tokens)},
    {name: 'Screen / Dialogs and Sheets', build: () => buildDialogs(tokens)},
    {name: 'Screen / Menus and Overlays', build: () => buildMenus(tokens)},
    {name: 'Screen / Projects Sidebar', build: () => buildProjectsSidebar(tokens)},
    {name: 'Screen / Mobile', build: () => buildMobile(tokens, root)},
    {name: 'Screen / Disk Cleanup', build: () => buildDiskCleanup(tokens)},
    {name: 'Screen / Onboarding', build: () => buildOnboarding(tokens, root)},
    {name: 'Screen / Factory', build: () => buildFactory(tokens)},
    {name: 'Screen / Core move', build: () => buildCoreMove(tokens, root)},

  ];
}
