import { REGISTRY_URL } from '$lib/registry.js';

export async function load({ fetch, url }) {
  const q = url.searchParams.get('q') || '';
  if (!q.trim()) return { packages: [], query: q };
  try {
    const [res, listing] = await Promise.all([
      fetch(`${REGISTRY_URL}/api/v1/packages?search=${encodeURIComponent(q)}`),
      fetch(`${REGISTRY_URL}/api/v1/packages`),
    ]);
    if (!res.ok) return { packages: [], query: q, error: 'Registry unavailable' };
    const hits = await res.json();

    // The registry's full-text index keeps "owner/name" as one token, so a
    // search for "json" or "mesh-slug" misses a package by its own name. Name
    // matches come first, then the ranked full-text hits.
    // ponytail: the listing is the registry's first 100 packages; index name parts in search_vec once it outgrows that.
    const needle = q.trim().toLowerCase();
    const byName = listing.ok ? (await listing.json()).filter((pkg) => pkg.name.toLowerCase().includes(needle)) : [];
    const packages = [...byName, ...hits.filter((hit) => !byName.some((pkg) => pkg.name === hit.name))];
    return { packages, query: q };
  } catch {
    return { packages: [], query: q, error: 'Search failed' };
  }
}
