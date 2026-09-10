// Keep the production preview in Playwright's process group. Astro 7's CLI
// auto-backgrounds in agent environments, which breaks webServer ownership.
import { preview } from 'astro';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const port = Number(process.env.DOCS_TEST_PORT || 46327);
const server = await preview({ root, server: { host: '127.0.0.1', port } });
if (server.port !== port) {
  await server.stop();
  throw new Error(`Expected test port ${port}, got ${server.port}; refusing another server.`);
}
console.log(`Production test server: ${root} at http://127.0.0.1:${port}/rusty-bacnet/`);
let stopping = false;
async function stop() {
  if (stopping) return;
  stopping = true;
  await server.stop();
}
process.once('SIGTERM', stop);
process.once('SIGINT', stop);
