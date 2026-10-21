---
title: "CLI reference entry points"
description: "Find the right command family, and tell one-shot commands from the interactive front ends."
---

Use `bacnet --help` and the subcommand's help from the exact installed binary as the immediate reference for accepted syntax. This page is an orientation, not a separately maintained copy of every CLI option.

## Common command families

| Task | Command family | Operational note |
|---|---|---|
| Discover devices | `discover` | Use `discover --json` in scripts, or the terminal UI's device list |
| Find an object | `find --name` | The name goes in `--name` or as a plain word, `find "Zone Temp"` |
| Read one or several properties | `read`, `readm` | Handle object/property errors individually |
| Read log/list ranges | `read-range` | Bound request size and result volume |
| Observe changes | `subscribe` | Creates remote subscription state |
| Write a value | `write`, `writem` | Requires explicit site authorization, cleanup and `--allow-write` |
| Inspect BBMD tables | `bdt`, `fdt` | Table inspection is different from changing registration |
| Packet analysis | `capture` | In the Linux release executables; a source build needs the `pcap` feature |
| Join an MS/TP trunk | `--mstp PORT --mstp-mac N`, `mstp-ports` | In the Linux release executables; a source build needs the `serial` feature |
| Alarm/device/file operations | See the full reference | Not ordinary read-only onboarding |
| Interactive use | plain `bacnet` or `tui` | Plain `bacnet` on a terminal opens the terminal UI; the shell of earlier releases is gone |

## Exit status

| Status | Meaning |
|---|---|
| 0 | The command completed |
| 1 | It failed while running, for example a timeout or an error from the device |
| 2 | The arguments did not parse; nothing is sent. Also plain `bacnet` where the terminal UI can't open |
| 3 | `sc-check` could not complete |
| 4 | The write guard refused the command (`--allow-write` or `--yes` missing); nothing is sent |

## Names and aliases

The Cargo package is `bacnet-cli`; its executable is `bacnet`. Examples use `ai:1` for analog input 1, `av:1` for analog value 1, and `pv` for present value. Full object/property names are useful in teaching material; short aliases are convenient in an operator's terminal.

Do not use an object-name string where an object identifier is required. Do not assume a BACnet device instance is an IP address or that a discovered downstream device can be addressed by a router's IP alone.

## Session behavior matters

Plain `bacnet` opens the terminal UI when stdin and stdout are terminals; in a pipe or a script, with `--no-tui`, or with `BACNET_NO_TUI` set, it prints the usage and exits with status 2. Scripts that piped commands into the shell call the one-shot commands instead, with `--json`, which also read better with a screen reader than the terminal UI. `writem`, once a shell command, is a one-shot command. The [terminal UI guide](/rusty-bacnet/tui/overview/) covers its screens, and its [key reference](/rusty-bacnet/tui/keys/) lists every key and command.

A new one-shot process has its own lifecycle and keeps no discovered devices. One-shot `read`, `readm`, `write`, `writem` and `subscribe` take a routed `DNET:instance` or `DNET:0xMAC` target: the CLI asks that network for the device with a Who-Is and sends the request through the router that answers. Other one-shot commands refuse routed targets.

## Machine-readable output

Use `--json` explicitly in automation. Preserve protocol errors and nonzero process outcomes rather than interpreting an empty result as an empty building. If scripts depend on particular JSON fields, add a small fixture test against the supported CLI version; a formatted example is not a schema guarantee.

## Full documentation

[Read the versioned CLI guide](https://github.com/jscott3201/rusty-bacnet/blob/v0.14.0/docs/CLI.md), but resolve discrepancies against the parser, dispatch, and actual installed command help, and report any conflict you find.

## Sources and release scope

These instructions target **v0.14.0**. Source review is not a claim of hardware qualification.

[CLI documentation](https://github.com/jscott3201/rusty-bacnet/blob/v0.14.0/docs/CLI.md) · [Argument source](https://github.com/jscott3201/rusty-bacnet/blob/v0.14.0/crates/bacnet-cli/src/args.rs) · [Dispatch source](https://github.com/jscott3201/rusty-bacnet/blob/v0.14.0/crates/bacnet-cli/src/main.rs).
