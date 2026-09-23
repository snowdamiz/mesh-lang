import { test } from 'node:test';
import assert from 'node:assert/strict';
import { renderReadme } from './readme.js';

test('renders markdown but not the HTML or script URLs a publisher puts in it', () => {
  const html = renderReadme(
    [
      '# Title',
      '<img src=x onerror=alert(1)>',
      'Inline <b onclick="alert(2)">bold</b>',
      '[bad](javascript:alert(3)) [ok](https://example.com) ![pic](JaVaScRiPt:alert(4)) <javascript:alert(5)>',
    ].join('\n\n'),
  );
  assert.match(html, /<h1>Title<\/h1>/);
  assert.match(html, /<a href="https:\/\/example.com">ok<\/a>/);
  assert.match(html, /&lt;img src=x onerror=alert\(1\)&gt;/);
  assert.doesNotMatch(html, /<img src=x|<b onclick|(href|src)="javascript:/i);
});
