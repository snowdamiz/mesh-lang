const relative = new Intl.RelativeTimeFormat('en', { numeric: 'auto' });
const units = [
  ['year', 31536000],
  ['month', 2592000],
  ['day', 86400],
  ['hour', 3600],
  ['minute', 60],
];

export function ago(iso, now = Date.now()) {
  const seconds = (now - Date.parse(iso)) / 1000;
  for (const [unit, size] of units) {
    if (seconds >= size) return relative.format(-Math.floor(seconds / size), unit);
  }
  return 'just now';
}

export function formatBytes(bytes) {
  if (bytes < 1024) return bytes + ' B';
  if (bytes < 1048576) return (bytes / 1024).toFixed(1) + ' KB';
  return (bytes / 1048576).toFixed(1) + ' MB';
}

export function formatDate(iso) {
  return new Date(iso).toLocaleDateString('en-US', { year: 'numeric', month: 'short', day: 'numeric', timeZone: 'UTC' });
}

// Registry dependencies are exact versions, keyed by the quoted scoped name.
export const dependencyLine = (name, version) => `"${name}" = "${version}"`;
