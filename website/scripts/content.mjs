// Plain-text projection of this site's small, explicit MDX component vocabulary.
// This is not a general MDX renderer and is not used to build the website HTML.
import { readFile } from 'node:fs/promises';
import { resolve, dirname } from 'node:path';
export function frontmatter(source) {
  const match = source.match(/^---\r?\n([\s\S]*?)\r?\n---\r?\n([\s\S]*)$/);
  if (!match) throw new Error('Missing frontmatter');
  const field = (name) => match[1].match(new RegExp(`^${name}:\\s*(.+)$`, 'm'))?.[1].replace(/^['"]|['"]$/g, '');
  return { title: field('title'), description: field('description') || '', body: match[2] };
}
const attr = (tag, name) => tag.match(new RegExp(`${name}="([^"]*)"`))?.[1] || '';
export async function plainBody(body, file, downloads) {
  const raws = new Map();
  for (const match of body.matchAll(/^import (\w+) from ['"]([^'"]+\?raw)['"];?$/gm)) {
    raws.set(match[1], await readFile(resolve(dirname(file), match[2].slice(0, -4)), 'utf8'));
  }
  let text = body.replace(/^import .*?;?\r?\n/gm, '');
  text = text.replace(/<Code\b[^>]*\/>/g, tag => {
    const name = tag.match(/code=\{(\w+)\}/)?.[1];
    if (!raws.has(name)) throw new Error(`Unknown raw code import in ${file}`);
    return `\n\n\`\`\`${attr(tag, 'lang')}\n${raws.get(name).trim()}\n\`\`\`\n\n`;
  });
  text = text.replace(/<DownloadList\s*\/>/g, () => downloads.map(item => `- [${item.os} · ${item.architecture}](${item.url}) — \`${item.file}\`; ${item.features}.`).join('\n'));
  text = text.replace(/<Diagram\b[^>]*\/>/g, tag => `\n\n![${attr(tag, 'description')}](/rusty-bacnet/diagrams/${attr(tag, 'file')})\n\n**${attr(tag, 'title')}**\n\n`);
  text = text.replace(/<NextStep\b[^>]*\/>/g, tag => `\n\n[Continue: ${attr(tag, 'label')}](${attr(tag, 'href')}) — ${attr(tag, 'description')}\n\n`);
  text = text.replace(/<Checkpoint\b[^>]*>/g, tag => `\n\n**${attr(tag, 'title') || 'What success looks like'}**\n\n`).replace(/<\/Checkpoint>/g, '');
  text = text.replace(/<TabItem\b[^>]*>/g, tag => `\n\n**${attr(tag, 'label')}**\n\n`).replace(/<\/?Tabs\b[^>]*>|<\/TabItem>/g, '');
  // Test remaining component tags only outside code fences.
  const outsideCode = text.replace(/^```[^\n]*\n[\s\S]*?^```\s*$/gm, '');
  if (/<\/?[A-Z][A-Za-z]*\b/.test(outsideCode)) throw new Error(`Unknown component in ${file}; update the plain-text projection.`);
  return text.replace(/\]\(\/rusty-bacnet\//g, '](https://jscott3201.github.io/rusty-bacnet/').trim();
}

export function isGuideFile(file) {
  const name = file.replaceAll('\\', '/').split('/').at(-1);
  return /\.mdx?$/.test(name) && !/^(?:index|404)\.mdx?$/.test(name);
}
