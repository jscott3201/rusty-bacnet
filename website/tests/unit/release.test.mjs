import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import navigation from '../../src/data/navigation.json' with { type: 'json' };
import { release } from '../../src/lib/site.mjs';

// A release PR changes `release` in src/lib/site.mjs. MDX pages and components
// read it from there; this test lists every literal copy that doesn't follow it.
const root = new URL('../../', import.meta.url);
const read = path => readFile(new URL(path, root), 'utf8');
const series = release.split('.').slice(0, 2).join('.');
// A 0.x.y version, alone or as v0.x.y, but not part of an address such as 0.0.0.0.
const versions = text => [...new Set([...text.matchAll(/(?<![\w.])v?(0\.\d+\.\d+)(?![.\d])/g)].map(match => match[1]))];

test('every copy of the release version matches site.mjs', async () => {
  const stale = [];
  const expect = (where, ok) => { if (!ok) stale.push(where); };
  // MDX takes the version from site.mjs, so it names none itself.
  const install = 'src/content/docs/start/installation.mdx';
  expect(`${install} names ${versions(await read(install))}; use {release}`, versions(await read(install)).length === 0);
  expect('start/local-lab.mdx: rusty-bacnet=={release}', /rusty-bacnet==\{release\}/.test(await read('src/content/docs/start/local-lab.mdx')));
  // Markdown start pages can't, so each names the release and nothing else.
  const pages = (await readdir(new URL('src/content/docs/start/', root))).filter(name => name.endsWith('.md'));
  assert.ok(pages.length > 0);
  for (const name of pages) {
    const found = versions(await read(`src/content/docs/start/${name}`));
    expect(`start/${name} names ${found.join(', ') || 'no version'}`, found.length === 1 && found[0] === release);
  }
  const lab = await read('examples/python/loopback_lab.py');
  expect(`loopback_lab.py names ${versions(lab).join(', ')}`, versions(lab).join() === release
    && lab.match(/^EXPECTED_VERSION = "([^"]+)"$/m)?.[1] === release);
  expect(`navigation.json: "Start with v${series}"`, navigation.some(group => group.label === `Start with v${series}`));
  expect(`project/support.md: describe **v${release}**`,
    (await read('src/content/docs/project/support.md')).includes(`(/rusty-bacnet/start/installation/) describe **v${release}**`));
  expect(`development/overview.md: [v${series} installation]`,
    (await read('src/content/docs/development/overview.md')).includes(`[v${series} installation](/rusty-bacnet/start/installation/)`));
  assert.deepEqual(stale, [], `release is ${release}; update these`);
});
