// The design canvas as the generators would write it.
//
// One pipeline serves both entrypoints: gen-pen.mjs writes its output and
// check-pen.mjs compares the file against it. The rename pass runs first (D-21's
// mechanical `--color-X` migration), then tokens are applied so every value agrees
// with design/tokens.json, then Foundations and every System / <Part> sheet are
// redrawn from the result; both compare against exactly the text this function
// returns.

import fs from 'node:fs';
import {read, loadCanvas, apply, renamed, unclaimed, retiredNames} from './pen-tokens.mjs';
import {layout, serialize} from './pen-bands.mjs';
import {updateUsageMasters} from './pen-usage.mjs';
import {updateFocusMasters} from './pen-focus.mjs';
import {foldLegacyMasters} from './pen-system.mjs';

export function generate(root) {
  const {map, tokens, expected} = read(root);
  const {file, document: original} = loadCanvas(root);
  const renamedDocument = renamed(original);
  const orphans = unclaimed(map, renamedDocument, expected);
  const tokened = JSON.parse(apply(renamedDocument, expected, retiredNames(map)));

  // Pen currently renders variable-bound node opacity as invisible. Materialize
  // only the declared bindings, including instance descendant overrides, while
  // keeping the token as authority. Missing targets fail rather than going stale.
  const nodes = new Map();
  function visit(node) {
    nodes.set(node.id, node);
    for (const child of node.children ?? []) visit(child);
  }
  for (const child of tokened.children) visit(child);
  for (const {node, descendant, variable} of map.renderedOpacity ?? []) {
    const owner = nodes.get(node);
    const target = descendant ? owner?.descendants?.[descendant] : owner;
    const token = expected.get(variable);
    if (!target || token?.type !== 'number' || token.value < 0 || token.value > 1) {
      throw new Error(`Invalid rendered opacity binding: ${node}/${descendant ?? ''} -> ${variable}`);
    }
    target.opacity = token.value;
  }

  updateUsageMasters(tokened);
  updateFocusMasters(tokened);
  const legacy = foldLegacyMasters(tokened);
  const {document: laid, unknown} = layout(tokened, tokens, legacy);
  return {map, expected, file, document: tokened, orphans, unknown, before: fs.readFileSync(file, 'utf8'), after: serialize(laid)};
}

// Placement and document order belong to the designer, not the generator.
// A drag never fails a gate or gets undone by regeneration.
export function unplaced(text) {
  const document = JSON.parse(text);
  const children = document.children.map(({x, y, ...node}) => node).sort((a, b) => a.id.localeCompare(b.id));
  return serialize({...document, children});
}

/**
 * Each id that names more than one node in `children`, with where each use
 * sits. A node written whole inside a ref's `descendants` counts: its key
 * already addresses the node it replaces, so an id of its own is a second
 * node by that id, which Pen's loader reports as duplicate ids.
 */
export function duplicateIds(children) {
  const seen = new Map();
  (function walk(node, path) {
    if (Array.isArray(node)) return node.forEach((child, index) => walk(child, `${path}[${index}]`));
    if (!node || typeof node !== 'object') return;
    if (typeof node.id === 'string') seen.set(node.id, [...(seen.get(node.id) ?? []), path]);
    if (node.children) walk(node.children, `${path}/${node.id ?? node.name ?? '?'}`);
    for (const [target, patch] of Object.entries(node.descendants ?? {})) walk(patch, `${path}/${node.id}.descendants.${target}`);
  })(children, '');
  return [...seen].filter(([, paths]) => paths.length > 1);
}
