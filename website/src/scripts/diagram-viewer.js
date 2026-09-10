/** Progressive enhancement: the original SVG link works without JavaScript. */
class DiagramViewer extends HTMLElement {
  initialized = false;
  /** @type {AbortController | undefined} */
  controller;
  connectedCallback() {
    if (this.initialized || typeof HTMLDialogElement === 'undefined') return;
    const dialog = this.querySelector('dialog');
    const opener = this.querySelector('[data-diagram-open]');
    const slider = this.querySelector('[data-diagram-zoom]');
    const output = this.querySelector('[data-diagram-percent]');
    const image = this.querySelector('[data-diagram-full]');
    const viewport = this.querySelector('.rb-diagram-viewport');
    const fit = this.querySelector('[data-diagram-fit]');
    if (!(dialog instanceof HTMLDialogElement) || typeof dialog.showModal !== 'function' ||
        !(opener instanceof HTMLButtonElement) || !(slider instanceof HTMLInputElement) ||
        !(image instanceof HTMLImageElement) || !viewport || !output || !fit) return;
    this.initialized = true;
    this.controller = new AbortController();
    const options = { signal: this.controller.signal };
    const updateZoom = () => {
      image.style.width = `${slider.value}%`;
      image.style.maxWidth = 'none';
      output.textContent = `${slider.value}%`;
    };
    const reset = () => { slider.value = '100'; updateZoom(); viewport.scrollTop = 0; viewport.scrollLeft = 0; };
    opener.hidden = false;
    opener.addEventListener('click', () => { reset(); dialog.showModal(); }, options);
    slider.addEventListener('input', updateZoom, options);
    fit.addEventListener('click', reset, options);
    dialog.addEventListener('close', () => opener.focus(), options);
    // Escape and focus containment are handled by the native modal dialog.
    dialog.addEventListener('click', (event) => {
      if (event.target !== dialog) return;
      const box = dialog.getBoundingClientRect();
      if (event.clientX < box.left || event.clientX > box.right ||
          event.clientY < box.top || event.clientY > box.bottom) dialog.close();
    }, options);
  }
  disconnectedCallback() { this.controller?.abort(); this.initialized = false; }
}
if (!customElements.get('rb-diagram')) customElements.define('rb-diagram', DiagramViewer);
