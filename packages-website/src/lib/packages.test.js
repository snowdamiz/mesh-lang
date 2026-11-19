import { test } from 'node:test';
import assert from 'node:assert/strict';
import { matches, sortPackages } from './packages.js';

const pkg = (name, download_count, updated_at, description = '') => ({ name, version: '0.1.0', description, download_count, updated_at });
const listing = [
  pkg('hyperpush/mesh-solana', 1840, '2026-11-12T08:00:00.5Z', 'Typed Solana RPC primitives.'),
  pkg('ada/mesh-anchor', 620, '2026-10-30T00:00:00Z', 'Anchor account validation.'),
  pkg('hyperpush/mesh-borsh', 910, '2026-11-12T08:00:00Z', 'Bounded Borsh readers.'),
];
const names = (packages) => packages.map((p) => p.name);

test('each sort orders the listing without changing it', () => {
  assert.deepEqual(names(sortPackages(listing, 'updated')), ['hyperpush/mesh-solana', 'hyperpush/mesh-borsh', 'ada/mesh-anchor']);
  assert.deepEqual(names(sortPackages(listing, 'downloads')), ['hyperpush/mesh-solana', 'hyperpush/mesh-borsh', 'ada/mesh-anchor']);
  assert.deepEqual(names(sortPackages(listing, 'name')), ['ada/mesh-anchor', 'hyperpush/mesh-borsh', 'hyperpush/mesh-solana']);
  assert.deepEqual(names(sortPackages(listing, 'relevance')), names(listing));
  assert.equal(listing[0].name, 'hyperpush/mesh-solana');
});

test('the filter matches the owner, the name, or the description, ignoring case', () => {
  assert.deepEqual(names(listing.filter((p) => matches(p, '  ADA/'))), ['ada/mesh-anchor']);
  assert.deepEqual(names(listing.filter((p) => matches(p, 'borsh'))), ['hyperpush/mesh-borsh']);
  assert.deepEqual(names(listing.filter((p) => matches(p, 'solana rpc'))), ['hyperpush/mesh-solana']);
  assert.equal(listing.filter((p) => matches(p, '')).length, 3);
});

test('a package without a description is not matched by the words "null" or "undefined"', () => {
  const bare = [pkg('kim/mesh-json', 1, '2026-01-01T00:00:00Z', null), { ...pkg('kim/mesh-yaml', 1, '2026-01-01T00:00:00Z'), description: undefined }];
  assert.deepEqual(names(bare.filter((p) => matches(p, 'null'))), []);
  assert.deepEqual(names(bare.filter((p) => matches(p, 'undefined'))), []);
  assert.deepEqual(names(bare.filter((p) => matches(p, 'json'))), ['kim/mesh-json']);
});
