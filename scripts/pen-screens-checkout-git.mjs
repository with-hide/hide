// WorkspaceTitleBar / CheckoutGitHeader: approved A, synthetic Git facts.
import {frame, text, icon, num} from './pen-system.mjs';

export function checkoutHeader(tokens, {screenButton, screenIconButton}, id, {width = 1220, state = 'rest'} = {}) {
  const muted = '$--subtle-foreground';
  const label = (key, value, props = {}) => text(`${id}-${key}`, value, {size:'$--text-caption',fill:muted,...props});
  const glyph = (key, name) => icon(`${id}-${key}`, name, {size:num(tokens,'--size-icon-sm'),fill:muted});
  const button = (key, value) => screenButton(`${id}-${key}`, value, {variant:'ghost',height:num(tokens,'--size-control-sm')});
  const children = [button('project','herdr-ide'),label('slash','/'),label('branch',state === 'long' ? '한글과-English의-긴-checkout' : 'main',{fill:'$--foreground'})];
  if (state === 'rest' || state === 'long') children.push(glyph('files','file-diff'),label('count','5'),glyph('behind','arrow-down'),label('behind-count','3'));
  if (state === 'conflict') children.push(glyph('conflict','triangle-alert'),label('count','2'));
  if (['loading','unavailable','stale'].includes(state)) children.push(glyph('state',state === 'loading' ? 'loader-circle' : state === 'stale' ? 'clock' : 'triangle-alert'),label('unknown','?'));
  children.push(frame(`${id}-space`,'Window drag space',{width:'fill_container',height:1},[]));
  if (!['unborn','home','loading','unavailable'].includes(state)) {
    children.push(glyph('commit','git-commit-horizontal'));
    if (width > 720) children.push(label('subject','탭 전환 시 포커스 유지'),label('age','12분 전',{fill:'$--muted-foreground'}));
    children.push(glyph('ci',state === 'failed' ? 'x' : state === 'pending' ? 'loader-circle' : state === 'none' ? 'circle-minus' : state === 'unknown' ? 'circle-help' : state === 'stale' ? 'clock' : 'check'),label('ci-label','CI'));
  }
  children.push(screenIconButton(`${id}-overview`,'layout-dashboard',{size:24}),screenIconButton(`${id}-server`,'globe',{size:24}));
  return frame(id,`Checkout Git / ${state}`,{width,height:num(tokens,'--size-tab-strip'),layout:'horizontal',alignItems:'center',gap:'$--spacing-xs',padding:[0,'$--spacing-sm'],fill:'$--sidebar',stroke:'$--border',strokeWidth:{bottom:'$--size-hairline'},clip:true},children);
}

export function checkoutHeaderBoards(tokens, parts, suffix) {
  const theme = suffix === 'l' ? 'light' : 'dark';
  return [checkoutHeader(tokens,parts,`checkout-git-${theme}`), ...['clean','conflict','loading','unavailable','stale','pending','failed','none','unknown','unborn','home','long'].map(state => checkoutHeader(tokens,parts,`checkout-git-${theme}-${state}`,{state,width:state === 'long' ? 720 : 1220}))];
}
