// The boards of `Screen / Onboarding` (PRD first-run-onboarding D-20, D-27,
// B1-B52): the three-step wizard as a dialog over the dimmed shell with every
// state its steps carry, the start panel it closes into, the empty Home, the
// sidebar's Add project on the Projects label, Settings >
// Notifications on each system, the one-time line on Factory, and the startup
// and failure screen. Drawn on this document's local tokens plus library refs
// (Button, Kbd, Switch, Select, Tabs, Icon Button, Tooltip), the way every other
// Screen sheet is, and called from pen-screens.mjs, which owns the ref helpers
// and the sheet frame. Paths, project names and the build line are invented.

import fs from 'node:fs';
import path from 'node:path';
import {frame, icon, num, text as baseText} from './pen-system.mjs';
import {settingsParts} from './pen-screens-settings.mjs';

/**
 * Every board as {id, name, spec, lightOnly, build(suffix)}; `build` returns the
 * board's frames for one theme. The board ids are unique within the sheet and
 * the caller prefixes them for the document.
 */
export function onboardingBoards(tokens, root, {themedXref, screenButton, screenIconButton, screenSelect, screenSwatch, screenTabs}) {
  // Each agent's mark as web/src/agentLogos.ts reads it, by its path from design/.
  const manifest = JSON.parse(fs.readFileSync(path.join(root, 'web/src/assets/agents/manifest.json'), 'utf8'));
  const LOGO = Object.fromEntries([...manifest.existing, ...manifest.logos].map(({id, file}) => [id, path.posix.join('../web/src/assets/agents', file)]));
  const W = 1280, H = 820, SIDEBAR = num(tokens, '--size-sidebar-ideal');
  const DW = num(tokens, '--size-onboarding-dialog-w');
  const PAD = num(tokens, '--spacing-lg');
  const INNER = DW - 2 * PAD;
  const SCRIM = num(tokens, '--opacity-secondary');
  const DIM = num(tokens, '--opacity-dimmed');
  const SHADOW = {type: 'shadow', shadowType: 'outer', offset: {x: 0, y: 12}, blur: 32, spread: 0, color: '#00000040'};
  const SIZES = {micro: 10, caption: 11, body: 12, subhead: 13, title: 14, headline: 18};

  // Text scale and language for the longest-copy board (B41).
  let SCALE = 1;
  const sz = name => SCALE === 1 ? `$--text-${name}` : Math.round(SIZES[name] * SCALE * 10) / 10;
  const T = (id, content, {size = 'body', ...rest} = {}) => baseText(id, content, {size: sz(size), ...rest});
  const F = frame;
  // Pen image fills use raster derivatives of the canonical vector stills.
  // They are design artwork, not screenshots of a running application.
  const owl = (p, pose, size) => F(p, `Decorative owl / ${pose} / static`, {width: num(tokens, size.slice(1)), height: num(tokens, size.slice(1)), fill: {type: 'image', url: `./assets/owls/owl-${pose}.png`, mode: 'contain'}}, []);
  const at = (node, x, y) => ({...node, x: Math.round(x), y: Math.round(y)});
  const spacer = id => F(id, 'Spacer', {width: 'fill_container', height: 1}, []);

  function scaleRef(node, key) {
    if (SCALE === 1) return node;
    const k = `hideui:${key}`;
    return {...node, descendants: {...node.descendants, [k]: {...(node.descendants?.[k] ?? {}), fontSize: sz(key === 'kbd-t' ? 'caption' : 'body')}}};
  }
  const btn = (id, label, opts = {}) => scaleRef(screenButton(id, label, opts), 'btn-lb');
  const kbd = (id, label) => scaleRef(themedXref(id, 'kbd-m', label, {}, {'kbd-t': {content: label}}), 'kbd-t');
  const sw = (id, on) => themedXref(id, 'sw-m', on ? 'On' : 'Off', on ? {fill: '$--primary', justifyContent: 'end'} : {});
  const iconBtn = (id, glyph) => screenIconButton(id, glyph);

  // settings-rows.tsx Status: a mono symbol and words, never color alone.
  const TONE = {ok: ['✓', '$--success'], warn: ['!', '$--warning'], error: ['✕', '$--destructive'], pending: ['…', '$--subtle-foreground'], muted: ['·', '$--muted-foreground']};
  function status(id, tone, label, {size = 'body', weight = '400'} = {}) {
    const [symbol, fill] = TONE[tone];
    return F(id, `Status · ${label}`, {layout: 'horizontal', gap: '$--spacing-xs', alignItems: 'center'}, [
      T(`${id}-s`, symbol, {size, fill, mono: true}),
      T(`${id}-t`, label, {size, fill, weight}),
    ]);
  }
  const link = (id, label, {size = 'body'} = {}) => T(id, label, {size, fill: '$--foreground', weight: '500'});

  // -- copy -------------------------------------------------------------------------

  const EN = {
    of: n => `${n} of 3`,
    s1Title: 'Choose your agents',
    s1Sub: 'Hide adds its own entries to each agent you turn on, so it can tell when an agent needs you. Nothing changes until you continue.',
    ready: 'Ready', nosign: 'Not signed in', checking: 'Checking…', installed: 'Installed', none: 'Not installed', install: 'Install ↗',
    what: 'What changes',
    devices: 'Connected devices get the same choice, each by what is installed there.',
    hideAi: 'Hide AI uses Claude Code',
    cont: 'Continue',
    noneInstalled: 'No agent is installed yet.', installClaude: 'Install Claude Code ↗', installCodex: 'Install Codex ↗',
    failed: 'Codex was not set up: ~/.codex/hooks.json could not be written.', retry: 'Try again',
    hook: 'hook', skill: 'skill', plugin: 'plugin', extension: 'extension', herdr: 'Herdr integration', command: 'the hide command',
    s2Title: 'Know when an agent needs you',
    s2Sub: 'When an agent needs your attention while Hide is in the background, macOS shows a notification. Click it to go to that agent.',
    s2SubWin: 'When an agent needs your attention while Hide is in the background, Windows shows a notification. Click it to go to that agent.',
    s2SubLinux: 'When an agent needs your attention while Hide is in the background, your desktop shows a notification. Click it to go to that agent.',
    noService: 'This desktop has no notification service, so Hide can’t show notifications here.',
    notOnWindows: 'Not on Windows yet',
    turnOn: 'Turn on notifications', notNow: 'Not now', waiting: 'Waiting for your answer…',
    on: 'Notifications are on', test: 'Send a test', blocked: 'macOS is blocking Hide’s notifications', openSettings: 'Open System Settings',
    unreadable: 'Hide can’t read the notification setting', phone: 'Also on your phone →',
  };
  const KO = {
    of: n => `${n} / 3`,
    s1Title: '함께 쓸 에이전트를 고르세요',
    s1Sub: '켠 에이전트마다 Hide가 자기 항목을 더해, 에이전트가 나를 부르는 순간을 알아챕니다. 계속을 누르기 전에는 아무것도 바꾸지 않습니다.',
    ready: '준비됨', nosign: '로그인 필요', checking: '확인 중…', installed: '설치됨', none: '설치 안 됨', install: '설치 ↗',
    what: '무엇이 바뀌나요',
    devices: '연결된 기기에도 같은 선택이 적용되며, 각 기기에 설치된 에이전트 기준입니다.',
    hideAi: 'Hide AI가 쓰는 에이전트: Claude Code',
    cont: '계속',
    hook: '훅', skill: '스킬', plugin: '플러그인', extension: '확장', herdr: 'Herdr 연동', command: 'hide 명령',
    s2Title: '에이전트가 나를 부르면 알려 드릴게요',
    s2Sub: 'Hide가 뒤에 있을 때 에이전트가 나를 부르면 macOS 알림을 보냅니다. 누르면 그 에이전트로 갑니다.',
    turnOn: '알림 켜기', notNow: '나중에', waiting: 'macOS 창에서 답을 기다리는 중…',
    on: '알림이 켜졌습니다', test: '테스트 알림', blocked: 'macOS가 Hide 알림을 막고 있습니다', openSettings: '시스템 설정 열기',
    unreadable: 'Hide가 알림 설정을 읽지 못합니다', phone: '폰으로도 받기 →',
  };
  let L = EN;

  // What the shortcut table prints per system (web/src/shortcuts.ts pcCommand,
  // docs/UI_BEHAVIOR.md Keyboard shortcuts per system; fieldLabel for ⌘↵).
  let OS = 'mac';
  const KEYS = {
    mac: {enter: '⌘↵', add: '⇧⌘N', start: '⌘N', overview: '⇧⌘O', factory: '⇧⌘F'},
    pc: {enter: 'Ctrl+Enter', add: 'Alt+Shift+N', start: 'Ctrl+Shift+N', overview: 'Alt+Shift+O', factory: 'Alt+Shift+F'},
  };
  const K = () => KEYS[OS === 'mac' ? 'mac' : 'pc'];

  // -- agents (step 1) -------------------------------------------------------------

  const AGENT_LABEL = {'claude-code': 'Claude Code', codex: 'Codex', grok: 'Grok', opencode: 'OpenCode', pi: 'Pi', omp: 'omp', cursor: 'Cursor'};
  // The order the kit lists; each state is one the system can answer (D-06).
  const MIXED = [
    ['claude-code', 'ready', true], ['codex', 'nosign', true], ['grok', 'none', false], ['opencode', 'installed', true],
    ['pi', 'checking', true], ['omp', 'installed', false], ['cursor', 'none', false],
  ];
  const NOTHING = Object.keys(AGENT_LABEL).map(id => [id, 'none', false]);

  const TILE_W = Math.floor((INNER - 3 * 8) / 4);
  function tile(p, [agent, state, on], {tileH = Math.round(112 * SCALE)} = {}) {
    const none = state === 'none' || state === 'unsupported';
    const switchable = !none;
    const plate = F(`${p}-plate`, 'Logo plate', {width: 40, height: 40, cornerRadius: '$--radius-md', fill: '$--logo-plate', layout: 'horizontal', alignItems: 'center', justifyContent: 'center', clip: true, ...(none ? {opacity: DIM} : {})}, [
      F(`${p}-logo`, 'Logo', {width: 32, height: 32, fill: {type: 'image', enabled: true, url: LOGO[agent], mode: 'contain'}}, []),
    ]);
    const line = {
      ready: () => status(`${p}-st`, 'ok', L.ready, {size: 'caption'}),
      nosign: () => status(`${p}-st`, 'warn', L.nosign, {size: 'caption'}),
      checking: () => T(`${p}-st`, L.checking, {size: 'caption', fill: '$--subtle-foreground'}),
      installed: () => T(`${p}-st`, L.installed, {size: 'caption', fill: '$--muted-foreground'}),
      none: () => T(`${p}-st`, L.none, {size: 'caption', fill: '$--muted-foreground'}),
      unsupported: () => T(`${p}-st`, L.notOnWindows, {size: 'caption', fill: '$--muted-foreground'}),
    }[state]();
    const children = [
      plate,
      T(`${p}-name`, AGENT_LABEL[agent], {weight: '600', ...(none ? {fill: '$--muted-foreground'} : {})}),
      line,
      ...(state === 'none' ? [link(`${p}-install`, L.install, {size: 'caption'})] : []),
    ];
    if (switchable) {
      children.push(F(`${p}-check`, 'Check', {
        width: 16, height: 16, cornerRadius: 8, layout: 'horizontal', alignItems: 'center', justifyContent: 'center',
        ...(on ? {fill: '$--primary'} : {}), stroke: on ? '$--primary' : '$--border', strokeWidth: 1, strokeAlignment: 'inner',
        layoutPosition: 'absolute', x: TILE_W - 16 - 6, y: 6,
      }, on ? [icon(`${p}-checki`, 'check', {size: 12, fill: '$--primary-foreground'})] : []));
    }
    return F(p, `Tile · ${AGENT_LABEL[agent]} · ${state}${switchable ? (on ? ' · on' : ' · off') : ''}`, {
      width: TILE_W, height: tileH, layout: 'vertical', alignItems: 'center', justifyContent: 'center', gap: '$--spacing-xs', padding: '$--spacing-sm',
      cornerRadius: '$--radius-md', stroke: on ? '$--primary' : '$--border', strokeWidth: 1, strokeAlignment: 'inner', ...(on ? {fill: '$--card'} : {}),
    }, children);
  }
  function tiles(p, agents, opts) {
    const rows = [];
    for (let i = 0; i < agents.length; i += 4) rows.push(F(`${p}-r${i}`, 'Row', {layout: 'horizontal', gap: 8}, agents.slice(i, i + 4).map(a => tile(`${p}-${a[0]}`, a, opts))));
    return F(p, 'Agents', {layout: 'vertical', gap: 8}, rows);
  }

  function whatChanges(p, {open, agents, width = INNER}) {
    const on = new Set(agents.filter(([, state, isOn]) => state !== 'none' && state !== 'unsupported' && isOn).map(([id]) => id));
    // Each location as KitAgentSnapshot reports it (hide-kit agent_kit.rs:
    // hook config_path, plugin file.path, skill_location, the Herdr integration
    // folder), with the home folder shown as ~ on every system.
    // On Windows only Claude Code (hook, skill), Codex (hook) and Pi (skill) get
    // Hide's own pieces (hide-agent-adapter skill_os, plugin and guidance
    // supported_here); the Herdr integration is Herdr's, listed for each agent on.
    const win = OS === 'win';
    const home = (...parts) => (win ? '~\\' : '~/') + parts.join(win ? '\\' : '/');
    const skill = dir => home(...dir, 'hide-browser', 'SKILL.md');
    const PIECES = {
      'claude-code': [[home('.claude', 'settings.json'), L.hook], [skill(['.claude', 'skills']), L.skill], [home('.claude'), L.herdr]],
      codex: win ? [[home('.codex', 'hooks.json'), L.hook], [home('.codex'), L.herdr]]
        : [[home('.codex', 'hooks.json'), L.hook], [skill(['.agents', 'skills']), L.skill], [home('.codex'), L.herdr]],
      grok: [[home('.grok', 'hooks', 'hide.json'), L.hook], [skill(['.agents', 'skills']), L.skill], [home('.grok'), L.herdr]],
      opencode: [[home('.config', 'opencode', 'plugins', 'hide.js'), L.plugin], [skill(['.agents', 'skills']), L.skill], [home('.config', 'opencode'), L.herdr]],
      pi: win ? [[skill(['.agents', 'skills']), L.skill], [home('.pi', 'agent'), L.herdr]]
        : [[home('.pi', 'agent', 'extensions', 'hide.ts'), L.extension], [skill(['.agents', 'skills']), L.skill], [home('.pi', 'agent'), L.herdr]],
      omp: [[home('.omp', 'agent', 'extensions', 'hide.ts'), L.extension], [skill(['.agents', 'skills']), L.skill], [home('.omp', 'agent'), L.herdr]],
      cursor: [[home('.cursor', 'hooks.json'), L.hook], [skill(['.agents', 'skills']), L.skill], [home('.cursor'), L.herdr]],
    };
    // Agents that share one skill file (~/.agents/skills) share its row.
    const byPath = new Map();
    for (const id of Object.keys(AGENT_LABEL).filter(id => on.has(id))) {
      for (const [where, what] of PIECES[id]) {
        if (!byPath.has(where)) byPath.set(where, {what, who: []});
        byPath.get(where).who.push(AGENT_LABEL[id]);
      }
    }
    const rows = [...byPath].map(([where, {what, who}]) => [where, `${who.join(', ')} · ${what}`]);
    rows.push([win ? home('.local', 'bin', 'hide.cmd') : home('.local', 'bin', 'hide'), L.command]);
    const head = F(`${p}-head`, 'Disclosure', {layout: 'horizontal', gap: '$--spacing-xs', alignItems: 'center'}, [
      icon(`${p}-chev`, open ? 'chevron-down' : 'chevron-right', {size: 14, fill: '$--muted-foreground'}),
      T(`${p}-t`, L.what, {fill: '$--subtle-foreground', weight: '500'}),
    ]);
    if (!open) return head;
    return F(p, 'What changes', {layout: 'vertical', gap: '$--spacing-xs', width}, [
      head,
      F(`${p}-list`, 'Paths', {layout: 'vertical', gap: 2, width, padding: ['$--spacing-xs', '$--spacing-md'], cornerRadius: '$--radius-md', fill: '$--card'},
        rows.map(([where, who], i) => F(`${p}-row${i}`, where, {layout: 'horizontal', gap: '$--spacing-md', alignItems: 'center', width: 'fill_container', height: Math.round(20 * SCALE)}, [
          T(`${p}-row${i}-p`, where, {size: 'caption', mono: true}),
          spacer(`${p}-row${i}-sp`),
          T(`${p}-row${i}-w`, who, {size: 'caption', fill: '$--muted-foreground'}),
        ]))),
    ]);
  }

  // -- the wizard surface: a dialog over the dimmed shell ---------------------------

  function progress(p, step) {
    return F(p, `${step} of 3`, {layout: 'horizontal', gap: '$--spacing-sm', alignItems: 'center'}, [
      F(`${p}-bar`, 'Segments', {layout: 'horizontal', gap: 4}, [1, 2, 3].map(i => F(`${p}-s${i}`, `Segment ${i}`, {width: 24, height: 4, cornerRadius: 2, fill: i <= step ? '$--primary' : '$--secondary'}, []))),
      T(`${p}-t`, L.of(step), {size: 'caption', fill: '$--muted-foreground'}),
    ]);
  }

  function wizard(p, {step, title, desc, body = [], footLeft = [], actions = [], close = false, width = DW}) {
    return F(p, `Wizard · step ${step}`, {
      width, layout: 'vertical', gap: '$--spacing-md', padding: '$--spacing-lg', cornerRadius: '$--radius-lg', fill: '$--popover',
      stroke: '$--border', strokeWidth: 1, strokeAlignment: 'inner', effect: SHADOW,
    }, [
      F(`${p}-hdr`, 'Header', {layout: 'vertical', gap: '$--spacing-sm', width: 'fill_container'}, [
        ...(step ? [F(`${p}-top`, 'Top', {layout: 'horizontal', alignItems: 'center', width: 'fill_container'}, [
          progress(`${p}-prog`, step), spacer(`${p}-topsp`), owl(`${p}-owl`, step === 1 ? 'agents' : step === 2 ? 'devices' : 'ai', '$--size-owl-onboarding'), ...(close ? [iconBtn(`${p}-x`, 'x')] : []),
        ])] : []),
        T(`${p}-title`, title, {size: 'headline', weight: '600'}),
        ...(desc ? [T(`${p}-desc`, desc, {fill: '$--subtle-foreground', width: width - 2 * PAD})] : []),
      ]),
      ...body,
      F(`${p}-ftr`, 'Footer', {layout: 'horizontal', gap: '$--spacing-sm', alignItems: 'center', width: 'fill_container'}, [...footLeft, spacer(`${p}-ftrsp`), ...actions]),
    ]);
  }

  const continueKeys = (p, label = L.cont, variant = 'default') => [btn(`${p}-go`, label, {variant}), kbd(`${p}-gok`, K().enter)];

  function step1(p, {agents = MIXED, open = false, failed = false, close = false, hideAi = true} = {}) {
    const empty = agents === NOTHING;
    return wizard(p, {
      step: 1, title: L.s1Title, desc: L.s1Sub, close,
      body: [
        ...(empty ? [F(`${p}-empty`, 'No agent', {layout: 'horizontal', gap: '$--spacing-md', alignItems: 'center'}, [
          status(`${p}-emptys`, 'muted', L.noneInstalled), link(`${p}-emptyc`, L.installClaude), link(`${p}-emptyx`, L.installCodex),
        ])] : []),
        tiles(`${p}-tiles`, agents),
        whatChanges(`${p}-what`, {open, agents}),
        T(`${p}-devices`, L.devices, {size: 'caption', fill: '$--muted-foreground', width: INNER}),
        ...(failed ? [F(`${p}-fail`, 'Apply failed', {layout: 'horizontal', gap: '$--spacing-sm', alignItems: 'center'}, [status(`${p}-fails`, 'error', L.failed)])] : []),
      ],
      footLeft: hideAi && !empty ? [T(`${p}-ai`, L.hideAi, {size: 'caption', fill: '$--muted-foreground'})] : [],
      actions: failed ? continueKeys(p, L.retry) : continueKeys(p),
    });
  }

  function step2(p, state, {windows = false, linux = false} = {}) {
    const phone = link(`${p}-phone`, L.phone);
    const line = {
      waiting: () => F(`${p}-state`, 'Waiting', {layout: 'horizontal', gap: '$--spacing-xs', alignItems: 'center'}, [icon(`${p}-state-i`, 'loader-circle', {size: 12, fill: '$--subtle-foreground'}), T(`${p}-state-t`, L.waiting, {fill: '$--subtle-foreground'})]),
      on: () => F(`${p}-state`, 'On', {layout: 'horizontal', gap: '$--spacing-md', alignItems: 'center', width: 'fill_container'}, [status(`${p}-state-s`, 'ok', L.on, {weight: '500'}), spacer(`${p}-state-sp`), btn(`${p}-test`, L.test, {variant: 'secondary'})]),
      blocked: () => F(`${p}-state`, 'Blocked', {layout: 'horizontal', gap: '$--spacing-md', alignItems: 'center', width: 'fill_container'}, [status(`${p}-state-s`, 'warn', L.blocked, {weight: '500'}), spacer(`${p}-state-sp`), btn(`${p}-open`, L.openSettings, {variant: 'secondary'})]),
      noService: () => status(`${p}-state`, 'muted', L.noService),
      unreadable: () => F(`${p}-state`, 'Unreadable', {layout: 'horizontal', gap: '$--spacing-md', alignItems: 'center', width: 'fill_container'}, [status(`${p}-state-s`, 'muted', L.unreadable), spacer(`${p}-state-sp`), btn(`${p}-test`, L.test, {variant: 'secondary'})]),
    }[state];
    const actions = {
      initial: () => [btn(`${p}-later`, L.notNow, {variant: 'ghost'}), btn(`${p}-on`, L.turnOn)],
      waiting: () => [btn(`${p}-later`, L.notNow, {variant: 'ghost'}), {...btn(`${p}-on`, L.turnOn), opacity: 0.72}],
      on: () => continueKeys(p),
      blocked: () => continueKeys(p, L.cont, 'secondary'),
      unreadable: () => continueKeys(p),
      noService: () => continueKeys(p),
    }[state]();
    return wizard(p, {step: 2, title: L.s2Title, desc: linux ? L.s2SubLinux : windows ? L.s2SubWin : L.s2Sub, body: line ? [line()] : [], footLeft: [phone], actions});
  }

  function step3(p, {close = false} = {}) {
    const way = (id, glyph, title, detail, {card = false, enter = false} = {}) => F(id, title, {
      layout: 'horizontal', gap: '$--spacing-md', alignItems: 'center', width: 'fill_container', padding: ['$--spacing-sm', '$--spacing-md'], cornerRadius: '$--radius-md',
      ...(card ? {fill: '$--card', stroke: '$--border', strokeWidth: 1, strokeAlignment: 'inner'} : {}),
    }, [
      F(`${id}-g`, 'Glyph', {width: 28, height: 28, cornerRadius: '$--radius-sm', fill: '$--secondary', layout: 'horizontal', alignItems: 'center', justifyContent: 'center'}, [icon(`${id}-gi`, glyph, {size: 14})]),
      F(`${id}-b`, 'Text', {layout: 'vertical', gap: 0, width: 'fill_container'}, [T(`${id}-t`, title, {size: 'subhead', weight: '500'}), T(`${id}-d`, detail, {fill: '$--muted-foreground'})]),
      ...(enter ? [kbd(`${id}-k`, '↵')] : []),
    ]);
    return wizard(p, {
      step: 3, title: 'Add your first project', close,
      body: [
        F(`${p}-host`, 'Host', {layout: 'horizontal', gap: '$--spacing-sm', alignItems: 'center'}, [T(`${p}-hostl`, 'Host', {fill: '$--subtle-foreground'}), screenSelect(`${p}-hostv`, {content: 'This Mac', width: 120})]),
        way(`${p}-browse`, 'folder-open', 'Browse folder', 'A project, a Git repository, or a folder of repositories', {card: true, enter: true}),
        F(`${p}-other`, 'Other ways', {layout: 'vertical', gap: '$--spacing-xs', width: 'fill_container'}, [
          T(`${p}-otherl`, 'Other ways to add', {size: 'caption', fill: '$--muted-foreground'}),
          way(`${p}-clone`, 'link', 'Clone from URL', 'A Git repository copied into a new folder'),
          way(`${p}-create`, 'folder-plus', 'Create new project', 'A new folder with its own Git repository'),
        ]),
      ],
      actions: [btn(`${p}-skip`, 'Skip for now', {variant: 'ghost'})],
    });
  }


  // -- the shell behind -------------------------------------------------------------

  function centered(p, message, button, keys, showOwl = false) {
    return F(p, 'Empty Home', {width: 'fill_container', height: 'fill_container', layout: 'vertical', gap: '$--spacing-md', alignItems: 'center', justifyContent: 'center'}, [
      ...(showOwl ? [owl(`${p}-owl`, 'ai', '$--size-owl-empty')] : []),
      T(`${p}-msg`, message, {size: 'subhead', fill: '$--subtle-foreground'}),
      ...(button ? [F(`${p}-act`, 'Action', {layout: 'horizontal', gap: '$--spacing-sm', alignItems: 'center'}, [btn(`${p}-b`, button), kbd(`${p}-k`, keys)])] : []),
    ]);
  }

  function windowFrame(p, children, {width = W, height = H} = {}) {
    return F(p, 'Window', {width, height, layout: 'none', fill: '$--background', cornerRadius: 10, clip: true, stroke: '$--border', strokeWidth: 1, strokeAlignment: 'inner'}, children);
  }

  function shellWithDialog(p, dialog, {project = null} = {}) {
    return windowFrame(p, [
      at(deviceRail(`${p}-rail`), 0, 0),
      at(realSidebar(`${p}-side`, {projects: project ? [project] : false}), RAIL, 0),
      at(homeMain(`${p}-main`, {projects: project ? [project] : false, showOwl: false}), RAIL + SIDEBAR, 0),
      at(F(`${p}-scrim`, 'Scrim', {width: W, height: H, fill: '$--background', opacity: SCRIM}, []), 0, 0),
      at(F(`${p}-center`, 'Center', {width: W, height: H, layout: 'vertical', alignItems: 'center', justifyContent: 'center'}, [dialog]), 0, 0),
    ]);
  }

  // -- start panel (B23) -------------------------------------------------------------

  function startPanel(p, {kind = 'claude', label = 'Claude Code', signedIn = true}) {
    const ASK_W = num(tokens, '--size-search-sheet-w');
    const dd = (id, lead, text, width) => F(id, text, {width, height: 26, layout: 'horizontal', gap: 6, alignItems: 'center', padding: [0, '$--spacing-sm'], cornerRadius: '$--radius-sm', fill: '$--secondary'}, [
      lead, T(`${id}-t`, text, {size: 'caption'}), spacer(`${id}-sp`), icon(`${id}-c`, 'chevron-down', {size: 12, fill: '$--muted-foreground'}),
    ]);
    const art = (id, provider) => F(id, `${provider} mark`, {width: 14, height: 14, fill: {type: 'image', enabled: true, url: LOGO[provider === 'claude' ? 'claude-code' : provider], mode: 'contain'}}, []);
    return F(`${p}-ask`, '⌘N', {width: ASK_W, layout: 'vertical', fill: '$--popover', cornerRadius: '$--radius-lg', stroke: '$--border', strokeWidth: 1, strokeAlignment: 'inner', clip: true, effect: SHADOW}, [
      F(`${p}-field`, 'Prompt', {width: ASK_W, height: 88, padding: ['$--spacing-md', '$--spacing-lg'], layout: 'horizontal', gap: 1, alignItems: 'start'}, [
        F(`${p}-caret`, 'Caret', {width: 1.5, height: 18, fill: '$--primary'}, []),
        T(`${p}-q`, 'What should the agent do?', {size: 'subhead', fill: '$--muted-foreground'}),
      ]),
      F(`${p}-rule`, 'Rule', {width: ASK_W, height: 1, fill: '$--secondary'}, []),
      F(`${p}-bar`, 'Bar', {width: ASK_W, height: 40, layout: 'horizontal', gap: 6, alignItems: 'center', padding: [0, '$--spacing-md']}, [
        dd(`${p}-dt`, icon(`${p}-dtg`, 'folder-git-2', {size: 12, fill: '$--subtle-foreground'}), 'my-app · main', 124),
        dd(`${p}-dk`, art(`${p}-dkg`, kind), label, signedIn ? 118 : 92),
        ...(signedIn ? [] : [status(`${p}-ns`, 'warn', 'Not signed in', {size: 'caption'})]),
        dd(`${p}-dm`, icon(`${p}-dmg`, 'cpu', {size: 12, fill: '$--subtle-foreground'}), kind === 'claude' ? 'opus' : 'gpt-6-astra', signedIn ? 104 : 110),
        spacer(`${p}-sp`),
        kbd(`${p}-kbd`, '⏎'),
        btn(`${p}-go`, 'Start', {height: 26}),
      ]),
    ]);
  }

  function shellWithStart(p, panel) {
    return windowFrame(p, [
      at(deviceRail(`${p}-rail`), 0, 0),
      at(realSidebar(`${p}-side`, {projects: ONE}), RAIL, 0),
      at(homeMain(`${p}-main`, {projects: ONE}), RAIL + SIDEBAR, 0),
      at(panel, (W - num(tokens, '--size-search-sheet-w')) / 2, 16),
    ]);
  }

  // -- settings (B30) ------------------------------------------------------------------

  // Settings > Notifications as Screen / Settings draws it, with the desktop rows in each answer (B30).
  const settings = settingsParts(tokens, root, {themedXref, screenIconButton, screenSwatch});
  // Windows calls its machine This PC and Linux This computer, as the list and the desktop group do there.
  const MACHINE = {'pc-on': 'This PC', 'pc-off': 'This PC', 'no-service': 'This computer'};
  const settingsSheet = (p, state) => settings.settingsSheet(p, {page: 'notifications', notify: state, height: 480, machine: MACHINE[state] ?? null});

  // -- first-visit line (B43) ------------------------------------------------------------

  function firstVisitFactory(p) {
    const w = 900, h = 420;
    return windowFrame(p, [
      at(F(`${p}-main`, 'Factory', {width: w, height: h, layout: 'vertical', gap: '$--spacing-md', padding: '$--spacing-lg'}, [
        F(`${p}-hdr`, 'Header', {width: 'fill_container', layout: 'horizontal', alignItems: 'center', gap: '$--spacing-md'}, [
          T(`${p}-title`, 'Factory', {size: 'title', weight: '600'}), spacer(`${p}-sp`),
        ]),
        F(`${p}-line`, 'First visit line', {width: 'fill_container', layout: 'horizontal', gap: '$--spacing-sm', alignItems: 'center', padding: ['$--spacing-sm', '$--spacing-md'], cornerRadius: '$--radius-md', fill: '$--card'}, [
          owl(`${p}-li`, 'factory', '$--size-owl-intro'),
          T(`${p}-lt`, 'Factory takes the GitHub issues you label and carries each to a merged pull request. Create one for a project to start.', {fill: '$--subtle-foreground'}),
        ]),
        F(`${p}-empty`, 'Empty', {width: 'fill_container', height: 'fill_container', layout: 'vertical', alignItems: 'center', justifyContent: 'center'}, [btn(`${p}-create`, 'Create Factory', {icon: 'plus'})]),
      ]), 0, 0),
    ], {width: w, height: h});
  }

  // -- startup and failure (B1, B2) --------------------------------------------------------

  function nativeWindow(p, children) {
    const w = 900, h = 560;
    return windowFrame(p, [at(F(`${p}-c`, 'Center', {width: w, height: h, layout: 'vertical', gap: '$--spacing-md', alignItems: 'center', justifyContent: 'center'}, children), 0, 0)], {width: w, height: h});
  }

  function starting(p) {
    return nativeWindow(p, [
      F(`${p}-flight`, 'Local owl-roam / reduced-motion still', {width: num(tokens, '--size-brand-flight'), height: 450, layout: 'none'}, [
        at(F(`${p}-brand`, 'Brand', {width: 800, height: 450, layout: 'vertical', gap: '$--spacing-md', alignItems: 'center', justifyContent: 'center'}, [
          baseText(`${p}-word`, 'hide', {size: '$--size-brand-word', weight: '750', fill: '$--foreground', letterSpacing: '$--tracking-brand'}),
          T(`${p}-tagline`, 'Run more agents. Hold less.', {fill: '$--subtle-foreground'}),
          F(`${p}-loading`, 'Preparing / localized host state', {layout: 'vertical', gap: '$--spacing-sm', alignItems: 'center', padding: ['$--spacing-md', 0, 0, 0]}, [
      F(`${p}-track`, 'Indeterminate / static in reduced motion', {width: num(tokens, '--size-brand-track'), height: num(tokens, '--size-brand-track-h'), cornerRadius: '$--radius-sm', fill: '$--border', layout: 'horizontal', justifyContent: 'center'}, [F(`${p}-sweep`, 'Sweep', {width: 48, height: num(tokens, '--size-brand-track-h'), fill: '$--primary'}, [])]),
      T(`${p}-preparing`, 'Starting Hide…', {size: 'caption', fill: '$--muted-foreground'}),
          ]),
        ]), 0, 0),
        at(F(`${p}-owl`, 'owl-roam', {width: 800, height: 450, fill: {type: 'image', url: './assets/owls/owl-roam.png', mode: 'contain'}}, []), 0, 0),
      ]),
    ]);
  }

  // -- the sidebar and Home ---------------------------------------------------------
  // Drawn from web/src/sidebar.tsx and components/sidebar-header.tsx: the device
  // rail, the device line (name, Search), Overview and Factory with their chords,
  // then the list: Home, the Projects label with Add project as its +, the
  // project rows, the footer gear (D-27, B51, B52).

  const RAIL = num(tokens, '--size-rail');
  const STRIP = num(tokens, '--size-tab-strip');
  const ROW = num(tokens, '--size-project-row');
  const PROJECTS = ['my-app', 'api-server', 'docs'];
  const ONE = ['my-app'];
  const sideIcon = (id, glyph, name, opts = {}) => themedXref(id, 'Nyvom', name, opts, {ZIZFR: {icon: glyph, fill: '$--subtle-foreground'}});
  const hairline = {stroke: '$--border', strokeWidth: {bottom: 1}, strokeAlignment: 'inner'};

  function deviceRail(p) {
    return F(p, 'Device rail', {width: RAIL, height: H, fill: '$--sidebar', stroke: '$--border', strokeWidth: {right: 1}, strokeAlignment: 'inner', layout: 'vertical', gap: 12, padding: [8, 4]}, [
      sideIcon(`${p}-mac`, 'laptop', 'This Mac', {width: 40, height: 40, stroke: '$--foreground', strokeWidth: 2, cornerRadius: '$--radius-lg'}),
      sideIcon(`${p}-add`, 'plus', 'Add device', {width: 40, height: 32, stroke: '$--border', strokeWidth: 1, cornerRadius: '$--radius-lg'}),
    ]);
  }

  // A full-width command row like Overview: icon, words, the chord at the right.
  function commandRow(p, glyph, label, chord, {border = true} = {}) {
    return F(p, label, {width: 'fill_container', height: STRIP, layout: 'horizontal', gap: '$--spacing-sm', alignItems: 'center', padding: [0, '$--spacing-md'], ...(border ? hairline : {})}, [
      icon(`${p}-i`, glyph, {size: 14, fill: '$--subtle-foreground'}),
      T(`${p}-t`, label, {fill: '$--subtle-foreground', width: 'fill_container'}),
      ...(chord ? [kbd(`${p}-k`, chord)] : []),
    ]);
  }

  function realSidebar(p, {projects}) {
    const names = projects === true ? PROJECTS : projects || [];
    projects = names.length > 0;
    const n = names.length;
    const label = F(`${p}-pl`, 'Projects label', {width: 'fill_container', layout: 'horizontal', alignItems: 'center', padding: [4, '$--spacing-xs', 0, '$--spacing-sm']}, [
      T(`${p}-plt`, projects ? `Projects · Recent activity · ${n}` : 'Projects', {size: 'micro', weight: '500', fill: '$--muted-foreground', width: 'fill_container'}),
      sideIcon(`${p}-pla`, 'plus', 'Add project', {width: 20, height: 20}),
    ]);
    const projectRow = (name, i) => F(`${p}-p${i}`, name, {width: 'fill_container', height: ROW, layout: 'horizontal', gap: '$--spacing-sm', alignItems: 'center', padding: [0, '$--spacing-xs', 0, '$--spacing-sm'], cornerRadius: '$--radius-sm', ...(i === 0 ? {fill: '$--secondary'} : {})}, [
      icon(`${p}-p${i}-i`, 'folder-git-2', {size: 14, fill: '$--subtle-foreground'}),
      T(`${p}-p${i}-t`, name, {size: 'subhead', weight: '600', width: 'fill_container'}),
      T(`${p}-p${i}-b`, 'main', {size: 'caption', mono: true, fill: '$--muted-foreground'}),
    ]);
    const empty = F(`${p}-empty`, 'No project', {width: 'fill_container', layout: 'vertical', gap: '$--spacing-sm', padding: '$--spacing-sm'}, [
      T(`${p}-emptyt`, 'No project on this machine yet.', {size: 'caption', fill: '$--muted-foreground'}),
    ]);
    const list = F(`${p}-list`, 'Project list', {width: 'fill_container', height: 'fill_container', layout: 'vertical', padding: [0, '$--spacing-xs']}, [
      label,
      ...(projects ? names.map(projectRow) : [empty]),
    ]);
    return F(p, 'Projects sidebar', {width: SIDEBAR, height: H, layout: 'vertical', fill: '$--sidebar', stroke: '$--border', strokeWidth: {right: 1}, strokeAlignment: 'inner'}, [
      F(`${p}-strip`, 'Device line', {width: 'fill_container', height: STRIP, layout: 'horizontal', gap: '$--spacing-sm', alignItems: 'center', padding: [0, '$--spacing-xs', 0, '$--spacing-md'], ...hairline}, [
        T(`${p}-name`, 'This Mac', {size: 'subhead', weight: '600', width: 'fill_container'}),
        sideIcon(`${p}-search`, 'search', 'Search'),
      ]),
      commandRow(`${p}-ov`, 'layout-dashboard', 'Overview', K().overview),
      commandRow(`${p}-fac`, 'factory', 'Factory', K().factory),
      list,
      F(`${p}-foot`, 'Footer', {width: 'fill_container', layout: 'horizontal', alignItems: 'center', padding: ['$--spacing-xs', '$--spacing-md'], stroke: '$--border', strokeWidth: {top: 1}, strokeAlignment: 'inner'}, [
        spacer(`${p}-footsp`), sideIcon(`${p}-set`, 'settings', 'Settings'),
      ]),
    ]);
  }

  // The device Overview as Overview.tsx and MainScreen.tsx draw it (PRD
  // overview-device-tasks): the title line (Overview and the device), the
  // Tasks, Backlog, Projects and Usage tabs, then B27 or B28 in the middle.
  function homeMain(p, {projects, showOwl = true}) {
    projects = projects === true || (Array.isArray(projects) && projects.length > 0);
    return F(p, 'Overview', {width: W - RAIL - SIDEBAR, height: H, layout: 'vertical'}, [
      F(`${p}-hdr`, 'Title line', {width: 'fill_container', layout: 'horizontal', gap: '$--spacing-lg', alignItems: 'center', padding: ['$--spacing-sm', '$--spacing-lg'], height: 48, ...hairline}, [
        F(`${p}-h1`, 'Title', {width: 'fill_container', layout: 'horizontal', gap: '$--spacing-sm', alignItems: 'end'}, [
          T(`${p}-h1t`, 'Overview', {size: 'headline', weight: '600'}), T(`${p}-h1d`, 'This Mac', {fill: '$--muted-foreground'}),
        ]),
      ]),
      F(`${p}-tabsrow`, 'Views', {width: 'fill_container', padding: ['$--spacing-sm', '$--spacing-lg']}, [screenTabs(`${p}-tabs`, ['Tasks', 'Backlog', 'Projects', 'Usage'], 0)]),
      projects
        ? centered(`${p}-c`, 'No agents are running.', 'Start an agent', K().start)
        : centered(`${p}-c`, 'Add a project to start an agent.', 'Add project', K().add, showOwl),
    ]);
  }

  function sidebarWindow(p, {projects, tip = false}) {
    const kids = [
      at(deviceRail(`${p}-rail`), 0, 0),
      at(realSidebar(`${p}-side`, {projects}), RAIL, 0),
      at(homeMain(`${p}-main`, {projects}), RAIL + SIDEBAR, 0),
    ];
    // The + under the pointer: Projects label row sits under the device line,
    // Overview, Factory and Home (3 × STRIP + ROW).
    if (tip) kids.push(at(themedXref(`${p}-tip`, 'tip-shortcut-l', 'Add project', {}, {'tip-shortcut-l-t': {content: 'Add project'}, 'tip-shortcut-l-k': {content: K().add}}), RAIL + SIDEBAR - 4 - 10 - 66, 3 * STRIP + ROW + 26));
    return windowFrame(p, kids);
  }

  const cell = (id, label, node) => F(`cell-${id}`, label, {layout: 'vertical', gap: '$--spacing-sm'}, [T(`cell-${id}-cap`, label, {size: 'caption', weight: '600', fill: '$--muted-foreground'}), node]);
  const grid = (id, rows) => F(id, 'Grid', {layout: 'vertical', gap: '$--spacing-xl'}, rows.map((r, i) => F(`${id}-row${i}`, 'Row', {layout: 'horizontal', gap: '$--spacing-xl', alignItems: 'start'}, r)));

  // A board is built where it is declared, so the system and the copy it is drawn in hold for its build.
  function board(id, name, spec, build, {lightOnly = false} = {}) {
    return {id, name, spec, frames: {l: build('l'), d: lightOnly ? null : build('d')}};
  }

  const onSystem = (os, fn) => { const was = OS; OS = os; try { return fn(); } finally { OS = was; } };
  // Windows tiles: OpenCode and Cursor are installed but nothing of Hide's can be
  // written for them there (availability unsupported_system), omp not installed.
  const WINDOWS = [
    ['claude-code', 'ready', true], ['codex', 'nosign', true], ['grok', 'none', false], ['opencode', 'unsupported', false],
    ['pi', 'checking', true], ['omp', 'none', false], ['cursor', 'unsupported', false],
  ];

  const boards = [
    board('b01a-frame-A', '01 · The wizard: a dialog over the dimmed shell',
      'The wizard is a dialog over Home with the three-segment bar and "1 of 3" on top, today’s first-run dialog grown into three steps. The shell stays in sight behind the scrim, so the first sight of Hide is Hide.',
      t => [shellWithDialog(`a-${t}`, step1(`a-${t}-w`))]),
    board('b02-step1-agents', '02 · Step 1, agents',
      'Seven tiles in the kit’s order. States drawn: Claude Code Ready (on), Codex Not signed in (on), Grok Not installed with Install ↗, OpenCode Installed (on), Pi Checking… (on), omp Installed (off), Cursor Not installed. "What changes" open lists each location as KitAgentSnapshot reports it, home shown as ~: hooks and plugins by their file, skills by their SKILL.md (agents sharing ~/.agents/skills share one row), each agent\u2019s Herdr integration by the folder Herdr installs into ("<agent> · Herdr integration"), and ~/.local/bin/hide; turning omp on adds its lines. The Hide AI line sits left of Continue. Extra states: apply failed for one agent (B12: the reason line and Try again in Continue’s place), no agent installed (B9: one line with two install links, Continue unchanged), and the wizard reopened from Help > Set up Hide (B46: a close button).',
      t => [
        shellWithDialog(`s1-${t}`, step1(`s1-${t}-w`, {open: true})),
        grid(`s1x-${t}`, [[
          cell(`s1f-${t}`, 'Apply failed for one agent (B12)', step1(`s1f-${t}-w`, {failed: true})),
          cell(`s1n-${t}`, 'No agent installed (B9)', step1(`s1n-${t}-w`, {agents: NOTHING})),
        ], [
          cell(`s1h-${t}`, 'Reopened from Help > Set up Hide (B45, B46): close button, current choice kept', step1(`s1h-${t}-w`, {close: true})),
        ]]),
      ]),
    board('b03-step2-notifications', '03 · Step 2, notifications',
      'macOS: initial (Turn on notifications, Not now), waiting for the macOS answer (the system prompt is the OS’s own and is not drawn), on (Send a test, Continue), blocked by macOS (Open System Settings; Continue stays, not counted done), the setting unreadable (B17). Windows and Linux have no permission state: Turn on notifications turns the setting on and sends one test, then the on row without a permission word (B19). "Also on your phone →" opens Settings > Mobile over the wizard (B21).',
      t => [grid(`s2-${t}`, [
        [cell(`s2i-${t}`, 'macOS · initial', step2(`s2i-${t}-w`, 'initial')), cell(`s2w-${t}`, 'macOS · waiting for the answer (B14)', step2(`s2w-${t}-w`, 'waiting'))],
        [cell(`s2o-${t}`, 'macOS · on (B15)', step2(`s2o-${t}-w`, 'on')), cell(`s2b-${t}`, 'macOS · blocked (B15)', step2(`s2b-${t}-w`, 'blocked'))],
        [cell(`s2u-${t}`, 'macOS · setting unreadable (B17)', step2(`s2u-${t}-w`, 'unreadable')), cell(`s2x-${t}`, 'Windows · after Turn on notifications (B19)', step2(`s2x-${t}-w`, 'on', {windows: true}))],
      ])]),
    board('b04-step3-project', '04 · Step 3, first project',
      'Today’s Add a project inside the wizard: Host (This Mac), Browse folder as the card with ↵, Other ways to add (Clone from URL, Create new project), and Skip for now. Clone and Create open their own views inside the wizard as they do in the dialog today; refusals and clone progress are unchanged (B22).',
      t => [shellWithDialog(`s3-${t}`, step3(`s3-${t}-w`))]),
    board('b05-start-panel', '05 · Start panel after step 3',
      'The wizard closes and the ⌘N panel opens on the new project: target my-app · main, the agent picked by D-15, keyboard in the prompt (the caret). Left: the picked agent is signed in, so nothing beside its chip. Right: no enabled agent is signed in, so the first enabled one (Codex) is picked and its chip carries Not signed in (B23).',
      t => [grid(`sp-${t}`, [[
        cell(`spa-${t}`, 'Signed-in agent picked', shellWithStart(`spa-${t}`, startPanel(`spa-${t}`, {}))),
      ], [
        cell(`spb-${t}`, 'Picked agent not signed in (B23)', shellWithStart(`spb-${t}`, startPanel(`spb-${t}`, {kind: 'codex', label: 'Codex', signedIn: false}))),
      ]])]),
    board('b06-empty-home', '06 · Empty Home (desktop)',
      'One line and one button, centered on Home. No project: "Add a project to start an agent." with Add project and ⇧⌘N (B27); this is also where the wizard closes to when every agent is left off. Projects but no agent: "No agents are running." with Start an agent and ⌘N, which opens the start panel (B28). Besides B27\u2019s button, the only Add project entry is the + on the sidebar\u2019s Projects label (board 17); the device line +, the Home title\u2019s ghost button and the empty-list button are gone.',
      t => [grid(`eh-${t}`, [[
        cell(`eh0-${t}`, 'No project', sidebarWindow(`eh0-${t}`, {projects: false})),
      ], [
        cell(`eh1-${t}`, 'Projects, no agent', sidebarWindow(`eh1-${t}`, {projects: ONE})),
      ]])]),
    board('b07-settings-notifications', '07 · Settings > Notifications',
      'The This Mac group of the Notifications page (PRD settings-left-nav B6), above the Phone push. On: the switch and one permission row, Allowed with Send a test. Off: only the switch; the permission row appears once it is on, since nothing on it is actionable while off. Blocked: Blocked by macOS with Open System Settings and no test (it could not arrive). The note says why none arrives while Hide is in front (B36).',
      t => [grid(`st-${t}`, [[cell(`sto-${t}`, 'On', settingsSheet(`sto-${t}`, 'on')), cell(`stf-${t}`, 'Off', settingsSheet(`stf-${t}`, 'off'))], [cell(`stb-${t}`, 'Blocked by macOS', settingsSheet(`stb-${t}`, 'blocked'))]])]),
    board('b08-first-visit-line', '08 · The one-time line on Factory (B43)',
      'Shown the first time Factory opens, above its own content, and never again; the existing empty state stays. One line says what Factory does and its first action. Settings > Devices gets no line: its description already says what the tab is for.',
      t => [grid(`fv-${t}`, [[cell(`fvf-${t}`, 'Factory, first visit', firstVisitFactory(`fvf-${t}`))]])]),
    board('b09-startup', '09 · Startup and connection failure (B1, B2)',
      'Starting: one line, no "hided". Failure: the reason line, Retry and Copy diagnostics. When the clipboard refuses, the same text appears as selectable text under the buttons: app build, reason code, log location.',
      t => [grid(`su-${t}`, [[
        cell(`sus-${t}`, 'Starting · packaged default is Hide dark', starting(`sus-${t}`)),
        cell(`suf-${t}`, 'Failed', nativeWindow(`suf-${t}`, [
          T(`suf-${t}-t`, 'Can’t start Hide', {size: 'title', weight: '600'}),
          T(`suf-${t}-r`, 'The hide command was not found.', {fill: '$--subtle-foreground'}),
          F(`suf-${t}-a`, 'Actions', {layout: 'horizontal', gap: '$--spacing-sm'}, [btn(`suf-${t}-retry`, 'Retry'), btn(`suf-${t}-copy`, 'Copy diagnostics', {variant: 'secondary'})]),
        ])),
      ], [
        cell(`suc-${t}`, 'Failed, clipboard refused', nativeWindow(`suc-${t}`, [
          T(`suc-${t}-t`, 'Can’t start Hide', {size: 'title', weight: '600'}),
          T(`suc-${t}-r`, 'The hide command was not found.', {fill: '$--subtle-foreground'}),
          F(`suc-${t}-a`, 'Actions', {layout: 'horizontal', gap: '$--spacing-sm'}, [btn(`suc-${t}-retry`, 'Retry'), btn(`suc-${t}-copy`, 'Copy diagnostics', {variant: 'secondary'})]),
          F(`suc-${t}-d`, 'Diagnostics', {width: 460, layout: 'vertical', gap: 2, padding: ['$--spacing-sm', '$--spacing-md'], cornerRadius: '$--radius-md', fill: '$--card', stroke: '$--border', strokeWidth: 1, strokeAlignment: 'inner'}, [
            T(`suc-${t}-d1`, 'hide 0.3.0 (desktop, macos-arm64)', {size: 'caption', mono: true}),
            T(`suc-${t}-d2`, 'reason: cli_missing', {size: 'caption', mono: true}),
            T(`suc-${t}-d3`, 'log: ~/Library/Application Support/hide-desktop/logs/desktop.log', {size: 'caption', mono: true}),
          ]),
        ])),
      ]])]),


    onSystem('linux', () => board('b11-linux-step2-on', '11 · Linux, step 2 after Turn on notifications (B19)',
      'Linux has no permission state: Turn on notifications turns the setting on and sends one test, then the on row. The description names the desktop, not macOS or Windows. Initial is the macOS initial with this description.',
      t => [grid(`lx-${t}`, [[
        cell(`lxi-${t}`, 'Linux · initial', step2(`lxi-${t}-w`, 'initial', {linux: true})),
        cell(`lxo-${t}`, 'Linux · after Turn on notifications', step2(`lxo-${t}-w`, 'on', {linux: true})),
      ]])])),
    onSystem('linux', () => board('b12-linux-no-notification-service', '12 · Linux with no notification service',
      'When Electron says the system cannot show notifications (Notification.isSupported() false, no notification daemon), step 2 shows one quiet line and Continue, with no Turn on button, because pressing it could do nothing. "Also on your phone →" stays, since the phone is the one way left to be told. Settings shows the switch disabled with the same line (board 13).',
      t => [grid(`ns-${t}`, [[
        cell(`nsw-${t}`, 'Linux · no notification service', step2(`nsw-${t}-w`, 'noService', {linux: true})),
      ]])])),
    onSystem('win', () => board('b13-settings-notifications-pc', '13 · Settings > Notifications on Windows and Linux',
      'No permission row: the system gives no permission state. On: the switch and a Test notification row with Send a test. Off: the switch only. Third (with board 12): Linux with no notification service, the switch disabled and one quiet line under it.',
      t => [grid(`sp2-${t}`, [[
        cell(`spo-${t}`, 'Windows · on (Linux reads This computer)', settingsSheet(`spo-${t}`, 'pc-on')),
        cell(`spf-${t}`, 'Windows · off', settingsSheet(`spf-${t}`, 'pc-off')),
      ], [
        cell(`spn-${t}`, 'Linux · no notification service', settingsSheet(`spn-${t}`, 'no-service')),
      ]])])),
    onSystem('win', () => board('b14-empty-home-pc', '14 · Empty Home on Windows and Linux',
      'The shortcut labels the shortcut table prints on Windows and Linux (web/src/shortcuts.ts, docs/UI_BEHAVIOR.md): Add project Alt+Shift+N, Start agent Ctrl+Shift+N, Overview Alt+Shift+O, Open Factory Alt+Shift+F.',
      t => [grid(`ehp-${t}`, [[
        cell(`ehp0-${t}`, 'No project', sidebarWindow(`ehp0-${t}`, {projects: false})),
      ], [
        cell(`ehp1-${t}`, 'Projects, no agent', sidebarWindow(`ehp1-${t}`, {projects: ONE})),
      ]])])),
    onSystem('win', () => board('b15-step1-windows', '15 · Step 1 on Windows, "What changes" open',
      'Each location as KitAgentSnapshot reports it on Windows, home shown as ~ with backslashes: each skill as its SKILL.md, the command as hide.cmd, and each agent\u2019s Herdr integration by its folder. On Windows only Claude Code (hook and skill), Codex (hook) and Pi (skill) get Hide\u2019s own pieces; OpenCode and Cursor are installed but Hide writes nothing for them there (availability unsupported_system), so their tiles read "Not on Windows yet" and cannot be switched on. Continue reads Ctrl+Enter.',
      t => [shellWithDialog(`w1-${t}`, step1(`w1-${t}-w`, {agents: WINDOWS, open: true}))])),

    board('b17-sidebar-add-project', '17 · Sidebar Add project: + on the Projects label',
      'Add project is a + at the right end of the Projects label, drawn at rest, not on hover; the pointer shows "Add project ⇧⌘N". The label shows even with no project, so the + is there on a first run. It is the only sidebar entry: the device line +, the Home title\u2019s ghost button and the empty-list button are removed. Empty Home keeps B27\u2019s center button. The rail\u2019s + is Add device and stays.',
      t => [grid(`ca-${t}`, [[
        cell(`cap-${t}`, 'With projects · pointer on +', sidebarWindow(`cap-${t}`, {projects: true, tip: true})),
        cell(`can-${t}`, 'No project', sidebarWindow(`can-${t}`, {projects: false})),
      ]])]),
  ];

  // The longest copy: Korean at the largest interface text (17 pt over 13 pt), Light only (B41).
  L = KO; SCALE = 17 / 13;
  try {
  boards.push(board('b10-longest-copy-ko', '10 · Longest copy: Korean at the largest interface text (17/13), Light',
    'Step 1 with "What changes" open and step 2 in its longest states, at the dialog’s own width. Text sizes are multiplied by 17/13; widths stay as they are, as the interface scale leaves them.',
    t => [grid(`ko-${t}`, [[
      cell(`ko1-${t}`, '1단계', step1(`ko1-${t}-w`, {open: true})),
    ], [
      cell(`ko2-${t}`, '2단계 · 처음', step2(`ko2-${t}-w`, 'initial')),
      cell(`ko3-${t}`, '2단계 · 막힘', step2(`ko3-${t}-w`, 'blocked')),
    ], [
      cell(`ko4-${t}`, '2단계 · 읽지 못함', step2(`ko4-${t}-w`, 'unreadable')),
      cell(`ko5-${t}`, '2단계 · 기다리는 중', step2(`ko5-${t}-w`, 'waiting')),
    ]])], {lightOnly: true}));
  } finally {
    L = EN; SCALE = 1;
  }
  return boards;
}
