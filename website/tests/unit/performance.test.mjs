import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { frontmatter, plainBody } from '../../scripts/content.mjs';
import { release } from '../../src/lib/site.mjs';
import { repo, splitDevSection } from './dev-section.mjs';

const file = fileURLToPath(new URL('../../src/content/docs/project/performance.mdx', import.meta.url));
const benchmarks = new URL('../../../Benchmarks.md', import.meta.url);
const flat = text => text.replace(/\s+/g, ' ');

async function page() {
  const source = await readFile(file, 'utf8');
  const { title, description, body } = frontmatter(source);
  return { title, description, body: await plainBody(body, file, []) };
}

// GitHub's heading anchor: lower case, punctuation dropped, spaces to hyphens.
const anchor = heading => heading.trim().toLowerCase().replace(/[^\p{L}\p{N} _-]/gu, '').replaceAll(' ', '-');

// Every figure the page quotes, with its unit and a few words of context.
// Each is checked against the working tree's Benchmarks.md, while the page's
// links point at Benchmarks.md as tagged for the release: a Benchmarks refresh
// that changes a figure fails here until the page follows it. A string stands
// for the same text on both sides; a pair is [page text, Benchmarks.md text]
// where the wording differs (or the source states the unit in a table header).
const figures = [
  'median of 29 microseconds over BACnet/IP', '25 microseconds over BACnet/IPv6',
  ['48 microseconds over BACnet/SC', '48 microseconds over BACnet Secure Connect'],
  'only 2 to 8 microseconds more than one ReadProperty', '38,600 requests per second',
  ['about 82,000 ReadProperty requests per second from 10 clients', 'about 82,000 ReadProperty requests per second from 10 concurrent clients'], 'peaks at 5 to 10 clients',
  'BACnet/SC peaks later, at 50 clients',
  ['passes BACnet/IP from 25 clients on', 'passed B/IP from 25 clients on'],
  ['0.5% on BACnet/IP', 'on B/IP (0.5%)'], ['0.3% on BACnet/IPv6', 'on B/IPv6 (0.3%)'],
  ['0.03% on BACnet/SC', 'on BACnet/SC (0.03%)'], ['counted 0.06% on BACnet/IP', 'B/IP requests (0.06%)'],
  ['BACnet/IP run was 49 megabytes', 'B/IP run was 49 megabytes'],
  '85,400 operations per second with no errors',
  '90% to 100% of the wire limit', 'about 7 milliseconds per request',
  '93% of the wire limit at 9,600 baud, 52% at 115,200',
  '16 milliseconds by default on FTDI parts',
  ['10.7 small requests per second at 9,600 baud', '| 9,600 | Small | 10.7 |'],
  ['71.8 at 115,200', '| 115,200 | Small | 71.8 |'],
  'median of 43 microseconds against 22 direct', ['router hop added about 21 microseconds', 'router hop adds about 21 microseconds'],
  '57,000 requests per second', "88% of the direct path's rate at 50 clients",
  ['one client got 67 requests per second', 'client through the router got 67 requests per second'], ['about 1.4 milliseconds', 'about 1.4 milliseconds to a request'],
  ['already holds for 14 milliseconds', 'already holds for 14.'],
  '16 confirmed requests in flight per client',
  'median of 24 microseconds from 100 to 10,000 objects', 'about 1.3 kilobytes per Analog Input',
  ['1,024 subscriptions spread over 16 subscribers', '1,024 subscriptions come from 16 subscribers'],
  '10 writes per second', '64 subscriptions from one subscriber',
  '250 objects in 3 segments', 'three and a half times as long',
  '256 results or 16,384 bytes',
  '70 to 100 microseconds', 'about 34,000 requests per second with 10',
  ['and 36,000 with 16', 'and 36,000 with 16'],
  '17.9 kilobytes of flash', '20% to 70% more requests', '16 gigabytes of memory',
];

test('performance page is release-scoped and links Benchmarks.md at the release', async () => {
  const { title, description, body } = await page();
  assert.ok(title && description);
  assert.ok(body.includes(`The page describes **v${release}**`));
  assert.ok(body.includes('(#a-later-repeated-comparison)'));
  assert.ok(body.includes('\n## A later repeated comparison\n'));
  assert.ok(body.includes('\n## Next steps\n'));
  const { released, onDev } = splitDevSection(body, 'project/performance');
  assert.equal(onDev, '', 'no dev section');
  assert.ok(released.includes(`${repo}/blob/v${release}/Benchmarks.md#101-repeated-comparison-for-1979`));
  for (const issue of [1978, 1979]) assert.ok(released.includes(`${repo}/issues/${issue}`), `#${issue}`);
  // The release-scoped text makes no capacity claim.
  assert.ok(released.includes('not capacity guarantees'));
  assert.ok(released.includes(`${repo}/blob/v${release}/Benchmarks.md#11-how-to-reproduce`));
});

test('every Benchmarks.md section the page links exists', async () => {
  const report = await readFile(benchmarks, 'utf8');
  const anchors = new Set([...report.matchAll(/^#{2,3} (.+)$/gm)].map(match => anchor(match[1])));
  const { body } = await page();
  const linked = [...body.matchAll(/\/Benchmarks\.md#([\w-]+)/g)].map(match => match[1]);
  assert.ok(linked.length >= 10);
  for (const name of linked) assert.ok(anchors.has(name), `Benchmarks.md#${name}`);
});

test('every figure the page quotes appears in Benchmarks.md with its unit', async () => {
  const report = flat(await readFile(benchmarks, 'utf8'));
  const { body } = await page();
  const text = flat(body);
  for (const figure of figures) {
    const [quoted, source] = Array.isArray(figure) ? figure : [figure, figure];
    assert.ok(text.includes(quoted), `page quotes ${quoted}`);
    assert.ok(report.includes(source), `Benchmarks.md states ${source}`);
  }
});

test('the reproduction commands are the ones Benchmarks.md lists', async () => {
  const report = await readFile(benchmarks, 'utf8');
  const { body } = await page();
  const lines = [...body.matchAll(/^```sh\n([\s\S]*?)^```/gm)].flatMap(match => match[1].split('\n')).filter(Boolean);
  assert.equal(lines.length, 5);
  for (const line of lines) {
    const listed = line.replace('./target/release/stress-test ', '$ST ');
    assert.ok(report.split('\n').includes(listed), line);
  }
});
