// The first load (spec §1.4): index.html, what it loads and preloads, and the fonts its
// styles use. Screens and sheets that load later are not counted.
import { readFileSync } from 'node:fs';

const html = readFileSync('dist/index.html', 'utf8');
const loaded = [...html.matchAll(/(?:src|href)="\/?(assets\/[^"]+\.(?:js|css))"/g)].map(
  (match) => `dist/${match[1]}`,
);
const fonts = loaded
  .filter((file) => file.endsWith('.css'))
  .flatMap((css) =>
    [...readFileSync(css, 'utf8').matchAll(/url\(["']?\/?(assets\/[^"')]+\.woff2)/g)].map(
      (match) => `dist/${match[1]}`,
    ),
  );

export default [
  {
    name: 'first load (JS, CSS, fonts)',
    path: ['dist/index.html', ...new Set([...loaded, ...fonts])],
    brotli: true,
    limit: '152 kB',
  },
];
