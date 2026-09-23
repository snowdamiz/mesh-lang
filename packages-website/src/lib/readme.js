import { Marked } from 'marked';

const escape = (text) =>
  text.replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
const webUrl = (href) => /^(https?:\/\/|mailto:|#)/i.test(href);

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
  },
});

export const renderReadme = (markdown) => marked.parse(markdown);
