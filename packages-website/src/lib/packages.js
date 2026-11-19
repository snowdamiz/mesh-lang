/**
 * One row of `GET /api/v1/packages` (and of its `?search=` form).
 * @typedef {object} PackageSummary
 * @property {string} name "owner/name"; the owner is the publisher's GitHub login.
 * @property {string} version The latest version.
 * @property {string | null} [description]
 * @property {number} download_count
 * @property {string} updated_at ISO 8601, UTC.
 */

/**
 * `GET /api/v1/packages/<owner>/<name>`. The server renders its `readme` to HTML and drops it from this object.
 * @typedef {object} PackageDetail
 * @property {string} name
 * @property {string} description
 * @property {string} owner
 * @property {number} download_count
 * @property {{ version: string, sha256: string }} latest
 * @property {string} [license]
 * @property {string} [repository]
 */

/**
 * One row of `GET /api/v1/packages/<owner>/<name>/versions`, newest first.
 * @typedef {object} PackageVersion
 * @property {string} version
 * @property {string} published_at ISO 8601, UTC.
 * @property {number} download_count
 * @property {number} size_bytes
 */

/** @param {{ name: string }} pkg */
export function splitName(pkg) {
  const slash = pkg.name.indexOf('/');
  return { owner: pkg.name.slice(0, slash), slug: pkg.name.slice(slash + 1) };
}

/** @typedef {(a: PackageSummary, b: PackageSummary) => number} Compare */

/** @type {Record<string, { label: string, compare: Compare }>} */
export const SORTS = {
  relevance: { label: 'Best match', compare: () => 0 },
  updated: { label: 'Recently updated', compare: (a, b) => Date.parse(b.updated_at) - Date.parse(a.updated_at) },
  downloads: { label: 'Most downloaded', compare: (a, b) => b.download_count - a.download_count },
  name: {
    label: 'Name',
    compare: (a, b) => splitName(a).slug.localeCompare(splitName(b).slug) || a.name.localeCompare(b.name),
  },
};

/**
 * @param {PackageSummary[]} packages
 * @param {keyof typeof SORTS} sort
 */
export const sortPackages = (packages, sort) => [...packages].sort(SORTS[sort].compare);

/**
 * @param {PackageSummary} pkg
 * @param {string} query
 */
export function matches(pkg, query) {
  const needle = query.trim().toLowerCase();
  return !needle || `${pkg.name} ${pkg.description ?? ''}`.toLowerCase().includes(needle);
}
