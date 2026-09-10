---
title: "CLI reference entry points"
description: "Find the right command family without confusing shell and one-shot behavior."
---

Use `bacnet --help` and the subcommand's help from the exact installed binary as the immediate reference for accepted syntax. This page is an orientation, not a separately maintained copy of every CLI option.

## Common command families

| Task | Command family | Operational note |
|---|---|---|
| Discover devices | `discover`, `devices` | `devices` is useful with a populated session cache |
| Find an object | `find --name` | One-shot form is parser-backed; shell grammar may differ |
| Read one or several properties | `read`, `readm` | Handle object/property errors individually |
| Read log/list ranges | `read-range` | Bound request size and result volume |
| Observe changes | `subscribe` | Creates remote subscription state |
| Write a value | `write` | Requires explicit site authorization and cleanup |
| Inspect BBMD tables | `bdt`, `fdt` | Table inspection is different from changing registration |
| Packet analysis | `capture` | Optional feature and platform prerequisites |
| Alarm/device/file operations | See the full reference | Not ordinary read-only onboarding |

## Names and aliases

The Cargo package is `bacnet-cli`; its executable is `bacnet`. Examples use `ai:1` for analog input 1, `av:1` for analog value 1, and `pv` for present value. Full object/property names are useful in teaching material; short aliases are convenient in an operator's terminal.

Do not use an object-name string where an object identifier is required. Do not assume a BACnet device instance is an IP address or that a discovered downstream device can be addressed by a router's IP alone.

## Session behavior matters

The interactive shell retains a default target and discovered devices for its session. A new one-shot process has a different lifecycle. The reviewed one-shot `DNET:instance` path rejects routed expressions; successful discovery of a network does not establish a routed read capability on that path.

## Machine-readable output

Use `--json` explicitly in automation. Preserve protocol errors and nonzero process outcomes rather than interpreting an empty result as an empty building. If scripts depend on particular JSON fields, add a small fixture test against the supported CLI version; a formatted example is not a schema guarantee.

## Full documentation

[Read the versioned CLI guide](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/docs/CLI.md), but resolve discrepancies against the parser, dispatch, and actual installed command help. The website migration should fix known conflicts rather than reproduce them on a prettier page.

## Sources and release scope

These instructions target **v0.11.0**. Source review is not a claim of hardware qualification.

[CLI documentation](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/docs/CLI.md) · [Argument source](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/crates/bacnet-cli/src/args.rs) · [Dispatch source](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/crates/bacnet-cli/src/main.rs).
