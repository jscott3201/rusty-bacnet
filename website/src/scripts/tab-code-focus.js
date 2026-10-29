// Expressive Code drops a code block's tabindex while the block is hidden (its
// width is 0, so it doesn't overflow) and doesn't always restore it when a tab
// panel shows the block again, which leaves an overflowing block in a newly
// selected tab unreachable by keyboard. When a panel is shown, give each of its
// overflowing blocks the keyboard access the build gave it (ec.config.mjs).
function restore(panel) {
  for (const block of panel.querySelectorAll('.expressive-code pre')) {
    if (block.scrollWidth > block.clientWidth && !block.hasAttribute('tabindex')) {
      block.setAttribute('tabindex', '0');
      block.setAttribute('role', 'region');
    }
  }
}

const shown = new MutationObserver(records => {
  for (const { target } of records) {
    if (target instanceof HTMLElement && target.getAttribute('role') === 'tabpanel' && !target.hidden) {
      requestAnimationFrame(() => restore(target));
    }
  }
});
for (const panel of document.querySelectorAll('[role="tabpanel"]')) {
  shown.observe(panel, { attributes: true, attributeFilter: ['hidden'] });
}
