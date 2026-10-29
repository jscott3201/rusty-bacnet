// Generate the CLI reference from the clap definitions (#1989).
//
// The generator is the `args::reference::cli_reference` test in bacnet-cli
// (crates/bacnet-cli/src/args/reference.rs): it renders the commands and
// options with clap's own API and rewrites the generated region of
// src/content/docs/reference/cli.md. This runs it with every feature on, so a
// command behind a feature is listed too.
//
//   node website/scripts/reference-cli.mjs          # rewrite the page
//   node website/scripts/reference-cli.mjs --check  # exit 1 if stale
//
// Needs the Rust toolchain, and libpcap for the `pcap` feature.
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const repo = fileURLToPath(new URL('../../', import.meta.url));
const mode = process.argv.includes('--check') ? 'check' : 'write';
const result = spawnSync('cargo', ['test', '--locked', '-p', 'bacnet-cli', '--all-features', '--bin', 'bacnet', '--',
  '--exact', 'args::reference::cli_reference'], {
  cwd: repo,
  stdio: 'inherit',
  env: { ...process.env, BACNET_CLI_REFERENCE: mode },
});
if (result.error) throw result.error;
if (result.status !== 0) {
  console.error(mode === 'check'
    ? 'website/src/content/docs/reference/cli.md is stale (or the generator failed); run node website/scripts/reference-cli.mjs'
    : 'the CLI reference generator failed');
}
process.exit(result.status ?? 1);
