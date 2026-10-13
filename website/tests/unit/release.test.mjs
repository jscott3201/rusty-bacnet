import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import { parseDocument } from 'yaml';
import navigation from '../../src/data/navigation.json' with { type: 'json' };
import downloads from '../../src/data/downloads.json' with { type: 'json' };
import { release } from '../../src/lib/site.mjs';

// A release PR changes `release` in src/lib/site.mjs. MDX pages and components
// read it from there; this test lists every literal copy that doesn't follow it.
const root = new URL('../../', import.meta.url);
const repo = new URL('../', root);
const docs = new URL('src/content/docs/', root);
const read = path => readFile(new URL(path, root), 'utf8');
const readRepo = path => readFile(new URL(path, repo), 'utf8');
const series = release.split('.').slice(0, 2).join('.');
// The release project/upgrading.md moves from. The upgrade guide and the lab's
// recorded evidence may name it; nothing else may.
const previous = '0.12.0';
const history = new Set(['project/upgrading.md', 'start/local-lab.mdx']);
// A 0.x.y version, alone or as v0.x.y, but not part of an address such as 0.0.0.0.
const versions = text => [...new Set([...text.matchAll(/(?<![\w.])v?(0\.\d+\.\d+)(?!\.?\d)/g)].map(match => match[1]))];
// A v0.x series such as "v0.12", not followed by a patch number.
const serieses = text => [...new Set([...text.matchAll(/(?<![\w.])v(0\.\d+)(?!\.?\d)/g)].map(match => match[1]))];
const install = 'src/content/docs/start/installation.mdx';

test('every copy of the release version matches site.mjs', async () => {
  const stale = [];
  const expect = (where, ok) => { if (!ok) stale.push(where); };
  const pages = (await readdir(docs, { recursive: true })).filter(name => /\.mdx?$/.test(name)).map(name => name.replaceAll('\\', '/'));
  assert.ok(pages.length > 20);
  for (const name of pages) {
    const text = await read(`src/content/docs/${name}`);
    const allowed = history.has(name) ? [release, previous] : [release];
    // MDX takes the release from site.mjs ({release}), so it names none itself.
    const found = versions(text).filter(version => name.endsWith('.mdx') ? version !== previous || !history.has(name) : !allowed.includes(version));
    expect(`${name} names ${found.join(', ')}${name.endsWith('.mdx') ? '; use {release}' : ''}`, found.length === 0);
    const series_ = serieses(text).filter(found => found !== series && !(history.has(name) && previous.startsWith(`${found}.`)));
    expect(`${name} names v${series_.join(', v')}`, series_.length === 0);
  }
  expect('start/local-lab.mdx: rusty-bacnet=={release}', /rusty-bacnet==\{release\}/.test(await read('src/content/docs/start/local-lab.mdx')));
  const lab = await read('examples/python/loopback_lab.py');
  expect(`loopback_lab.py names ${versions(lab).join(', ')}`, versions(lab).join() === release
    && lab.match(/^EXPECTED_VERSION = "([^"]+)"$/m)?.[1] === release);
  const labels = navigation.flatMap(group => [group.label, ...group.items.map(item => item.label)]).join('\n');
  expect(`navigation.json names v${serieses(labels).join(', v')}`, serieses(labels).join() === series);
  expect(`navigation.json: "Start with v${series}"`, navigation.some(group => group.label === `Start with v${series}`));
  expect(`index.mdx banner: 'Guides for the v${series} release`,
    (await read('src/content/docs/index.mdx')).includes(`content: 'Guides for the v${series} release`));
  const support = await read('src/content/docs/project/support.md');
  expect(`project/support.md: describe **v${release}**`, support.includes(`and the operating guides describe **v${release}**`));
  expect(`project/support.md: describes v${release}, with its release notes`,
    support.includes(`This summary describes v${release}; its [release notes](https://github.com/jscott3201/rusty-bacnet/releases/tag/v${release})`));
  const upgrading = await read('src/content/docs/project/upgrading.md');
  expect(`project/upgrading.md: from v${previous} to v${release}`, upgrading.includes(`title: "Upgrade from v${previous} to v${release}"`));
  assert.deepEqual(stale, [], `release is ${release}; update these`);
});

// The installation page names exactly what .github/workflows/release.yml
// publishes, so a change there fails here until the page follows it.
const units = ['', 'one', 'two', 'three', 'four', 'five', 'six', 'seven', 'eight', 'nine', 'ten', 'eleven', 'twelve',
  'thirteen', 'fourteen', 'fifteen', 'sixteen', 'seventeen', 'eighteen', 'nineteen'];
const tens = ['', '', 'twenty', 'thirty', 'forty', 'fifty'];
const word = n => n < 20 ? units[n] : tens[Math.floor(n / 10)] + (n % 10 ? `-${units[n % 10]}` : '');
// The values of one check_artifacts.py option: the words after it up to the next option.
const option = (args, name) => {
  const start = args.indexOf(name);
  assert.notEqual(start, -1, `release.yml passes ${name} to check_artifacts.py`);
  const end = args.findIndex((arg, i) => i > start && arg.startsWith('--'));
  return args.slice(start + 1, end === -1 ? undefined : end);
};

test('the installation page lists exactly the assets release.yml publishes', async () => {
  const document = parseDocument(await readRepo('.github/workflows/release.yml'), { uniqueKeys: true });
  assert.deepEqual(document.errors, []);
  const workflow = document.toJS();
  const pythons = workflow.env.PYTHONS.split(/\s+/);
  const glibc = workflow.env.GLIBC;
  const check = workflow.jobs.verify.steps.filter(step => step.run?.includes('check_artifacts.py'));
  assert.equal(check.length, 1, 'one verify step runs check_artifacts.py');
  const args = check[0].run.replace(/\\\n/g, ' ').split(/\s+/);
  const wheelPlatforms = option(args, '--wheel-platform');
  const cli = Object.fromEntries(option(args, '--cli').map(pair => pair.split('=')));
  const page = await read(install);

  // The CLI executables: the download list, their minimum OS versions, and the summary.
  assert.deepEqual(downloads.map(item => item.file).sort(), Object.keys(cli).sort());
  const macos = Object.fromEntries(workflow.jobs.build.strategy.matrix.include.filter(build => build.macos).map(build => [build.cli, build.macos]));
  for (const item of downloads) {
    const platform = cli[item.file];
    if (platform.startsWith('linux_')) assert.match(item.runtime, new RegExp(`glibc ${glibc} or later`), item.file);
    if (platform.startsWith('macosx_')) assert.ok(item.runtime.includes(`macOS ${macos[item.file]} or later`), item.file);
    assert.ok(page.includes(`\`${item.file}\``), `${install} names ${item.file}`);
  }
  assert.ok(page.includes(`${word(downloads.length)} CLI executables`));

  // The wheels: one per CPython in PYTHONS per platform tag, and the sdist.
  const rows = Object.fromEntries([...page.matchAll(/^\| [^|`]+ \| `([^`]+)` \| ([^|]+) \|$/gm)].map(match => [match[1], match[2]]));
  assert.deepEqual(Object.keys(rows).sort(), [...wheelPlatforms].sort(), `${install}'s wheel table`);
  for (const tag of wheelPlatforms.filter(tag => tag.startsWith('manylinux_'))) assert.equal(rows[tag], `glibc ${glibc} or newer`, tag);
  for (const [build, version] of Object.entries(macos)) assert.equal(rows[cli[build]], `macOS ${version} or later`, cli[build]);
  assert.ok(page.includes(`CPython **${pythons.slice(0, -1).join(', ')}, and ${pythons.at(-1)}**`), 'the Python versions');
  assert.ok(page.includes(`(\`cp${pythons[0].replace('.', '')}\` to \`cp${pythons.at(-1).replace('.', '')}\`)`));
  assert.ok(page.includes(`${word(pythons.length * wheelPlatforms.length)} Python wheels, <code>rusty_bacnet-{release}-cp3XY-cp3XY-PLATFORM.whl</code>`));
  assert.ok(page.includes('<code>rusty_bacnet-{release}.tar.gz</code>'));

  // THIRD-PARTY-NOTICES goes into the release beside them, and release_api.py adds SHA256SUMS.
  const collect = workflow.jobs.verify.steps.find(step => step.name === 'Collect the release assets');
  assert.match(collect?.run ?? '', /notices\/THIRD-PARTY-NOTICES release\//);
  assert.match(await readRepo('scripts/release/release_api.py'), /^SUMS = "SHA256SUMS"$/m);
  for (const name of ['THIRD-PARTY-NOTICES', 'SHA256SUMS']) assert.ok(page.includes(`\`${name}\``), name);

  // The crates: every workspace member that crates.io may take (publish_crates.sh's `publish != []`).
  const manifest = await readRepo('Cargo.toml');
  const members = [...manifest.match(/^members = \[([\s\S]*?)^\]/m)[1].matchAll(/^\s*"([^"]+)"/gm)].map(match => match[1]);
  const crates = [];
  for (const member of members) {
    const toml = await readRepo(`${member}/Cargo.toml`);
    if (!/^publish = false$/m.test(toml)) crates.push(toml.match(/^name = "([^"]+)"$/m)[1]);
  }
  const listed = page.match(/^The release publishes these crates to crates\.io, all at the same version: (.*?)\. /m)?.[1];
  assert.ok(listed, `${install} lists the crates`);
  assert.deepEqual([...listed.matchAll(/`([^`]+)`/g)].map(match => match[1]).sort(), crates.sort());
  assert.ok(page.includes(`the ${word(crates.length)} crates listed under Rust`));
});
