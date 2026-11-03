// Every `bacnet` command a page or the README displays is checked against
// the CLI's definition in crates/bacnet-cli/src/args*.rs (#1989): its long
// and short options exist, and its subcommand does. The Python site-sample
// tests also run each one (or its --help) against the built CLI; this check
// needs no build, so Docs validation sees a renamed flag too.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import { checkCliWords, cliDefinitions, shellWords } from './cli-args.mjs';

const docs = new URL('../../src/content/docs/', import.meta.url);
const readme = new URL('../../../README.md', import.meta.url);
const generatedStart = /^(?:<!--|\{\/\*) generated:start\b/;
const generatedEnd = /^(?:<!-- generated:end -->|\{\/\* generated:end \*\/\})$/;
// Words that may run `bacnet` for the reader: a login or an agent host.
const wrappers = new Set(['ssh', 'claude', 'sudo']);

/** Every docs page, by its path under the docs directory, and the README. */
async function pages() {
  const names = (await readdir(docs, { recursive: true })).filter(name => /\.mdx?$/.test(name)).sort();
  const found = await Promise.all(names.map(async name => [name, await readFile(new URL(name, docs), 'utf8')]));
  return [...found, ['README.md', await readFile(readme, 'utf8')]];
}

/** The fenced blocks of a page outside generated reference regions: language, first line number, body. */
function fences(source) {
  const blocks = [];
  let fence = null;
  let generated = false;
  source.split('\n').forEach((line, index) => {
    const stripped = line.trim();
    if (!fence && generatedStart.test(stripped)) generated = true;
    else if (!fence && generatedEnd.test(stripped)) generated = false;
    else if (generated) return;
    else if (fence && stripped === '```') {
      blocks.push(fence);
      fence = null;
    } else if (fence) fence.lines.push(line);
    else if (stripped.startsWith('```')) fence = { lang: stripped.slice(3).trim(), line: index + 1, lines: [] };
  });
  return blocks;
}

/** The `bacnet` commands in a block, as words: shell lines (continuations joined), or an MCP host's servers. */
function commands({ lang, lines }) {
  if (lang === 'json') {
    const servers = JSON.parse(lines.join('\n')).mcpServers ?? {};
    return Object.values(servers).map(({ command, args }) => [command, ...args]);
  }
  if (!['sh', 'bash', 'shell', ''].includes(lang)) return [];
  const found = [];
  for (const line of lines.join('\n').replace(/\\\n/g, ' ').split('\n')) {
    const words = shellWords(line.trim());
    if (words.length === 0 || words[0].startsWith('#')) continue;
    const at = words.indexOf('bacnet');
    if (at < 0) continue;
    const before = words.slice(0, at);
    if (before.every(word => /^[A-Z_][A-Z0-9_]*=/.test(word)) || wrappers.has(before[0])) {
      found.push(words.slice(at));
    }
  }
  return found;
}

test('every displayed bacnet command uses options and subcommands the CLI defines', async () => {
  const cli = await cliDefinitions();
  let checked = 0;
  for (const [name, source] of await pages()) {
    for (const block of fences(source)) {
      for (const words of commands(block)) {
        checkCliWords(words, cli, `${name}:${block.line}: ${words.join(' ')}`);
        checked++;
      }
    }
  }
  // Every page's commands, not a vacuous pass: there are about seventy.
  assert.ok(checked >= 60, `only ${checked} commands found`);
});

test('the check refuses a renamed option, short option or subcommand', async () => {
  const cli = await cliDefinitions();
  for (const line of [
    'bacnet --foreign-bbmd 192.168.1.10 --tll 600 discover',
    'bacnet -x 127.0.0.1 read 127.0.0.1 ai:1 pv',
    'bacnet --port 47810 raed 192.168.1.100 ai:1 pv',
    'bacnet mcp serv --stdio',
  ]) {
    assert.throws(() => checkCliWords(shellWords(line), cli, line), assert.AssertionError, line);
  }
  for (const line of ['bacnet', 'bacnet --version', 'bacnet -i 127.0.0.1 -vv --port=0 read 1234 ai:1 pv']) {
    checkCliWords(shellWords(line), cli, line);
  }
});
