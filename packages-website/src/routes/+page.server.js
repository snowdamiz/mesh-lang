import { REGISTRY_URL } from '$lib/registry.js';

export async function load({ fetch }) {
  try {
    const res = await fetch(`${REGISTRY_URL}/api/v1/packages`);
    if (!res.ok) return { packages: [], error: 'Registry unavailable' };
    const packages = await res.json();
    return { packages };
  } catch {
    return { packages: [], error: 'Failed to fetch packages' };
  }
}
