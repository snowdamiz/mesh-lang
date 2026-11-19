export function formatDate(iso) {
  return new Date(iso).toLocaleDateString('en-US', { year: 'numeric', month: 'short', day: 'numeric', timeZone: 'UTC' });
}

export function formatShortDate(iso, now = Date.now()) {
  const date = new Date(iso);
  const thisYear = date.getUTCFullYear() === new Date(now).getUTCFullYear();
  return date.toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: thisYear ? undefined : 'numeric', timeZone: 'UTC' });
}

// Registry dependencies are exact versions, keyed by the quoted scoped name.
export const dependencyLine = (name, version) => `"${name}" = "${version}"`;

export const installCommand = (name) => `meshpkg install ${name}`;
