// Vite fills import.meta.env at build time; plain Node (the unit tests) has none.
const env = import.meta.env ?? {};

/** The main site. A dev server links to the local one; VITE_SITE_URL overrides either. */
export const MAIN_SITE = `${String(env.VITE_SITE_URL || (env.DEV ? 'http://localhost:5173' : 'https://meshlang.dev')).replace(/\/$/, '')}/`;
export const REPO_URL = 'https://github.com/hyperpush-org/mesh-lang';
export const AUTH_URL = 'https://api.packages.meshlang.dev/auth/github';
/* global __MESH_VERSION__ */
const VERSION = typeof __MESH_VERSION__ === 'string' ? __MESH_VERSION__ : '';
export const STATUS = `v${VERSION} · in active development`;

const docs = (slug) => `${MAIN_SITE}docs/${slug}/`;
export const STATUS_URL = docs('getting-started');

export const NAV_LINKS = [
  { label: 'Docs', href: docs('getting-started') },
  { label: 'Packages', href: '/', current: true },
  { label: 'GitHub', href: REPO_URL },
];

export const FOOTER_COLUMNS = [
  {
    title: 'Learn',
    links: [
      { label: 'Getting Started', href: docs('getting-started') },
      { label: 'Language Basics', href: docs('language-basics') },
      { label: 'Concurrency', href: docs('concurrency') },
      { label: 'Autonomous Clusters', href: docs('autonomous-clusters') },
      { label: 'Syntax Cheatsheet', href: docs('cheatsheet') },
    ],
  },
  {
    title: 'Ecosystem',
    links: [
      { label: 'Packages', href: '/' },
      { label: 'Publish a package', href: '/publish' },
      { label: 'Developer Tools', href: docs('tooling') },
    ],
  },
  {
    title: 'Project',
    links: [
      { label: 'GitHub', href: REPO_URL },
      { label: 'Release Notes', href: docs('autonomous-release-notes') },
    ],
  },
];
