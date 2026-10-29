#!/usr/bin/env python3
"""Install only a benchmark bootstrap into an A/B checkout; never modify production."""
import argparse
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument("historical", type=Path)
parser.add_argument("--adapter", choices=("v013", "current"), required=True)
args = parser.parse_args()
candidate = Path(__file__).resolve().parents[2]
source = candidate / "benchmarks/src/stress"
modules = ["client_diagnostics", "client_counters", "client_measure", "client_run", "client_bip", "output"]
lines = ["//! Recorded #1979 bootstrap; production sources remain unchanged.",
         "#![allow(dead_code, missing_docs, clippy::print_stdout, clippy::print_stderr)]",
         "#![forbid(unsafe_code)]", "mod stress {"]
for module in modules:
    lines.append(f'#[path = "{source / (module + ".rs")}"] pub mod {module};')
if args.adapter == "current":
    lines.append(f'#[path = "{source / "harness.rs"}"] pub mod harness;')
    adapter = source / "client_adapter.rs"
else:
    adapter = candidate / "benchmarks/compat/v013_adapter.rs"
lines.extend([f'#[path = "{adapter}"] pub mod client_adapter;', "}",
              f'include!("{candidate / "benchmarks/compat/normalized_main.rs"}");'])
target = args.historical.resolve() / "benchmarks/src/bin/normalized-clients.rs"
if target.exists():
    raise SystemExit(f"Refusing to overwrite {target}; inspect/remove the prior bootstrap explicitly")
target.write_text("\n".join(lines) + "\n")
print(target)
