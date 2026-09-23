import { REGISTRY_URL } from '$lib/registry.js';
import { renderReadme } from '$lib/readme.js';

export async function load({ fetch, params }) {
  // Every registry name is owner/package; a longer path would reach the per-version API route.
  if (!/^[^/]+\/[^/]+$/.test(params.name)) return { pkg: null, versions: [], notFound: true };
  try {
    // Fetch package metadata and versions list in parallel
    const [pkgRes, versionsRes] = await Promise.all([
      fetch(`${REGISTRY_URL}/api/v1/packages/${params.name}`),
      fetch(`${REGISTRY_URL}/api/v1/packages/${params.name}/versions`),
    ]);

    if (pkgRes.status === 404) return { pkg: null, versions: [], notFound: true };
    if (!pkgRes.ok) return { pkg: null, versions: [], error: 'Registry unavailable' };

    // Rendered here so the browser gets the sanitised HTML and never loads marked.
    const { readme, ...pkg } = await pkgRes.json();
    const versions = versionsRes.ok ? await versionsRes.json() : [];

    return { pkg, versions, readmeHtml: readme ? renderReadme(readme) : null };
  } catch {
    return { pkg: null, versions: [], error: 'Failed to fetch package' };
  }
}
