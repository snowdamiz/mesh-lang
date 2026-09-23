import { test } from 'node:test';
import assert from 'node:assert/strict';
import { ago, formatBytes, formatDate } from './format.js';

test('ago rounds down to the largest whole unit', () => {
  const now = Date.parse('2026-09-23T12:00:00Z');
  const at = (seconds) => new Date(now - seconds * 1000).toISOString();
  assert.equal(ago(at(20), now), 'just now');
  assert.equal(ago(at(90), now), '1 minute ago');
  assert.equal(ago(at(3 * 3600 + 3500), now), '3 hours ago');
  assert.equal(ago(at(26 * 3600), now), 'yesterday');
  assert.equal(ago(at(45 * 86400), now), 'last month');
  assert.equal(ago(at(800 * 86400), now), '2 years ago');
});

test('formatBytes picks a unit', () => {
  assert.equal(formatBytes(318), '318 B');
  assert.equal(formatBytes(18200), '17.8 KB');
  assert.equal(formatBytes(3 * 1048576), '3.0 MB');
});

test('formatDate is fixed to UTC so server and browser agree', () => {
  assert.equal(formatDate('2026-09-22T23:55:10Z'), 'Sep 22, 2026');
});
