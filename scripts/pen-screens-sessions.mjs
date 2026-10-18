// The pane header (PRD agent-hierarchy-screens D-14 to D-24, D-37 to D-43;
// pane-header-progress D-01 to D-09). The Sessions panel it once sat beside is
// retired: a device's tasks are the sidebar's 태스크 list (PRD
// sidebar-task-lens D-16), drawn in the Task lens sheets. Controls are refs
// of the existing System masters supplied by pen-screens, and the verb, descendant
// mark, tree button and PR mark are refs of Component / Agent tree parts.
import {AGENT_TREE_MASTERS, PR_STATES} from './pen-agent-tree.mjs';
import {legacyMark, markSlot} from './pen-system.mjs';
export function sessionScreens(tokens, {frame, text, icon, themedXref, screenButton}) {
  const caption = (id, value, fill = '$--muted-foreground', extra = {}) => text(id, value, {size:'$--text-caption', fill, ...extra});
  const verb = (id, word) => themedXref(id, 'ath-verb', word, {}, {'ath-verb-t': {content: word}});
  const provider = (id, kind = 'claude') => frame(id, 'Provider', {width:14,height:14,fill:{type:'image',enabled:true,url:`../web/src/assets/agent-${kind}.png`,mode:'contain'}}, []);
  // The PR mark (web/src/components/pr-mark.tsx), a ref of Component / Agent tree parts: the state's icon and colour and `#number` for one PR, and for several, an agent's own or a root's family, each state's icon and count in its colour, worst first.
  const prPart = (id, state, content, tinted) => {
    const [, glyph, fill] = PR_STATES.find(([key]) => key === state);
    return themedXref(id, AGENT_TREE_MASTERS.prIcon, tinted ? `PR ${content} ${state}` : `PR ${content}`, {}, {'ath-pr-g': {icon: glyph, fill}, 'ath-pr-n': {content, ...(tinted ? {fill} : {})}});
  };
  const prChip = (id, {number, state, counts}) => counts
    ? frame(id, `PR ${counts.reduce((sum, [, count]) => sum + count, 0)}`, {layout:'horizontal', gap:'$--spacing-xs', alignItems:'center', width:'fit_content'}, counts.map(([key, count]) => prPart(`${id}-${key}`, key, String(count), true)))
    : prPart(id, state, `#${number}`, false);

  // A pane: its header (the ancestor path, the marks and title, the fork mark of
  // a pane Fork agent made, which is a root and has no path, the own PR chip,
  // then menu, zoom and close) over its terminal. The second-row overlay
  // starts with one direct-child tree button, then a request or own progress.
  // with the one band a pane carries overlaid at the top (D-14, D-43; B20, B21).
  // The task chip (web/src/PaneTaskChip.tsx, PRD task-split-checklist D-05):
  // `<done>/<tasks>` from one task, before the PR chip; child progress never
  // rides it. Open draws the ring the pinned or hovered chip carries.
  const taskChip = (id, tasks, open = false) => frame(id, `Tasks ${tasks}`, {height:18,layout:'horizontal',gap:'$--spacing-xxs',padding:[0,'$--spacing-xs'],alignItems:'center',cornerRadius:'$--radius-xs',fill:'$--background',stroke:open ? '$--ring' : '$--border',strokeWidth:1,strokeAlignment:'inner'}, [
    icon(`${id}-g`, 'list-checks', {size:12,fill:open ? '$--foreground' : '$--muted-foreground'}),
    text(`${id}-n`, tasks, {size:'$--text-micro',fill:open ? '$--foreground' : '$--muted-foreground',mono:true}),
  ]);
  function pane(id, {path = [], mark = '○', tone = '$--agent-working', provider: kind = 'codex', title, forked, tasks, pr, children, progress, summary, band, terminal}) {
    const line = progress ?? summary;
    const tree = children ? [themedXref(`${id}-tree`, 'ath-tree-button', 'Child states', {fill:'$--background'}, {
      'ath-tb-count':{content:String(children)},
    })] : [];
    const header = frame(`${id}-header`, 'Header', {width:'fill_container',height:28,layout:'horizontal',gap:6,alignItems:'center',padding:[0,8],fill:'$--card'}, [
      ...path.flatMap((name, index) => [caption(`${id}-path-${index}`, name, '$--muted-foreground'), caption(`${id}-sep-${index}`, '›', '$--muted-foreground')]),
      markSlot(`${id}-mark`, legacyMark(mark, tone)),
      provider(`${id}-provider`, kind),
      caption(`${id}-title`, title, '$--foreground'),
      ...(forked ? [icon(`${id}-fork`, 'git-fork', {size:12,fill:'$--subtle-foreground'})] : []),
      ...(tasks ? [taskChip(`${id}-tasks`, tasks.count, tasks.open)] : []),
      ...(pr ? [prChip(`${id}-pr`, pr)] : []),
      frame(`${id}-sp`, 'Spacer', {width:'fill_container',height:1}, []),
      icon(`${id}-zoom`, 'maximize-2', {size:12,fill:'$--subtle-foreground'}),
      icon(`${id}-menu`, 'ellipsis', {size:14,fill:'$--subtle-foreground'}),
      icon(`${id}-close`, 'x', {size:14,fill:'$--subtle-foreground'}),
    ]);
    const bandRow = band ? [frame(`${id}-band`, 'Ask band', {width:'fill_container',height:28,layout:'horizontal',gap:6,alignItems:'center',padding:[0,8],fill:'$--secondary',stroke:'$--warning',strokeWidth:{left:2},strokeAlignment:'inner'}, [
      ...tree,
      verb(`${id}-band-verb`, band.verb),
      ...(band.who ? [caption(`${id}-band-who`, `${band.who[1]} (자식)`, '$--muted-foreground')] : []),
      caption(`${id}-band-what`, band.what, '$--foreground'),
      ...(band.unreceived ? [caption(`${id}-band-unreceived`, band.unreceived, '$--muted-foreground')] : [caption(`${id}-band-age`, band.age ?? '12m', '$--muted-foreground', {mono:true})]),
      ...(!children && band.more ? [caption(`${id}-band-more`, `외 ${band.more}건`, '$--muted-foreground')] : []),
      frame(`${id}-band-sp`, 'Spacer', {width:'fill_container',height:1}, []),
      screenButton(`${id}-band-open`, '열기', {variant:'outline', height:20}),
    ])] : children || line ? [frame(`${id}-children`, 'Progress overlay', {width:'fill_container',height:28,layout:'horizontal',gap:'$--spacing-xs',alignItems:'center',padding:[0,'$--spacing-sm'],fill:'$--secondary'},[
      ...tree,...(line ? [caption(`${id}-progress`,line,'$--muted-foreground',{width:'fill_container'})] : []),
    ])] : [];
    return frame(id, 'Pane', {width:'fill_container',height:120,layout:'vertical',gap:0,clip:true,fill:'$--background',stroke:'$--border',strokeWidth:1,strokeAlignment:'inner'}, [
      header, ...bandRow,
      frame(`${id}-terminal`, 'Terminal', {width:'fill_container',height:'fill_container',padding:[8,12]}, [caption(`${id}-terminal-text`, terminal ?? '› 테스트 작성 결과를 기다리는 중', '$--muted-foreground', {mono:true})]),
    ]);
  }
  function headerBoard(suffix) {
    const id=`sfu-ph-board-${suffix}`;
    const labeled = (key, label, children) => frame(`${id}-sec-${key}`, label, {width:'fill_container',layout:'vertical',gap:8}, [caption(`${id}-sec-${key}-label`, label, '$--subtle-foreground', {weight:'600'}), ...children]);
    return frame(id,'Pane header states',{width:840,layout:'vertical',gap:20,padding:[0,0,24,0],fill:'$--background'},[
      labeled('progress','01 · 진행 중',[pane(`${id}-progress`,{title:'Group tab 기능 설계',children:3,progress:'상황별 헤더 시안을 정리하고 있어요'})]),
      labeled('question','02 · 답변이 필요함',[pane(`${id}-question`,{title:'Group tab 기능 설계',children:3,band:{verb:'질문',what:'그룹 이름은 직접 정하게 할까요?',age:'2분'}})]),
      labeled('approval','03 · 승인이 필요함',[pane(`${id}-approval`,{title:'Group tab 기능 설계',children:3,band:{verb:'승인',what:'권한 승인을 기다리고 있어요',age:'1분'}})]),
      labeled('blocked','04 · 막힘',[pane(`${id}-blocked`,{title:'Group tab 기능 설계',children:3,band:{verb:'막힘',what:'접근 권한을 확인해 주세요',age:'4분'}})]),
      labeled('raised','05 · 자식에게 사용자 승인이 필요함',[pane(`${id}-raised`,{title:'Group tab 기능 설계',children:3,band:{verb:'승인',what:'권한 승인을 기다리고 있어요',who:['codex','UI 검증'],age:'3분'}})]),
      labeled('stopped','06 · 현재 세션이 멈춤',[pane(`${id}-stopped`,{title:'Group tab 기능 설계',children:3,band:{verb:'멈춤',what:'응답이 중단됐어요',age:'1분'}})]),
    ]);
  }
  function edgeBoard(suffix) {
    const id=`sfu-ph-edge-${suffix}`;
    const labeled = (key, label, children) => frame(`${id}-sec-${key}`, label, {width:'fill_container',layout:'vertical',gap:8}, [caption(`${id}-sec-${key}-label`, label, '$--subtle-foreground', {weight:'600'}), ...children]);
    const popover = frame(`${id}-popover`, 'Child tree popover', {width:280,layout:'vertical',gap:6,padding:10,fill:'$--popover',stroke:'$--border',strokeWidth:1,cornerRadius:'$--radius-md'}, [
      caption(`${id}-popover-title`,'Group tab 기능 설계','$--foreground',{weight:'600'}),
      caption(`${id}-popover-a`,'›  레이아웃 설계 · 상태별 헤더 시안 정리 중','$--foreground'),
      caption(`${id}-popover-b`,'›  구현 · 진행 문구 연결 중','$--foreground'),
      caption(`${id}-popover-c`,'›  검증 · 검토 결과 도착','$--muted-foreground'),
    ]);
    return frame(id,'Pane header edge states',{width:840,layout:'vertical',gap:20,padding:[0,0,24,0],clip:true,fill:'$--background'},[
      labeled('solo','07 · 자식 없이 진행 중',[pane(`${id}-solo`,{title:'Group tab 기능 설계',progress:'상황별 헤더 시안을 정리하고 있어요'})]),
      labeled('children','08 · progress 없음 · 자식 있음',[pane(`${id}-children`,{title:'Group tab 기능 설계',children:3})]),
      labeled('empty','09 · progress도 자식도 없음',[pane(`${id}-empty`,{title:'Group tab 기능 설계'})]),
      labeled('finished','10 · 작업 완료 · 자식 있음',[pane(`${id}-finished`,{mark:'✓',tone:'$--success',title:'Group tab 기능 설계',children:3})]),
      labeled('popover','11 · 자식 목록 열기',[pane(`${id}-open`,{title:'Group tab 기능 설계',children:3,progress:'검토 결과를 정리하고 있어요'}),popover]),
      labeled('narrow','12 · 좁은 패널 · 긴 progress',[frame(`${id}-narrow-wrap`,'Narrow pane',{width:440,layout:'vertical'},[pane(`${id}-narrow`,{title:'Group tab 기능 설계',children:12,progress:'헤더 상태별 표시와 요청 우선순위를 검토하고 있어요'})])]),
    ]);
  }
  // The task chip and its checklist (PRD task-split-checklist B9-B14): a chip
  // from one task, none without a task or with Agent summaries off, and the
  // list the pointer, the keyboard's focus or ⇧⌘U opens under it.
  function taskBoard(suffix) {
    const id=`tsc-ph-board-${suffix}`;
    const labeled = (key, label, children) => frame(`${id}-sec-${key}`, label, {width:'fill_container',layout:'vertical',gap:8}, [caption(`${id}-sec-${key}-label`, label, '$--subtle-foreground', {weight:'600'}), ...children]);
    // One request's tasks share one state (B5): four in review, the request's PR mark on its
    // first task and `리뷰 대기` in words on the rest; every task is a checkbox (PRD
    // record-card-readability D-05, B7, B8), `!` in front while the session asks.
    const box = (key, done) => done
      ? frame(key, 'Checkbox checked', {width:12,height:12,cornerRadius:2,fill:'$--success',layout:'horizontal',justifyContent:'center',alignItems:'center'}, [icon(`${key}-g`, 'check', {size:9,fill:'$--background'})])
      : frame(key, 'Checkbox', {width:12,height:12,cornerRadius:2,stroke:'$--muted-foreground',strokeWidth:1,strokeAlignment:'inner'}, []);
    const item = (key, state, name, {pr, word} = {}) => frame(`${id}-pop-${key}`, name, {width:'fill_container',layout:'horizontal',gap:'$--spacing-xs',alignItems:'center',padding:[2,0]}, [
      box(`${id}-pop-${key}-g`, state === 'done'),
      caption(`${id}-pop-${key}-n`, name, state === 'done' ? '$--subtle-foreground' : '$--foreground', {width:'fill_container'}),
      ...(word ? [text(`${id}-pop-${key}-w`, word, {size:'$--text-micro',fill:'$--muted-foreground'})] : []),
      ...(pr ? [prChip(`${id}-pop-${key}-pr`, pr)] : []),
    ]);
    // This pane's own 지금 box, the record's state box part: edged in the state's colour, the line quiet.
    const now = frame(`${id}-pop-now`, '지금', {width:'fill_container',layout:'horizontal',gap:'$--spacing-xs',alignItems:'start',padding:['$--spacing-xs','$--spacing-sm'],cornerRadius:'$--radius-md',fill:'$--card',stroke:'$--agent-working',strokeWidth:1,strokeAlignment:'inner'}, [
      caption(`${id}-pop-now-w`, '지금 · 작업 중', '$--foreground', {weight:'600'}),
      caption(`${id}-pop-now-t`, '파일 페이지 댓글 PR 검증 중', '$--muted-foreground', {width:'fill_container'}),
    ]);
    const list = frame(`${id}-pop`, 'Task list', {width:340,layout:'vertical',gap:0,fill:'$--popover',stroke:'$--border',strokeWidth:1,strokeAlignment:'inner',cornerRadius:'$--radius-md'}, [
      frame(`${id}-pop-body`, 'Tasks', {width:'fill_container',layout:'vertical',gap:2,padding:['$--spacing-sm','$--spacing-sm',0,'$--spacing-sm']}, [
        frame(`${id}-pop-h`, 'Heading', {width:'fill_container',layout:'horizontal',gap:'$--spacing-xs',alignItems:'end',padding:[0,0,4,0]}, [text(`${id}-pop-h-t`, '시킨 일', {size:'$--text-micro',fill:'$--muted-foreground',weight:'500'}), caption(`${id}-pop-h-n`, '0/4', '$--foreground', {weight:'600'})]),
        item('t0', 'review', 'README의 HTML 태그가 보이게 하기', {pr:{number:1233,state:'mergeable'}}),
        item('t1', 'review', '댓글 + 마크 위치 맞추기', {word:'리뷰 대기'}),
        item('t2', 'review', 'HTML 페이지에서 댓글이 동작하게 하기', {word:'리뷰 대기'}),
        item('t3', 'review', '드래그한 영역 남겨두기', {word:'리뷰 대기'}),
        frame(`${id}-pop-now-gap`, 'Gap', {width:'fill_container',height:6}, []),
        now,
        frame(`${id}-pop-kids`, '맡긴 일', {width:'fill_container',layout:'horizontal',gap:'$--spacing-xs',alignItems:'end',padding:[8,0,0,0]}, [text(`${id}-pop-kids-t`, '맡긴 일', {size:'$--text-micro',fill:'$--muted-foreground',weight:'500'}), caption(`${id}-pop-kids-n`, '5', '$--foreground', {weight:'600'}), text(`${id}-pop-kids-d`, '· 끝남 4', {size:'$--text-micro',fill:'$--muted-foreground'})]),
      ]),
      frame(`${id}-pop-foot`, 'Open record', {width:'fill_container',layout:'horizontal',alignItems:'center',padding:['$--spacing-xs','$--spacing-sm'],stroke:'$--border',strokeWidth:{top:1},strokeAlignment:'inner'}, [
        caption(`${id}-pop-open`, '기록 열기', '$--foreground', {width:'fill_container'}),
        frame(`${id}-pop-kbd`, 'Shortcut', {height:18,layout:'horizontal',alignItems:'center',padding:[0,5],cornerRadius:'$--radius-xs',stroke:'$--border',strokeWidth:1,strokeAlignment:'inner'}, [text(`${id}-pop-kbd-t`, '⇧⌘U', {size:'$--text-micro',fill:'$--subtle-foreground'})]),
      ]),
    ]);
    return frame(id,'Pane task chip states',{width:761,layout:'vertical',gap:20,padding:[0,0,24,0],fill:'$--background'},[
      labeled('one','One task: the chip alone, done over tasks',[pane(`${id}-one`,{provider:'codex',title:'README 번역 정리',tasks:{count:'0/1'}})]),
      labeled('split','A request split into four, with a PR and children: the chip counts tasks only; the children keep their own row',[pane(`${id}-split`,{mark:'●',provider:'claude',title:'Pre-launch bugfix pass for File views',tasks:{count:'0/4'},pr:{number:1233,state:'mergeable'},children:5,counts:{working:1,result:4},summary:'파일 페이지 댓글 기능 구현 · 작업 중'})]),
      labeled('none','No task, or Agent summaries off: no chip',[pane(`${id}-none`,{title:'아침 점검 위임',children:2,counts:{working:2},summary:'두 자식이 점검하는 중'})]),
      labeled('open','The task box under the chip: checkbox tasks with the request\'s PR once, this pane\'s 지금 box, 맡긴 일 N · 끝남 k and 기록 열기 (⇧⌘U pins it, Esc closes a pinned list)',[
        pane(`${id}-open`,{mark:'●',provider:'claude',title:'Pre-launch bugfix pass for File views',tasks:{count:'0/4',open:true},pr:{number:1233,state:'mergeable'},terminal:'● 머지 가능한 건 바로 머지했어요.'}),
        list,
      ]),
    ]);
  }
  return {headerBoard,edgeBoard,taskBoard};
}
