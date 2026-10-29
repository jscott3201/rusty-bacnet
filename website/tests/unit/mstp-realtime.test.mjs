import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { frontmatter, plainBody } from '../../scripts/content.mjs';
import { release } from '../../src/lib/site.mjs';
import { repo, splitDevSection } from './dev-section.mjs';
import { checkCliLine, cliDefinitions } from './cli-args.mjs';

const file = fileURLToPath(new URL('../../src/content/docs/connect/mstp-realtime.mdx', import.meta.url));

async function page() {
  const source = await readFile(file, 'utf8');
  const { title, description, body } = frontmatter(source);
  return { title, description, body: await plainBody(body, file, []) };
}

test('MS/TP real-time page is release-scoped, with the baud scan in the monitor section', async () => {
  const { title, description, body } = await page();
  assert.ok(title && description);
  assert.ok(body.includes(`The page describes **v${release}**.`));
  assert.ok(!body.includes('on-dev-after-the-release'));
  assert.ok(body.includes('\n## Next steps\n'));
  const { released, onDev } = splitDevSection(body, 'connect/mstp-realtime');
  assert.equal(onDev, '', 'no dev section');
  const monitor = released.slice(released.indexOf('\n## Diagnose with the receive-only monitor\n'), released.indexOf('\n## Test without hardware\n'));
  assert.ok(monitor.includes(':mstp scan SECS'), 'the baud scan is in the monitor section');
  for (const path of [
    'docs/mstp-qualification.md',
    'docs/rust-api.md#real-time-scheduling-and-cpu-affinity-linux',
    'docs/rust-api.md#mstp-serial-rs-485',
    'docs/CLI.md#mstp',
    'docs/CLI.md#mstp-monitor-and-safe-join',
    'Benchmarks.md#44-mstp',
  ]) assert.ok(released.includes(`${repo}/blob/v${release}/${path}`), path);
  // No qualification claim: the open hardware issues stay named.
  for (const issue of [501, 502, 707]) assert.ok(released.includes(`${repo}/issues/${issue}`), `#${issue}`);
  assert.ok(released.includes('No adapter or host is qualified for on-wire timing yet.'));
});

test('every CLI option and subcommand in an MS/TP real-time sample is defined by the CLI', async () => {
  const cli = await cliDefinitions();
  for (const flag of ['mstp', 'mstp-mac', 'mstp-baud', 'mstp-rt-priority', 'mstp-cpus', 'mstp-monitor']) {
    assert.equal(cli.options.get(flag), true, `--${flag} takes a value`);
  }
  const { body } = await page();
  // The page's shell blocks also hold sysfs and cargo commands; check the CLI's.
  const lines = [...body.matchAll(/^```sh\n([\s\S]*?)^```/gm)]
    .flatMap(match => match[1].split('\n'))
    .filter(line => line.startsWith('bacnet '));
  assert.equal(lines.length, 2, 'the participant and monitor samples');
  for (const line of lines) checkCliLine(line, cli, 'connect/mstp-realtime');
  // The ring flags the page names in prose exist too.
  for (const flag of ['mstp-max-master', 'mstp-max-info-frames']) {
    assert.ok(body.includes(`\`--${flag}\``) && cli.options.has(flag), flag);
  }
});

// The host timing figures on the page come from constants in the source; a
// changed constant fails here until the page says the new value.
test('the page states the host timing constants the source defines', async () => {
  const read = path => readFile(new URL(`../../../crates/${path}`, import.meta.url), 'utf8');
  const transport = await read('bacnet-transport/src/mstp/mod.rs');
  const monitor = await read('bacnet-transport/src/mstp/monitor.rs');
  const core = await read('bacnet-mstp-core/src/config.rs');
  // `pub const NAME: ... from_millis(N)` in `source`, or the core constant it names.
  const millis = (source, name) => {
    const match = source.match(new RegExp(String.raw`pub const ${name}:[^;]*?(?:from_millis\((\d+)\)|bacnet_mstp_core::(\w+))`));
    assert.ok(match, `${name} is defined`);
    return match[1] !== undefined ? Number(match[1]) : millis(core, match[2]);
  };
  const usage = millis(transport, 'HOST_USAGE_TIMEOUT');
  const reply = millis(transport, 'HOST_REPLY_TIMEOUT');
  const late = millis(transport, 'HOST_LATE_DEADLINE');
  const pad = millis(transport, 'USB_TRANSMIT_LATENCY_PAD');
  const stamps = millis(monitor, 'DEFAULT_TIMESTAMP_RESOLUTION');
  const abort = millis(core, 'MAX_FRAME_ABORT');
  const { body } = await page();
  for (const text of [
    `the longest values the standard allows, ${usage} ms and ${reply} ms`,
    `It counts a wake ${late} ms or more late as a late deadline.`,
    `by default the station adds ${pad} ms to each frame's time`,
    `only as fine as the adapter's latency (${stamps} ms by default)`,
    `waits up to its ${abort} ms maximum before it drops a partial frame`,
  ]) assert.ok(body.includes(text), text);
});
