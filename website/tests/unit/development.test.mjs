import { test } from 'node:test';
import assert from 'node:assert/strict';
import { access, readFile } from 'node:fs/promises';
import navigation from '../../src/data/navigation.json' with { type: 'json' };
import { frontmatter, plainBody } from '../../scripts/content.mjs';
import { fileURLToPath } from 'node:url';
import { release } from '../../src/lib/site.mjs';
import { splitDevSection } from './dev-section.mjs';

const root = new URL('../../', import.meta.url);
async function guidePath(slug) {
  const matches = [];
  for (const extension of ['md', 'mdx']) {
    const path = fileURLToPath(new URL(`src/content/docs/${slug}.${extension}`, root));
    try {
      await access(path);
      matches.push(path);
    } catch (error) {
      if (error.code !== 'ENOENT') throw error;
    }
  }
  assert.equal(matches.length, 1, `${slug}: exactly one Markdown or MDX source`);
  return matches[0];
}
test('integration navigation resolves task guides with release-pinned source links', async () => {
  const group = navigation.find(group => group.label === 'Build applications');
  assert.ok(group);
  const slugs = group.items.map(item => item.slug);
  assert.deepEqual(slugs, ['integrate/overview', 'integrate/shared-endpoints', 'integrate/network-ports', 'integrate/server',
    'integrate/router', 'integrate/security', 'integrate/pics']);
  assert.deepEqual(navigation.find(group => group.label === 'Connect to a network').items.map(item => item.slug),
    ['connect/overview', 'connect/bacnet-ip', 'connect/across-subnets', 'connect/nat', 'connect/multicast',
      'connect/ipv6', 'connect/bacnet-sc', 'connect/mstp', 'connect/mstp-realtime']);
  assert.deepEqual(navigation.find(group => group.label === 'Contribute').items.map(item => item.slug),
    ['development/build-from-source', 'development/documentation']);
  assert.deepEqual(navigation.find(group => group.label === 'Evaluate and get help').items.map(item => item.slug),
    ['help/troubleshooting', 'project/support', 'project/conformance', 'project/sample-pics', 'project/performance', 'project/upgrading', 'project/upgrading/v0-15', 'project/upgrading/v0-14']);
  const pages = navigation.flatMap(group => group.items.flatMap(item => item.items ?? [item]));
  assert.equal(new Set(pages.map(item => item.slug)).size, pages.length);
  for (const slug of [...slugs, 'connect/overview', 'connect/bacnet-sc', 'reference/transports', 'project/performance']) {
    const path = await guidePath(slug);
    const source = await readFile(path, 'utf8');
    const page = frontmatter(source);
    assert.ok(page.title && page.description, slug);
    const body = await plainBody(page.body, path, []);
    assert.ok(body.includes(`https://github.com/jscott3201/rusty-bacnet/blob/v${release}/docs/`), slug);
    // dev is the release: no page keeps a dev section.
    const { onDev } = splitDevSection(body, slug);
    assert.equal(onDev, '', `${slug}: no dev section`);
    assert.ok(body.includes(slug === 'reference/transports' ? '## Which guide next?' : '## Next steps'), slug);
    assert.ok(!body.includes('cargo install bacnet-cli --version'), slug);
  }
});

test('release CLI source path is pinned while the lab remains release-scoped', async () => {
  const path = fileURLToPath(new URL('src/content/docs/start/installation.mdx', root));
  const install = await plainBody(frontmatter(await readFile(path, 'utf8')).body, path, []);
  assert.ok(install.includes(`git clone --branch v${release} `));
  assert.match(install, /cargo install --path crates\/bacnet-cli --locked/);
  // bacnet-cli is on crates.io from 0.12.0; any crates.io install names the release.
  for (const match of install.matchAll(/cargo install bacnet-cli --version (\S+)/g)) assert.equal(match[1], release);
  const lab = await readFile(new URL('src/content/docs/start/local-lab.mdx', root), 'utf8');
  assert.match(lab, /rusty-bacnet==\{release\}/);
  assert.match(lab, /loopback_lab\.py\?raw/);
});


test('source builds preserve release and development requirements separately', async () => {
  const build = await readFile(new URL('src/content/docs/development/build-from-source.md', root), 'utf8');
  assert.match(build, /git clone --branch dev/);
  assert.match(build, /git rev-parse HEAD/);
  assert.match(build, /edition 2024 and requires Rust 1\.95/);
  assert.ok(build.includes(`clone with \`--branch v${release}\` instead`));
  assert.match(build, /maturin develop --manifest-path crates\/rusty-bacnet\/Cargo.toml --locked/);
  assert.match(build, /Native compilation alone does not exercise the Python API/);
});
