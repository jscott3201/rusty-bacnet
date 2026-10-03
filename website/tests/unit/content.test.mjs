import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { frontmatter, plainBody } from '../../scripts/content.mjs';
import { release, repository } from '../../src/lib/site.mjs';
const root = new URL('../../', import.meta.url);
test('Markdown metadata is retained', () => {
  assert.equal(frontmatter('---\ntitle: "A guide"\n---\nBody').title, 'A guide');
});
test('all install tabs and assets survive the text export', async () => {
  const file = fileURLToPath(new URL('src/content/docs/start/installation.mdx', root));
  const downloads = JSON.parse(await readFile(new URL('src/data/downloads.json', root), 'utf8'));
  const body = await plainBody(frontmatter(await readFile(file, 'utf8')).body, file, downloads);
  const assets = `${repository}/releases/download/v${release}/`;
  for (const value of ['**CLI**', '**Python**', '**Rust**', 'only-binary', 'bacnet-macos-arm64', 'glibc 2.17',
    `[SHA256SUMS](${assets}SHA256SUMS)`, `(${assets}bacnet-windows-amd64.exe)`, `rusty-bacnet==${release}`,
    String.raw`.\.venv\Scripts\python.exe -m pip install`, `bacnet-client = "=${release}"`, `**${release}**`, `\`${release}\``,
    `[Release workflow](${repository}/blob/v${release}/.forgejo/workflows/release.yml)`]) assert.ok(body.includes(value), value);
  assert.ok(!body.includes('<TabItem'));
  assert.doesNotMatch(body, /\$\{|\{release\}|<a\b|<code>|libpcap\.so/);
});
test('local-lab export includes the real source and an absolute diagram link', async () => {
  const file = fileURLToPath(new URL('src/content/docs/start/local-lab.mdx', root));
  const body = await plainBody(frontmatter(await readFile(file, 'utf8')).body, file, []);
  assert.ok(body.includes('async def run_lab'));
  assert.ok(body.includes('https://jscott3201.github.io/rusty-bacnet/diagrams/local-lab.svg'));
  assert.ok(body.includes(`\`rusty-bacnet==${release}\``));
});
test('new MDX components cannot be silently dropped from plain-text output', async () => {
  await assert.rejects(() => plainBody('<NewWidget />', '/tmp/content.mdx', []));
});
test('MDX expressions are limited to the site constants', async () => {
  const body = await plainBody('Use v{release}.\n\n<Code lang="sh" code={`echo ${release}`} />\n\n```js\nconst a = {b};\n```\n', '/tmp/content.mdx', []);
  assert.ok(body.includes(`Use v${release}.`));
  assert.ok(body.includes(`\`\`\`sh\necho ${release}\n\`\`\``));
  assert.ok(body.includes('const a = {b};'), 'fenced code is left alone');
  await assert.rejects(() => plainBody('Use {other}.', '/tmp/content.mdx', []), /Unknown expression/);
  await assert.rejects(() => plainBody('<Code lang="sh" code={`echo ${other}`} />', '/tmp/content.mdx', []), /Unknown expression/);
  await assert.rejects(() => plainBody('<Code lang="sh" code={`a\\b`} />', '/tmp/content.mdx', []), /String\.raw/);
  await assert.rejects(() => plainBody('<a href={link}>text</a>', '/tmp/content.mdx', []), /Unknown expression/);
  await assert.rejects(() => plainBody('<a href="https://example.com/">text</a>', '/tmp/content.mdx', []), /Unprojected/);
});

test('guide selection is portable across Windows and POSIX paths', async () => {
  const { isGuideFile } = await import('../../scripts/content.mjs');
  assert.equal(isGuideFile('C:\\website\\docs\\index.mdx'), false);
  assert.equal(isGuideFile('C:\\website\\docs\\404.md'), false);
  assert.equal(isGuideFile('C:\\website\\docs\\start\\local-lab.mdx'), true);
  assert.equal(isGuideFile('/website/docs/start/local-lab.mdx'), true);
});
