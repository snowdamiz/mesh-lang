import { Marked, Renderer } from 'marked';
import { iconSvg } from './icons.js';

const escape = (text) =>
  text.replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
const webUrl = (href) => /^(https?:\/\/|mailto:|#)/i.test(href);

const copyButton = `<button type="button" class="icon-btn codeblock-copy" data-copy-code data-state="idle" aria-label="Copy code">${iconSvg('copy', 'when-idle')}${iconSvg('check', 'when-copied')}${iconSvg('cross', 'when-failed')}</button>`;

// A README is whatever the publisher put in their tarball, so its raw HTML is
// shown as text and links or images may only point at web or mail addresses.
// Returning false hands a token back to marked's default renderer.
const marked = new Marked({
  renderer: {
    html: ({ text }) => escape(text),
    link(token) {
      return webUrl(token.href) ? false : this.parser.parseInline(token.tokens);
    },
    image(token) {
      return webUrl(token.href) ? false : escape(token.text);
    },
    code(token) {
      return `<figure class="codeblock">${copyButton}${Renderer.prototype.code.call(this, token)}</figure>\n`;
    },
    table(token) {
      return `<div class="table-wrap">${Renderer.prototype.table.call(this, token)}</div>\n`;
    },
  },
});

export const renderReadme = (markdown) => marked.parse(markdown);
