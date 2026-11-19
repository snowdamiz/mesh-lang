import { test } from 'node:test';
import assert from 'node:assert/strict';
import { formatDate, formatShortDate } from './format.js';

test('formatDate is fixed to UTC so server and browser agree', () => {
  assert.equal(formatDate('2026-09-22T23:55:10Z'), 'Sep 22, 2026');
});

test('formatShortDate drops the year only inside the current UTC year', () => {
  const now = Date.parse('2026-11-18T12:00:00Z');
  assert.equal(formatShortDate('2026-01-01T00:30:00Z', now), 'Jan 1');
  assert.equal(formatShortDate('2025-12-31T23:59:00Z', now), 'Dec 31, 2025');
});
