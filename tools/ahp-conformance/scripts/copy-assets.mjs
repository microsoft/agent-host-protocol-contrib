import { copyFile } from 'node:fs/promises';

await Promise.all(['page.html', 'page.css', 'page.js'].map((name) =>
  copyFile(new URL(`../src/report/html/${name}`, import.meta.url), new URL(`../dist/report/html/${name}`, import.meta.url)),
));
