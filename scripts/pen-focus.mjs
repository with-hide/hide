// Shared input focus and selected-View provenance, preserving existing master IDs.
import {findNode, frame, text} from './pen-system.mjs';

export function updateFocusMasters(document) {
  const required = id => {
    const node = findNode(document, id);
    if (!node) throw new Error(`Missing focus component: ${id}`);
    return node;
  };
  Object.assign(required('ZvLjg'), {stroke: '$--primary', strokeWidth: 2, strokeAlignment: 'inner'});
  required('Z3BnL').fill = '$--color-selected-wash';
  const tree = required('lane-tree-button');
  tree.stroke = [];
  required('lane-tree-button-label').content = 'Explorer';
  required('lane-tree-button-chevron').enabled = false;
  const header = required('lane-header');
  if (!header.children.some(node => node.id === 'lane-header-explorer')) header.children.splice(3, 0,
    {id: 'lane-header-explorer', type: 'ref', ref: 'lane-tree-button', name: 'Explorer', fill: '$--secondary', enabled: false});
  required('lane-header-explorer').enabled = false;
  required('GGXfK-tree').enabled = false;

  if (!document.children.some(node => node.id === 'cmp-view-opener')) document.children.push(frame('cmp-view-opener', 'Component / View opener', {
    x: 12000, y: 0, layout: 'vertical', gap: '$--spacing-md', padding: '$--spacing-xl', fill: '$--card',
  }, [
    text('view-opener-spec', 'Selected View toolbar · tooltip names the opener · press returns to its pane', {size: '$--text-caption', fill: '$--subtle-foreground'}),
    frame('view-opener', 'Component / View opener mark', {reusable: true, width: '$--size-view-opener-mark', height: '$--size-view-opener-mark', cornerRadius: '$--radius-xs', fill: {type: 'image', url: '../web/src/assets/agent-codex.png', mode: 'contain'}}, []),
    {id: 'view-opener-closed', type: 'ref', ref: 'view-opener', name: 'Closed opener · identity retained, jump disabled', opacity: 0.5},
    frame('view-opener-unknown', 'Unknown provider · actual opener name in tooltip', {width: '$--size-view-opener-mark', height: '$--size-view-opener-mark', stroke: '$--muted-foreground', strokeWidth: 1, cornerRadius: '$--radius-xs'}, []),
  ]));
  for (const id of ['GGXfK', 'hxdu7', 'eMlZD']) {
    const toolbar = required(id);
    if (!toolbar.children.some(node => node.id === `${id}-opener`)) toolbar.children.push({id: `${id}-opener`, type: 'ref', ref: 'view-opener', name: 'Selected View opener'});
  }
}
