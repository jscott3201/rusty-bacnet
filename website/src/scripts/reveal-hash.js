// A search result may link to a heading inside an inactive native Starlight tab.
// Activate its ancestor tabs through their existing controls; do not replace Tabs.
async function revealHashTarget() {
  if (!document.querySelector('starlight-tabs')) return;
  await customElements.whenDefined('starlight-tabs');
  let id;
  try { id = decodeURIComponent(location.hash.slice(1)); } catch { return; }
  const target = document.getElementById(id);
  if (!target) return;
  const panels = [];
  for (let node = target; node; node = node.parentElement) {
    if (node.getAttribute('role') === 'tabpanel') panels.unshift(node);
  }
  let revealed = false;
  for (const panel of panels) {
    if (!panel.hidden) continue;
    const control = document.getElementById(panel.getAttribute('aria-labelledby') || '');
    if (control instanceof HTMLElement) { control.click(); revealed = true; }
  }
  if (revealed) {
    // Native tab activation focuses its control. Move reading/keyboard focus
    // to the revealed destination instead of leaving it above the scrolled view.
    if (!target.hasAttribute('tabindex')) target.setAttribute('tabindex', '-1');
    target.focus({ preventScroll: true });
    target.scrollIntoView({ block: 'start' });
  }
}
window.addEventListener('hashchange', revealHashTarget);
window.addEventListener('load', revealHashTarget);
if (document.readyState === 'complete') revealHashTarget();
