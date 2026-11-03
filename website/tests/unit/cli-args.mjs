// The CLI's options and subcommands, read from crates/bacnet-cli/src/args.rs
// and its submodules (args/*.rs), so a page's `bacnet` samples fail a test
// when a flag or command they use is renamed or removed.
import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';

/**
 * Read args.rs and args/*.rs: `options` maps each long option to whether it
 * takes a value, `shorts` each short one; `commands` holds the subcommands,
 * and `mcpActions` the `bacnet mcp` subcommands.
 */
export async function cliDefinitions() {
  const args = await readFile(new URL('../../../crates/bacnet-cli/src/args.rs', import.meta.url), 'utf8');
  const dir = new URL('../../../crates/bacnet-cli/src/args/', import.meta.url);
  const parts = (await readdir(dir)).filter(name => name.endsWith('.rs')).sort();
  const subs = await Promise.all(parts.map(name => readFile(new URL(name, dir), 'utf8')));
  const all = [args, ...subs].join('\n');
  const mcp = subs[parts.indexOf('mcp.rs')] ?? '';
  return {
    options: cliOptions(all),
    shorts: cliShorts(all),
    commands: cliCommands(args),
    mcpActions: cliCommands(mcp, 'pub(crate) enum McpAction {'),
  };
}

const paren = String.raw`(?:[^()]|\((?:[^()]|\([^()]*\))*\))*`;
const field = new RegExp(String.raw`#\[arg\((${paren})\)\]\s*(?:#\[[^\]]*\]\s*)*(?:pub\(crate\)\s+)?(\w+):\s*([^,\n]+)`, 'g');

/** Whether an `#[arg(...)]` field of this type takes a value: not a flag or a count. */
function takesValue(attr, type) {
  return type.trim() !== 'bool' && !/ArgAction::Count/.test(attr);
}

// The long options, from the `#[arg(...)]` fields: name (`long = "..."` or
// the field name in kebab case) to whether it takes a value.
function cliOptions(args) {
  const options = new Map();
  for (const [, attr, name, type] of args.matchAll(field)) {
    if (!/\blong\b/.test(attr)) continue;
    const long = attr.match(/\blong\s*=\s*"([^"]+)"/)?.[1] ?? name.replaceAll('_', '-');
    options.set(long, takesValue(attr, type));
  }
  return options;
}

// The short options: `short = 'x'`, or a bare `short` (the field name's first letter).
function cliShorts(args) {
  const shorts = new Map();
  for (const [, attr, name, type] of args.matchAll(field)) {
    if (!/\bshort\b/.test(attr)) continue;
    const short = attr.match(/\bshort\s*=\s*'(.)'/)?.[1] ?? name[0];
    shorts.set(short, takesValue(attr, type));
  }
  return shorts;
}

// The subcommands: the variants of an enum (`enum Command` by default), in kebab case, and their aliases.
function cliCommands(args, header = 'pub(crate) enum Command {') {
  const start = args.indexOf(header);
  assert.notEqual(start, -1, `the CLI defines ${header}`);
  const body = args.slice(start, args.indexOf('\n}\n', start));
  const names = new Set();
  for (const [, variant] of body.matchAll(/^ {4}([A-Z]\w*)\s*(?:\{|,|\()/gm)) {
    names.add(variant.replace(/(?<!^)[A-Z]/g, letter => `-${letter}`).toLowerCase());
  }
  for (const [, alias] of body.matchAll(/#\[command\(alias = "([^"]+)"\)\]/g)) names.add(alias);
  return names;
}

/** Check one `bacnet ...` sample line: every long option exists, and the first operand is a subcommand. */
export function checkCliLine(line, { options, commands }, label) {
  const words = line.split(/\s+/);
  assert.equal(words[0], 'bacnet', `${label}: a CLI sample`);
  let command;
  for (let i = 1; i < words.length; i++) {
    const option = words[i].match(/^--([a-z][a-z0-9-]*)$/)?.[1];
    if (option) {
      assert.ok(options.has(option), `${label}: --${option} is a CLI option`);
      if (options.get(option)) i++;
    } else if (command === undefined) {
      command = words[i];
    }
  }
  assert.ok(commands.has(command), `${label}: ${command} is a CLI subcommand`);
}

/** Split a shell line into words, honouring single and double quotes. */
export function shellWords(line) {
  const words = [];
  for (const [, double, single, bare] of line.matchAll(/"((?:[^"\\]|\\.)*)"|'([^']*)'|(\S+)/g)) {
    words.push(double ?? single ?? bare);
  }
  return words;
}

/**
 * Check a whole `bacnet` command: every long and short option exists (and
 * its value is skipped), the first operand is a subcommand, and a `bacnet
 * mcp` command's second operand is one of its subcommands. A line with no
 * operand at all (`bacnet`, `bacnet --version`) needs no subcommand.
 */
export function checkCliWords(words, { options, shorts, commands, mcpActions }, label) {
  assert.equal(words[0], 'bacnet', `${label}: a CLI sample`);
  const operands = [];
  for (let i = 1; i < words.length; i++) {
    const word = words[i];
    const long = word.match(/^--([a-z][a-z0-9-]*)(=.*)?$/);
    if (long) {
      if (['help', 'version'].includes(long[1])) continue;
      assert.ok(options.has(long[1]), `${label}: --${long[1]} is a CLI option`);
      if (options.get(long[1]) && !long[2]) i++;
    } else if (/^-[A-Za-z]+$/.test(word)) {
      for (const letter of word.slice(1)) {
        if ('hV'.includes(letter)) continue;
        assert.ok(shorts.has(letter), `${label}: -${letter} is a CLI option`);
        if (shorts.get(letter)) i++;
      }
    } else {
      operands.push(word);
    }
  }
  if (operands.length === 0) return;
  assert.ok(commands.has(operands[0]), `${label}: ${operands[0]} is a CLI subcommand`);
  if (operands[0] === 'mcp') assert.ok(mcpActions.has(operands[1]), `${label}: mcp ${operands[1]} is a subcommand`);
}
