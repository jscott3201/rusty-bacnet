// Run before build/dev when the example or raw exports change. No network calls.
import { readdir, readFile, mkdir, writeFile, rm, copyFile } from 'node:fs/promises';
import { join, relative, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { frontmatter, plainBody, isGuideFile } from './content.mjs';
import legacyRoutes from '../src/data/legacy-routes.json' with { type: 'json' };
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
for (const script of ['loopback_lab.py', 'server_device.py', 'bip_router.py']) {
  await copyFile(join(root, 'examples/python', script), join(root, 'public/examples', script));
}
for (const example of ['shared-endpoint', 'sc-operator-policy', 'server-device', 'bip-router']) {
  for (const file of ['Cargo.toml', 'src/main.rs']) {
    const relative = `examples/rust/${example}/${file}`;
    const target = join(root, 'public', relative);
    await mkdir(dirname(target), { recursive: true });
    await copyFile(join(root, relative), target);
  }
}
const links = ['# Rusty BACnet', '', `> Guides for the v${release} release; pre-1.0 APIs and partial conformance.`, '', `Use start/ to install and make a first read, integrate/ for application owners: serving objects, routing between networks, shared endpoints, Network Port and Number controls, server authorization and PICS drafts; development/ for source builds and documentation contributions, connect/ to choose and set up a data link (BACnet/IP with BBMDs, NAT and multicast, BACnet/IPv6, BACnet/SC, MS/TP), guides/ for discovery, subscriptions, writes and AI agents over MCP, reference/ for the CLI, Python and Rust references and transport evidence, tui/ for the terminal UI, and project/upgrading for the upgrade guides between releases; project/conformance and project/sample-pics are generated from the dev branch. A dev checkout may differ from v${release}; record its commit. The local lab is loopback-only.`, '', '## Guides', ''];
let count = 0;
for (const file of await walk(docs)) {
  if (!isGuideFile(file)) continue;
  const page = frontmatter(await readFile(file, 'utf8'));
  if (!page.title) throw new Error(`Missing title: ${file}`);
  const name = relative(docs, file).replaceAll('\\', '/').replace(/\.mdx$/, '.md');
  const target = join(output, name);
  await mkdir(dirname(target), { recursive: true });
  await writeFile(target, `# ${page.title}\n\n${await plainBody(page.body, file, downloads)}\n`);
  const slug = name.replace(/\.md$/, '');
  if (!legacyRoutes.some(route => route.from === slug)) {
    const category = slug.startsWith('integrate/') ? 'Integration: ' : slug.startsWith('development/') ? 'Contribution: ' : '';
    links.push(`- [${category}${page.title}](https://jscott3201.github.io/rusty-bacnet/raw/${name})`);
  }
  count++;
}
await writeFile(join(root, 'public/llms.txt'), links.join('\n') + '\n');
console.log(`Prepared ${count} plain Markdown guides and the downloadable examples.`);
