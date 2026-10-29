import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { frontmatter, plainBody } from '../../scripts/content.mjs';
import { release } from '../../src/lib/site.mjs';
import { repo, splitDevSection } from './dev-section.mjs';
import { checkCliLine, cliDefinitions } from './cli-args.mjs';

const root = new URL('../../', import.meta.url);
const read = path => readFile(new URL(path, root), 'utf8');
const readCrate = path => readFile(new URL(`../crates/${path}`, root), 'utf8');
const site = 'https://jscott3201.github.io/rusty-bacnet/';

async function page(slug) {
  const file = fileURLToPath(new URL(`src/content/docs/${slug}.mdx`, root));
  const source = await readFile(file, 'utf8');
  const { title, description, body } = frontmatter(source);
  return { source, title, description, body: await plainBody(body, file, []) };
}

const fences = (text, lang) => [...text.matchAll(new RegExp(`^\`\`\`${lang}\\n([\\s\\S]*?)^\`\`\``, 'gm'))].map(match => match[1]);
const cliLines = text => fences(text, 'sh').flatMap(block => block.split('\n')).filter(line => line.startsWith('bacnet '));

// The page imports each example file once, by this exact line.
function assertImports(source, imports) {
  for (const [name, path] of imports) {
    assert.equal(source.split('\n').filter(line => line === `import ${name} from '../../../../examples/${path}?raw';`).length, 1, path);
  }
}

test('server page shows the exact release-pinned programs and no dev section', async () => {
  const { source, title, description, body } = await page('integrate/server');
  assert.ok(title && description);
  assertImports(source, [
    ['serverSource', 'rust/server-device/src/main.rs'],
    ['serverManifest', 'rust/server-device/Cargo.toml'],
    ['serverPython', 'python/server_device.py'],
  ]);
  const program = await read('examples/rust/server-device/src/main.rs');
  const manifest = await read('examples/rust/server-device/Cargo.toml');
  const python = await read('examples/python/server_device.py');
  assert.deepEqual(fences(body, 'rust'), [program], 'raw source bytes, including the final newline');
  assert.deepEqual(fences(body, 'toml'), [manifest]);
  assert.deepEqual(fences(body, 'python'), [python]);
  for (const crate of ['bacnet-objects', 'bacnet-server', 'bacnet-transport', 'bacnet-types']) {
    assert.ok(manifest.includes(`${crate} = "=${release}"`), `${crate} follows the site release`);
  }
  assert.ok(manifest.includes('rust-version = "1.95"') && body.includes('minimum Rust version is **1.95**'));
  assert.ok(body.includes(`This page builds one on BACnet/IP in Rust and in Python and describes **v${release}**.`));
  assert.ok(body.includes(`These files target **v${release}**`));
  assert.ok(body.includes(`with the v${release} \`rusty-bacnet\` package`));
  const { onDev } = splitDevSection(body, 'integrate/server');
  assert.equal(onDev, '', 'everything on the page is in the release');
  for (const path of ['examples/rust/server-device/Cargo.toml', 'examples/rust/server-device/src/main.rs', 'examples/server_device.py']) {
    assert.ok(body.includes(`${site}${path}`), path);
  }
  for (const heading of [
    'Build a server', 'Example: a small air handler', 'Supply values from your application',
    'Act on commands from the network', 'Serve COV subscriptions', 'Know the limits',
    'Describe the device in a PICS', 'Stop the server', 'Read the contracts', 'Next steps',
  ]) assert.ok(body.includes(`\n## ${heading}\n`), heading);
  // What the page says the programs print, they print.
  for (const line of ['Serving device 1001 at ', 'Press Enter to stop (or send EOF).', 'Supply fan: on', 'Supply fan: off', 'Stopped']) {
    assert.ok(body.includes(line), line);
  }
  for (const text of [program, python]) {
    for (const piece of ['Serving device ', 'Press Enter to stop (or send EOF).', 'Supply fan: ', 'Stopped', '1001', '21.5', '19.0']) {
      assert.ok(text.includes(piece), piece);
    }
  }
  // The tests the page names exist.
  assert.ok(body.includes('`server_device` integration tests') && body.includes('`test_website_server.py`'));
  await readCrate('bacnet-integration-tests/tests/server_device.rs');
  await readCrate('rusty-bacnet/tests/test_website_server.py');
});

test('every CLI sample on the server page is defined by the CLI', async () => {
  const cli = await cliDefinitions();
  const { body } = await page('integrate/server');
  const lines = cliLines(body);
  assert.equal(lines.length, 4, 'two reads and two writes');
  for (const line of lines) checkCliLine(line, cli, 'integrate/server');
  assert.equal(lines.filter(line => line.includes(' write ')).every(line => line.includes('--allow-write')), true);
});

// The limits the server page states come from the source; a changed default
// fails here until the page says the new value.
test('the server page states the limits the source defines', async () => {
  const policy = await readCrate('bacnet-server/src/cov/policy.rs');
  const defaults = policy.slice(policy.indexOf('impl Default for CovPolicy'));
  const field = name => Number(defaults.match(new RegExp(`${name}: (\\d+),`))[1]);
  const create = await readCrate('bacnet-server/src/handlers/object_mgmt.rs');
  const objects = Number(create.match(/const MAX_OBJECTS: usize = ([\d_]+);/)[1].replaceAll('_', ''));
  const { body } = await page('integrate/server');
  const grouped = n => n.toLocaleString('en-US');
  for (const row of [
    `| COV subscriptions, all peers | ${grouped(field('max_subscriptions_global'))} |`,
    `| COV subscriptions, one peer | ${field('max_subscriptions_per_peer')} |`,
    `| Objects created by CreateObject | Until the database holds ${grouped(objects)} objects |`,
  ]) assert.ok(body.includes(row), row);
});

test('router page shows the release-pinned programs, the Python router and no dev section', async () => {
  const { source, title, description, body } = await page('integrate/router');
  assert.ok(title && description);
  assert.ok(body.includes(`The page describes **v${release}**.`));
  assert.ok(!body.includes('on-dev-after-the-release'));
  assertImports(source, [
    ['routerSource', 'rust/bip-router/src/main.rs'],
    ['routerManifest', 'rust/bip-router/Cargo.toml'],
    ['routerPython', 'python/bip_router.py'],
  ]);
  const program = await read('examples/rust/bip-router/src/main.rs');
  const manifest = await read('examples/rust/bip-router/Cargo.toml');
  const python = await read('examples/python/bip_router.py');
  const { onDev } = splitDevSection(body, 'integrate/router');
  assert.equal(onDev, '', 'everything on the page is in the release');
  assert.deepEqual(fences(body, 'rust'), [program]);
  assert.deepEqual(fences(body, 'toml'), [manifest]);
  assert.deepEqual(fences(body, 'python'), [python]);
  // The manifest pins the site release, and the page says so.
  for (const crate of ['bacnet-client', 'bacnet-network', 'bacnet-server', 'bacnet-transport', 'bacnet-types']) {
    assert.ok(manifest.includes(`${crate} = "=${release}"`), `${crate} follows the site release`);
  }
  assert.ok(!manifest.includes('branch = '));
  assert.ok(manifest.includes('rust-version = "1.95"'));
  assert.ok(body.includes(`The Rust manifest pins **v${release}**, whose minimum Rust version is **1.95**.`));
  assert.ok(body.includes(`The Python program runs with the v${release} \`rusty-bacnet\` package`));
  assert.ok(program.includes('.ports()') && program.includes('stopped?;'));
  for (const path of ['examples/rust/bip-router/Cargo.toml', 'examples/rust/bip-router/src/main.rs', 'examples/bip_router.py']) {
    assert.ok(body.includes(`${site}${path}`), path);
  }
  // The output block is what the programs print, as their tests check.
  const output = fences(body, 'text');
  assert.equal(output.length, 1);
  const expected = 'Router started: networks 1 and 5\nRouter to network 5 found, reaching networks [5]\n'
    + 'Device 1005 found on network 5\nRead through the router: 8.5\nStopped\n';
  assert.equal(output[0], expected);
  const rustTest = await readCrate('bacnet-integration-tests/tests/bip_router_guide.rs');
  const pythonTest = await readCrate('rusty-bacnet/tests/test_website_router.py');
  for (const line of expected.trimEnd().split('\n')) {
    assert.ok(rustTest.includes(line) && pythonTest.includes(`"${line}"`), line);
  }
  // Names that came with the router changes are in the page body.
  for (const name of ['BipRouterPort', 'RouterStopError', 'router.ports()', 'originate', 'into_application_ports()', 'application="client_and_server"', 'file-read', 'alarm-summary', 'enrollment-summary']) {
    assert.ok(body.includes(name), name);
  }
  for (const heading of ['A router in Python', 'Stop the router', 'Example: two B/IP networks on one host', 'Read the contracts']) {
    assert.ok(body.includes(`\n## ${heading}\n`), heading);
  }
  for (const path of ['docs/rust-api.md#bacnet-network', 'docs/rust-api.md#who-may-change-routes-518', 'docs/CLI.md', 'docs/python-api.md#bip-router']) {
    assert.ok(body.includes(`${repo}/blob/v${release}/${path}`), path);
  }
  assert.ok(body.includes('`router.stop().await` returns `Result<(), RouterStopError>`'));
});

test('every CLI sample on the router page is defined by the CLI', async () => {
  const cli = await cliDefinitions();
  assert.ok(cli.commands.has('whois-router') && cli.options.get('dnet') && cli.options.get('wait'));
  const { body } = await page('integrate/router');
  const lines = cliLines(body);
  assert.equal(lines.length, 4);
  for (const line of lines) checkCliLine(line, cli, 'integrate/router');
});

test('the router page states the limits the source defines', async () => {
  const table = await readCrate('bacnet-network/src/router_table.rs');
  const routes = Number(table.match(/pub const ROUTE_CAP: usize = (\d+);/)[1]);
  const queue = await readCrate('bacnet-network/src/router/send_queue.rs');
  const queued = Number(queue.match(/const CAPACITY: usize = (\d+);/)[1]);
  const start = await readCrate('bacnet-network/src/router/mod.rs');
  const ports = Number(start.match(/Fails if there are no ports or more than (\d+)/)[1]);
  const { body } = await page('integrate/router');
  for (const row of [`| Ports | 1 to ${ports} |`, `| Routes, direct ones included | ${routes} |`, `| Sends queued per port | ${queued} |`]) {
    assert.ok(body.includes(row), row);
  }
});
