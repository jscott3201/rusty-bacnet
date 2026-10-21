import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import { plainBody } from '../../scripts/content.mjs';

// The terminal UI pages: their screenshots match the capture script's
// manifest, and the key reference matches the TUI's key map.
const root = new URL('../../', import.meta.url);
const repo = new URL('../', root);
const read = path => readFile(new URL(path, root), 'utf8');
const readRepo = path => readFile(new URL(path, repo), 'utf8');
const pagesDir = 'src/content/docs/';

async function pages() {
  const names = (await readdir(new URL(pagesDir, root), { recursive: true }))
    .map(name => name.replaceAll('\\', '/')).filter(name => /\.mdx?$/.test(name));
  return Promise.all(names.map(async name => ({ name, text: await read(pagesDir + name) })));
}

test('screenshots, the manifest and capture.py name the same shots', async () => {
  const manifest = JSON.parse(await read('src/assets/tui/manifest.json'));
  const names = manifest.shots.map(shot => shot.name);
  assert.equal(new Set(names).size, names.length, 'no shot is listed twice');
  for (const shot of manifest.shots) {
    assert.equal(shot.file, `${shot.name}.webp`);
    assert.ok(shot.alt.length > 20, `${shot.name} has alt text`);
  }
  const files = (await readdir(new URL('src/assets/tui/', root))).filter(name => name !== 'manifest.json');
  assert.deepEqual(files.sort(), manifest.shots.map(shot => shot.file).sort(), 'src/assets/tui holds exactly the manifest');
  const script = await read('scripts/tui-screenshots/capture.py');
  const shots = script.slice(script.indexOf('SHOTS = {'), script.indexOf('\n}\n', script.indexOf('SHOTS = {')));
  const scripted = [...shots.matchAll(/^ {4}"([a-z0-9-]+)": \(/gm)].map(match => match[1]);
  assert.deepEqual(scripted, names, 'capture.py SHOTS and manifest.json list the same shots in the same order');
});

test('every page screenshot is a manifest shot, and every shot is used', async () => {
  const manifest = JSON.parse(await read('src/assets/tui/manifest.json'));
  const known = new Set(manifest.shots.map(shot => shot.file));
  const used = new Set();
  for (const { name, text } of await pages()) {
    for (const match of text.matchAll(/!\[([^\]]*)\]\(([^)\s]+)\)/g)) {
      if (!match[2].includes('assets/tui/')) continue;
      assert.equal(match[2], `../../../assets/tui/${match[2].split('/').at(-1)}`, `${name}: ${match[2]}`);
      const file = match[2].split('/').at(-1);
      assert.ok(known.has(file), `${name} uses ${file}, which capture.py does not take`);
      assert.ok(match[1].length > 20, `${name}: ${file} has alt text`);
      used.add(file);
    }
  }
  assert.deepEqual([...known].filter(file => !used.has(file)), [], 'shots no page uses');
});

test('the plain-text export keeps a screenshot as its alt text', async () => {
  const body = await plainBody('Before.\n\n![The device table.](../../../assets/tui/discovery-devices.png)\n\n```md\n![kept](../../../assets/x.png)\n```\n', '/tmp/page.md', []);
  assert.ok(body.includes('*Screenshot: The device table.*'));
  assert.ok(body.includes('![kept](../../../assets/x.png)'), 'fenced code is left alone');
});

// keymap.rs's per-context functions, and the key reference section for each.
const sections = {
  main: 'Devices and the browser', watch: 'Watch list', network: 'Network screen', sc: 'BACnet/SC screen',
  mstp: 'MS/TP screen', capture: 'Capture screen', join: 'MS/TP join confirmation', connection: 'Profile dialog', picker: 'Interface picker',
  form: 'Who-Is form', filter: 'Filters', safety: 'Write dialogs', command_line: 'Command bar',
  global: 'Everywhere', help: 'Help overlay', browse: 'Devices and the browser',
};
const named = { Esc: 'Esc', Enter: 'Enter', Tab: 'Tab', BackTab: 'Shift-Tab', Up: 'Up', Down: 'Down', Left: 'Left',
  Right: 'Right', PageUp: 'PgUp', PageDown: 'PgDn', Home: 'Home', End: 'End', Backspace: 'Backspace', Delete: 'Delete' };

function keysOf(body) {
  const keys = new Set();
  for (const [, inner] of body.matchAll(/KeyCode::Char\(([^)]*)\)/g)) {
    for (const [, ch] of inner.matchAll(/'(.)'/g)) keys.add(ch === ' ' ? 'Space' : ch);
  }
  for (const [, name] of body.matchAll(/KeyCode::(\w+)/g)) if (named[name]) keys.add(named[name]);
  return keys;
}

test('the key reference lists every key the key map handles', async () => {
  const keymap = await readRepo('crates/bacnet-cli/src/tui/keymap.rs');
  const page = await read('src/content/docs/tui/keys.mdx');
  const section = title => {
    const start = page.indexOf(`\n## ${title}\n`);
    assert.notEqual(start, -1, `keys.mdx has a "${title}" section`);
    const end = page.indexOf('\n## ', start + 1);
    return page.slice(start, end === -1 ? undefined : end);
  };
  const functions = Object.fromEntries([...keymap.matchAll(/^fn (\w+)\([^)]*\)[^{]*\{([\s\S]*?)^\}/gm)].map(m => [m[1], m[2]]));
  // A screen's keys are a table of rows (#1926): a const the fn names, whose
  // rows may name shared consts (the movement keys) in turn.
  const consts = Object.fromEntries([...keymap.matchAll(/^(?:pub\(crate\) )?const (?!fn\b)(\w+)\b[^=]*=([\s\S]*?)^[\])];/gm)].map(m => [m[1], m[2]]));
  assert.ok(['GLOBAL', 'DEVICES', 'MOVE'].every(name => consts[name]), 'the key tables were found');
  const expand = (body, seen = new Set()) => body + [...body.replace(/"[^"]*"/g, '""').matchAll(/\b[A-Z][A-Z_]+\b/g)]
    .map(([name]) => name).filter(name => consts[name] && !seen.has(name) && seen.add(name))
    .map(name => expand(consts[name], seen)).join('\n');
  for (const [fn, title] of Object.entries(sections)) {
    assert.ok(functions[fn], `keymap.rs still has fn ${fn}`);
    const text = section(title);
    const missing = [...keysOf(expand(functions[fn]))].filter(key => !text.includes(`\`${key}\``));
    assert.deepEqual(missing, [], `keys.mdx "${title}" lists every key of keymap.rs fn ${fn}`);
  }
  assert.deepEqual(Object.keys(functions).filter(fn => !(fn in sections) && !['typed', 'movement', 'chorded', 'lookup', 'chord', 'screen'].includes(fn)), [],
    'every key-map context has a section');
});

test('the key reference lists every command-bar command', async () => {
  const source = await readRepo('crates/bacnet-cli/src/tui/app/command_bar.rs');
  // Each form in the COMMANDS table, which the parser reads.
  const table = source.slice(source.indexOf('pub(crate) const COMMANDS'), source.indexOf('\n];\n', source.indexOf('pub(crate) const COMMANDS')));
  const usage = [...table.matchAll(/usage: &\[([^\n]*)\],\n/g)].flatMap(m => [...m[1].matchAll(/"([^"]*)"/g)].map(f => f[1]));
  assert.ok(usage.length >= 6, 'the COMMANDS table was found');
  const page = await read('src/content/docs/tui/keys.mdx');
  for (const command of usage) {
    // "csv on|off" and "mstp rotation|hold MS" are listed one form a row.
    const [head, ...rest] = command.split(' ');
    for (const form of rest[0]?.includes('|') ? rest[0].split('|').map(word => [head, word, ...rest.slice(1)].join(' ')) : [command]) {
      assert.ok(page.includes(`\`${form}\``), `keys.mdx lists \`${form}\``);
    }
  }
});
