// The Sessions record (PRD session-record-redesign, record-card-readability):
// the PRs this session and its descendants made, the assigned work as a
// checklist of boxes with each delegated child hanging under the task that
// handed it on, the open missed work, the operator's requests at the end, and
// the summary-off and state boards. Controls reference the existing library
// through pen-screens' themed helpers; the PR mark is a ref of Component /
// Agent tree parts, as the Sessions rows draw it.
import {AGENT_TREE_MASTERS, PR_STATES} from './pen-agent-tree.mjs';
import {markSlot} from './pen-system.mjs';

export function sessionWorkScreens({frame, text, icon, screenButton, themedXref}) {
  const WIDTH = 372;
  const label = (id, content, fill = '$--foreground', extra = {}) => text(id, content, {size: '$--text-caption', fill, ...extra});
  const micro = (id, content, fill = '$--muted-foreground', extra = {}) => text(id, content, {size: '$--text-micro', fill, ...extra});
  const spacer = id => frame(id, 'Spacer', {width: 'fill_container', height: 1}, []);
  const provider = (id, kind = 'claude', size = 14) => frame(id, 'Provider', {width: size, height: size, fill: {type: 'image', enabled: true, url: `../web/src/assets/agent-${kind}.png`, mode: 'contain'}}, []);
  const prMark = (id, number, state) => {
    const [, glyph, fill] = PR_STATES.find(([key]) => key === state);
    return themedXref(id, AGENT_TREE_MASTERS.prIcon, `PR #${number}`, {}, {'ath-pr-g': {icon: glyph, fill}, 'ath-pr-n': {content: `#${number}`}});
  };
  // A section's head: its name, its count in the foreground, and a muted summary after a dot.
  const heading = (id, title, count, summary) => frame(id, title, {width: 'fill_container', height: 24, layout: 'horizontal', gap: 6, alignItems: 'end', padding: [8, 0, 0, 0]}, [
    micro(`${id}-t`, title, '$--muted-foreground', {weight: '500'}),
    ...(count === undefined ? [] : [label(`${id}-n`, count, '$--foreground', {weight: '600'})]),
    ...(summary ? [micro(`${id}-s`, `· ${summary}`)] : []),
  ]);
  const top = (id, {title, state, kind = 'claude'}) => [
    screenButton(`${id}-back`, 'Sessions로 돌아가기', {variant: 'ghost', icon: 'arrow-left'}),
    frame(`${id}-head`, 'Title', {width: 'fill_container', layout: 'horizontal', gap: 6, alignItems: 'center', padding: [4, 0, 0, 0]}, [
      provider(`${id}-mark`, kind, 16),
      text(`${id}-title`, title, {size: '$--text-subhead', weight: '600', width: 'fill_container'}),
    ]),
    micro(`${id}-state`, state),
  ];

  // Every PR the session and its descendants made, worst first; a descendant's
  // PR names that child at the row's end.
  const prRow = (id, {number, state, title, by}) => frame(id, `PR #${number}`, {width: 'fill_container', height: 22, layout: 'horizontal', gap: 6, alignItems: 'center'}, [
    prMark(`${id}-mark`, number, state),
    label(`${id}-title`, title, '$--foreground', {width: 'fill_container'}),
    ...(by ? [frame(`${id}-by`, 'Made by', {layout: 'horizontal', gap: 3, alignItems: 'center'}, [provider(`${id}-by-mark`, by[0], 12), micro(`${id}-by-name`, by[1])])] : []),
  ]);

  // A task's mark is a checkbox (record-card-readability D-05, B7, B8): checked
  // once done, empty otherwise; a live session's question or block puts `!` in
  // front of the box and says it in words at the row's end.
  const box = (id, done) => done
    ? frame(id, 'Checkbox checked', {width: 12, height: 12, cornerRadius: 2, fill: '$--success', layout: 'horizontal', justifyContent: 'center', alignItems: 'center'}, [icon(`${id}-g`, 'check', {size: 9, fill: '$--background'})])
    : frame(id, 'Checkbox', {width: 12, height: 12, cornerRadius: 2, stroke: '$--muted-foreground', strokeWidth: 1, strokeAlignment: 'inner'}, []);
  const WORDS = {unfinished: '일 남음', question: '내 차례 · 질문', review: '리뷰 대기', ci_failed: 'CI 실패'};
  const task = (id, {state, name, pr}) => {
    const done = state === 'done';
    const word = WORDS[state] && !(pr && (state === 'review' || state === 'ci_failed')) ? WORDS[state] : null;
    return frame(id, name, {width: 'fill_container', layout: 'horizontal', gap: 8, alignItems: 'center', padding: [3, 0]}, [
      ...(state === 'question' ? [label(`${id}-ask`, '!', '$--warning', {weight: '700'})] : []),
      box(`${id}-box`, done),
      label(`${id}-name`, name, done ? '$--subtle-foreground' : '$--foreground', {width: 'fill_container', weight: done ? '400' : '500'}),
      ...(word ? [micro(`${id}-word`, word, state === 'question' ? '$--warning' : '$--muted-foreground')] : []),
      ...(pr ? [prMark(`${id}-pr`, pr[0], pr[1])] : []),
    ]);
  };
  // A delegated child on one line: `↳` under the task that handed it on, its
  // state mark, its agent's logo, its name, its state word unless done, its
  // PR marks; with nothing to hang from it drops the arrow.
  const child = (id, {title, kind = 'claude', state, word, pr, hang = true}) => frame(id, 'Delegated child', {width: 'fill_container', height: 24, layout: 'horizontal', gap: 6, alignItems: 'center', padding: [0, 0, 0, hang ? 20 : 0]}, [
    ...(hang ? [micro(`${id}-arrow`, '↳')] : []),
    state === 'done' ? icon(`${id}-mark`, 'check', {size: 12, fill: '$--success'}) : markSlot(`${id}-mark`, state),
    provider(`${id}-logo`, kind, 14),
    label(`${id}-title`, title, state === 'done' ? '$--subtle-foreground' : '$--foreground', {width: 'fill_container'}),
    ...(word ? [label(`${id}-state`, word, '$--muted-foreground')] : []),
    ...(pr ? [prMark(`${id}-pr`, pr[0], pr[1])] : []),
  ]);
  // Missed work still open: one warning line with where it was found, then its two actions.
  const missed = (id, content, note) => frame(id, 'Missed work', {width: 'fill_container', layout: 'horizontal', gap: 5, alignItems: 'center', padding: [3, 0]}, [
    icon(`${id}-flag`, 'flag', {size: 11, fill: '$--warning'}),
    label(`${id}-text`, content, '$--warning'),
    micro(`${id}-note`, `· ${note}`),
    spacer(`${id}-sp`),
    screenButton(`${id}-issue`, '이슈로', {variant: 'outline', height: 22}),
    screenButton(`${id}-dismiss`, '무시', {variant: 'ghost', height: 22}),
  ]);
  // The requests at the record's end, last four, with who asked and when; the
  // shell cuts a first line to one row, so the board draws it already cut.
  const request = (id, {who = '나', content, at}) => frame(id, content, {width: 'fill_container', height: 20, layout: 'horizontal', gap: 8, alignItems: 'center'}, [
    micro(`${id}-who`, who, '$--muted-foreground', {width: 32}),
    label(`${id}-text`, content, '$--subtle-foreground', {width: 'fill_container'}),
    micro(`${id}-at`, at, '$--muted-foreground', {mono: true}),
  ]);
  const panel = (id, name, body) => frame(id, name, {width: WIDTH, layout: 'vertical', gap: 2, padding: [8, 12, 16, 12], fill: '$--card', stroke: '$--border', strokeWidth: 1, strokeAlignment: 'inner', cornerRadius: '$--radius-sm'}, body);

  // The one session the boards draw, read from a real record: three tasks, the
  // child the third handed on and its PR, one missed item still open.
  const TITLE = '성능 최적화 리뷰와 서비스 개선안';
  const REQUESTS = [
    ['r3', '15:39', '버그 3건 한 PR로 묶어서 진행해줘 opus 5.5로…'],
    ['r4', '15:53', '그러면 지금 처리하는 작업들 말고 그다음에…'],
    ['r5', '16:00', 'PR 열리면 리뷰하고 머지까지 진행해줘'],
    ['r6', '16:43', '잘 되고 있어?'],
  ];
  const BUGS = {title: '상주 데몬 버그 3건 수정 PR', state: 'done', pr: [1093, 'merged']};
  const words = (id, count) => [
    heading(`${id}-words`, '내 말', String(count)),
    micro(`${id}-earlier`, `이전 ${count - REQUESTS.length}턴 ›`),
    ...REQUESTS.map(([key, at, content]) => request(`${id}-${key}`, {content, at})),
  ];
  function record(suffix) {
    const id = `swr-record-${suffix}`;
    return panel(id, 'Session record', [
      ...top(id, {title: TITLE, state: '쉬는 중'}),
      heading(`${id}-prs`, 'PR', '2'),
      prRow(`${id}-pr1`, {number: 1100, state: 'pending', title: 'Measure the install build after the fixes'}),
      prRow(`${id}-pr2`, {number: 1093, state: 'merged', title: 'Stop three resident hided loops', by: ['claude', '상주 데몬 버그…']}),
      heading(`${id}-work`, '시킨 일', '2/3', '맡긴 일 2'),
      task(`${id}-t1`, {state: 'done', name: 'codex 성능 최적화 작업 리뷰 및 서비스 개선안 도출'}),
      task(`${id}-t2`, {state: 'done', name: '설치 빌드 실측 및 개선 제안 안전성 평가'}),
      child(`${id}-k0`, {title: '설치 빌드 실측 보조', kind: 'codex', state: 'done'}),
      task(`${id}-t3`, {state: 'unfinished', name: '버그 3건 한 PR로 묶어 수정 (Opus 에이전트)', pr: [1093, 'merged']}),
      child(`${id}-k1`, BUGS),
      heading(`${id}-missed`, '놓친 것', '1'),
      missed(`${id}-m1`, '재측정 전 설치본 버전 확인 안 됨', '설치 후 재측정'),
      ...words(id, 7),
    ]);
  }
  // Summaries off: no task judgment, so the PRs, the children as one list, and the requests.
  function off(suffix) {
    const id = `swr-off-${suffix}`;
    return panel(id, 'Summaries off', [
      ...top(id, {title: TITLE, state: '쉬는 중'}),
      heading(`${id}-prs`, 'PR', '1'),
      prRow(`${id}-pr1`, {number: 1093, state: 'merged', title: 'Stop three resident hided loops', by: ['claude', '상주 데몬 버그…']}),
      heading(`${id}-children`, '맡긴 일', '1'),
      child(`${id}-k1`, {...BUGS, hang: false}),
      ...words(id, 7),
    ]);
  }
  function states(suffix) {
    const id = `swr-states-${suffix}`;
    const group = (key, title, body) => frame(`${id}-${key}`, title, {width: 'fill_container', layout: 'vertical', gap: 4}, [micro(`${id}-${key}-cap`, title, '$--subtle-foreground', {weight: '600'}), ...body]);
    return panel(id, 'States', [
      group('asks', '살아 있는 세션의 질문: 박스 앞 ! 와 끝의 말', [
        task(`${id}-asks-t`, {state: 'question', name: '가격 페이지 시안 세 가지'}),
        child(`${id}-asks-k`, {title: '요금 비교 표 정리', state: 'going', word: '작업 중'}),
      ]),
      group('now', '판정 전 마지막 요청: 목록 끝에 방금 시킨 것', [
        task(`${id}-now-t`, {state: 'done', name: '설치 빌드 실측 및 개선 제안 안전성 평가'}),
        frame(`${id}-now-row`, 'Not yet judged', {width: 'fill_container', layout: 'horizontal', gap: 6, alignItems: 'start', padding: [4, 0]}, [
          label(`${id}-now-w`, '방금 시킨 것', '$--agent-working', {weight: '500'}),
          label(`${id}-now-t2`, '설치하고 재측정해줘', '$--foreground', {width: 'fill_container'}),
        ]),
      ]),
      group('delegated', '어느 일에도 걸리지 않은 자식: 끝에 맡긴 일', [
        task(`${id}-del-t`, {state: 'review', name: '⌘K 세션 검색', pr: [1122, 'failed']}),
        micro(`${id}-del-h`, '맡긴 일', '$--muted-foreground', {weight: '500'}),
        child(`${id}-del-k`, {title: '세션 검색 결과 정리', state: 'quiet', word: '쉬는 중', hang: false}),
      ]),
      group('loading', '읽는 중', [frame(`${id}-skeleton`, 'Skeleton', {width: 'fill_container', height: 32, fill: '$--secondary', cornerRadius: '$--radius-sm'}, [])]),
      group('failure', '기록을 읽지 못함', [screenButton(`${id}-retry`, '다시 시도', {variant: 'ghost'})]),
      group('search', '세션 검색 결과', [label(`${id}-search-title`, '세션 작업 기록과 검색 구현'), micro(`${id}-matched`, '세션 검색을 연결하고 좁은 칸에서도 읽기 좋게 해줘', '$--muted-foreground', {width: 'fill_container'})]),
      group('draft', '이슈로: 새 이슈', [
        micro(`${id}-repo`, 'example/hide'),
        micro(`${id}-note`, '내용을 확인하고 수정한 뒤 만들어 주세요.', '$--subtle-foreground'),
        label(`${id}-draft-title`, '재측정 전 설치본 버전 확인'),
        label(`${id}-body`, '확인할 일\n재측정 전에 설치된 앱의 버전과 빌드를 기록합니다.\n\n완료 조건\n측정 기록에 버전이 함께 남습니다.', '$--foreground', {width: 'fill_container'}),
        frame(`${id}-footer`, 'Explicit creation', {width: 'fill_container', layout: 'horizontal', gap: 8, justifyContent: 'end'}, [screenButton(`${id}-cancel`, '취소', {variant: 'secondary'}), screenButton(`${id}-create`, '이슈 만들기')]),
      ]),
    ]);
  }
  return suffix => [record(suffix), off(suffix), states(suffix)];
}
