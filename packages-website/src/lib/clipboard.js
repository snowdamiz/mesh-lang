/** @typedef {'idle' | 'copied' | 'failed'} CopyState */

/**
 * Copies `text` and resolves to whether it worked. Falls back to a selected
 * textarea where the Clipboard API is missing or refused, as on plain-HTTP
 * origins and in older browsers.
 * @param {string} text
 */
export async function copyText(text) {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return copyBySelection(text);
  }
}

/** @param {string} text */
function copyBySelection(text) {
  const previous = /** @type {HTMLElement | null} */ (document.activeElement);
  const area = document.createElement('textarea');
  area.value = text;
  area.setAttribute('readonly', '');
  area.style.cssText = 'position: fixed; top: 0; left: 0; opacity: 0';
  document.body.append(area);
  area.focus({ preventScroll: true });
  area.select();
  try {
    return document.execCommand('copy');
  } catch {
    return false;
  } finally {
    area.remove();
    previous?.focus({ preventScroll: true });
  }
}

/**
 * Feedback for one copy control: `show` gets the outcome of each copy, then
 * 'idle' once `ms` has passed. Call `stop` when the control goes away; a copy
 * still in flight then shows nothing.
 * @param {(state: CopyState) => void} show
 */
export function copyFeedback(show, ms = 1600) {
  let timer;
  let stopped = false;
  return {
    /** @param {string} text */
    async copy(text) {
      const state = (await copyText(text)) ? 'copied' : 'failed';
      if (stopped) return;
      clearTimeout(timer);
      show(state);
      timer = setTimeout(() => show('idle'), ms);
    },
    stop() {
      stopped = true;
      clearTimeout(timer);
    },
  };
}

const CODE_COPY_LABEL = { idle: 'Copy code', copied: 'Copied', failed: 'Copy failed' };

/**
 * Action for a server-rendered README, given its HTML: wires the code-block copy buttons by delegation.
 * Each render's controllers are stopped when the HTML is replaced, as moving to another package does,
 * so a copy in flight on a dropped button shows nothing.
 * @param {HTMLElement} node
 * @param {string} _html
 */
export function copyCodeBlocks(node, _html) {
  /** @type {Map<HTMLElement, ReturnType<typeof copyFeedback>>} */
  let active = new Map();
  const stopAll = () => {
    for (const feedback of active.values()) feedback.stop();
    active = new Map();
  };
  const onClick = (event) => {
    const button = event.target.closest('[data-copy-code]');
    if (!button) return;
    if (!active.has(button)) {
      const show = (state) => {
        button.dataset.state = state;
        button.setAttribute('aria-label', CODE_COPY_LABEL[state]);
      };
      active.set(button, copyFeedback(show));
    }
    active.get(button).copy(button.parentElement.querySelector('code').textContent);
  };
  node.addEventListener('click', onClick);
  return {
    update: stopAll,
    destroy() {
      node.removeEventListener('click', onClick);
      stopAll();
    },
  };
}
