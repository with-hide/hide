// The right lane's boards in `Screen / Workspace` (PRD right-lane B25, D-17):
// the approved v3 states, closed, over the agents, pinned, the tree on over
// the agents, the tree alone, expanded, PRs pinned (PRD sidebar-task-lens
// D-17) and a narrow window,
// each a whole window with its measure line, in one theme. The rail, the lane
// header, the pin, the tree button, the document header and the dividers are
// refs of `Component / Workspace columns` in design/hide-ui.lib.pen; the
// terminals, document, tree and pull request rows are drawn here. Called from
// pen-screens.mjs, which owns the ref helpers and the window template; names,
// paths and agents are invented.

import {frame, icon, num, text} from './pen-system.mjs';

/**
 * Every board for one theme (`suffix` `l` or `d`), ids `rl-<state>-<suffix>`
 * with the window as `rl-<state>-<suffix>-win`, so a review exports the same
 * nodes the approved bundle holds.
 */
export function laneBoards(tokens, {themedXref, template, alias}, suffix) {
  const n = name => num(tokens, name);
  const mode = suffix === 'l' ? 'Light' : 'Dark';
  const TOP = n('--size-tab-strip');
  const HEIGHT = template.height;
  const BODY_H = HEIGHT - TOP;
  const SIDE = template.children.find(child => child.name === 'Projects sidebar').x + template.children.find(child => child.name === 'Projects sidebar').width;
  const RAIL_W = n('--size-tool-rail');
  const LANE_W = n('--size-lane-ideal');
  const TREE_W = n('--size-tree-ideal');
  const GRIP_W = 8;

  const cap = (id, value, fill = '$--muted-foreground', extra = {}) => text(id, value, {size: '$--text-caption', fill, ...extra});
  const mic = (id, value, fill = '$--muted-foreground', extra = {}) => text(id, value, {size: '$--text-micro', fill, ...extra});
  const body = (id, value, fill = '$--foreground', extra = {}) => text(id, value, {size: '$--text-body', fill, ...extra});
  const sub = (id, value, fill = '$--foreground', extra = {}) => text(id, value, {size: '$--text-subhead', fill, ...extra});
  const spacer = id => frame(id, 'Spacer', {width: 'fill_container', height: 1}, []);
  const dot = (id, fill, size = 6) => frame(id, 'Dot', {width: size, height: size, cornerRadius: size / 2, fill}, []);
  const iconButton = (id, glyph, name, size = 20) => themedXref(id, 'Nyvom', name, {width: size, height: size}, {ZIZFR: {icon: glyph, width: 12, height: 12}});
  const tip = (id, label, at) => themedXref(id, 'tip-open-l', label, at, {'tip-open-l-t': {content: label}});

  // Pen has no ellipsis and wraps Korean by character: break at spaces on an estimated advance.
  const advance = (ch, size) => /[ᄀ-ᇿ　-鿿가-힯]/.test(ch) ? size * 0.98 : /[A-Z]/.test(ch) ? size * 0.66 : /[ .,:·'()]/.test(ch) ? size * 0.32 : size * 0.56;
  const measure = (value, size) => [...value].reduce((width, ch) => width + advance(ch, size), 0);
  function wrap(value, px, size = 12) {
    const lines = [];
    let line = '';
    for (const word of value.split(' ')) {
      const next = line ? `${line} ${word}` : word;
      if (line && measure(next, size) > px * 0.94) { lines.push(line); line = word; } else line = next;
    }
    if (line) lines.push(line);
    return lines.join('\n');
  }

  // -- Agent Views: one tab of four live terminals --
  const PANES = [
    {title: '설정 화면 정리', status: '작업 중', dot: '$--agent-working', lines: [['cols', '❯ 설정 화면 정리'], ['', '관련 자료를 보며 작업을 이어갑니다.'], ['link', 'docs/설정검토.md', ' 를 열었습니다.'], ['cols', 'settings-ui · 터미널 폭은 View와 무관']]},
    {title: '파일 보기 개선', status: '', dot: '$--warning', lines: [['cols', '❯ 파일 보기 개선'], ['link', 'web/src/components/SettingsPanel.tsx'], ['', '변경을 확인해 주세요.'], ['cols', '확인 필요']]},
    {title: '문서 검토', status: '', dot: '$--success', lines: [['cols', '❯ 문서 검토'], ['link', 'docs/작업계획.md'], ['ok', '✓ 검토 완료']]},
    {title: '동기화 검증', status: '', dot: '$--agent-working', lines: [['cols', '❯ 동기화 검증'], ['link', 'web/src/styles/lane.css'], ['', 'memory-sync 와 비교 중.']]},
  ];
  function terminalLine(id, width, [kind, a, b]) {
    const fill = {cols: '$--muted-foreground', ok: '$--success', '': '$--subtle-foreground', link: '$--foreground'}[kind];
    if (kind !== 'link') return text(id, a, {size: '$--text-caption', mono: true, fill, width});
    return frame(id, 'Link line', {width, layout: 'horizontal', alignItems: 'center'}, [
      {...text(`${id}-a`, a, {size: '$--text-caption', mono: true, fill}), underline: true},
      ...(b ? [text(`${id}-b`, b, {size: '$--text-caption', mono: true, fill: '$--subtle-foreground'})] : []),
    ]);
  }
  function pane(id, spec, width, height) {
    return frame(id, `Agent pane · ${spec.title}`, {width, height, layout: 'vertical', fill: '$--background', stroke: '$--border', strokeWidth: {right: 1, bottom: 1}, strokeAlignment: 'inner', clip: true}, [
      frame(`${id}-h`, 'Agent pane header', {width: 'fill_container', height: 28, layout: 'horizontal', gap: 6, alignItems: 'center', padding: [0, 4, 0, 10], stroke: '$--border', strokeWidth: {bottom: 1}, strokeAlignment: 'inner'}, [
        dot(`${id}-h-d`, spec.dot), body(`${id}-h-t`, spec.title, '$--foreground', {width: 'fill_container', weight: '500'}),
        ...(spec.status ? [cap(`${id}-h-s`, spec.status)] : []),
        iconButton(`${id}-h-m`, 'ellipsis', 'Pane menu'), iconButton(`${id}-h-x`, 'x', 'Close pane'),
      ]),
      frame(`${id}-t`, 'Live terminal', {width: 'fill_container', height: 'fill_container', layout: 'vertical', gap: 2, padding: [10, 12]}, [
        ...spec.lines.map((line, index) => terminalLine(`${id}-t${index}`, width - 24, line)),
        frame(`${id}-gap`, 'Gap', {width: 1, height: 10}, []),
        frame(`${id}-pr`, 'Prompt', {layout: 'horizontal', gap: 6, alignItems: 'center'}, [cap(`${id}-pr-c`, '❯', '$--muted-foreground', {mono: true}), frame(`${id}-pr-k`, 'Cursor', {width: 7, height: 13, fill: '$--foreground'}, [])]),
      ]),
    ]);
  }
  function agentViews(id, width) {
    const column = Math.floor(width / 2);
    const row = (BODY_H - TOP) / 2;
    const tab = (key, label, on) => frame(`${id}-tab-${key}`, label, {height: TOP, layout: 'horizontal', alignItems: 'center', padding: [0, 12], ...(on ? {fill: '$--card', stroke: '$--primary', strokeWidth: {bottom: 2}, strokeAlignment: 'inner'} : {})}, [body(`${id}-tab-${key}-t`, label, on ? '$--foreground' : '$--muted-foreground', {weight: on ? '600' : '400'})]);
    return frame(id, 'Agent Views', {x: 0, y: 0, width, height: BODY_H, layout: 'vertical', fill: '$--background', clip: true}, [
      frame(`${id}-tabs`, 'Agent tab strip', {width: 'fill_container', height: TOP, layout: 'horizontal', alignItems: 'center', stroke: '$--border', strokeWidth: {bottom: 1}, strokeAlignment: 'inner'}, [tab('a', '작업 중 · 4', true), tab('b', '검증', false)]),
      frame(`${id}-r1`, 'Pane row 1', {width: 'fill_container', height: row, layout: 'horizontal'}, [pane(`${id}-p1`, PANES[0], column, row), pane(`${id}-p2`, PANES[1], width - column, row)]),
      frame(`${id}-r2`, 'Pane row 2', {width: 'fill_container', height: row, layout: 'horizontal'}, [pane(`${id}-p3`, PANES[2], column, row), pane(`${id}-p4`, PANES[3], width - column, row)]),
    ]);
  }

  // -- the rail: icons alone, each tool's name in its hint --
  const TOOLS = ['memory', 'prs', 'files', 'changes'];
  // The PRs tool's Needs You rows, as the rail's warning badge draws it (PRD sidebar-task-lens B29).
  const PRS_COUNT = '2';
  function rail(id, active) {
    const descendants = {
      'lane-rail-prs/lane-tool-badge': {enabled: true, fill: '$--warning'},
      'lane-rail-prs/lane-tool-badge-t': {content: PRS_COUNT},
    };
    for (const key of TOOLS) {
      if (key !== active) continue;
      descendants[`lane-rail-${key}`] = {fill: '$--secondary'};
      descendants[`lane-rail-${key}/lane-tool-bar`] = {enabled: true};
      descendants[`lane-rail-${key}/lane-tool-glyph`] = {fill: '$--foreground'};
    }
    return themedXref(id, 'lane-rail', 'Tool rail', {height: BODY_H}, descendants);
  }

  // -- the lane --
  function header(id, tool, scope, {pinned, expanded}) {
    return themedXref(id, 'lane-header', 'Lane header', {width: 'fill_container'}, {
      'lane-header-tool': {content: tool},
      'lane-header-explorer': {enabled: tool === 'Files'},
      'lane-header-scope': {content: scope},
      // The scope's icon: a folder for the Project's tools, the default branch glyph for the checkout's (PRD work-tab-pr-lens D-37).
      ...(tool === 'PRs' ? {'lane-header-scope-glyph': {icon: 'folder'}} : {}),
      ...(pinned ? {'lane-header-pin': {fill: '$--secondary'}, 'lane-header-pin/lane-pin-glyph': {fill: '$--primary', stroke: '$--primary'}} : {}),
      ...(expanded ? {'lane-header-expand': {fill: '$--secondary'}, 'lane-header-expand/ZIZFR': {icon: 'minimize-2', fill: '$--foreground'}} : {}),
    });
  }
  function viewTab(id, title, {on}) {
    return themedXref(id, 'view-tab', title, {width: 140, ...(on ? {fill: '$--card', stroke: '$--primary', strokeWidth: {bottom: 2}, strokeAlignment: 'inner'} : {fill: []})}, {
      'view-tab-mark': {icon: 'file-text', fill: '$--file-document'},
      'view-tab-title': {content: title, fill: on ? '$--foreground' : '$--muted-foreground'},
      'view-tab-dirty': {enabled: false},
      'view-tab-close': {enabled: true},
    });
  }
  const viewTabs = (id, tabs) => frame(id, 'View tab strip', {width: 'fill_container', height: TOP, layout: 'horizontal', alignItems: 'center', stroke: '$--border', strokeWidth: {bottom: 1}, strokeAlignment: 'inner'}, tabs);
  const documentHeader = (id, treeOn) => themedXref(id, 'GGXfK', 'Document header', {width: 'fill_container'}, {
    BUboy: {content: 'docs › 설정검토.md'},
    'GGXfK-tree': {enabled: false},
  });
  const DOC = {
    title: '설정 화면 검토', path: 'docs/설정검토.md · settings-ui',
    sections: [
      ['1. 공간', 'View를 열어도 왼쪽 터미널의 폭과 pane 배치는 그대로다. 핀을 켜면 그때 한 번만 자리를 나눈다.'],
      ['2. 찾기', 'Files 최상단의 탐색기 버튼으로 트리를 켜고 찾는다. 트리는 끌 때까지 남고, View가 그만큼 넓어져 문서는 작아지지 않는다.'],
      ['3. 읽기', '길게 읽을 땐 확대한다. 돌아오면 이전 배치가 그대로 있다.'],
    ],
    code: ['- 핀 끄면 겹침, 켜면 분할', '- 트리는 켜고 끄기, 끌 때까지 남음', '- 문서가 없으면 트리만'],
  };
  function document(id, width) {
    const inner = width - 48;
    return frame(id, 'Document', {width: 'fill_container', height: 'fill_container', layout: 'vertical', gap: 14, padding: [20, 24], clip: true}, [
      frame(`${id}-top`, 'Title block', {width: 'fill_container', layout: 'vertical', gap: 4}, [
        text(`${id}-title`, DOC.title, {size: '$--text-editor-heading-3', weight: '700'}), cap(`${id}-path`, DOC.path),
      ]),
      ...DOC.sections.map(([head, copy], index) => frame(`${id}-s${index}`, head, {width: 'fill_container', layout: 'vertical', gap: 4}, [
        sub(`${id}-s${index}-h`, head, '$--foreground', {weight: '600'}), {...body(`${id}-s${index}-b`, wrap(copy, inner), '$--subtle-foreground'), lineHeight: 1.5},
      ])),
      frame(`${id}-code`, 'Code block', {width: 'fill_container', layout: 'vertical', gap: 3, padding: [10, 12], fill: '$--secondary', cornerRadius: '$--radius-md'}, DOC.code.map((line, index) => cap(`${id}-code${index}`, line, '$--subtle-foreground', {mono: true}))),
    ]);
  }
  // The Explorer's rows as the shell draws them: a chevron column per folder,
  // an indent per level with a hairline under each ancestor's chevron, then the
  // mark. The shell's marks are Material Icon Theme SVGs, which Pen fills only
  // from a raster, so lucide glyphs in the theme's hues stand in for them here.
  const TREE = [
    {depth: 0, folder: 'open', name: 'settings-ui'}, {depth: 1, folder: 'open', name: 'docs', mark: 'folder-open', fill: '$--file-blue'}, {depth: 2, mark: 'file-text', fill: '$--file-blue', name: '설정검토.md'}, {depth: 2, mark: 'file-text', fill: '$--file-blue', name: '작업계획.md'},
    {depth: 1, folder: 'open', name: 'web', mark: 'folder-open', fill: '$--file-blue'}, {depth: 2, folder: 'open', name: 'src', mark: 'folder-open', fill: '$--success'},
    {depth: 3, folder: 'closed', name: 'components', mark: 'folder', fill: '$--file-yellow'}, {depth: 3, mark: 'atom', fill: '$--file-blue', name: 'SettingsPanel.tsx'}, {depth: 3, mark: 'file-code', fill: '$--file-purple', name: 'lane.css'},
  ];
  const INDENT = n('--size-tree-indent');
  const MARK = n('--size-tree-mark');
  const INSET = n('--spacing-sm');
  function tree(id, width, {only}) {
    const selected = only ? null : '설정검토.md';
    const guide = (rowId, level) => frame(`${rowId}-i${level}`, 'Indent', {width: INDENT, height: n('--size-pane-child-row'), layout: 'none'}, [
      frame(`${rowId}-i${level}-l`, 'Guide', {x: MARK / 2, y: 0, width: 1, height: n('--size-pane-child-row'), fill: '$--border'}, []),
    ]);
    const row = (entry, index) => {
      const rowId = `${id}-r${index}`;
      return frame(rowId, entry.name, {width: 'fill_container', height: n('--size-pane-child-row'), layout: 'horizontal', gap: n('--spacing-xs'), alignItems: 'center', padding: [0, 8, 0, INSET], ...(entry.name === selected ? {fill: '$--secondary'} : {})}, [
        frame(`${rowId}-in`, 'Indent guides', {height: n('--size-pane-child-row'), layout: 'horizontal'}, Array.from({length: entry.depth}, (_, level) => guide(rowId, level))),
        frame(`${rowId}-c`, 'Chevron', {width: MARK, height: MARK, layout: 'horizontal', justifyContent: 'center', alignItems: 'center'}, entry.folder ? [icon(`${rowId}-g`, entry.folder === 'open' ? 'chevron-down' : 'chevron-right', {size: MARK, fill: '$--muted-foreground'})] : []),
        icon(`${rowId}-k`, entry.mark ?? 'folder-open', {size: MARK, fill: entry.fill ?? '$--file-neutral'}),
        sub(`${rowId}-t`, entry.name, '$--foreground'),
      ]);
    };
    return frame(id, only ? 'Tree (only)' : 'Tree column', {width, height: 'fill_container', layout: 'vertical', fill: '$--sidebar', ...(only ? {} : {stroke: '$--border', strokeWidth: {right: 1}, strokeAlignment: 'inner'})}, [
      frame(`${id}-find`, 'Find row', {width: 'fill_container', layout: 'vertical', padding: [8, 8, 4, 8]}, [
        frame(`${id}-find-f`, 'Find field', {width: 'fill_container', height: 28, layout: 'horizontal', gap: 8, alignItems: 'center', padding: [0, 8], fill: '$--background', stroke: '$--border', strokeWidth: 1, strokeAlignment: 'inner', cornerRadius: '$--radius-sm'}, [
          icon(`${id}-find-g`, 'search', {size: 12, fill: '$--muted-foreground'}), body(`${id}-find-t`, '파일 이름 찾기', '$--muted-foreground', {width: 'fill_container'}),
        ]),
      ]),
      frame(`${id}-hintrow`, 'Hint', {width: 'fill_container', padding: [0, 10, 6, 10]}, [mic(`${id}-hint`, only ? '열린 문서 없음 · 트리만' : '한 번 클릭 미리보기 · 두 번 클릭 고정')]),
      frame(`${id}-rows`, 'Rows', {width: 'fill_container', height: 'fill_container', layout: 'vertical'}, TREE.map(row)),
      frame(`${id}-foot`, 'Foot', {width: 'fill_container', height: 28, layout: 'horizontal', alignItems: 'center', padding: [0, 10], stroke: '$--border', strokeWidth: {top: 1}, strokeAlignment: 'inner'}, [mic(`${id}-foot-t`, only ? '파일을 고르면 옆에 열림' : '끌 때까지 남음 · ⌘⇧E')]),
    ]);
  }
  // The PRs tool's groups as the core decides them (status-model.md, Pull request groups).
  const PULLS = [
    ['Needs You', '$--warning', [['git-pull-request', '$--success', '#1217', '사이드바 태스크 목록', '리뷰 요청 · 4분'], ['git-pull-request', '$--destructive', '#1209', '설정 화면 정리', '검사 실패 · 12분']]],
    ['Agent is working', '$--agent-working', [['git-pull-request-draft', '$--muted-foreground', '#1214', '동기화 검증', '작업 중 · 31분']]],
    ['Waiting', '$--muted-foreground', [['git-pull-request', '$--success', '#1201', '문서 검토', '리뷰 대기 · 1시간']]],
  ];
  function pulls(id) {
    const chip = (key, label, on) => frame(`${id}-chip-${key}`, label, {height: 20, layout: 'horizontal', alignItems: 'center', padding: [0, 8], cornerRadius: 999, stroke: '$--border', strokeWidth: 1, strokeAlignment: 'inner', ...(on ? {fill: '$--secondary'} : {})}, [cap(`${id}-chip-${key}-t`, label, on ? '$--foreground' : '$--muted-foreground', {weight: on ? '600' : '400'})]);
    return frame(id, 'PRs tool', {width: 'fill_container', height: 'fill_container', layout: 'vertical', padding: [8, 6]}, [
      frame(`${id}-head`, 'Scope and count', {width: 'fill_container', layout: 'horizontal', gap: 6, alignItems: 'center', padding: [0, 6, 6, 6]}, [chip('all', 'All checkouts', true), chip('main', 'main only', false), spacer(`${id}-head-sp`), mic(`${id}-head-n`, '열린 PR 4')]),
      ...PULLS.flatMap(([group, fill, rows], groupIndex) => [
        frame(`${id}-g${groupIndex}`, group, {width: 'fill_container', height: 28, layout: 'horizontal', gap: 6, alignItems: 'center', padding: [0, 6]}, [cap(`${id}-g${groupIndex}-t`, group, fill, {weight: '600'}), cap(`${id}-g${groupIndex}-n`, String(rows.length))]),
        ...rows.map(([glyph, glyphFill, number, title, meta], rowIndex) => frame(`${id}-g${groupIndex}r${rowIndex}`, title, {width: 'fill_container', height: 34, layout: 'horizontal', gap: 8, alignItems: 'center', padding: [0, 8], cornerRadius: '$--radius-xs'}, [
          icon(`${id}-g${groupIndex}r${rowIndex}-g`, glyph, {size: 12, fill: glyphFill}), cap(`${id}-g${groupIndex}r${rowIndex}-n`, number, '$--muted-foreground', {mono: true}),
          body(`${id}-g${groupIndex}r${rowIndex}-t`, title, '$--foreground', {width: 'fill_container', weight: '500'}), cap(`${id}-g${groupIndex}r${rowIndex}-m`, meta),
        ])),
      ]),
    ]);
  }
  // A lane over the agents is set apart by a shadow and its left border (B3).
  const SHADOW = {type: 'shadow', shadowType: 'outer', offset: {x: -16, y: 0}, blur: 40, spread: 0, color: suffix === 'l' ? '#00000052' : '#000000f0'};
  function lane(id, {tool, tree: treeMode = 'off', pinned, expanded = false, overlay, width}) {
    const head = tool === 'prs' ? header(`${id}-head`, 'PRs', 'hide 전체', {pinned, expanded}) : header(`${id}-head`, '파일', 'settings-ui', {pinned, expanded});
    let content;
    if (tool === 'prs') content = pulls(`${id}-prs`);
    else if (treeMode === 'only') content = tree(`${id}-tree`, 'fill_container', {only: true});
    else {
      const documentWidth = treeMode === 'on' ? width - TREE_W : width;
      const column = frame(`${id}-doccol`, 'Document column', {width: treeMode === 'on' ? documentWidth : 'fill_container', height: 'fill_container', layout: 'vertical'}, [
        viewTabs(`${id}-tabs`, [viewTab(`${id}-tab1`, '설정검토.md', {on: true}), viewTab(`${id}-tab2`, '작업계획.md', {on: false})]),
        documentHeader(`${id}-crumb`, treeMode === 'on'),
        document(`${id}-doc`, documentWidth),
      ]);
      content = treeMode === 'on' ? frame(`${id}-files`, 'Files', {width: 'fill_container', height: 'fill_container', layout: 'horizontal'}, [tree(`${id}-tree`, TREE_W, {only: false}), column]) : column;
    }
    return frame(id, `Right lane · ${tool}${overlay ? ' · over the agents' : ' · docked'}`, {width, height: BODY_H, layout: 'vertical', fill: '$--card', stroke: '$--border', strokeWidth: {left: 1}, strokeAlignment: 'inner', clip: true, ...(overlay ? {effect: SHADOW} : {})}, [
      head, frame(`${id}-content`, 'Lane content', {width: 'fill_container', height: 'fill_container', layout: 'vertical'}, [content]),
    ]);
  }

  // -- the window: chrome, device rail, sidebar and top row from the template --
  function reid(node, prefix) {
    const copy = structuredClone(node);
    const walk = child => { child.id = `${prefix}-${child.id}`; (child.children ?? []).forEach(walk); };
    walk(copy);
    return copy;
  }
  function sidebarRow(id, label, {y, child, selected, strong, glyph, mark}) {
    return frame(id, label, {x: 4, y, width: 284, height: 28, gap: 8, padding: [0, 8, 0, child ? 26 : 8], alignItems: 'center', cornerRadius: '$--radius-sm', ...(selected ? {fill: '$--secondary'} : {})}, [
      ...(glyph ? [icon(`${id}-g`, glyph, {size: 12, fill: '$--subtle-foreground'})] : []), ...(mark ? [dot(`${id}-d`, mark, 7)] : []),
      body(`${id}-t`, label, strong || selected ? '$--foreground' : '$--subtle-foreground', {weight: strong ? '600' : '400', width: 'fill_container'}),
    ]);
  }
  function window(id, spec) {
    const width = spec.width ?? template.width;
    const bodyWidth = width - SIDE;
    const railX = bodyWidth - RAIL_W;
    const named = name => {
      const node = template.children.find(child => child.name === name);
      if (!node) throw new Error(`Workspace template lacks ${name}`);
      return reid(node, id);
    };
    const chrome = named('Window top row: device');
    const deviceRail = named('Device rail');
    const sidebar = named('Projects sidebar');
    const toolbar = named('Top row: Workspace path and icons');
    const keep = sidebar.children.filter(child => ['Search row', 'Overview row', 'Sidebar footer'].includes(child.name));
    const label = sidebar.children.find(child => child.name === 'Recent activity');
    const section = (key, content, y) => ({...structuredClone(label), id: `${id}-sb-${key}`, content, y});
    sidebar.children = [
      ...keep.filter(child => child.name !== 'Sidebar footer'),
      section('lp', 'Projects', 84),
      sidebarRow(`${id}-sb-hide`, 'hide', {y: 102, strong: true, glyph: 'chevron-down'}), sidebarRow(`${id}-sb-main`, 'main', {y: 130, child: true}),
      sidebarRow(`${id}-sb-sui`, 'settings-ui', {y: 158, child: true, selected: true}), sidebarRow(`${id}-sb-sync`, 'memory-sync', {y: 186, child: true}),
      section('lw', '현재 작업', 226),
      sidebarRow(`${id}-sb-w1`, '설정 화면 정리', {y: 244, mark: '$--agent-working'}), sidebarRow(`${id}-sb-w2`, '파일 보기 개선', {y: 272, mark: '$--warning'}),
      ...keep.filter(child => child.name === 'Sidebar footer'),
    ];
    const crumb = `${alias}:bkh81`;
    Object.assign(toolbar, {width: bodyWidth, descendants: {...toolbar.descendants, [crumb]: {...toolbar.descendants[crumb], content: 'hide  /  settings-ui'}}});
    const spec_ = spec.lane;
    const kids = [];
    let agentsWidth = railX;
    if (spec_ && !spec_.overlay && !spec_.expanded) agentsWidth = railX - GRIP_W - spec_.width;
    if (!spec_?.expanded) kids.push(agentViews(`${id}-av`, agentsWidth));
    if (spec_) {
      const laneId = `${id}-lane`;
      if (spec_.expanded) kids.push({...lane(laneId, {...spec_, width: railX}), x: 0, y: 0});
      else if (spec_.overlay) kids.push({...lane(laneId, spec_), x: railX - spec_.width, y: 0});
      else {
        kids.push(themedXref(`${id}-grip`, 'side-panel-grip', 'Agents / lane divider', {x: agentsWidth, y: 0, height: BODY_H}, {'side-panel-grip-line-top': {height: BODY_H}}));
        kids.push({...lane(laneId, spec_), x: agentsWidth + GRIP_W, y: 0});
      }
    }
    if (spec.tooltip) kids.push(tip(`${id}-tip`, spec.tooltip.label, {x: railX - spec_.width + spec.tooltip.dx, y: spec.tooltip.y}));
    kids.push({...rail(`${id}-rail`, spec.tool), x: railX, y: 0});
    return frame(id, `Workspace · ${spec.name}`, {width, height: HEIGHT, layout: 'none', fill: '$--background', cornerRadius: '$--radius-lg', stroke: '$--border', strokeWidth: 1, strokeAlignment: 'inner', clip: true, theme: {Mode: mode}}, [
      chrome, deviceRail, sidebar, toolbar,
      frame(`${id}-body`, 'Workspace body', {x: SIDE, y: TOP, width: bodyWidth, height: BODY_H, layout: 'none', clip: true}, kids),
    ]);
  }

  // -- boards: a title, the window, and the measure line under it --
  function board(id, {no, title, win, measures, caption}) {
    return frame(id, `${no}. ${title}`, {layout: 'vertical', gap: 8, padding: 16, fill: '$--muted', cornerRadius: '$--radius-md', theme: {Mode: mode}, width: 'fit_content'}, [
      sub(`${id}-title`, `${no}. ${title}`, '$--foreground', {weight: '600'}),
      win,
      frame(`${id}-measure`, 'Measure', {width: 'fill_container', layout: 'horizontal', gap: 18, alignItems: 'center', padding: [4, 4]}, [
        ...measures.map(([key, value], index) => frame(`${id}-m${index}`, key, {layout: 'horizontal', gap: 6, alignItems: 'center'}, [cap(`${id}-m${index}-k`, key), cap(`${id}-m${index}-v`, value, '$--foreground', {weight: '600'})])),
        spacer(`${id}-m-sp`), cap(`${id}-m-cap`, caption, '$--subtle-foreground'),
      ]),
    ]);
  }
  const STATES = [
    {key: 'closed', no: 1, title: '닫힘 · 레일만', name: 'closed', lane: null,
      measures: [['터미널', '1068px · 전체'], ['View', '없음']], caption: '레일 아이콘을 누르면 열림'},
    {key: 'overlay', no: 2, title: '겹침 (기본)', name: 'overlay', lane: {tool: 'files', pinned: false, overlay: true, width: LANE_W},
      measures: [['터미널', '1068px · 유지'], ['View', '520px · 겹침'], ['트리', '없음']], caption: '터미널 폭 유지 · 터미널 클릭·Esc로 사라짐'},
    {key: 'pinned', no: 3, title: '핀 (도킹)', name: 'pinned', lane: {tool: 'files', pinned: true, overlay: false, width: LANE_W},
      measures: [['터미널', '540px · 한 번 좁아짐'], ['View', '520px · 핀'], ['트리', '없음']], caption: '핀 · 터미널을 눌러도 남음'},
    {key: 'tree-overlay', no: 4, title: '트리 켬 · 겹침', name: 'tree on, overlay', lane: {tool: 'files', tree: 'on', pinned: false, overlay: true, width: LANE_W + TREE_W},
      measures: [['터미널', '1068px · 유지'], ['View', '760px · 겹침'], ['트리', '240px · 켬'], ['문서', '520px 그대로']], caption: '트리는 끌 때까지 남음'},
    {key: 'tree-only', no: 5, title: '트리만 (열린 문서 없음)', name: 'tree only, pinned', lane: {tool: 'files', tree: 'only', pinned: true, overlay: false, width: LANE_W},
      measures: [['터미널', '540px'], ['View', '520px · 핀'], ['트리', '520px · 트리만']], caption: '파일을 고르면 문서가 옆에 열림'},
    {key: 'expanded', no: 6, title: '확대', name: 'expanded', lane: {tool: 'files', tree: 'on', pinned: false, expanded: true, overlay: true, width: LANE_W},
      measures: [['터미널', '1068px · 아래에서 그대로'], ['View', '1068px · 확대'], ['트리', '240px · 켬']], caption: '다시 누르거나 Esc로 이전 배치'},
    {key: 'prs', no: 7, title: 'PRs 도구 · 핀', name: 'prs, pinned', lane: {tool: 'prs', pinned: true, overlay: false, width: LANE_W},
      measures: [['터미널', '540px'], ['View', '520px · 핀'], ['범위', 'hide 전체'], ['PRs 숫자', '2']], caption: '핀은 도구를 바꿔도 그대로 · PR 카드는 lane의 보기 자리에 열림'},
    {key: 'narrow', no: 8, title: '좁은 창 · 핀을 켰지만 겹침', name: 'narrow 1040', width: 1040, lane: {tool: 'files', pinned: true, overlay: true, width: LANE_W}, tooltip: {label: '핀 · 폭이 부족해 겹쳐 열림', dx: 262, y: 4},
      measures: [['터미널', '652px · 유지'], ['View', '520px · 겹침'], ['남는 터미널 폭', '124px < 480px']], caption: '핀 켬 · 480px이 남지 않아 겹쳐 열림, 터미널을 눌러도 남음'},
  ];
  const toolOf = spec => spec.lane ? (spec.lane.tool === 'prs' ? 'prs' : 'files') : null;
  return STATES.map(spec => board(`rl-${spec.key}-${suffix}`, {
    no: spec.no, title: spec.title, measures: spec.measures, caption: spec.caption,
    win: window(`rl-${spec.key}-${suffix}-win`, {name: spec.name, width: spec.width, lane: spec.lane, tooltip: spec.tooltip, tool: toolOf(spec)}),
  }));
}
