import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { frontmatter, plainBody } from '../../scripts/content.mjs';
const root = new URL('../../', import.meta.url);
test('Markdown metadata is retained', () => {
  assert.equal(frontmatter('---\ntitle: "A guide"\n---\nBody').title, 'A guide');
});
test('all install tabs and assets survive the text export', async () => {
  const file = fileURLToPath(new URL('src/content/docs/start/installation.mdx', root));
  const downloads = JSON.parse(await readFile(new URL('src/data/downloads.json', root), 'utf8'));
  const body = await plainBody(frontmatter(await readFile(file, 'utf8')).body, file, downloads);
  for (const value of ['**CLI**', '**Python**', '**Rust**', 'libpcap.so.0.8', 'only-binary', 'bacnet-macos-arm64']) assert.ok(body.includes(value), value);
  assert.ok(!body.includes('<TabItem'));
});
test('local-lab export includes the real source and an absolute diagram link', async () => {
  const file = fileURLToPath(new URL('src/content/docs/start/local-lab.mdx', root));
  const body = await plainBody(frontmatter(await readFile(file, 'utf8')).body, file, []);
  assert.ok(body.includes('async def run_lab'));
  assert.ok(body.includes('https://jscott3201.github.io/rusty-bacnet/diagrams/local-lab.svg'));
});
test('new MDX components cannot be silently dropped from plain-text output', async () => {
  await assert.rejects(() => plainBody('<NewWidget />', '/tmp/content.mdx', []));
});

test('guide selection is portable across Windows and POSIX paths', async () => {
  const { isGuideFile } = await import('../../scripts/content.mjs');
  assert.equal(isGuideFile('C:\\website\\docs\\index.mdx'), false);
  assert.equal(isGuideFile('C:\\website\\docs\\404.md'), false);
  assert.equal(isGuideFile('C:\\website\\docs\\start\\local-lab.mdx'), true);
  assert.equal(isGuideFile('/website/docs/start/local-lab.mdx'), true);
});
