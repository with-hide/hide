// One expression the measurement harness (run.sh) evaluates in the page on
// each poll until the fixture checkout is in front: true once it is. It opens
// the Projects sidebar, the folds that hide the fixture's project and checkout
// rows at a scale, and the checkout itself, each once, because a fold is the
// core's state and a second click closes it.
(() => {
  if (document.querySelector('[data-workspace-screen]')) return true;
  const opened = (window.__measureOpened ??= new Set());
  const once = (key, element) => { if (!element || opened.has(key)) return false; opened.add(key); element.click(); return true; };
  const projects = document.querySelector('[data-sidebar-mode="projects"]');
  if (projects && projects.getAttribute('aria-selected') !== 'true') { projects.click(); return false; }
  const checkout = document.querySelector('[data-checkout-kind="branch"]:not([disabled])');
  if (checkout) { once('checkout', checkout); return false; }
  // The fixture's project is the one a scale did not add: fixture.mjs (and scale.sh) label each of their own project-N.
  const project = [...document.querySelectorAll('[data-project]')].find((row) => !/^project-\d+$/.test(row.querySelector('[data-row-name]')?.textContent?.trim() ?? ''));
  if (!project) {
    once('no-agents', document.querySelector('[data-no-agent-projects][aria-expanded="false"]')) || once('inactive-projects', document.querySelector('[data-inactive-projects][aria-expanded="false"]'));
    return false;
  }
  for (const fold of ['data-project-toggle', 'data-empty-worktrees', 'data-inactive-checkouts']) {
    if (once(fold, project.querySelector(`[${fold}][aria-expanded="false"]`))) return false;
  }
  return false;
})()
