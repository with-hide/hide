// The boards of `Screen / Settings` (PRD settings-left-nav D-11, B1-B15): the
// ⌘, sheet with the list on its left, App = General, Notifications, Devices,
// Shortcuts and This Mac = Agents, Hide AI, Mobile, GitHub, with About at the
// list's foot, and the chosen page on its right under its title alone. The
// rows are settings-rows.tsx's Group and Row on the library's System parts;
// the copy is the shipped catalog's and the values are invented. Drawn on this
// document's local tokens plus library refs, called from pen-screens.mjs, which
// owns the ref helpers and the sheet frame. `settingsFrame` is the sheet around
// any page, so a board that draws Settings elsewhere (Onboarding's
// Notifications, Core move's Devices) draws the same list and title.

import fs from 'node:fs';
import path from 'node:path';
import {BUTTON_VARIANTS, frame, icon, num, text as baseText} from './pen-system.mjs';

const SIZES = {micro: 10, caption: 11, body: 12, subhead: 13, title: 14, headline: 18};

// The list top to bottom (web/src/settings.ts SETTINGS_GROUPS) with each page's lucide glyph (web/src/SettingsSheet.tsx TAB_ICONS).
const GROUPS = [
  ['app', [['general', 'sliders-horizontal'], ['notifications', 'bell'], ['usage', 'gauge'], ['devices', 'monitor-smartphone'], ['shortcuts', 'keyboard']]],
  ['machine', [['agents', 'bot'], ['hideAi', 'sparkles'], ['mobile', 'smartphone'], ['github', 'github']]],
];

const COPY = {
  en: {
    settings: 'Settings', groups: {app: 'App', machine: 'This Mac'}, about: 'About',
    nav: {general: 'General', notifications: 'Notifications', usage: 'Usage', devices: 'Devices', shortcuts: 'Shortcuts', agents: 'Agents', hideAi: 'Hide AI', mobile: 'Mobile', github: 'GitHub'},
    general: {appearance: 'Appearance', language: 'Language', languageValue: 'Use system language', colorTheme: 'Color theme', colorThemes: ['Hide', 'Neutral'], theme: 'Mode', themes: ['System', 'Light', 'Dark'], accent: 'Accent', interfaceText: 'Interface text', density: 'Terminal and editor text keep their own size (⌘= and ⌘- in a pane or document).', pt: '13 pt'},
    notifications: {desktop: 'This Mac', notify: 'Notify me when an agent needs me', done: 'Notify me when an agent finishes', permission: 'macOS permission', allowed: 'Allowed', blocked: 'Blocked by macOS', openSystem: 'Open System Settings', testRow: 'Test notification', test: 'Send a test', sentWhen: 'Sent only while no Hide window is in front. The app icon counts agents in the notification groups you enable.', noService: 'This desktop has no notification service, so Hide can’t show notifications here.', phone: 'Phone', pushTo: 'Push to phone', pushValue: 'Only when the app is closed', pushDetail: "Send to your phone only while desktop hide isn't connected, and only when an agent needs your attention or finishes."},
    agents: {machines: ['This Mac', 'Mac mini'], installed: 'Installed 4', checkAgain: 'Check again', sessions: '2 sessions', ready: 'Ready', basic: 'Basic', notInstalled: 'Not installed 3', idle: 'Idle agents', sleepAfter: 'Sleep after', sleepValue: '3 days', sleeping: '4 sleeping now. Working, unread and on-screen agents never sleep.', starting: 'Starting work', link: 'Link the issue in pull requests'},
    hideAi: {use: 'Use Hide AI', useDesc: "Off: Hide makes no AI calls. Agent titles show each session's own text, and you name worktrees yourself.", runsOn: 'Runs on', runsOnNote: 'Uses your own login. Hide never asks for an API key.', agent: 'Agent', model: 'Model', signedIn: 'Signed in', modelValue: 'sonnet', fallbackTitle: "If Claude Code can't answer", fallbackNote: 'Tried in this order when Claude Code is signed out, out of usage or unavailable. None added: Hide AI waits for Claude Code.', cliDefault: 'CLI default', addAgent: 'Add agent', features: 'Features', summaries: 'Agent summaries', summariesDesc: 'A title and a one-line status for each agent', worktree: 'Worktree names', worktreeDesc: 'Suggested from the issue title when you start work'},
    github: {name: 'GitHub', description: 'Issues and pull requests through the gh CLI', connected: 'Connected'},
    aboutPage: {hide: 'hide', version: '0.1.0-main.9596ea18', herdr: 'Herdr', connected: 'Connected · 0.9.3', details: 'Details'},
  },
  ko: {
    settings: '설정', groups: {app: '앱', machine: '이 Mac'}, about: '정보',
    nav: {general: '일반', notifications: '알림', usage: '사용량', devices: '기기', shortcuts: '단축키', agents: '에이전트', hideAi: 'Hide AI', mobile: '모바일', github: 'GitHub'},
    hideAi: {use: 'Hide AI 사용', useDesc: '끄면 Hide가 AI를 호출하지 않아요. 에이전트 제목은 세션 원문으로 보이고, 워크트리 이름은 직접 입력해요.', runsOn: '사용 에이전트', runsOnNote: '내 로그인으로 동작해요. Hide는 API 키를 요구하지 않아요.', agent: '에이전트', model: '모델', signedIn: '로그인됨', modelValue: 'sonnet', fallbackTitle: 'Claude Code에서 답할 수 없을 때', fallbackNote: 'Claude Code에서 로그아웃, 사용량 소진, 사용 불가일 때 이 순서대로 시도해요. 추가하지 않으면 Claude Code만 사용해요.', cliDefault: 'CLI 기본값', addAgent: '에이전트 추가', features: '기능', summaries: '에이전트 요약', summariesDesc: '에이전트마다 제목과 한 줄 상태', worktree: '워크트리 이름', worktreeDesc: '작업을 시작할 때 이슈 제목으로 이름을 제안해요'},
  },
};

// Pen has no ellipsis and wraps Korean by character: a paragraph is broken at spaces here, on an estimated advance.
const HANGUL = /[ᄀ-ᇿ㄰-㆏가-힯　-〿]/;
const advance = (s, size) => [...s].reduce((a, ch) => a + (HANGUL.test(ch) ? size * 0.95 : ch === ' ' ? size * 0.3 : /[A-Z0-9]/.test(ch) ? size * 0.66 : /[.,:;'|!ilIj·\-()/]/.test(ch) ? size * 0.3 : size * 0.56), 0);
function wrap(s, width, size) {
  const lines = [];
  let line = '';
  for (const word of s.split(' ')) {
    const next = line ? `${line} ${word}` : word;
    if (line && advance(next, size) > width * 0.9) { lines.push(line); line = word; } else line = next;
  }
  if (line) lines.push(line);
  return lines.join('\n');
}

/**
 * The sheet and its pages for one document. `settingsSheet` draws a whole page
 * of the shipped catalog; `settingsFrame` wraps any page body in the list and
 * the title, at the widths `--size-settings-sheet-w` and `--size-settings-nav-w` set.
 */
export function settingsParts(tokens, root, {themedXref, screenIconButton, screenSwatch}) {
  const SHEET_W = num(tokens, '--size-settings-sheet-w');
  const NAV_W = num(tokens, '--size-settings-nav-w');
  const PAD_X = num(tokens, '--spacing-xl');
  const ALIAS = 'hideui';
  const manifest = JSON.parse(fs.readFileSync(path.join(root, 'web/src/assets/agents/manifest.json'), 'utf8'));
  const marks = [...manifest.existing, ...manifest.logos];
  const LOGO = Object.fromEntries(marks.map(({id, file}) => [id, path.posix.join('../web/src/assets/agents', file)]));
  const PLATED = new Set(marks.filter(mark => mark.plate).map(mark => mark.id));

  // The interface text scale of the longest-copy frame (B12); everything else draws at the token sizes.
  let SCALE = 1;
  const px = name => Math.round(SIZES[name] * SCALE * 10) / 10;
  const sz = name => SCALE === 1 ? `$--text-${name}` : px(name);
  const T = (id, content, {size = 'body', ...rest} = {}) => baseText(id, content, {size: sz(size), ...rest});
  const P = (id, content, {width, size = 'body', ...rest}) => T(id, wrap(content, width, px(size)), {size, width, ...rest});
  const spacer = id => frame(id, 'Spacer', {width: 'fill_container', height: 1}, []);
  const at = (node, x, y) => ({...node, x: Math.round(x), y: Math.round(y)});
  const scaleKey = (node, key, size = 'body') => SCALE === 1 ? node : {...node, descendants: {...node.descendants, [`${ALIAS}:${key}`]: {...(node.descendants?.[`${ALIAS}:${key}`] ?? {}), fontSize: sz(size)}}};

  // -- library parts -----------------------------------------------------------------------
  const sw = (id, on) => themedXref(id, 'sw-m', on ? 'On' : 'Off', on ? {fill: '$--primary', justifyContent: 'end'} : {});
  const select = (id, content, width = 200) => scaleKey(themedXref(id, 'sel-m', content, {width, justifyContent: 'space_between'}, {'sel-t': {content, fill: '$--foreground'}}), 'sel-t');
  function button(id, label, {variant = 'default', glyph, height} = {}) {
    const v = BUTTON_VARIANTS[variant];
    return scaleKey(themedXref(id, 'btn-m', label, {...v.overrides, ...(height ? {height} : {})}, {
      'btn-ic': glyph ? {icon: glyph, fill: v.fg, enabled: true} : {enabled: false}, 'btn-lb': {content: label, fill: v.fg},
    }), 'btn-lb');
  }
  const toggle = (id, items, lit) => frame(id, 'Toggle Group', {layout: 'horizontal', gap: '$--spacing-xxs', padding: '$--spacing-xxs', fill: '$--card', cornerRadius: '$--radius-sm'},
    items.map((label, i) => scaleKey(themedXref(`${id}-${i}`, 'tog-m', label, i === lit ? {fill: '$--secondary'} : {}, {'tog-t': {content: label, fill: i === lit ? '$--foreground' : '$--subtle-foreground'}}), 'tog-t')));
  // System / Slider's master lays its thumb out after the track, so an override of the thumb's x does not move it: the value is drawn here on the master's own sizes.
  function slider(id, fraction) {
    const w = Math.round(160 * fraction);
    return frame(id, 'Slider', {width: 160, height: 16, layout: 'none'}, [
      at(frame(`${id}-track`, 'Track', {width: 160, height: 4, fill: '$--secondary', cornerRadius: '$--radius-xl'}, []), 0, 6),
      at(frame(`${id}-range`, 'Range', {width: w, height: 4, fill: '$--primary', cornerRadius: '$--radius-xl'}, []), 0, 6),
      at({type: 'ellipse', id: `${id}-thumb`, name: 'Thumb', width: 16, height: 16, fill: '$--background', stroke: '$--primary', strokeWidth: 1, strokeAlignment: 'inner'}, w - 8, 0),
    ]);
  }
  // settings-rows.tsx Status: a mono symbol and words, never color alone.
  const TONE = {ok: ['✓', '$--success'], warn: ['!', '$--warning'], error: ['✕', '$--destructive'], pending: ['…', '$--subtle-foreground'], muted: ['·', '$--muted-foreground']};
  function status(id, tone, label) {
    const [symbol, fill] = TONE[tone];
    return frame(id, `Status · ${label}`, {layout: 'horizontal', gap: '$--spacing-xs', alignItems: 'center'}, [T(`${id}-s`, symbol, {fill, mono: true}), T(`${id}-t`, label, {fill})]);
  }
  const logo = (id, agent, size = 16) => PLATED.has(agent)
    ? frame(id, `${agent} mark`, {width: size, height: size, cornerRadius: '$--radius-xs', fill: '$--logo-plate', padding: 2, layout: 'horizontal'}, [
      frame(`${id}-img`, 'Mark', {width: size - 4, height: size - 4, fill: {type: 'image', enabled: true, url: LOGO[agent], mode: 'contain'}}, [])])
    : frame(id, `${agent} mark`, {width: size, height: size, cornerRadius: '$--radius-xs', fill: {type: 'image', enabled: true, url: LOGO[agent], mode: 'contain'}}, []);

  // -- the list and the title -------------------------------------------------------------
  function navItem(id, glyph, label, current) {
    return frame(id, label, {width: 'fill_container', height: Math.max(28, Math.round(px('subhead') + 14)), layout: 'horizontal', gap: '$--spacing-sm', alignItems: 'center', padding: [0, '$--spacing-sm'], cornerRadius: '$--radius-sm', ...(current ? {fill: '$--secondary'} : {})}, [
      icon(`${id}-i`, glyph, {size: 14, fill: current ? '$--foreground' : '$--subtle-foreground'}),
      T(`${id}-t`, label, {size: 'subhead', weight: current ? '500' : '400', fill: current ? '$--foreground' : '$--subtle-foreground'}),
    ]);
  }
  function nav(p, L, page, machine) {
    const groupLabel = (id, label) => frame(id, label, {width: 'fill_container', padding: ['$--spacing-xs', '$--spacing-sm', '$--spacing-xs', '$--spacing-sm']}, [T(`${id}-t`, label, {size: 'micro', weight: '500', fill: '$--muted-foreground'})]);
    return frame(`${p}-nav`, 'List', {width: NAV_W, height: 'fill_container', layout: 'vertical', gap: '$--spacing-xxs', padding: ['$--spacing-sm', '$--spacing-sm', '$--spacing-md', '$--spacing-sm'], fill: '$--sidebar', stroke: '$--border', strokeWidth: {right: 1}, strokeAlignment: 'inner'}, [
      frame(`${p}-nav-h`, 'Title', {width: 'fill_container', padding: [0, '$--spacing-sm', '$--spacing-xs', '$--spacing-sm']}, [T(`${p}-nav-ht`, L.settings, {size: 'title', weight: '600'})]),
      frame(`${p}-nav-pages`, 'Pages · scroll when taller than the available height', {width: 'fill_container', height: 'fill_container', layout: 'vertical', gap: '$--spacing-xxs', clip: true}, GROUPS.flatMap(([group, items]) => [
        groupLabel(`${p}-nav-${group}`, group === 'machine' && machine ? machine : L.groups[group]),
        ...items.map(([key, glyph]) => navItem(`${p}-nav-${key}`, glyph, L.nav[key], page === key)),
      ])),
      navItem(`${p}-nav-about`, 'info', L.about, page === 'about'),
    ]);
  }

  /**
   * The sheet with `page` lit in the list and `body(inner)` under its title; `device` is a remote device's chip beside the title (B4),
   * `machine` what this window's system calls itself (web/src/screenMachine.ts thisMachineKey: This Mac, This PC, This computer).
   */
  function settingsFrame(p, {page, lang = 'en', width = SHEET_W, height = 640, device = null, machine = null, body}) {
    const L = COPY[lang];
    const bodyW = width - NAV_W;
    const inner = bodyW - 2 * PAD_X;
    const title = page === 'about' ? L.about : L.nav[page];
    const chip = device ? [frame(`${p}-dev`, device, {height: 18, layout: 'horizontal', alignItems: 'center', padding: [0, '$--spacing-xs'], cornerRadius: '$--radius-sm', stroke: '$--border', strokeWidth: 1, strokeAlignment: 'inner'}, [T(`${p}-dev-t`, device, {size: 'caption', fill: '$--muted-foreground'})])] : [];
    return frame(`${p}-sheet`, `Settings · ${title}`, {width, height, layout: 'horizontal', cornerRadius: '$--radius-lg', fill: '$--popover', stroke: '$--border', strokeWidth: '$--size-hairline', strokeAlignment: 'inner', clip: true}, [
      nav(p, L, page, machine),
      frame(`${p}-body`, 'Page', {width: bodyW, height: 'fill_container', layout: 'vertical', gap: '$--spacing-lg', padding: ['$--spacing-lg', PAD_X, PAD_X, PAD_X], clip: true}, [
        frame(`${p}-head`, 'Title', {width: 'fill_container', layout: 'horizontal', alignItems: 'center'}, [
          frame(`${p}-title-g`, 'Title', {layout: 'horizontal', gap: '$--spacing-sm', alignItems: 'center'}, [T(`${p}-title`, title, {size: 'headline', weight: '600'}), ...chip]),
          spacer(`${p}-head-sp`), screenIconButton(`${p}-close`, 'x', {size: 24}),
        ]),
        ...body(inner),
      ]),
    ]);
  }

  // -- settings-rows.tsx Group and Row ----------------------------------------------------
  function group(id, inner, {title, action, rows, note}) {
    return frame(id, title ?? 'Group', {layout: 'vertical', gap: '$--spacing-sm', width: inner}, [
      ...(title || action ? [frame(`${id}-h`, 'Title', {width: 'fill_container', layout: 'horizontal', alignItems: 'center', gap: '$--spacing-md', ...(action ? {height: 28} : {})}, [
        ...(title ? [T(`${id}-t`, title, {weight: '600', fill: '$--subtle-foreground'})] : []), spacer(`${id}-hs`), ...(action ? [action] : []),
      ])] : []),
      frame(`${id}-box`, 'Box', {layout: 'vertical', width: inner, cornerRadius: '$--radius-md', fill: '$--card', stroke: '$--border', strokeWidth: '$--size-hairline', strokeAlignment: 'inner'},
        rows.flatMap((r, i) => i === 0 ? [r] : [frame(`${id}-sep${i}`, 'Divider', {width: 'fill_container', height: 1, fill: '$--border'}, []), r])),
      ...(note ? [P(`${id}-n`, note, {width: inner, fill: '$--muted-foreground'})] : []),
    ]);
  }
  const CONTROL_GAP = 12;
  function row(id, inner, {label, labelNode, desc, detail, controls = [], controlsWidth = 0}) {
    // A description stays in the label column beside the controls; a detail runs under the whole row.
    const labelWidth = inner - 24 - (controls.length ? controlsWidth + 2 * CONTROL_GAP : 0);
    const lead = labelNode ?? (desc
      ? frame(`${id}-lc`, 'Label', {layout: 'vertical', gap: '$--spacing-xxs', width: labelWidth}, [T(`${id}-l`, label, {size: 'subhead'}), P(`${id}-d`, desc, {width: labelWidth, fill: '$--muted-foreground'})])
      : T(`${id}-l`, label, {size: 'subhead'}));
    return frame(id, 'Row', {layout: 'vertical', gap: '$--spacing-xs', width: 'fill_container', padding: ['$--spacing-sm', '$--spacing-md']}, [
      frame(`${id}-top`, 'Line', {layout: 'horizontal', gap: CONTROL_GAP, alignItems: 'center', width: 'fill_container'}, [
        lead, spacer(`${id}-sp`),
        ...(controls.length ? [frame(`${id}-c`, 'Controls', {layout: 'horizontal', gap: '$--spacing-sm', alignItems: 'center'}, controls)] : []),
      ]),
      ...(detail ? [typeof detail === 'string' ? P(`${id}-dt`, detail, {width: inner - 24, fill: '$--muted-foreground'}) : detail] : []),
    ]);
  }
  function disclosure(id, label) {
    return frame(id, 'Disclosure', {layout: 'horizontal', gap: '$--spacing-sm', alignItems: 'center', width: 'fill_container', padding: ['$--spacing-sm', '$--spacing-md']}, [
      icon(`${id}-i`, 'chevron-right', {size: 14, fill: '$--muted-foreground'}), T(`${id}-t`, label, {size: 'subhead'}),
    ]);
  }
  const tag = (id, label) => frame(id, label, {height: 18, layout: 'horizontal', alignItems: 'center', padding: [0, 6], cornerRadius: '$--radius-xs', stroke: '$--border', strokeWidth: 1, strokeAlignment: 'inner'}, [T(`${id}-t`, label, {size: 'caption', fill: '$--subtle-foreground'})]);
  const agentName = (id, agent, name, extra = []) => frame(id, name, {layout: 'horizontal', gap: '$--spacing-sm', alignItems: 'center'}, [logo(`${id}-m`, agent), T(`${id}-t`, name, {size: 'subhead'}), ...extra]);
  // A select whose value carries the agent's mark (the Runs on Agent select).
  function markSelect(id, agent, content, width = 200) {
    return frame(id, content, {width, height: 28, layout: 'horizontal', gap: 6, alignItems: 'center', padding: [0, '$--spacing-sm'], cornerRadius: '$--radius-sm', fill: '$--background', stroke: '$--input', strokeWidth: 1, strokeAlignment: 'inner'}, [
      logo(`${id}-m`, agent, 14), T(`${id}-t`, content), spacer(`${id}-sp`), icon(`${id}-c`, 'chevron-down', {size: 14, fill: '$--muted-foreground'}),
    ]);
  }

  // -- pages -----------------------------------------------------------------------------
  /**
   * Notifications' desktop rows in each answer the system gives (first-run-onboarding B30, B19, B20):
   * on, off, blocked (macOS), pc-on, pc-off (Windows and Linux) and no-service (Linux).
   */
  function desktopRows(p, inner, C, state) {
    const on = state !== 'off' && state !== 'pc-off' && state !== 'no-service' && state !== 'done-only';
    const done = state === 'done-only' || state === 'both';
    if (state === 'no-service') {
      return [
        row(`${p}-notify`, inner, {label: C.notify, detail: status(`${p}-nsv`, 'muted', C.noService), controls: [{...sw(`${p}-nsw`, false), opacity: num(tokens, '--opacity-disabled')}], controlsWidth: 28}),
        row(`${p}-done`, inner, {label: C.done, controls: [{...sw(`${p}-dsw`, false), opacity: num(tokens, '--opacity-disabled')}], controlsWidth: 28}),
      ];
    }
    const rows = [
      row(`${p}-notify`, inner, {label: C.notify, controls: [sw(`${p}-nsw`, on)], controlsWidth: 28}),
      row(`${p}-done`, inner, {label: C.done, controls: [sw(`${p}-dsw`, done)], controlsWidth: 28}),
    ];
    if (state === 'on' || state === 'done-only' || state === 'both') rows.push(row(`${p}-perm`, inner, {label: C.permission, controls: [status(`${p}-ps`, 'ok', C.allowed), button(`${p}-test`, C.test, {variant: 'secondary'})], controlsWidth: 180}));
    if (state === 'blocked') rows.push(row(`${p}-perm`, inner, {label: C.permission, controls: [status(`${p}-ps`, 'warn', C.blocked), button(`${p}-open`, C.openSystem, {variant: 'secondary'})], controlsWidth: 260}));
    if (state === 'pc-on') rows.push(row(`${p}-tr`, inner, {label: C.testRow, controls: [button(`${p}-test`, C.test, {variant: 'secondary'})], controlsWidth: 100}));
    return rows;
  }

  const PAGES = {
    usage: (p, _L, inner) => [
      ...['Claude Code', 'Codex'].map((name, i) => group(`${p}-agent${i}`, inner, {rows: [
        row(`${p}-head${i}`, inner, {labelNode: agentName(`${p}-name${i}`, i === 0 ? 'claude' : 'codex', name), controls: [button(`${p}-hide${i}`, 'Hide', {variant: 'ghost', height: 24})], controlsWidth: 60}),
        ...(i === 0 ? [row(`${p}-five`, inner, {label: '5 hours', controls: [T(`${p}-five-value`, '11% · in 3h 12m', {mono: true, fill: '$--muted-foreground'}), sw(`${p}-five-switch`, false)], controlsWidth: 170})] : []),
        row(`${p}-week${i}`, inner, {label: '7 days', controls: [T(`${p}-week-value${i}`, '81% · in 5d 4h', {mono: true, fill: '$--muted-foreground'}), sw(`${p}-week-switch${i}`, true)], controlsWidth: 170}),
      ]})),
      group(`${p}-unsupported`, inner, {title: 'No limit reading', rows: [
        ['Grok', 'Hide does not read its weekly limit yet.'], ['OpenCode', 'Go limits need an API key, which Hide does not hold.'],
        ['Pi', 'No limit of its own; counts toward its signed-in account.'], ['omp', 'No limit of its own; counts toward its signed-in account.'],
        ['Cursor', 'Not supported: needs its stored sign-in.'],
      ].map(([label, detail], i) => row(`${p}-no${i}`, inner, {labelNode: agentName(`${p}-no-name${i}`, ['grok', 'opencode', 'pi', 'omp', 'cursor'][i], label), detail, controls: [], controlsWidth: 0}))}),
    ],
    general: (p, _L, inner, {theme}) => {
      const C = COPY.en.general;
      return [group(`${p}-app`, inner, {title: C.appearance, rows: [
        row(`${p}-lang`, inner, {label: C.language, controls: [select(`${p}-langv`, C.languageValue, 180)], controlsWidth: 180}),
        row(`${p}-color`, inner, {label: C.colorTheme, controls: [toggle(`${p}-colorv`, C.colorThemes, 0)], controlsWidth: 170}),
        row(`${p}-theme`, inner, {label: C.theme, controls: [toggle(`${p}-themev`, C.themes, theme === 'l' ? 1 : 2)], controlsWidth: 170}),
        row(`${p}-accent`, inner, {label: C.accent, controls: ['$--accent-choice-lime', '$--accent-choice-sky', '$--accent-choice-violet', '$--accent-choice-amber'].map((tk, i) => screenSwatch(`${p}-sw${i}`, tk, i === 0)), controlsWidth: 100}),
        row(`${p}-text`, inner, {label: C.interfaceText, detail: C.density, controls: [slider(`${p}-sl`, 1 / 3), T(`${p}-pt`, C.pt, {mono: true, fill: '$--subtle-foreground'})], controlsWidth: 210}),
      ]})];
    },
    notifications: (p, _L, inner, {notify = 'on', machine}) => {
      const C = COPY.en.notifications;
      const sends = notify === 'on' || notify === 'blocked' || notify === 'pc-on' || notify === 'done-only' || notify === 'both';
      return [
        group(`${p}-desk`, inner, {title: machine ?? C.desktop, note: sends ? C.sentWhen : null, rows: desktopRows(p, inner, C, notify)}),
        group(`${p}-phone`, inner, {title: C.phone, rows: [
          row(`${p}-push`, inner, {label: C.pushTo, detail: C.pushDetail, controls: [select(`${p}-pushv`, C.pushValue, 220)], controlsWidth: 220}),
        ]}),
      ];
    },
    agents: (p, _L, inner) => {
      const C = COPY.en.agents;
      return [
        toggle(`${p}-machines`, C.machines, 0),
        group(`${p}-inst`, inner, {title: C.installed, action: button(`${p}-check`, C.checkAgain, {variant: 'ghost', glyph: 'refresh-cw', height: 24}), rows: [
          row(`${p}-a0`, inner, {labelNode: agentName(`${p}-a0n`, 'claude-code', 'Claude Code'), controls: [status(`${p}-a0s`, 'ok', C.sessions), sw(`${p}-a0w`, true)], controlsWidth: 120}),
          row(`${p}-a1`, inner, {labelNode: agentName(`${p}-a1n`, 'codex', 'Codex'), controls: [status(`${p}-a1s`, 'ok', C.ready), sw(`${p}-a1w`, true)], controlsWidth: 100}),
          row(`${p}-a2`, inner, {labelNode: agentName(`${p}-a2n`, 'pi', 'Pi'), controls: [status(`${p}-a2s`, 'ok', C.ready), sw(`${p}-a2w`, true)], controlsWidth: 100}),
          row(`${p}-a3`, inner, {labelNode: agentName(`${p}-a3n`, 'cursor', 'Cursor', [tag(`${p}-a3c`, C.basic)]), controls: [sw(`${p}-a3w`, false)], controlsWidth: 28}),
        ]}),
        group(`${p}-not`, inner, {rows: [disclosure(`${p}-notd`, C.notInstalled)]}),
        group(`${p}-idle`, inner, {title: C.idle, note: C.sleeping, rows: [row(`${p}-sleep`, inner, {label: C.sleepAfter, controls: [select(`${p}-sleepv`, C.sleepValue, 160)], controlsWidth: 160})]}),
        group(`${p}-start`, inner, {title: C.starting, rows: [row(`${p}-link`, inner, {label: C.link, controls: [sw(`${p}-linkw`, true)], controlsWidth: 28})]}),
      ];
    },
    hideAi: (p, L, inner) => {
      const C = L.hideAi;
      const selW = 200;
      return [
        group(`${p}-use`, inner, {rows: [row(`${p}-usew`, inner, {label: C.use, desc: C.useDesc, controls: [sw(`${p}-usesw`, true)], controlsWidth: 28})]}),
        group(`${p}-runs`, inner, {title: C.runsOn, note: C.runsOnNote, rows: [
          row(`${p}-agent`, inner, {label: C.agent, controls: [markSelect(`${p}-agentv`, 'claude-code', 'Claude Code', selW), status(`${p}-agents`, 'ok', C.signedIn)], controlsWidth: selW + 8 + px('body') * 7}),
          row(`${p}-model`, inner, {label: C.model, controls: [select(`${p}-modelv`, C.modelValue, selW)], controlsWidth: selW}),
        ]}),
        group(`${p}-fb`, inner, {title: C.fallbackTitle, note: C.fallbackNote, rows: [
          row(`${p}-fb0`, inner, {labelNode: frame(`${p}-fb0n`, 'Codex', {layout: 'horizontal', gap: '$--spacing-sm', alignItems: 'center'}, [T(`${p}-fb0i`, '1', {mono: true, fill: '$--muted-foreground'}), logo(`${p}-fb0m`, 'codex'), T(`${p}-fb0t`, 'Codex', {size: 'subhead'})]),
            controls: [select(`${p}-fb0v`, C.cliDefault, 160), screenIconButton(`${p}-fb0x`, 'x', {size: 24})], controlsWidth: 192}),
          row(`${p}-fbadd`, inner, {labelNode: frame(`${p}-fbaddl`, 'Empty', {width: 1, height: 1}, []), controls: [button(`${p}-fbaddb`, C.addAgent, {variant: 'ghost', glyph: 'plus', height: 24})], controlsWidth: 120}),
        ]}),
        group(`${p}-feat`, inner, {title: C.features, rows: [
          row(`${p}-sum`, inner, {label: C.summaries, detail: C.summariesDesc, controls: [sw(`${p}-sumw`, true)], controlsWidth: 28}),
          row(`${p}-wt`, inner, {label: C.worktree, detail: C.worktreeDesc, controls: [sw(`${p}-wtw`, true)], controlsWidth: 28}),
        ]}),
      ];
    },
    github: (p, _L, inner) => {
      const C = COPY.en.github;
      const label = frame(`${p}-ghl`, 'Label', {layout: 'vertical', gap: '$--spacing-xxs'}, [T(`${p}-ghl-t`, C.name, {size: 'subhead'}), T(`${p}-ghl-d`, C.description, {fill: '$--muted-foreground'})]);
      return [group(`${p}-gh`, inner, {rows: [row(`${p}-ghr`, inner, {labelNode: label, controls: [status(`${p}-ghs`, 'ok', C.connected)], controlsWidth: 100})]})];
    },
    about: (p, _L, inner) => {
      const C = COPY.en.aboutPage;
      return [group(`${p}-about`, inner, {rows: [
        row(`${p}-hide`, inner, {label: C.hide, controls: [T(`${p}-hidev`, C.version, {mono: true, fill: '$--subtle-foreground'})], controlsWidth: 150}),
        row(`${p}-herdr`, inner, {label: C.herdr, controls: [status(`${p}-herdrs`, 'ok', C.connected)], controlsWidth: 140}),
        disclosure(`${p}-details`, C.details),
      ]})];
    },
  };

  /** The sheet with `page` open; `theme` picks which Theme item is lit, `lang` the copy, `scale` the interface text, `notify` the desktop notifications' answer. */
  function settingsSheet(p, {page, theme, lang = 'en', height = 640, width = SHEET_W, device = null, machine = null, scale = 1, notify}) {
    SCALE = scale;
    const L = COPY[lang];
    try {
      return settingsFrame(p, {page, lang, width, height, device, machine, body: inner => PAGES[page](p, L, inner, {theme, notify, machine})});
    } finally {
      SCALE = 1;
    }
  }

  return {settingsFrame, settingsSheet, group, row, select, status, button, sw, paragraph: P, SHEET_W, NAV_W, PAD_X};
}

/** The `Screen / Settings` boards, each {id, name, spec, build(suffix)} returning its frames for one theme. */
export function settingsBoards(tokens, root, helpers) {
  const {settingsSheet} = settingsParts(tokens, root, helpers);
  const inset = num(tokens, '--size-settings-sheet-window-inset');
  const at = (node, x, y) => ({...node, x: Math.round(x), y: Math.round(y)});
  const cell = (id, label, node) => frame(`${id}-cell`, label, {layout: 'vertical', gap: '$--spacing-sm'}, [
    baseText(`${id}-cell-cap`, label, {size: '$--text-caption', weight: '600', fill: '$--muted-foreground'}), node,
  ]);
  const hstack = (id, children) => frame(id, 'Row', {layout: 'horizontal', gap: '$--spacing-xl', alignItems: 'start'}, children);
  return [
    {id: 'set-pages', name: 'Pages', spec: 'Each page under its title alone, the list lit on it. General holds Appearance alone; Notifications holds This Mac and Phone; Agents keeps its machine switch inside the page (B2-B10).', build: s => [
      hstack(`set-r1-${s}`, [
        cell(`set-c-general-${s}`, 'General · Appearance alone', settingsSheet(`set-general-${s}`, {page: 'general', theme: s})),
        cell(`set-c-notif-${s}`, 'Notifications · This Mac and Phone in one page', settingsSheet(`set-notifications-${s}`, {page: 'notifications', theme: s})),
        cell(`set-c-usage-${s}`, 'Usage · account windows', settingsSheet(`set-usage-${s}`, {page: 'usage', theme: s, height: 800})),
        cell(`set-c-agents-${s}`, 'Agents · two machines, the switch stays in the page', settingsSheet(`set-agents-${s}`, {page: 'agents', theme: s, height: 700})),
      ]),
      hstack(`set-r2-${s}`, [
        cell(`set-c-hideai-${s}`, 'Hide AI', settingsSheet(`set-hideAi-${s}`, {page: 'hideAi', theme: s, height: 700})),
        cell(`set-c-github-${s}`, 'GitHub · the gh row as it was', settingsSheet(`set-github-${s}`, {page: 'github', theme: s})),
        cell(`set-c-about-${s}`, 'About · at the foot of the list', settingsSheet(`set-about-${s}`, {page: 'about', theme: s})),
      ]),
    ]},
    {id: 'set-states', name: 'States', spec: 'A remote device in front keeps its chip, now beside the page title on every page (B4). The smallest window, 720×480: the sheet is the window less its inset, the list keeps its width and only the page scrolls (B13). The longest copy, Korean at the largest interface text (17/13), Light only: the list keeps its width and nothing wraps or is cut in it (B12).', build: s => [
      hstack(`set-r3-${s}`, [
        cell(`set-c-remote-${s}`, 'Mac mini in front', settingsSheet(`set-remote-${s}`, {page: 'general', theme: s, device: 'Mac mini'})),
        cell(`set-c-small-${s}`, 'The smallest window, 720×480', frame(`set-small-${s}`, 'Window 720×480', {width: 720, height: 480, layout: 'none', fill: '$--background', cornerRadius: '$--radius-lg', clip: true, stroke: '$--border', strokeWidth: 1, strokeAlignment: 'inner'}, [
          at(frame(`set-small-scrim-${s}`, 'Scrim', {width: 720, height: 480, fill: '$--sidebar', opacity: num(tokens, '--opacity-secondary')}, []), 0, 0),
          at(settingsSheet(`set-small-in-${s}`, {page: 'notifications', theme: s, width: 720 - 2 * inset, height: 480 - 2 * inset}), inset, inset),
        ])),
        ...(s === 'l' ? [cell(`set-c-ko-${s}`, 'Hide AI · 한국어 · interface text 17/13', settingsSheet(`set-ko-${s}`, {page: 'hideAi', theme: s, lang: 'ko', height: 800, scale: 17 / 13}))] : []),
      ]),
    ]},
    {id: 'set-notification-choices', name: 'Notification choices', spec: 'Needs You and Done are independent choices in the existing desktop group. The permission/test row appears while either is on; the phone keeps its own settings.', build: s => [
      hstack(`set-nchoices-${s}`, ['off', 'on', 'done-only', 'both'].map(state => cell(`set-nchoice-${state}-${s}`, state, settingsSheet(`set-notification-${state}-${s}`, {page: 'notifications', theme: s, notify: state})))),
    ]},
  ];
}
