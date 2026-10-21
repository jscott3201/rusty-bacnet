import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import navigation from '../../src/data/navigation.json' with { type: 'json' };

// The embedded page (#1771): where it sits, what routes to it, and sizes that
// follow the firmware link test's budgets.
const root = new URL('../../', import.meta.url);
const repo = new URL('../', root);
const read = path => readFile(new URL(path, root), 'utf8');
const page = () => read('src/content/docs/start/embedded.md');

// The `[table]` sections of a budget file, each a map of integer keys.
const budgets = text => {
  const tables = {};
  let current;
  for (const line of text.split('\n')) {
    const table = line.match(/^\[([^\]]+)\]$/);
    if (table) tables[current = table[1]] = {};
    const entry = line.match(/^([\w-]+) = (\d+)$/);
    if (entry && current) tables[current][entry[1]] = Number(entry[2]);
  }
  return tables;
};

test('the embedded page follows the Rust page and choose-your-path links to it', async () => {
  const start = navigation.find(group => group.label.startsWith('Start with'));
  const slugs = start.items.map(item => item.slug);
  assert.equal(slugs.indexOf('start/embedded'), slugs.indexOf('start/rust') + 1);
  const choose = await read('src/content/docs/start/choose-your-path.md');
  const section = choose.split('\n## ').find(part => part.startsWith('Run on a microcontroller\n'));
  assert.ok(section, 'choose-your-path has a "Run on a microcontroller" section');
  assert.ok(section.includes('](/rusty-bacnet/start/embedded/)'));
});

test('the embedded page covers crates, the clock, sizing, adapters and alloc', async () => {
  const text = await page();
  for (const heading of ['Crates by product', 'One clock for every core', 'Storage sizing', 'Adapter rules', 'What still needs an allocator']) {
    assert.ok(text.includes(`\n## ${heading}\n`), heading);
  }
  for (const product of ['MS/TP device', 'B/IP device', 'B/IP BBMD', 'Router between B/IP and MS/TP']) {
    assert.match(text, new RegExp(`^\\| ${product} \\|`, 'm'), product);
  }
  for (const value of ['`bacnet-mstp-core`', '`bacnet-bip-core`', '`bacnet-router-core`', '`bacnet-npdu`', '`default-features = false`',
    'bacnet_time::earliest', '`Option::min`', '`on_transmit_started`', '`on_transmit_complete`', '`complete_request`', '`take_counters`',
    'examples/node.rs', 'examples/bip_mstp_router.rs']) assert.ok(text.includes(value), value);
});

test('the sizes on the embedded page are within the firmware budgets', async () => {
  const text = await page();
  const size = budgets(await readFile(new URL('firmware/link/size-budget.toml', repo), 'utf8'));
  const stack = budgets(await readFile(new URL('firmware/link/stack-budget.toml', repo), 'utf8'));
  const rows = [...text.matchAll(/^\| `([\w-]+)` \| [^|]+ \| ([\d,]+) \| ([\d,]+) \| ([\d,]+) \|$/gm)];
  assert.deepEqual(rows.map(row => row[1]), ['mstp', 'mstp-extended', 'node', 'bbmd', 'device', 'router']);
  const number = cell => Number(cell.replaceAll(',', ''));
  for (const [, image, flash, ram, depth] of rows) {
    const budget = size[`thumbv6m-none-eabi.${image}`];
    // Measured, so within the budget and no further under it than the check
    // allows before it asks for a tighter budget (20%).
    for (const [cited, limit] of [[number(flash), budget.flash], [number(ram), budget.ram]]) {
      assert.ok(cited <= limit && cited >= limit * 0.8, `${image}: ${cited} against a budget of ${limit}`);
    }
    // The router doesn't fit the micro:bit, so its stack is the LM3S6965's.
    const board = image === 'router' ? 'lm3s6965evb' : 'microbit';
    assert.equal(number(depth), stack[board][image], `${image}'s stack budget on ${board}`);
  }
});
