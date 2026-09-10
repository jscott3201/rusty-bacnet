import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import navigation from './src/data/navigation.json' with { type: 'json' };
import { satteri } from '@astrojs/markdown-satteri';
import { accessibleTables } from './scripts/accessible-content.mjs';

export default defineConfig({
  site: 'https://jscott3201.github.io',
  base: '/rusty-bacnet',
  trailingSlash: 'always',
  markdown: { processor: satteri({ hastPlugins: [accessibleTables] }) },
  integrations: [starlight({
    title: 'Rusty BACnet',
    description: 'Install, configure, and use Rusty BACnet with the CLI, Python, or Rust.',
    logo: { src: './src/assets/mark.svg' },
    favicon: '/favicon.svg',
    social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/jscott3201/rusty-bacnet' }],
    customCss: ['./src/styles/tokens.css', './src/styles/site.css', './src/styles/components.css'],
    components: { Footer: './src/components/DocsFooter.astro' },
    editLink: { baseUrl: 'https://github.com/jscott3201/rusty-bacnet/edit/dev/website/' },
    tableOfContents: { minHeadingLevel: 2, maxHeadingLevel: 3 },
    lastUpdated: false,
    sidebar: navigation,
  })],
});
