// Run before build/dev when the example or raw exports change. No network calls.
import { readdir, readFile, mkdir, writeFile, rm, copyFile } from 'node:fs/promises';
import { join, relative, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { frontmatter, plainBody, isGuideFile } from './content.mjs';
import { release } from '../src/lib/site.mjs';
const root = fileURLToPath(new URL('../', import.meta.url));
const docs = join(root, 'src/content/docs');
const output = join(root, 'public/raw');
const downloads = JSON.parse(await readFile(join(root, 'src/data/downloads.json'), 'utf8'));
async function walk(path) {
  const entries = await readdir(path, { withFileTypes: true });
  return (await Promise.all(entries.map(e => e.isDirectory() ? walk(join(path, e.name)) : join(path, e.name)))).flat().sort();
}
await rm(output, { recursive: true, force: true });
await mkdir(output, { recursive: true });
await mkdir(join(root, 'public/examples'), { recursive: true });
await copyFile(join(root, 'examples/python/loopback_lab.py'), join(root, 'public/examples/loopback_lab.py'));
const links = ['# Rusty BACnet', '', `> Guides for the v${release} release; pre-1.0 APIs and partial conformance.`, '', `Use start/ to install and make a first read, development/ to integrate (shared endpoints, Network Port and Number controls, transports, BACnet/SC, source builds), guides/ to operate the CLI, tui/ for the terminal UI on the development branch, and project/upgrading for the move from the previous release. A dev checkout may differ from v${release}; record its commit. The local lab is loopback-only.`, '', '## Guides', ''];
let count = 0;
for (const file of await walk(docs)) {
  if (!isGuideFile(file)) continue;
  const page = frontmatter(await readFile(file, 'utf8'));
  if (!page.title) throw new Error(`Missing title: ${file}`);
  const name = relative(docs, file).replaceAll('\\', '/').replace(/\.mdx$/, '.md');
  const target = join(output, name);
  await mkdir(dirname(target), { recursive: true });
  await writeFile(target, `# ${page.title}\n\n${await plainBody(page.body, file, downloads)}\n`);
  links.push(`- [${name.startsWith('development/') ? 'Integration: ' : ''}${page.title}](https://jscott3201.github.io/rusty-bacnet/raw/${name})`);
  count++;
}
await writeFile(join(root, 'public/llms.txt'), links.join('\n') + '\n');
console.log(`Prepared ${count} plain Markdown guides and the downloadable local lab.`);
