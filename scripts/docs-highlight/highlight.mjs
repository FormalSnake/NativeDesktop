// Highlights the code blocks in the built docs site in place: node highlight.mjs <site dir>.
// nd-docs writes `<pre><code class="language-x">` with text escaped by html.rs `esc`; each block keeps
// that markup and gains spans coloured with CSS light-dark(), which follows the page's color-scheme.
import { readdir, readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { codeToHtml, isSupportedLanguage } from '@pierre/highlights';
import dark from '@pierre/highlights/themes/github-dark-default';
import light from '@pierre/highlights/themes/github-light-default';

const site = process.argv[2];
if (!site) {
  console.error('usage: highlight.mjs <site dir>');
  process.exit(2);
}

const block = /<pre><code class="language-([^"]+)">([^]*?)<\/code><\/pre>/g;
const unescape = (text) =>
  text.replaceAll('&lt;', '<').replaceAll('&gt;', '>').replaceAll('&quot;', '"').replaceAll('&amp;', '&');
const decoder = new TextDecoder();

function highlight(lang, escaped) {
  const html = decoder.decode(
    codeToHtml(unescape(escaped), { lang, themes: { light, dark }, defaultColor: 'light-dark()' }),
  );
  // The theme's own background and foreground on <pre> are dropped for the site's tokens.
  const inner = html.slice(html.indexOf('<code>') + '<code>'.length, html.lastIndexOf('</code>'));
  return `<pre><code class="language-${lang}">${inner}</code></pre>`;
}

// The docs app under app/ is the wasm bundle, not generated pages.
const pages = (await readdir(site, { recursive: true }))
  .filter((p) => p.endsWith('.html') && !p.startsWith('app/'))
  .map((p) => join(site, p));

let blocks = 0;
const skipped = new Set();
for (const page of pages) {
  const before = await readFile(page, 'utf8');
  const after = before.replace(block, (whole, lang, code) => {
    if (!isSupportedLanguage(lang)) {
      skipped.add(lang);
      return whole;
    }
    blocks++;
    return highlight(lang, code);
  });
  if (after !== before) await writeFile(page, after);
}
console.log(`highlighted ${blocks} code blocks in ${pages.length} pages`);
if (skipped.size) console.log(`no lexer for: ${[...skipped].join(', ')}`);
