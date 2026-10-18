// The pane header (PRD agent-hierarchy-screens D-14 to D-24, D-37 to D-43;
// the approved Pen round 2, D-44). The Sessions panel it once sat beside is
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
  // then menu, zoom and close) over its terminal. B2 moves the child trigger
  // into the same second-row overlay as the core band.
  // with the one band a pane carries overlaid at the top (D-14, D-43; B20, B21).
  // The task chip (web/src/PaneTaskChip.tsx, PRD task-split-checklist D-05):
  // `<done>/<tasks>` from one task, before the PR chip; child progress never
  // rides it. Open draws the ring the pinned or hovered chip carries.
  const taskChip = (id, tasks, open = false) => frame(id, `Tasks ${tasks}`, {height:18,layout:'horizontal',gap:'$--spacing-xxs',padding:[0,'$--spacing-xs'],alignItems:'center',cornerRadius:'$--radius-xs',fill:'$--background',stroke:open ? '$--ring' : '$--border',strokeWidth:1,strokeAlignment:'inner'}, [
    icon(`${id}-g`, 'list-checks', {size:12,fill:open ? '$--foreground' : '$--muted-foreground'}),
    text(`${id}-n`, tasks, {size:'$--text-micro',fill:open ? '$--foreground' : '$--muted-foreground',mono:true}),
  ]);
  function pane(id, {path = [], mark = '○', tone = '$--agent-working', provider: kind = 'codex', title, forked, tasks, pr, children, counts = {}, disconnected = false, summary = '작업 상태 칩과 팝업을 연결하는 중', band, terminal}) {
    // Only the counts a row mark stands for are drawn; a disconnected device's children draw none (status-marks B16, B21).
    const parts = disconnected ? [] : (band ? ['raised'] : ['raised', 'working', 'result']).filter(key=>counts[key]);
    const tree = children ? [themedXref(`${id}-tree`, 'ath-tree-button', 'Child states', {fill:'$--background'}, {
      'ath-tb-tree':{enabled:parts.length===0},'ath-tb-counts':{enabled:parts.length>0},
      ...Object.fromEntries(parts.flatMap(key=>[[`ath-tb-${key}`,{enabled:true}],[`ath-tb-${key}-n`,{content:String(counts[key])}]])),
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
      caption(`${id}-band-what`, band.what, '$--foreground'),
      ...(band.who ? [caption(`${id}-band-who`, band.who[1], '$--muted-foreground')] : []),
      ...(band.unreceived ? [caption(`${id}-band-unreceived`, band.unreceived, '$--muted-foreground')] : [caption(`${id}-band-age`, band.age ?? '12m', '$--muted-foreground', {mono:true})]),
      ...(!children && band.more ? [caption(`${id}-band-more`, `외 ${band.more}건`, '$--muted-foreground')] : []),
      frame(`${id}-band-sp`, 'Spacer', {width:'fill_container',height:1}, []),
      screenButton(`${id}-band-open`, '열기', {variant:'outline', height:20}),
    ])] : children ? [frame(`${id}-children`, 'Child status overlay', {width:'fill_container',height:28,layout:'horizontal',gap:'$--spacing-xs',alignItems:'center',padding:[0,'$--spacing-sm'],fill:'$--secondary'},[
      ...tree,caption(`${id}-child-summary`,summary,'$--muted-foreground',{width:'fill_container'}),
    ])] : [];
    return frame(id, 'Pane', {width:'fill_container',height:120,layout:'vertical',gap:0,clip:true,fill:'$--background',stroke:'$--border',strokeWidth:1,strokeAlignment:'inner'}, [
      header, ...bandRow,
      frame(`${id}-terminal`, 'Terminal', {width:'fill_container',height:'fill_container',padding:[8,12]}, [caption(`${id}-terminal-text`, terminal ?? '› 테스트 작성 결과를 기다리는 중', '$--muted-foreground', {mono:true})]),
    ]);
  }
  function headerBoard(suffix) {
    const id=`sfu-ph-board-${suffix}`;
    const root='hide 에이전트 지원 PR 묶…';
    const bands=[
      ['approval',{verb:'승인',what:'권한 요청에서 멈춤',who:['codex','테스트 작성'],age:'12m'}],
      ['answer',{verb:'질문',what:'PR을 하나로 합칠까요, 나눌까요?',who:['claude','P7 Pi·omp 재우기'],unreceived:'hide 에이전트 지원 PR… 60분째 못 받음'}],
      ['confirm',{verb:'확인',what:'리뷰 지적 3건 반영하고 멈춤',who:['claude','사전 리뷰'],age:'1h'}],
      ['draft',{verb:'초안',what:'내 입력 초안 때문에 질문이 못 감',who:['codex','P8 읽기 전용 준비'],age:'8m'}],
    ];
    // One column at the approved board's width, each pane under the line that says what it shows.
    const labeled = (key, label, children) => frame(`${id}-sec-${key}`, label, {width:'fill_container',layout:'vertical',gap:8}, [caption(`${id}-sec-${key}-label`, label, '$--subtle-foreground', {weight:'600'}), ...children]);
    return frame(id,'Pane header states',{width:761,layout:'vertical',gap:20,padding:[0,0,24,0],fill:'$--background'},[
      labeled('root','B2 raised ask: ! count replaces 외 N건, with no duplicate total',[pane(`${id}-root`,{mark:'!',tone:'$--warning',provider:'claude',title:'hide 에이전트 지원 PR 묶음 머지 조율',pr:{number:812,state:'mergeable'},children:8,counts:{raised:2},band:{verb:'승인',what:'e2e 테스트 돌리던 중',who:['codex','테스트 작성'],age:'12m',more:1},terminal:'› 에이전트 지원 PR 다섯 개의 머지 순서를 맞추는 중'})]),
      labeled('child','B2 child pane: own PR on the identity row, child states and representative line on the second',[pane(`${id}-child`,{path:[root],title:'P8 읽기 전용 준비',pr:{number:811,state:'failed'},children:5,counts:{working:1,result:1,waiting:1,unknown:1,idle:1},summary:'기존 PR에 합칠지 부모 응답을 기다리는 중'})]),
      labeled('unknown','Unknown child: a tilde, never a resting ring',[pane(`${id}-unknown`,{title:'자식 상태 확인',children:1,counts:{unknown:1},summary:'상태 확인 중인 자식'})]),
      labeled('disconnected','Unavailable states: topology access with a connection reason',[pane(`${id}-disconnected`,{title:'다른 기기의 자식 상태',children:5,disconnected:true,summary:'Mac mini 연결되지 않음'})]),
      labeled('grand','Grandchild pane: two ancestors, no PR and no children (B20)',[pane(`${id}-grand`,{path:[root,'P8 읽기 전용 준비'],mark:'!',tone:'$--warning',title:'테스트 작성',terminal:'Allow bash: pnpm --dir web e2e ? (y/n)'})]),
      labeled('fork','Forked pane: a root of its own, the fork mark after the title opens the source it was forked from (fork-operator-root B8)',[pane(`${id}-fork`,{provider:'claude',title:'P8 읽기 전용 준비',forked:true})]),
      labeled('two','Two own PRs: one mark per state (B22)',[pane(`${id}-two`,{path:[root],mark:'✓',tone:'$--success',provider:'claude',title:'P6 OpenCode 세션 리더',pr:{counts:[['failed',1],['merged',1]]}})]),
      labeled('bands','Band per cause: child counts compact to preserve the request (B3)',bands.map(([key,band])=>pane(`${id}-band-${key}`,{path:[root],title:'P8 읽기 전용 준비',children:2,band}))),
      caption(`${id}-rule`,'승인은 label 줄이 없으면 "권한 요청에서 멈춤". 답변의 대기는 부모가 못 받은 시간. 초안은 초안이 걸린 부모를 연다. 띠는 PTY 크기를 바꾸지 않는다.'),
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
  return {headerBoard,taskBoard};
}
