// The CLI's options and subcommands, read from crates/bacnet-cli/src/args.rs
// and its submodules (args/*.rs), so a page's `bacnet` samples fail a test
// when a flag or command they use is renamed or removed.
import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';

/** Read args.rs and args/*.rs: `options` maps each long option to whether it takes a value; `commands` holds the subcommands. */
export async function cliDefinitions() {
  const args = await readFile(new URL('../../../crates/bacnet-cli/src/args.rs', import.meta.url), 'utf8');
  const dir = new URL('../../../crates/bacnet-cli/src/args/', import.meta.url);
  const parts = (await readdir(dir)).filter(name => name.endsWith('.rs')).sort();
  const subs = await Promise.all(parts.map(name => readFile(new URL(name, dir), 'utf8')));
  return { options: cliOptions([args, ...subs].join('\n')), commands: cliCommands(args) };
}

// The long options, from the `#[arg(...)]` fields: name (`long = "..."` or
// the field name in kebab case) to whether it takes a value.
function cliOptions(args) {
  const options = new Map();
  const paren = String.raw`(?:[^()]|\((?:[^()]|\([^()]*\))*\))*`;
  const field = new RegExp(String.raw`#\[arg\((${paren})\)\]\s*(?:#\[[^\]]*\]\s*)*(?:pub\(crate\)\s+)?(\w+):\s*([^,\n]+)`, 'g');
  for (const [, attr, name, type] of args.matchAll(field)) {
    if (!/\blong\b/.test(attr)) continue;
    const long = attr.match(/\blong\s*=\s*"([^"]+)"/)?.[1] ?? name.replaceAll('_', '-');
    options.set(long, type.trim() !== 'bool');
  }
  return options;
}

// The subcommands: the variants of `enum Command`, in kebab case, and their aliases.
function cliCommands(args) {
  const start = args.indexOf('pub(crate) enum Command {');
  assert.notEqual(start, -1, 'args.rs defines enum Command');
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
