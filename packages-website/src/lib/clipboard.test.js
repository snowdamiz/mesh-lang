import { afterEach, beforeEach, mock, test } from 'node:test';
import assert from 'node:assert/strict';
import { copyCodeBlocks, copyFeedback } from './clipboard.js';

let settleCopy;

beforeEach(() => {
  mock.timers.enable({ apis: ['setTimeout'] });
  Object.defineProperty(globalThis, 'navigator', {
    configurable: true,
    value: { clipboard: { writeText: () => new Promise((resolve) => (settleCopy = resolve)) } },
  });
});

afterEach(() => mock.timers.reset());

test('a copy shows its outcome, then goes back to idle', async () => {
  const shown = [];
  const feedback = copyFeedback((state) => shown.push(state), 1600);
  const copying = feedback.copy('meshpkg publish');
  settleCopy();
  await copying;
  assert.deepEqual(shown, ['copied']);
  mock.timers.tick(1600);
  assert.deepEqual(shown, ['copied', 'idle']);
});

test('a copy that settles after stop shows nothing and starts no timer', async () => {
  const shown = [];
  const feedback = copyFeedback((state) => shown.push(state), 1600);
  const copying = feedback.copy('meshpkg publish');
  feedback.stop();
  settleCopy();
  await copying;
  mock.timers.tick(60_000);
  assert.deepEqual(shown, []);
});

const settled = () => new Promise((resolve) => setImmediate(resolve));

// A server-rendered README article: `swap` replaces its HTML the way moving to another package does.
function readmeArticle() {
  const listeners = new Map();
  const article = {
    buttons: [],
    addEventListener: (type, listener) => listeners.set(type, listener),
    removeEventListener: (type) => listeners.delete(type),
    querySelectorAll: () => article.buttons,
  };
  const codeBlock = () => {
    const shown = [];
    const button = {
      dataset: { state: 'idle' },
      setAttribute: (name, value) => shown.push(`${name}=${value}`),
      closest: () => button,
      parentElement: { querySelector: () => ({ textContent: 'meshpkg install snowdamiz/mesh-registry-proof' }) },
    };
    article.buttons = [button];
    return { click: () => listeners.get('click')({ target: button }), shown };
  };
  return { article, codeBlock };
}

test('a README code-block copy shows its outcome on its button, then goes back to idle', async () => {
  const { article, codeBlock } = readmeArticle();
  copyCodeBlocks(article, '<pre>first</pre>');
  const block = codeBlock();
  block.click();
  settleCopy();
  await settled();
  assert.deepEqual(block.shown, ['aria-label=Copied']);
  mock.timers.tick(1600);
  assert.deepEqual(block.shown, ['aria-label=Copied', 'aria-label=Copy code']);
});

test('a README copy still pending when the package changes shows nothing on the old button', async () => {
  const { article, codeBlock } = readmeArticle();
  const action = copyCodeBlocks(article, '<pre>first</pre>');
  const old = codeBlock();
  old.click();
  codeBlock();
  action.update?.('<pre>second</pre>');
  settleCopy();
  await settled();
  mock.timers.tick(60_000);
  assert.deepEqual(old.shown, []);
});

test('a README copy still pending when the README goes away shows nothing', async () => {
  const { article, codeBlock } = readmeArticle();
  const action = copyCodeBlocks(article, '<pre>first</pre>');
  const block = codeBlock();
  block.click();
  action.destroy();
  settleCopy();
  await settled();
  mock.timers.tick(60_000);
  assert.deepEqual(block.shown, []);
});
