/* global document */
export async function measure(page) {
  return page.evaluate(() => {
    const root=document.querySelector('[data-workspace-toolbar]');
    if (!root) throw new Error('Checkout header is missing');
    const overflow=[],partProblems=[],rowsFit=[];
    if (root.scrollWidth > root.clientWidth+1) overflow.push('Header scrolls sideways');
    const controls=[...root.children].filter(node=>node.getClientRects().length);
    for (const [i,a] of controls.entries()) for (const b of controls.slice(i+1)) {
      const ra=a.getBoundingClientRect(),rb=b.getBoundingClientRect();
      if (Math.min(ra.right,rb.right)-Math.max(ra.left,rb.left)>1) partProblems.push('Header controls overlap');
    }
    const bounds=root.getBoundingClientRect();
    const buttons=[...root.querySelectorAll('button')];
    for (const [index,node] of buttons.entries()) {
      const box=node.getBoundingClientRect();
      if (box.left < bounds.left-1 || box.right > bounds.right+1 || box.bottom > bounds.bottom+1) overflow.push('Header button leaves its row');
      for (const other of buttons.slice(index+1)) {
        const second=other.getBoundingClientRect();
        if (Math.min(box.right,second.right)-Math.max(box.left,second.left)>1) partProblems.push('Header buttons overlap');
      }
      if (node.hasAttribute('data-checkout-ci') || node.hasAttribute('data-checkout-commit')) {
        for (const child of node.children) {
          const content=child.getBoundingClientRect();
          if (content.left<box.left-1 || content.right>box.right+1) partProblems.push('Commit or CI content leaves its button');
        }
      }
    }
    return {overflow,partProblems,rowsFit};
  });
}
