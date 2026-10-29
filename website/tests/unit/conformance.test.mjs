// The capability tables page, project/conformance (#1496). Its data,
// src/data/support.json, is generated from the code by
// crates/bacnet-integration-tests/support_tables.rs, and a Rust test fails
// when the committed file is stale; these tests check the page and the
// data's shape, and keep both free of conformance claims.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import support from '../../src/data/support.json' with { type: 'json' };
import { frontmatter, plainBody } from '../../scripts/content.mjs';
import { release } from '../../src/lib/site.mjs';
import { supportTables, supportTableMarkdown, typeLabel } from '../../src/lib/support.mjs';

const page = fileURLToPath(new URL('../../src/content/docs/project/conformance.mdx', import.meta.url));
const source = await readFile(page, 'utf8');
const forbidden = /\b(conformant|compliant|certified)\b/i;

test('support.json has the generated shape and records the release the site names', () => {
  assert.equal(support.release, release);
  assert.equal(support.tracks, 'dev');
  assert.match(support.generatedBy, /support_tables\.rs/);
  assert.ok(support.objectTypes.length > 0);
  for (const type of support.objectTypes) {
    assert.ok(Number.isInteger(type.id) && /^[A-Z_]+$/.test(type.name), JSON.stringify(type));
    assert.equal(typeof type.create, 'boolean');
    assert.equal(typeof type.delete, 'boolean');
  }
  const ids = support.objectTypes.map(type => type.id);
  assert.deepEqual(ids, [...ids].sort((a, b) => a - b), 'object types in type order');
  for (const type of support.excludedObjectTypes) assert.ok(!ids.includes(type.id), type.name);
  const flags = ['serverExecutes', 'serverInitiates', 'clientSends', 'pythonSends'];
  for (const service of support.services) {
    assert.ok(['confirmed', 'unconfirmed'].includes(service.kind), service.label);
    for (const flag of flags) assert.equal(typeof service[flag], 'boolean', `${service.label}.${flag}`);
    assert.ok(service.endpoint === null || typeof service.endpoint === 'string', service.label);
    assert.ok(flags.some(flag => service[flag]) || service.endpoint, `${service.label} is on some surface`);
  }
  assert.ok(support.services.some(service => service.label === 'ReadProperty' && service.endpoint === 'always'));
  for (const link of support.dataLinks) {
    assert.equal(typeof link.python, 'boolean', link.id);
    assert.deepEqual(Object.keys(link.cli).sort(), [...support.cliTargets].sort(), link.id);
  }
});

test('the tables and the page make no conformance claim', () => {
  assert.doesNotMatch(JSON.stringify(support), forbidden);
  for (const id of Object.keys(supportTables(support))) assert.doesNotMatch(supportTableMarkdown(id, support), forbidden, id);
  assert.doesNotMatch(source, forbidden);
  assert.doesNotMatch(source, /\bBIBB/);
});

test('the page renders every table and types no count of its own', async () => {
  const ids = Object.keys(supportTables(support));
  assert.deepEqual(ids, ['objects', 'services-confirmed', 'services-unconfirmed', 'links']);
  for (const id of ids) assert.equal(source.split(`<SupportTable table="${id}" />`).length, 2, `${id} once`);
  // Every number on the page comes from the data: the authored text has no
  // digits outside link targets and the frontmatter.
  const authored = frontmatter(source).body
    .replace(/\]\([^)]*\)/g, '](...)')
    .replace(/href=\{`[^`]*`\}/g, 'href');
  assert.doesNotMatch(authored, /\d/, 'a hand-typed number');
});

test('the page links the evidence guidance, the gap label and says it tracks dev', async () => {
  const body = await plainBody(frontmatter(source).body, page, []);
  assert.ok(body.includes('(https://jscott3201.github.io/rusty-bacnet/project/support/#understand-what-a-result-proves)'));
  assert.ok(body.includes('/issues?q=is%3Aissue%20is%3Aopen%20label%3Aconformance-gap)'));
  assert.ok(source.includes(':::note[On `dev` after the release]'));
  assert.ok(body.includes(`merged after v${release}`));
});

test('the plain-text export carries each table row for row', async () => {
  const body = await plainBody(frontmatter(source).body, page, []);
  assert.doesNotMatch(body, /<SupportTable/);
  for (const type of support.objectTypes) assert.ok(body.includes(`| ${typeLabel(type.name)} | ${type.id} |`), type.name);
  for (const service of support.services) assert.ok(body.includes(`| ${service.label} |`), service.label);
  for (const link of support.dataLinks) assert.ok(body.includes(`| ${link.label} |`), link.label);
});

test('table cells follow the data', () => {
  const data = {
    objectTypes: [{ id: 13, name: 'MULTI_STATE_INPUT', create: true, delete: false }],
    excludedObjectTypes: [],
    services: [
      { label: 'ReadProperty', kind: 'confirmed', serverExecutes: true, serverInitiates: false, clientSends: true, endpoint: 'always', pythonSends: true },
      { label: 'WriteProperty', kind: 'confirmed', serverExecutes: true, serverInitiates: false, clientSends: true, endpoint: 'with_device_writes', pythonSends: false },
      { label: 'I-Am', kind: 'unconfirmed', serverExecutes: false, serverInitiates: true, clientSends: false, endpoint: null, pythonSends: false },
    ],
    cliTargets: ['a', 'b'],
    dataLinks: [
      { label: 'Always', feature: null, python: true, cli: { a: true, b: true } },
      { label: 'Some', feature: 'serial', python: false, cli: { a: false, b: true } },
      { label: 'None', feature: 'ethernet', python: false, cli: { a: false, b: false } },
    ],
  };
  const tables = supportTables(data);
  assert.deepEqual(tables.objects.rows, [['Multi State Input', '13', 'Yes', 'No']]);
  assert.match(tables.objects.caption, /^1 constructible object types; not built: none$/);
  assert.deepEqual(tables['services-confirmed'].rows.map(row => row[4]),
    ['Always', { text: 'Opt-in: ', code: 'with_device_writes()' }]);
  assert.deepEqual(tables['services-unconfirmed'].rows, [['I-Am', 'No', 'Yes', 'No', 'No', 'No']]);
  assert.deepEqual(tables.links.rows, [
    ['Always', 'Always built', 'Yes', 'Every release target'],
    ['Some', { code: 'serial' }, 'No', { code: 'b' }],
    ['None', { code: 'ethernet' }, 'No', 'No'],
  ]);
  assert.match(supportTableMarkdown('links', data), /\| Some \| `serial` \| No \| `b` \|/);
});
