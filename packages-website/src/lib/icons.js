export const ICONS = {
  search: '<circle cx="7" cy="7" r="4.5"/><path d="M10.5 10.5L14 14"/>',
  copy: '<rect x="5.5" y="5.5" width="8" height="8" rx="1.5"/><path d="M10.5 5.5V3.5A1.5 1.5 0 0 0 9 2H3.5A1.5 1.5 0 0 0 2 3.5V9a1.5 1.5 0 0 0 1.5 1.5h2"/>',
  check: '<path d="M3 8.5l3.2 3L13 4.5"/>',
  cross: '<path d="M4.5 4.5l7 7M11.5 4.5l-7 7"/>',
  menu: '<path d="M2 5h12M2 11h12"/>',
  arrow: '<path d="M3 8h10M9 4l4 4-4 4"/>',
  chevron: '<path d="M5 6.5l3 3 3-3"/>',
  sun: '<circle cx="8" cy="8" r="3"/><path d="M8 1.5v1.5M8 13v1.5M1.5 8H3M13 8h1.5M3.4 3.4l1.06 1.06M11.54 11.54l1.06 1.06M3.4 12.6l1.06-1.06M11.54 4.46l1.06-1.06"/>',
  moon: '<path d="M13.5 9.6A5.5 5.5 0 0 1 6.4 2.5a5.5 5.5 0 1 0 7.1 7.1z"/>',
};

/** @param {keyof typeof ICONS} name */
export const iconSvg = (name, className = '') =>
  `<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true"${className ? ` class="${className}"` : ''}>${ICONS[name]}</svg>`;
