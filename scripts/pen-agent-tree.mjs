// `Component / Agent tree parts` (PRD agent-hierarchy-screens D-37 to D-40,
// D-44): the parts every agent tree draws the same way, as
// web/src/components/agent-tree.tsx renders them. Each part is one reusable
// master; every state in the Light and Dark frames is a `ref` of it with
// descendant overrides, as pen-system.mjs's System parts are. The screens in
// design/hide-screens.pen ref these masters by id, so the ids are stable.
import {frame, icon, markShapes, markSlot, markSwitch, num, text} from './pen-system.mjs';

export const AGENT_TREE_MASTERS = {
  verb: 'ath-verb',
  prIcon: 'ath-pr-icon',
  mark: 'ath-mark',
  treeButton: 'ath-tree-button',
  row: 'ath-row',
};

// The one PR mark's six states (web/src/prMark.ts PR_LOOK), worst first; one token each.
export const PR_STATES = [
  ['failed', 'git-pull-request', '$--pr-failed', 'CI 실패'],
  ['pending', 'git-pull-request', '$--pr-pending', 'CI 도는 중 · 리뷰 대기'],
  ['mergeable', 'git-pull-request', '$--pr-mergeable', '머지 가능'],
  ['draft', 'git-pull-request-draft', '$--pr-draft', 'draft'],
  ['merged', 'git-merge', '$--pr-merged', '머지됨'],
  ['closed', 'git-pull-request-closed', '$--pr-closed', '닫힘'],
];

export function buildAgentTreeParts(tokens) {
  const n = name => num(tokens, name);
  const HAIR = n('--size-hairline');
  const MARK = n('--size-agent-mark');
  const ICON = n('--size-icon-sm');
  const PR = n('--size-pr-icon');
  const LINE = n('--size-sidebar-line');
  const DETAIL = n('--size-sidebar-line-detail');
  const INDENT = n('--size-lineage-indent');
  const LANE = n('--size-lineage-chevron');
  const RAIL_X = n('--size-lineage-rail-x');
  const ELBOW_Y = n('--size-lineage-elbow-y');
  const DIMMED = n('--opacity-dimmed');
  const caption = (id, content, fill, opts = {}) => text(id, content, {size: '$--text-caption', fill, ...opts});
  const ref = (id, master, name, overrides = {}, descendants) => ({id, type: 'ref', ref: master, name, ...overrides, ...(descendants ? {descendants} : {})});
  const cell = (label, instance) => frame(`${instance.id}-cell`, label, {layout: 'vertical', gap: '$--spacing-xs', alignItems: 'start', width: 'fit_content'}, [
    text(`${instance.id}-cap`, label, {size: '$--text-micro', fill: '$--muted-foreground', weight: '600'}),
    instance,
  ]);

  // Verb: 승인 · 질문 · 확인 · 초안, coloured text with no box (D-37).
  const verb = frame(AGENT_TREE_MASTERS.verb, 'Verb', {reusable: true, layout: 'horizontal', width: 'fit_content'}, [
    caption('ath-verb-t', '승인', '$--warning', {weight: '600'}),
  ]);

  // PR mark (web/src/components/pr-mark.tsx): the state's icon in its colour
  // and the number in the subtle ink; several PRs are one mark per state, its
  // count in the state's colour. No border, a fill only under the pointer;
  // dimmed while GitHub cannot be read. The compact variant hides the number
  // where the row already names the pull request.
  const prIcon = frame(AGENT_TREE_MASTERS.prIcon, 'PR mark', {reusable: true, layout: 'horizontal', gap: '$--spacing-xxs', alignItems: 'center', width: 'fit_content', height: PR}, [
    icon('ath-pr-g', 'git-pull-request', {size: PR, fill: '$--pr-failed'}),
    caption('ath-pr-n', '924', '$--subtle-foreground', {mono: true}),
  ]);

  // Descendant mark: the turn mark and N raised, on a folded parent only; working children are not counted (B17; status-marks D-24, B35).
  const mark = frame(AGENT_TREE_MASTERS.mark, 'Descendant mark', {reusable: true, layout: 'horizontal', gap: '$--spacing-xxs', alignItems: 'center', width: 'fit_content'}, [
    markSlot('ath-mark-bang', 'turn'),
    caption('ath-mark-n', '2', '$--warning', {weight: '600', mono: true}),
  ]);

  // The pane header exposes direct children through one stable tree/count button.
  const treeButton = frame(AGENT_TREE_MASTERS.treeButton, 'Tree button', {
    reusable: true, layout: 'horizontal', gap: '$--spacing-xxs', alignItems: 'center', height: DETAIL, padding: [0, '$--spacing-xs'],
    cornerRadius: '$--radius-xs', stroke: '$--border', strokeWidth: HAIR, strokeAlignment: 'inner',
  }, [
    icon('ath-tb-tree', 'list-tree', {size: ICON, fill: '$--muted-foreground'}),
    caption('ath-tb-count', '3', '$--muted-foreground', {size:'$--text-micro',mono:true}),
    icon('ath-tb-chev', 'chevron-down', {size: ICON, fill: '$--muted-foreground'}),
  ]);

  // Tree row: one line; the rails and the chevron lane left of the marks, the
  // PR icon and the folded parent's mark before the age; a second line only
  // for a block's cause, a waiting ring's 대기 line (status-marks B29) or an
  // unknown row's reason, and a device line only for a child on another
  // machine (D-38, B14).
  const rail = (id, props) => frame(id, 'Rail', {fill: '$--lineage-rail', ...props}, []);
  const row = frame(AGENT_TREE_MASTERS.row, 'Tree row', {reusable: true, layout: 'horizontal', alignItems: 'start', width: 260, cornerRadius: '$--radius-sm'}, [
    frame('ath-row-rail', 'Ancestor rail', {layout: 'none', width: INDENT, height: LINE + 8, enabled: false}, [rail('ath-row-rail-v', {x: RAIL_X, y: 0, width: HAIR, height: LINE + 8})]),
    frame('ath-row-elbow', 'Elbow', {layout: 'none', width: INDENT, height: LINE + 8, enabled: false}, [
      rail('ath-row-elbow-v', {x: RAIL_X, y: 0, width: HAIR, height: ELBOW_Y}),
      rail('ath-row-elbow-h', {x: RAIL_X, y: ELBOW_Y, width: INDENT - RAIL_X, height: HAIR}),
    ]),
    frame('ath-row-lane', 'Chevron lane', {layout: 'horizontal', width: LANE, height: LINE + 8, justifyContent: 'center', padding: ['$--spacing-xs', 0, 0, 0]}, [
      icon('ath-row-chev', 'chevron-right', {size: ICON, fill: '$--muted-foreground'}),
    ]),
    frame('ath-row-body', 'Body', {layout: 'vertical', gap: 0, width: 'fill_container', padding: ['$--spacing-xs', '$--spacing-xs', '$--spacing-xs', '$--spacing-xs']}, [
      frame('ath-row-l1', 'Line one', {layout: 'horizontal', gap: '$--spacing-xs', alignItems: 'center', width: 'fill_container', height: LINE}, [
        frame('ath-row-mark', 'Status mark', {width: MARK, height: MARK, layout: 'horizontal', justifyContent: 'center', alignItems: 'center'}, markShapes('ath-row')),
        {...text('ath-row-title', '에이전트 제목', {fill: '$--subtle-foreground'}), textGrowth: 'fixed-width', width: 'fill_container'},
        ref('ath-row-pr', AGENT_TREE_MASTERS.prIcon, 'PR mark'),
        ref('ath-row-desc', AGENT_TREE_MASTERS.mark, 'Descendant mark', {enabled: false}),
        caption('ath-row-age', '3m', '$--muted-foreground', {mono: true}),
      ]),
      {...caption('ath-row-line', '대기 · 자식 1명 작업 중', '$--subtle-foreground'), enabled: false},
      frame('ath-row-device', 'Device line', {layout: 'horizontal', gap: '$--spacing-xxs', alignItems: 'center', height: DETAIL, padding: [0, '$--spacing-xs'], cornerRadius: '$--radius-xs', stroke: '$--border', strokeWidth: HAIR, strokeAlignment: 'inner', enabled: false}, [
        icon('ath-row-device-g', 'server', {size: ICON, fill: '$--muted-foreground'}),
        caption('ath-row-device-t', 'Mac mini', '$--muted-foreground'),
      ]),
    ]),
  ]);

  const states = suffix => {
    const id = name => `ath-${suffix}-${name}`;
    const verbs = ['승인', '질문', '확인', '초안'].map((word, index) => cell(`Verb ${index + 1}`, ref(id(`verb-${index}`), AGENT_TREE_MASTERS.verb, 'Verb', {}, {'ath-verb-t': {content: word}})));
    const icons = PR_STATES.map(([state, glyph, fill, label]) => cell(label, ref(id(`pr-${state}`), AGENT_TREE_MASTERS.prIcon, 'PR mark', {}, {'ath-pr-g': {icon: glyph, fill}})));
    // Several PRs: each state's icon and count in its colour, worst first (a mergeable and a merged PR read green 1 purple 1).
    const counted = [['mergeable', 1], ['merged', 1]].map(([state, count]) => {
      const [, glyph, fill] = PR_STATES.find(([key]) => key === state);
      return ref(id(`pr-more-${state}`), AGENT_TREE_MASTERS.prIcon, 'PR mark', {}, {'ath-pr-g': {icon: glyph, fill}, 'ath-pr-n': {content: String(count), fill}});
    });
    const more = cell('여러 PR (상태별 개수)', frame(id('pr-more'), 'PR state counts', {layout: 'horizontal', gap: '$--spacing-xs', alignItems: 'center', width: 'fit_content'}, counted));
    const stale = cell('흐림 (GitHub 못 읽음)', ref(id('pr-stale'), AGENT_TREE_MASTERS.prIcon, 'PR mark', {opacity: DIMMED}, {'ath-pr-g': {icon: 'git-pull-request', fill: '$--pr-pending'}}));
    const hover = cell('포인터 아래', ref(id('pr-hover'), AGENT_TREE_MASTERS.prIcon, 'PR mark', {fill: '$--accent', cornerRadius: '$--radius-xs', padding: [0, '$--spacing-xxs']}));
    const compact = cell('좁은 변형 (번호 없음)', ref(id('pr-compact'), AGENT_TREE_MASTERS.prIcon, 'PR mark', {}, {'ath-pr-g': {icon: 'git-pull-request', fill: '$--pr-mergeable'}, 'ath-pr-n': {enabled: false}}));
    const marks = [
      cell('! N raised', ref(id('mark-raised'), AGENT_TREE_MASTERS.mark, 'Descendant mark')),
    ];
    const buttons = [
      cell('Tree button rest', ref(id('tb-rest'), AGENT_TREE_MASTERS.treeButton, 'Tree button')),
      cell('Tree button open', ref(id('tb-open'), AGENT_TREE_MASTERS.treeButton, 'Tree button', {fill: '$--secondary'})),
      cell('Tree button with five children', ref(id('tb-counts'), AGENT_TREE_MASTERS.treeButton, 'Tree button', {fill:'$--background'}, {'ath-tb-count':{content:'5'}})),
      cell('Tree button beside a raised ask', ref(id('tb-raised'), AGENT_TREE_MASTERS.treeButton, 'Tree button', {fill:'$--background'}, {'ath-tb-count':{content:'2'}})),
      cell('Unavailable children keep the last known count', ref(id('tb-unknown'), AGENT_TREE_MASTERS.treeButton, 'Tree button', {fill:'$--background'}, {'ath-tb-count':{content:'3'}})),
    ];
    const rows = [
      cell('Root, opened', ref(id('row-root'), AGENT_TREE_MASTERS.row, 'Tree row', {}, {'ath-row-chev': {icon: 'chevron-down'}, ...markSwitch('ath-row', 'going'), 'ath-row-title': {content: '루트 에이전트', fill: '$--foreground'}, 'ath-row-pr/ath-pr-g': {fill: '$--pr-mergeable'}})),
      cell('Child, folded with a raised descendant', ref(id('row-child'), AGENT_TREE_MASTERS.row, 'Tree row', {}, {'ath-row-elbow': {enabled: true}, ...markSwitch('ath-row', 'going'), 'ath-row-title': {content: '자식'}, 'ath-row-desc': {enabled: true}, 'ath-row-age': {content: '19h'}})),
      cell('Grandchild, no children', ref(id('row-grand'), AGENT_TREE_MASTERS.row, 'Tree row', {}, {'ath-row-rail': {enabled: true}, 'ath-row-elbow': {enabled: true}, 'ath-row-chev': {enabled: false}, ...markSwitch('ath-row', 'quiet'), 'ath-row-title': {content: '손자'}, 'ath-row-pr': {enabled: false}, 'ath-row-age': {content: '40m'}})),
      cell('Waiting ring: dashed, its line led by 대기', ref(id('row-waiting'), AGENT_TREE_MASTERS.row, 'Tree row', {}, {'ath-row-elbow': {enabled: true}, 'ath-row-chev': {enabled: false}, ...markSwitch('ath-row', 'waiting'), 'ath-row-title': {content: 'CI 기다리는 자식'}, 'ath-row-line': {enabled: true, content: '대기 · CI · PR #214', fill: '$--subtle-foreground'}, 'ath-row-pr': {enabled: false}, 'ath-row-age': {content: '12m'}})),
      cell('Child on another device', ref(id('row-remote'), AGENT_TREE_MASTERS.row, 'Tree row', {}, {'ath-row-elbow': {enabled: true}, 'ath-row-chev': {enabled: false}, ...markSwitch('ath-row', 'quiet'), 'ath-row-title': {content: '다른 기기의 자식'}, 'ath-row-pr': {enabled: false}, 'ath-row-device': {enabled: true}, 'ath-row-age': {content: '5h'}})),
    ];
    return [...verbs, ...icons, more, stale, hover, compact, ...marks, ...buttons, ...rows];
  };
  const themeFrame = (frameId, mode, cells) => frame(frameId, mode, {
    theme: {Mode: mode}, layout: 'horizontal', gap: '$--spacing-lg', alignItems: 'end', padding: '$--spacing-lg',
    fill: '$--background', width: 'fit_content', cornerRadius: '$--radius-md',
  }, cells);

  const id = 'cmp-agent-tree-parts';
  return frame(id, 'Component / Agent tree parts', {
    layout: 'vertical', gap: '$--spacing-xl', padding: '$--spacing-xl', fill: '$--card', cornerRadius: '$--radius-lg', width: 'fit_content',
  }, [
    text(`${id}-title`, 'Agent tree parts', {size: '$--text-headline', weight: '600'}),
    text(`${id}-spec`, 'web/src/components/agent-tree.tsx (PRD agent-hierarchy-screens D-37 to D-40): the verb a Needs You line and an ask band start with, coloured text with no box; the one PR mark every PR surface draws (web/src/components/pr-mark.tsx): the state\'s icon in its colour (failed red, pending amber, mergeable green, draft grey, merged purple, closed dim), the number for one PR and, for several, each state\'s icon and count in that state\'s colour, worst first, no border, a fill under the pointer, dimmed while GitHub cannot be read, and a compact variant without the number; the one descendant mark a folded parent wears, ! N while raised, nothing for working alone; the pane header\'s tree button, state counts without a separate total or internal divider, a tree icon when no count is drawn; and the one-line tree row with its elbow rails, chevron lane, PR icon, mark and age, a second line only for a block\'s cause, a waiting ring (led by 대기) or an unknown row, plus a device line only for a child on another machine. Sessions rows, the pane header, the sidebar row and the tree popover all draw the same PR mark.', {size: '$--text-caption', fill: '$--subtle-foreground', width: 820}),
    frame(`${id}-masters`, 'Masters', {layout: 'horizontal', gap: '$--spacing-xl', alignItems: 'start'}, [verb, prIcon, mark, treeButton, row]),
    themeFrame(`${id}-light`, 'Light', states('l')),
    themeFrame(`${id}-dark`, 'Dark', states('d')),
  ]);
}
