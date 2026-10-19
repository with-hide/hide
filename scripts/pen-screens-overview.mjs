// Five-tab Overview screen documentation follows the approved HTML structure.
// All names and amounts below are invented design examples, never runtime data.
import {markSlot, num} from './pen-system.mjs';

export function overviewScreens(tokens, {frame, text, screenButton, screenInput}) {
  const FG = '$--foreground', SUB = '$--subtle-foreground', MUTED = '$--muted-foreground';
  const hair = num(tokens, '--size-hairline');
  const sm = num(tokens, '--spacing-sm'), md = num(tokens, '--spacing-md');
  const lg = num(tokens, '--spacing-lg'), xl = num(tokens, '--spacing-xl'), xxl = num(tokens, '--spacing-xxl');
  const width = num(tokens, '--size-overview-sheet-w'), height = 900;
  const row = (id, name, children, extra = {}) => frame(id, name, {layout: 'horizontal', gap: sm, alignItems: 'center', width: 'fill_container', ...extra}, children);
  const col = (id, name, children, extra = {}) => frame(id, name, {layout: 'vertical', gap: md, width: 'fill_container', ...extra}, children);
  const label = (id, words, extra = {}) => text(id, words, {size: '$--text-body', fill: SUB, ...extra});
  const small = (id, words, extra = {}) => label(id, words, {size: '$--text-caption', fill: MUTED, ...extra});
  const title = (id, words) => label(id, words, {size: '$--text-headline', fill: FG, weight: '600'});
  const spacer = id => frame(id, 'Spacer', {width: 'fill_container', height: hair}, []);
  const rule = id => frame(id, 'Rule', {width: 'fill_container', height: hair, fill: '$--border'}, []);
  const controls = (id, words) => row(id, 'Filters', words.map((word, i) => screenButton(`${id}-${i}`, word, {variant: 'outline'})));
  const content = (id, name, children) => col(id, name, children, {padding: [xxl, xl], gap: xl});
  function page(id, name, active, body, project = '모든 프로젝트') {
    return col(id, name, [
      row(`${id}-head`, 'Overview header', [title(`${id}-title`, '개요'), spacer(`${id}-head-sp`), screenButton(`${id}-search`, '작업, 세션, 프로젝트 검색  ⌘K', {variant: 'outline'}), screenButton(`${id}-device`, '이 Mac · 연결됨  ▾', {variant: 'outline'}), screenButton(`${id}-esc`, 'Esc', {variant: 'ghost'})], {padding: [lg, xl], stroke: '$--border', strokeWidth: {bottom: hair}}),
      row(`${id}-tabs`, 'Five tabs and shared scope', [
        ...[['work', '작업 14'], ['projects', '프로젝트 4'], ['backlog', '백로그 8'], ['history', '기록'], ['usage', '사용량']].map(([key, words]) => row(`${id}-tab-${key}`, words, [label(`${id}-tab-${key}-t`, words, {fill: key === active ? FG : MUTED, weight: key === active ? '600' : '400'})], {width: 'fit_content', padding: [md, sm], stroke: key === active ? '$--primary' : '$--background', strokeWidth: {bottom: 2 * hair}})),
        spacer(`${id}-tab-sp`), active === 'usage' ? spacer(`${id}-scope`) : screenButton(`${id}-scope`, `${project}  ▾`, {variant: 'outline'}),
      ], {padding: [0, xl], gap: lg, stroke: '$--border', strokeWidth: {bottom: hair}}), body,
    ], {width, height, gap: 0, clip: true, fill: '$--background', stroke: '$--border', strokeWidth: hair});
  }
  function graph(id) {
    const session = (key, name, state, child = false, context = false) => {
      const orb = num(tokens, child ? '--size-overview-graph-child-orb' : '--size-overview-graph-root-orb');
      return col(key, name, [frame(`${key}-orb`, 'Circular provider node', {width: orb, height: orb, cornerRadius: orb / 2, fill: '$--secondary', stroke: state === '내 차례' ? '$--warning' : child ? '$--border' : '$--primary', strokeWidth: hair, layout: 'horizontal', alignItems: 'center', justifyContent: 'center'}, [frame(`${key}-provider`, child ? 'Codex logo' : 'Claude Code logo', {width: orb * 0.65, height: orb * 0.65, fill: {type: 'image', enabled: true, url: `../web/src/assets/agent-${child ? 'codex' : 'claude'}.png`, mode: 'fit'}}, []), {...markSlot(`${key}-status`, state === '내 차례' ? 'turn' : state === '읽을 것' ? 'done' : context ? 'quiet' : 'going'), layoutPosition: 'absolute', x: orb - 12, y: orb - 12}]), label(`${key}-name`, name, {fill: FG, weight: child ? '500' : '600'}), small(`${key}-agent`, child ? 'Codex · model-c' : 'Claude Code · model-a'), small(`${key}-state`, state), ...(context ? [small(`${key}-context`, '상위 세션 문맥')] : [])], {width: num(tokens, child ? '--size-overview-graph-child-width' : '--size-overview-graph-root-width'), alignItems: 'center', opacity: context ? num(tokens, '--opacity-dimmed') : 1, gap: sm});
    };
    return content(id, 'Work graph focus', [
      row(`${id}-toolbar`, 'Graph controls', [title(`${id}-title`, '작업 그래프'), spacer(`${id}-sp`), screenButton(`${id}-collapse`, '모두 접기', {variant: 'ghost'}), screenButton(`${id}-graph`, '그래프', {variant: 'secondary'}), screenButton(`${id}-list`, '세션 목록', {variant: 'ghost'})]),
      controls(`${id}-filters`, ['전체 10', '내 차례 2', '작업 중 5', '안 읽은 결과 2', '☐ 잠든 세션 포함', '☑ 편지 관계']),
      small(`${id}-omitted`, '▸ 접힌 프로젝트 2개 · 잠든 세션뿐인 프로젝트 / 세션 없음'),
      small(`${id}-legend`, '실선: 부모와 자식 · 점선: 편지 · 관련 노드에 머물 때만 편지 건수 표시'),
      row(`${id}-groups`, 'Compound project groups at readable zoom', [
        col(`${id}-hide`, 'hide expanded', [label(`${id}-hide-title`, 'hide · 세션 7', {fill: FG, weight: '600'}), row(`${id}-family`, 'Verified parent and children', [session(`${id}-root`, '사이드바 가독성 개선', '확인함', false, true), label(`${id}-lineage`, '→'), col(`${id}-children`, 'Children', [session(`${id}-c1`, '컴포넌트 구조 정리', '작업 중', true), session(`${id}-c2`, '한글 가독성 확인', '읽을 것', true)], {width: 'fit_content'})], {gap: xl})], {padding: xl, gap: xl, cornerRadius: '$--radius-xl', stroke: '$--border', strokeWidth: hair}),
        col(`${id}-collapsed`, 'Collapsed active projects', ['billing-api', 'content-web'].map((name, i) => row(`${id}-project-${i}`, name, [label(`${id}-pn-${i}`, name, {fill: FG, weight: '600'}), spacer(`${id}-ps-${i}`), small(`${id}-pc-${i}`, i ? '세션 1' : '세션 2'), label(`${id}-pe-${i}`, '›')], {padding: lg, cornerRadius: '$--radius-xl', stroke: '$--border', strokeWidth: hair})), {width: num(tokens, '--size-overview-graph-group-width'), gap: lg}),
      ], {alignItems: 'start', gap: xxl}),
      small(`${id}-selection`, '선택한 세션 상세는 오른쪽에 · 좁은 화면은 기존 패널 · Escape로 닫기'),
    ]);
  }

  function backlog(id) {
    const rows = [['#163', '좁은 사이드바에서 긴 한국어 요청 줄바꿈 확인', 'GitHub', 'jules', 'mira', 'enhancement', '2시간 전'], ['#161', '프로세스 종료 뒤 남은 자식 정리', 'GitHub', 'mira', 'jules', 'bug', '어제'], ['L-2', '다음 검토에서 확인할 키보드 흐름', '메모', '-', '-', '-', '어제']];
    return content(id, 'Mixed backlog', [
      row(`${id}-heading`, 'Heading', [title(`${id}-title`, '백로그'), spacer(`${id}-sp`), screenButton(`${id}-new`, '+ 추가', {variant: 'outline'})]),
      controls(`${id}-status`, ['열림', '진행 중', '닫힘', '전체', '출처: 전체']),
      row(`${id}-filters`, 'Attributes and search', [screenButton(`${id}-author`, '작성자 ▾', {variant: 'outline'}), screenButton(`${id}-assignee`, '담당자 ▾', {variant: 'outline'}), screenButton(`${id}-label`, '라벨 ▾', {variant: 'outline'}), screenInput(`${id}-search`, {placeholder: '이슈와 메모 검색', width: 360})]),
      row(`${id}-columns`, 'Shared table columns', ['상태', '제목', '출처', '작성자', '담당자', '라벨', '업데이트'].map((words, i) => small(`${id}-col-${i}`, words, {width: i === 1 ? 440 : 100})), {padding: [md, sm]}), rule(`${id}-rule`),
      row(`${id}-project`, 'Project band', [label(`${id}-project-name`, 'hide', {fill: FG, weight: '600'}), small(`${id}-project-count`, '3개')], {padding: [md, sm], fill: '$--muted'}),
      ...rows.map((values, i) => row(`${id}-row-${i}`, values[1], [small(`${id}-state-${i}`, '○ 열림', {width: 100}), label(`${id}-item-${i}`, `${values[0]}  ${values[1]}`, {width: 440, fill: FG}), ...values.slice(2).map((words, j) => small(`${id}-meta-${i}-${j}`, words, {width: 100}))], {padding: [lg, sm], stroke: '$--border', strokeWidth: {bottom: hair}})),
    ]);
  }
  function detail(id) {
    return content(id, 'Wide backlog detail', [screenButton(`${id}-back`, '← 백로그', {variant: 'ghost'}), row(`${id}-heading`, 'Item heading', [title(`${id}-title`, '#163 좁은 사이드바에서 긴 한국어 요청 줄바꿈 확인'), spacer(`${id}-sp`), screenButton(`${id}-start`, '시작')]), row(`${id}-columns`, 'Body and metadata', [col(`${id}-body`, 'Readable Markdown and activity', [label(`${id}-body-title`, '목표', {size: '$--text-title', fill: FG, weight: '600'}), label(`${id}-body-text`, '긴 한국어와 English issue titles를 같은 목록에서 읽을 수 있어야 합니다.'), label(`${id}-check`, '☑ 제목과 메타데이터 비교\n☐ 좁은 화면에서 키보드 검토'), rule(`${id}-rule`), label(`${id}-activity`, '활동 2', {fill: FG, weight: '600'}), label(`${id}-comment`, 'mira · 본문과 활동을 함께 읽을 수 있습니다.'), screenButton(`${id}-more`, '댓글 더 보기', {variant: 'outline'})], {gap: xl}), col(`${id}-metadata`, 'Source and activity facts', [small(`${id}-repo`, 'GitHub · example/hide'), label(`${id}-state`, '열림'), label(`${id}-author`, '작성자 jules'), label(`${id}-assignee`, '담당자 mira'), label(`${id}-labels`, 'enhancement'), small(`${id}-updated`, '2시간 전 업데이트')], {width: 260, gap: lg})], {gap: xxl, alignItems: 'start'})]);
  }
  function projects(id, home = false) {
    return content(id, home ? 'Local project home' : 'Project list', home ? [screenButton(`${id}-back`, '← 모든 프로젝트', {variant: 'ghost'}), title(`${id}-title`, 'hide'), controls(`${id}-links`, ['작업 →', '백로그 →', '기록 →']), row(`${id}-facts`, 'Project facts', [col(`${id}-settings`, 'Existing controls', [label(`${id}-backlog`, 'GitHub 4 · 메모 1'), label(`${id}-roots`, '루트 작업 · 일하는 중 2'), label(`${id}-storage`, '체크아웃 6 · 디스크 38 GB'), controls(`${id}-actions`, ['열기', '새 에이전트', '설정', '정리'])]), col(`${id}-sessions`, 'Live sessions', [label(`${id}-count`, '세션 11', {fill: FG, weight: '600'}), label(`${id}-s1`, '사이드바 가독성 개선 · 작업 중'), label(`${id}-s2`, '컴포넌트 구조 정리 · 작업 중'), label(`${id}-s3`, '한글 가독성 확인 · 읽을 것')])], {gap: xxl, alignItems: 'start'})] : [
      row(`${id}-heading`, 'Projects header', [title(`${id}-title`, '프로젝트'), spacer(`${id}-sp`), screenButton(`${id}-add`, '+ 프로젝트 추가', {variant: 'outline'})]), small(`${id}-disk`, '디스크 여유 41 GB / 494 GB · 프로젝트 합계 63 GB · 정리하면 12 GB'),
      row(`${id}-columns`, 'Columns', ['프로젝트', '세션', '루트 작업', '백로그', '디스크', '챙길 것', '최근 활동', '열기'].map((words, i) => small(`${id}-col-${i}`, words, {width: [240, 170, 180, 85, 100, 160, 110, 80][i]}))),
      ...[['hide', '11 · 부모 4 · 자식 7', '일하는 중 2 · 읽을 것 2', '5', '38 GB', '4 merged → 정리', '3분 전', '열기'], ['billing-api', '1 · 부모 1 · 자식 0', '일하는 중 1', '2', '9.4 GB', '', '25분 전', '열기'], ['content-web', '1 · 부모 1 · 자식 0', '읽을 것 1', '1', '14 GB', '', '1시간 전', '열기'], ['team-notes', '1 · 부모 1 · 자식 0', '일감 없음', '0', '1.1 GB', '', '어제', '열기']].map((values, i) => row(`${id}-row-${i}`, values[0], values.map((words, j) => label(`${id}-cell-${i}-${j}`, words, {width: [240, 170, 180, 85, 100, 160, 110, 80][j], ...(j === 0 ? {size: '$--text-title', weight: '600', fill: FG} : {})})), {padding: [xl, 0], stroke: '$--border', strokeWidth: {bottom: hair}})),
    ]);
  }
  function history(id) {
    return content(id, 'Operator request search', [title(`${id}-title`, '기록'), row(`${id}-search-row`, 'Search and scope', [screenInput(`${id}-search`, {placeholder: '내가 쓴 요청 검색', width: 640}), screenButton(`${id}-scope`, '내 요청 ▾', {variant: 'outline'})]), controls(`${id}-filters`, ['전체 상태 ▾', '모든 에이전트 ▾', '전체 기간 ▾']), small(`${id}-limit`, '저장된 요청 앞 500자 · 보관된 세션 범위'),
      ...[['오늘', '사이드바 가독성 개선', '좁은 사이드바에서 긴 제목이 잘리는 문제부터 고쳐줘. 툴팁으로 전체 제목이 보이게', 'hide · Claude Code · 요청 9'], ['어제', '결제 재시도 흐름 정리', '실패한 결제를 다시 시도할 때 현재 상태를 확인해줘', 'billing-api · Codex · 보관됨'], ['10월 17일', '혼합 언어 목록 검토', '긴 한국어 제목과 English identifiers를 함께 확인해줘', 'hide · Claude Code · 보관됨']].map((values, i) => col(`${id}-result-${i}`, values[1], [small(`${id}-date-${i}`, values[0]), label(`${id}-name-${i}`, values[1], {fill: FG, weight: '600'}), label(`${id}-request-${i}`, `❯ ${values[2]}`), small(`${id}-meta-${i}`, values[3])], {padding: [lg, 0], gap: md, stroke: '$--border', strokeWidth: {bottom: hair}})),
    ]);
  }
  function usage(id) {
    return content(id, 'Account-wide usage', [small(`${id}-scope`, '계정 전체 · 이 Mac'), title(`${id}-title`, '사용량'), controls(`${id}-period`, ['7일', '30일', '90일', '1년', '모든 에이전트 ▾']), row(`${id}-kpis`, 'API estimate and new tokens', [col(`${id}-cost`, 'API estimate', [small(`${id}-cost-label`, 'API 요금 환산 추정'), label(`${id}-cost-value`, '$42.38', {size: '$--text-display', fill: FG, mono: true})]), col(`${id}-tokens`, 'New tokens', [small(`${id}-tokens-label`, '새 토큰'), label(`${id}-tokens-value`, '≥ 12.4M', {size: '$--text-display', fill: FG, mono: true})])], {gap: xxl}), row(`${id}-plot`, 'Daily tokens', [22, 35, 18, 60, 46, 90, 70].map((value, i) => frame(`${id}-bar-${i}`, `Day ${i + 1}`, {width: 70, height: value * 2, fill: '$--primary', cornerRadius: '$--radius-xs'}, [])), {height: 220, alignItems: 'end', gap: xl}), controls(`${id}-views`, ['일별', '모델별', '정확한 숫자']), row(`${id}-table-head`, 'Usage columns', ['날짜 / 모델', '추정 비용', '새 토큰', '입력', '출력', '캐시 쓰기', '캐시 읽기'].map((words, i) => small(`${id}-table-col-${i}`, words, {width: 160}))), rule(`${id}-rule`), label(`${id}-table-row`, '오늘 / model-a                       $8.12                 ≥ 2.1M                 1.2M                 0.4M'), small(`${id}-bounds`, '공개 API 요금 기준 추정 · 사용량 미보고 호출이 있는 기간은 하한값'), screenButton(`${id}-accounts`, '계정 한도 ▸', {variant: 'ghost'})]);
  }
  const quick = id => col(id, 'Quick add starts local', [title(`${id}-title`, '백로그에 추가'), screenButton(`${id}-project`, 'hide ▾', {variant: 'outline'}), screenInput(`${id}-input`, {placeholder: '다음에 할 일을 적으세요', width: 540}), controls(`${id}-destination`, ['메모 ✓', 'GitHub']), small(`${id}-local`, '이 기기에만 저장'), small(`${id}-public`, 'GitHub 선택 시 example/hide · 공개 이슈 생성'), screenButton(`${id}-submit`, '추가')], {width: 620, padding: xl, gap: lg, fill: '$--popover', cornerRadius: '$--radius-lg', stroke: '$--border', strokeWidth: hair});
  return {
    boards(s) { return [col(`ov-column-${s}`, 'Five-tab Overview and states', [page(`ov-work-${s}`, '작업 · verified graph', 'work', graph(`ov-work-${s}-content`)), page(`ov-projects-${s}`, '프로젝트 · all projects', 'projects', projects(`ov-projects-${s}-content`)), page(`ov-project-home-${s}`, '프로젝트 · local home', 'projects', projects(`ov-project-home-${s}-content`, true), 'hide'), page(`ov-backlog-${s}`, '백로그 · mixed sources', 'backlog', backlog(`ov-backlog-${s}-content`)), page(`ov-detail-${s}`, '백로그 · wide detail', 'backlog', detail(`ov-detail-${s}-content`)), page(`ov-history-${s}`, '기록 · request search', 'history', history(`ov-history-${s}-content`)), page(`ov-usage-${s}`, '사용량 · account scope', 'usage', usage(`ov-usage-${s}-content`)), page(`ov-unanswered-${s}`, 'Disconnected device', 'work', content(`ov-unanswered-${s}-content`, 'Unavailable', [title(`ov-offline-title-${s}`, 'studio · 연결 안 됨'), small(`ov-offline-why-${s}`, '마지막으로 받은 상태'), screenButton(`ov-offline-retry-${s}`, '다시 연결', {variant: 'outline'})])), quick(`ov-quick-${s}`)], {gap: xxl})]; },
    page(s) { return page(`mn-page-${s}`, 'Overview page · 작업', 'work', graph(`mn-page-${s}-content`)); },
  };
}
