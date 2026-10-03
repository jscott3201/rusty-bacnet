// Plain-text projection of this site's small, explicit MDX component vocabulary.
// This is not a general MDX renderer and is not used to build the website HTML.
import { readFile } from 'node:fs/promises';
import { resolve, dirname } from 'node:path';
import { release, repository, releaseAsset } from '../src/lib/site.mjs';
export function frontmatter(source) {
  const match = source.match(/^---\r?\n([\s\S]*?)\r?\n---\r?\n([\s\S]*)$/);
  if (!match) throw new Error('Missing frontmatter');
  const field = (name) => match[1].match(new RegExp(`^${name}:\\s*(.+)$`, 'm'))?.[1].replace(/^['"]|['"]$/g, '');
  return { title: field('title'), description: field('description') || '', body: match[2] };
}
const attr = (tag, name) => tag.match(new RegExp(`${name}="([^"]*)"`))?.[1] || '';
const fence = /^```[^\n]*\n[\s\S]*?^```\s*$/gm;
// The only values an MDX page may interpolate: {release} in text, and
// ${release} or ${repository} in a template literal given to Code's `code` or
// a link's `href`.
const constants = { release, repository };
function constant(name, file) {
  if (!Object.hasOwn(constants, name)) throw new Error(`Unknown expression ${name} in ${file}; update the plain-text projection.`);
  return constants[name];
}
function expandTemplate(body, raw, file) {
  // Outside String.raw, JavaScript reads a backslash as an escape, which this projection doesn't.
  if (!raw && body.includes('\\')) throw new Error(`Use String.raw for a template with backslashes in ${file}`);
  return body.replace(/\$\{([^}]*)\}/g, (_, name) => constant(name, file));
}
// MDX evaluates expressions in text, not in fenced code or code spans.
function outsideFences(text, fn) {
  let out = '';
  let last = 0;
  for (const match of text.matchAll(fence)) {
    out += fn(text.slice(last, match.index)) + match[0];
    last = match.index + match[0].length;
  }
  return out + fn(text.slice(last));
}
const outsideSpans = (text, fn) => text.split(/(`[^`\n]*`)/).map((part, i) => i % 2 ? part : fn(part)).join('');
export async function plainBody(body, file, downloads) {
  const mdx = file.endsWith('.mdx');
  const raws = new Map();
  for (const match of body.matchAll(/^import (\w+) from ['"]([^'"]+\?raw)['"];?$/gm)) {
    raws.set(match[1], await readFile(resolve(dirname(file), match[2].slice(0, -4)), 'utf8'));
  }
  let text = body.replace(/^import .*?;?\r?\n/gm, '');
  if (mdx) {
    text = text.replace(/<Code\b[^`>]*code=\{(String\.raw)?`([^`]*)`\}[^`>]*\/>/g, (tag, raw, code) =>
      `\n\n\`\`\`${attr(tag, 'lang')}\n${expandTemplate(code, raw, file).trim()}\n\`\`\`\n\n`);
  }
  text = text.replace(/<Code\b[^>]*\/>/g, tag => {
    const name = tag.match(/code=\{(\w+)\}/)?.[1];
    if (!raws.has(name)) throw new Error(`Unknown raw code import in ${file}`);
    return `\n\n\`\`\`${attr(tag, 'lang')}\n${raws.get(name).trim()}\n\`\`\`\n\n`;
  });
  text = text.replace(/<DownloadList\s*\/>/g, () => downloads.map(item => `- [${item.os} · ${item.architecture}](${releaseAsset(item.file)}) — \`${item.file}\`; ${item.features}; ${item.runtime}.`).join('\n'));
  text = text.replace(/<Diagram\b[^>]*\/>/g, tag => `\n\n![${attr(tag, 'description')}](/rusty-bacnet/diagrams/${attr(tag, 'file')})\n\n**${attr(tag, 'title')}**\n\n`);
  text = text.replace(/<NextStep\b[^>]*\/>/g, tag => `\n\n[Continue: ${attr(tag, 'label')}](${attr(tag, 'href')}) — ${attr(tag, 'description')}\n\n`);
  text = text.replace(/<Checkpoint\b[^>]*>/g, tag => `\n\n**${attr(tag, 'title') || 'What success looks like'}**\n\n`).replace(/<\/Checkpoint>/g, '');
  text = text.replace(/<TabItem\b[^>]*>/g, tag => `\n\n**${attr(tag, 'label')}**\n\n`).replace(/<\/?Tabs\b[^>]*>|<\/TabItem>/g, '');
  if (mdx) {
    // Links first: their template literals look like code spans.
    text = outsideFences(text, part => outsideSpans(part
      .replace(/<a href=\{(String\.raw)?`([^`]*)`\}>([^<]*)<\/a>/g, (_, raw, href, label) => `[${label}](${expandTemplate(href, raw, file)})`),
    span => span
      .replace(/\{(\w+)\}/g, (_, name) => constant(name, file))
      .replace(/<code>([^<`]*)<\/code>/g, '`$1`')));
  }
  // Test remaining component tags and expressions only outside code fences.
  const outsideCode = text.replace(fence, '');
  if (/<\/?[A-Z][A-Za-z]*\b/.test(outsideCode)) throw new Error(`Unknown component in ${file}; update the plain-text projection.`);
  if (mdx && /=\{|<\/?(?:a|code)\b/.test(outsideCode)) throw new Error(`Unprojected expression or element in ${file}; update the plain-text projection.`);
  return text.replace(/\]\(\/rusty-bacnet\//g, '](https://jscott3201.github.io/rusty-bacnet/').trim();
}

export function isGuideFile(file) {
  const name = file.replaceAll('\\', '/').split('/').at(-1);
  return /\.mdx?$/.test(name) && !/^(?:index|404)\.mdx?$/.test(name);
}
