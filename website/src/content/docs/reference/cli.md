---
title: "CLI reference"
description: "Every bacnet command and option with its defaults, generated from the CLI's own definitions, plus exit codes, targets and session behavior."
---

The `bacnet` executable (Cargo package `bacnet-cli`) runs one command and exits, or opens the terminal UI. The command and option lists below are generated from the definitions `bacnet --help` prints. Use `bacnet <command> --help` for the binary you actually have.

:::note[On `dev` after the release]
This page is generated from the `dev` branch and tracks the next release, so it can list commands and options merged after v0.15.0 that the released `bacnet` doesn't have. The [v0.15.0 release notes](https://github.com/jscott3201/rusty-bacnet/releases/tag/v0.15.0) and the [changes waiting for the next release](https://github.com/jscott3201/rusty-bacnet/tree/dev/changelog.d) say what changed.
:::

## Features and release executables

Some commands and options need a Cargo feature; their entries below say which. The macOS and Windows release executables carry the terminal UI (`tui`) and BACnet/SC (`sc-tls`); the Linux ones add MS/TP (`serial`) and packet capture (`pcap`). Every release executable also carries `mcp`, the [MCP server for AI agent hosts](/rusty-bacnet/guides/ai-agents/); a source build enables it with `--features mcp`. A source build turns on `tui` by default and the others with `cargo install bacnet-cli --features ...`. A command whose feature is missing still parses, then exits with an error that names the feature.

## Exit status

| Status | Meaning |
|---|---|
| 0 | The command completed |
| 1 | It failed while running, for example a timeout or an error from the device |
| 2 | The arguments did not parse; nothing is sent. Also plain `bacnet` where the terminal UI can't open |
| 3 | `sc-check` could not complete |
| 4 | The write guard refused the command (`--allow-write` or `--yes` missing); nothing is sent |

## Names and targets

Examples use `ai:1` for analog input 1, `av:1` for analog value 1, and `pv` for present value. Full object/property names are useful in teaching material; short aliases are convenient in an operator's terminal.

Do not use an object-name string where an object identifier is required. Do not assume a BACnet device instance is an IP address or that a discovered downstream device can be addressed by a router's IP alone.

A new one-shot process has its own lifecycle and keeps no discovered devices, so give it an address, a `bip6:` or `mstp:` target, or a routed `DNET:instance` or `DNET:0xMAC` target. For a routed target the CLI asks that network for the device with a Who-Is and sends the request through the router that answers. `read`, `readm`, `read-range`, `alarms`, `alarm-summary`, `enrollment-summary`, `file-read`, `file-write`, `write`, `writem`, `subscribe`, `control`, `reinit`, `time-sync`, `create-object`, `delete-object` and `ack-alarm` take routed targets; the BBMD commands `bdt`, `fdt`, `register` and `unregister` address local B/IP peers and refuse them.

## Session behavior

Plain `bacnet` opens the terminal UI when stdin and stdout are terminals; in a pipe or a script, with `--no-tui`, or with `BACNET_NO_TUI` set, it prints the usage and exits with status 2. Scripts call the one-shot commands with `--json`, which also read better with a screen reader than the terminal UI. The [terminal UI guide](/rusty-bacnet/tui/overview/) covers its screens, and its [key reference](/rusty-bacnet/tui/keys/) lists every key and command.

Commands that change a device need `--allow-write`; communication control and reinitialize also need `--yes` or a typed confirmation. The [safe writes guide](/rusty-bacnet/guides/safe-writes/) covers authorization and cleanup.

## Machine-readable output

Use `--json` explicitly in automation. Preserve protocol errors and nonzero process outcomes rather than interpreting an empty result as an empty building. If scripts depend on particular JSON fields, add a small fixture test against the supported CLI version; a formatted example is not a schema guarantee. Table output cleans the strings a device supplies, but JSON keeps the device's exact text, with control characters escaped as JSON requires; a script that prints a JSON string to a terminal should clean it first.

## Sources

The [CLI guide](https://github.com/jscott3201/rusty-bacnet/blob/v0.15.0/docs/CLI.md) covers target syntax, transports and session details at length. `crates/bacnet-cli/src/args.rs` defines the commands and options below; the test in `crates/bacnet-cli/src/args/reference.rs` writes this part of the page from them, so edit the definitions, not the page.

<!-- generated:start by crates/bacnet-cli/src/args/reference.rs (node website/scripts/reference-cli.mjs); do not edit below this line -->

## Usage

```text
Usage: bacnet [OPTIONS] [COMMAND]
```

BACnet command-line tool.

## Global options

These options go before or after the command. Each command also takes `-h` or `--help`.

- `--allow-write`: Let commands that change a device run: writes, object and file changes, alarm acknowledgment, time sync and unregister, and (with --yes or a typed confirmation) communication control and reinitialize. The TUI ignores it: its writes need an armed write session.
- `--yes`: Confirm communication control and reinitialize without typing the target; they need it when stdin is not a terminal.
- `-i, --interface <INTERFACE>`: Network interface IP address to bind (omit to bind all of them; the TUI asks which).
- `-p, --port <PORT>`: BACnet UDP port. Default: `47808`.
- `-b, --broadcast <BROADCAST>`: Broadcast address for WhoIs. Default: `255.255.255.255`.
- `-t, --timeout <TIMEOUT>`: APDU timeout in milliseconds. Default: `6000`.
- `--min-interval-ms <MIN_INTERVAL_MS>`: Least time in milliseconds from the latest request to a device finishing to the next being sent, or from its send while it is still outstanding (0: none, at most 3600000), so paging a log or polling leaves a slow device room for others. Default: `0`.
- `--foreign-bbmd <ADDR>`: Run as a foreign device registered with this BBMD (IP or IP:port), so broadcasts such as discover reach its network. A one-shot command waits for the registration first; it is renewed while the CLI runs. BACnet/IP only.
- `--ttl <TTL>`: Time-to-live in seconds of a foreign-device registration: with --foreign-bbmd, register and discover --bbmd. Default: `300`.
- `--renewal <SECS>`: Seconds between --foreign-bbmd registrations, less than --ttl (default: half the TTL). Behind NAT, renew often so the mapping the BBMD's replies use stays open.
- `--bip-multicast <GROUP>`: Join this IPv4 multicast group (class D) as a B/IP-M node (Annex J.8): broadcasts such as discover go to the group, and only the group's broadcasts are taken. Needs --interface. BACnet/IP only.
- `--bip-multicast-ttl <TTL>`: Multicast TTL for --bip-multicast, 1 to 255 (default 1, the local network only); raise it for a group that spans IP routers. Default: `1`.
- `--bip-multicast-port <PORT>`: UDP port of the --bip-multicast group (default: --port). Several stacks on one host at one address each need their own --port, and share this one.
- `--ipv6`: Use BACnet/IPv6 transport.
- `--ipv6-interface <IPV6_INTERFACE>`: IPv6 interface address to bind.
- `--device-instance <DEVICE_INSTANCE>`: Device instance whose VMAC the CLI uses on --ipv6. Without it the CLI picks a random VMAC, which needs a multicast-capable link at start (Linux loopback has none).
- `--sc`: Use BACnet/SC transport. Needs the `sc-tls` feature.
- `--sc-url <SC_URL>`: SC hub WebSocket URL. Needs the `sc-tls` feature.
- `--sc-failover-url <URL>`: SC failover hub WebSocket URL, used when the primary hub can't be reached or its reconnects run out. Turns on --sc-reconnect. Needs the `sc-tls` feature.
- `--sc-reconnect`: Redial a lost SC hub connection with backoff (10 s doubling to 10 min, 10 attempts). A TUI profile does this unless it sets reconnect = false. Needs the `sc-tls` feature.
- `--sc-connect-timeout <MS>`: SC connect timeout in milliseconds, for each hub dial and handshake (1 to 600000; default 10000). Needs the `sc-tls` feature.
- `--sc-ca <FILE>`: SC trusted site CA PEM file (required for SC; no system-root fallback). Needs the `sc-tls` feature.
- `--sc-cert <SC_CERT>`: SC TLS certificate PEM file. Needs the `sc-tls` feature.
- `--sc-key <SC_KEY>`: SC TLS private key PEM file. Needs the `sc-tls` feature.
- `--sc-vmac <SC_VMAC>`: BACnet/SC local VMAC as 12 hex digits or colon-separated bytes. Needs the `sc-tls` feature.
- `--sc-device-uuid <SC_DEVICE_UUID>`: BACnet/SC device UUID as 32 hex digits or RFC 4122 hyphenated text. Needs the `sc-tls` feature.
- `--mstp <PORT>`: Join an MS/TP trunk as a master station on this serial port (see `bacnet mstp-ports`); needs --mstp-mac and the `serial` feature. B/IP, B/IPv6 and SC flags are refused with it.
- `--mstp-baud <BAUD>`: MS/TP baud rate: 9600, 19200, 38400, 57600, 76800 or 115200. Default: `38400`. Needs the `serial` feature.
- `--mstp-mac <MAC>`: This station's MS/TP MAC, 0 to 127; no other device on the trunk may have it. Required with --mstp. Needs the `serial` feature.
- `--mstp-max-master <MAC>`: Highest master MAC on the trunk (Max\_Master), 0 to 127. Default: `127`. Needs the `serial` feature.
- `--mstp-max-info-frames <N>`: Frames this station sends per token (Max\_Info\_Frames), 1 to 255. Default: `1`. Needs the `serial` feature.
- `--mstp-rt-priority <N>`: Run this station's MS/TP token handling under SCHED\_FIFO at this priority, 1 to 99 (Linux only). Needs CAP\_SYS\_NICE or an RLIMIT\_RTPRIO of at least N; joining fails if it can't be applied. Needs the `serial` feature.
- `--mstp-cpus <CPUS>`: Pin this station's MS/TP token handling to these CPUs, such as 2 or 2,3 (Linux only); joining fails if the process may not use them. Needs the `serial` feature.
- `--format <FORMAT>`: Output format (table, json).
- `--json`: JSON output shorthand.
- `-v, --verbose`: Verbosity (-v, -vv, -vvv). Repeat it for more.
- `--no-tui`: With no command, print the usage instead of opening the terminal UI (as does BACNET\_NO\_TUI set to anything but "" or "0").
- `-h, --help`: Print help.
- `-V, --version`: Print version.

## Commands

| Command | Aliases | Summary |
|---|---|---|
| [`sc-check`](#bacnet-sc-check) | — | Inspect local SC credential files offline; no hub or transport identity is needed. Needs the `sc-tls` feature. |
| [`tui`](#bacnet-tui) | — | Open the full-screen terminal UI (read-only until W arms writes). Needs the `tui` feature. |
| [`mcp`](#bacnet-mcp) | — | Serve an MCP server to an AI agent host (needs the `mcp` feature). |
| [`mcp serve`](#bacnet-mcp-serve) | — | Serve MCP on stdin and stdout until the agent host closes them. Stdout carries only JSON-RPC; logs go to stderr. Needs the `mcp` feature. |
| [`mcp authorize`](#bacnet-mcp-authorize) | — | Record, after showing what an agent may reach, that you authorize the MCP server on a profile. Needs a terminal. Needs the `mcp` feature. |
| [`discover`](#bacnet-discover) | `whois` | Discover BACnet devices (WhoIs). |
| [`find`](#bacnet-find) | `whohas` | Find objects by name (WhoHas). |
| [`read`](#bacnet-read) | `rp` | Read a property. |
| [`readm`](#bacnet-readm) | `rpm` | Read multiple properties. |
| [`write`](#bacnet-write) | `wp` | Write a property. |
| [`writem`](#bacnet-writem) | `wpm` | Write multiple properties (WritePropertyMultiple). |
| [`subscribe`](#bacnet-subscribe) | `cov` | Subscribe to COV notifications. |
| [`control`](#bacnet-control) | `dcc` | Device communication control. |
| [`reinit`](#bacnet-reinit) | — | Reinitialize device. |
| [`alarms`](#bacnet-alarms) | — | List the objects with an active event or an unacknowledged transition (GetEventInformation), following the device's pages. |
| [`alarm-summary`](#bacnet-alarm-summary) | — | List the objects in alarm (GetAlarmSummary). |
| [`enrollment-summary`](#bacnet-enrollment-summary) | — | List the event-initiating objects that match the filters (GetEnrollmentSummary). |
| [`file-read`](#bacnet-file-read) | — | Read a file from a device. |
| [`file-write`](#bacnet-file-write) | — | Write a file to a device. |
| [`mstp-ports`](#bacnet-mstp-ports) | — | List the serial ports to pass to --mstp (a JSON array of names, possibly empty). Opens no port. Needs the `serial` feature. |
| [`bdt`](#bacnet-bdt) | — | Read BBMD broadcast distribution table. |
| [`fdt`](#bacnet-fdt) | — | Read BBMD foreign device table. |
| [`register`](#bacnet-register) | — | Register as foreign device with BBMD, once, for --ttl seconds. |
| [`unregister`](#bacnet-unregister) | — | Unregister from BBMD. |
| [`whois-router`](#bacnet-whois-router) | — | List the routers on the local network and the networks they reach (Who-Is-Router-To-Network). |
| [`ack-alarm`](#bacnet-ack-alarm) | `ack` | Acknowledge an alarm. |
| [`read-range`](#bacnet-read-range) | `rr` | Read a range of items from a list or log buffer. |
| [`create-object`](#bacnet-create-object) | — | Create an object on a remote device. |
| [`delete-object`](#bacnet-delete-object) | — | Delete an object on a remote device. |
| [`time-sync`](#bacnet-time-sync) | `ts` | Synchronize time with a device. |
| [`capture`](#bacnet-capture) | — | Capture and decode BACnet packets. Needs the `pcap` feature. |

### bacnet sc-check

Inspect local SC credential files offline; no hub or transport identity is needed.

Requires --sc-cert; --sc-ca and --sc-key are optional. This advisory inventory does not establish TLS acceptance or change connection policy.

Needs the `sc-tls` feature.

```text
Usage: bacnet sc-check [OPTIONS]
```

Options:

- `--at-unix <SECONDS>`: Inspect at this signed Unix second instead of sampling the clock.

### bacnet tui

Open the full-screen terminal UI (read-only until W arms writes).

Plain `bacnet` on a terminal opens it too, with these defaults. Opens the profile manager by default (B/IP, BACnet/SC and MS/TP profiles). Explicit transport flags without --profile make a nonpersistent B/IP, B/IPv6, SC or MS/TP connection. Writes need a profile, whose workspace holds the ledger and audit log. Needs an interactive terminal: stdin and stdout must be a TTY and TERM must not be "dumb".

Needs the `tui` feature.

```text
Usage: bacnet tui [OPTIONS]
```

Options:

- `--profile <NAME>`: Named B/IP, BACnet/SC or MS/TP profile to preview before connecting.
- `--passive`: Listen for B/IP announcements without sending. :find searches overheard I-Haves locally. No reads, registrations or writes.
- `--mstp-monitor <PORT>`: Listen to an MS/TP trunk on this serial port without transmitting: stations, token ring and errors on the MS/TP screen, where J finds a free MAC to join with. Takes --mstp-baud and --mstp-max-info-frames; not --mstp or a profile. Needs the `serial` feature.
- `--no-profile`: Use global transport flags without reading or writing profiles.
- `--fps <FPS>`: Frames per second for redraws (1-60). Default: `20`.
- `--log-file <FILE>`: Also write log lines to this file (the in-app log pane is `L`).
- `--max-rate <N>`: Most confirmed requests a second the device browser and the watch list send together, across all devices (0 for no cap). Each device has at most one outstanding at a time either way. Default: `20`.
- `--cov-lifetime <SECS>`: Lifetime of the watch list's COV subscriptions, in seconds. They are renewed before it runs out and cancelled on exit. Default: `300`.
- `--poll-interval <SECS>`: Least time between two polls of a watched point a device won't report by COV, in seconds. Default: `10`.
- `--capture-ring <N>`: Frames the Capture screen keeps; past that the oldest go. Default: `50000`.
- `--capture-device <DEVICE>`: Also capture B/IP traffic on the -p port from this network device with libpcap, for the Capture screen (needs the pcap feature and capture rights).

### bacnet mcp

Serve an MCP server to an AI agent host (needs the `mcp` feature).

`bacnet mcp serve --stdio --profile NAME` is what an agent host's configuration launches; `bacnet mcp authorize --profile NAME`, run once in a terminal, records the person's consent first.

```text
Usage: bacnet mcp [OPTIONS] <COMMAND>
```

### bacnet mcp serve

Serve MCP on stdin and stdout until the agent host closes them. Stdout carries only JSON-RPC; logs go to stderr.

Needs the `mcp` feature.

```text
Usage: bacnet mcp serve [OPTIONS] --stdio
```

Options:

- `--stdio`: Serve on stdin and stdout, the transport an agent host launches. Required.
- `--profile <NAME>`: Named profile to connect with; its mcp table limits what the agent may reach. Needed unless --passive; no transport flags with it.
- `--passive`: Build no client and send nothing: only the tools that need no network are offered.

### bacnet mcp authorize

Record, after showing what an agent may reach, that you authorize the MCP server on a profile. Needs a terminal.

Needs the `mcp` feature.

```text
Usage: bacnet mcp authorize [OPTIONS] --profile <NAME>
```

Options:

- `--profile <NAME>`: The profile to authorize. Required.

### bacnet discover

Discover BACnet devices (WhoIs).

Aliases: `whois`.

```text
Usage: bacnet discover [OPTIONS] [RANGE]
```

Arguments:

- `[RANGE]`: Device instance range (e.g., "1000-2000").

Options:

- `--wait <WAIT>`: Seconds to wait for responses. Default: `3`.
- `--target <TARGET>`: Send directed WhoIs to a specific address instead of broadcasting.
- `--bbmd <BBMD>`: Register as foreign device with a BBMD before discovering, once (for --ttl seconds). --foreign-bbmd also renews and sends the Who-Is through the BBMD.
- `--dnet <DNET>`: Target a specific remote network number.

### bacnet find

Find objects by name (WhoHas).

Aliases: `whohas`.

```text
Usage: bacnet find [OPTIONS] [NAME]
```

Arguments:

- `[NAME]`: Object name to find (quote a name with spaces). `--name NAME` says the same; give one or the other.

Options:

- `--name <NAME>`: Object name to find, as a flag: the same as the positional NAME.
- `--wait <WAIT>`: Seconds to wait for responses. Default: `3`.

### bacnet read

Read a property.

Aliases: `rp`.

```text
Usage: bacnet read [OPTIONS] <TARGET> <OBJECT> <PROPERTY>
```

Arguments:

- `<TARGET>`: Target device (IP address or instance number). Required.
- `<OBJECT>`: Object specifier (e.g., analog-input:1, ai:1). Required.
- `<PROPERTY>`: Property (e.g., present-value, pv). Required.

### bacnet readm

Read multiple properties.

Aliases: `rpm`.

```text
Usage: bacnet readm [OPTIONS] <TARGET> [SPECS]...
```

Arguments:

- `<TARGET>`: Target device. Required.
- `[SPECS]...`: Object and property specs: "ai:1 pv,object-name ao:1 pv".

### bacnet write

Write a property.

Aliases: `wp`.

```text
Usage: bacnet write [OPTIONS] <TARGET> <OBJECT> <PROPERTY> <VALUE>
```

Arguments:

- `<TARGET>`: Target device. Required.
- `<OBJECT>`: Object specifier. Required.
- `<PROPERTY>`: Property. Required.
- `<VALUE>`: Value to write (e.g., 72.5, true, "string", null). Required.

Options:

- `--priority <PRIORITY>`: Priority (1-16).

### bacnet writem

Write multiple properties (WritePropertyMultiple).

Aliases: `wpm`.

```text
Usage: bacnet writem [OPTIONS] <TARGET> <SPECS> <SPECS>...
```

Arguments:

- `<TARGET>`: Target device. Required.
- `<SPECS>...`: Each object, then its comma-separated writes, each with an optional @priority: "av:1 pv=72.5@8,on=Zone ao:1 pv=10". Required.

### bacnet subscribe

Subscribe to COV notifications.

Aliases: `cov`.

```text
Usage: bacnet subscribe [OPTIONS] <TARGET> <OBJECT>
```

Arguments:

- `<TARGET>`: Target device. Required.
- `<OBJECT>`: Object specifier. Required.

Options:

- `--lifetime <LIFETIME>`: Subscription lifetime in seconds. The subscription is renewed while the command watches and cancelled when it stops (Ctrl+C, or SIGTERM on Unix). 0 asks for one that never expires and stays after the CLI exits, which needs --allow-write. Default: `300`.
- `--confirmed`: Use confirmed notifications.

### bacnet control

Device communication control.

Aliases: `dcc`.

```text
Usage: bacnet control [OPTIONS] <TARGET> <ACTION>
```

Arguments:

- `<TARGET>`: Target device. Required.
- `<ACTION>`: Action: enable, disable, disable-initiation. Required.

Options:

- `--duration <DURATION>`: Duration in minutes.
- `--password <PASSWORD>`: Password, at most 20 characters.

### bacnet reinit

Reinitialize device.

```text
Usage: bacnet reinit [OPTIONS] <TARGET> <STATE>
```

Arguments:

- `<TARGET>`: Target device. Required.
- `<STATE>`: State: coldstart, warmstart. Required.

Options:

- `--password <PASSWORD>`: Password, at most 20 characters.

### bacnet alarms

List the objects with an active event or an unacknowledged transition (GetEventInformation), following the device's pages.

```text
Usage: bacnet alarms [OPTIONS] <TARGET>
```

Arguments:

- `<TARGET>`: Target device. Required.

Options:

- `--max <MAX>`: Stop after this many objects. Default: `10000`.

### bacnet alarm-summary

List the objects in alarm (GetAlarmSummary).

```text
Usage: bacnet alarm-summary [OPTIONS] <TARGET>
```

Arguments:

- `<TARGET>`: Target device. Required.

### bacnet enrollment-summary

List the event-initiating objects that match the filters (GetEnrollmentSummary).

```text
Usage: bacnet enrollment-summary [OPTIONS] <TARGET>
```

Arguments:

- `<TARGET>`: Target device. Required.

Options:

- `--acked <ACKED>`: Acknowledgment: all, acked or not-acked. Default: `all`.
- `--state <STATE>`: Event state: offnormal, fault, normal, all or active.
- `--event-type <EVENT_TYPE>`: Event algorithm, such as out-of-range or change-of-state.
- `--priority <PRIORITY>`: Notification priority, MIN-MAX (0 to 255) or one value.
- `--class <CLASS>`: Notification class number.

### bacnet file-read

Read a file from a device.

```text
Usage: bacnet file-read [OPTIONS] <TARGET> <FILE_INSTANCE>
```

Arguments:

- `<TARGET>`: Target device. Required.
- `<FILE_INSTANCE>`: File object instance. Required.

Options:

- `--access <ACCESS>`: BACnet file access mode. One of `stream`, `record`. Default: `stream`.
- `--start <START>`: Initial octet position or record index. Default: `0`.
- `--count <COUNT>`: Per-request octet or record window size. Default: `1024`.
- `--output <OUTPUT>`: Stream output file or required record output directory.

### bacnet file-write

Write a file to a device.

```text
Usage: bacnet file-write [OPTIONS] <TARGET> <FILE_INSTANCE> <INPUT>
```

Arguments:

- `<TARGET>`: Target device. Required.
- `<FILE_INSTANCE>`: File object instance. Required.
- `<INPUT>`: Input file path. Required.

Options:

- `--start <START>`: Start position. Default: `0`.

### bacnet mstp-ports

List the serial ports to pass to --mstp (a JSON array of names, possibly empty). Opens no port.

Needs the `serial` feature.

```text
Usage: bacnet mstp-ports [OPTIONS]
```

### bacnet bdt

Read BBMD broadcast distribution table.

```text
Usage: bacnet bdt [OPTIONS] <TARGET>
```

Arguments:

- `<TARGET>`: Target device. Required.

### bacnet fdt

Read BBMD foreign device table.

```text
Usage: bacnet fdt [OPTIONS] <TARGET>
```

Arguments:

- `<TARGET>`: Target device. Required.

### bacnet register

Register as foreign device with BBMD, once, for --ttl seconds.

```text
Usage: bacnet register [OPTIONS] <TARGET>
```

Arguments:

- `<TARGET>`: Target device. Required.

### bacnet unregister

Unregister from BBMD.

```text
Usage: bacnet unregister [OPTIONS] <TARGET>
```

Arguments:

- `<TARGET>`: Target device. Required.

### bacnet whois-router

List the routers on the local network and the networks they reach (Who-Is-Router-To-Network).

```text
Usage: bacnet whois-router [OPTIONS]
```

Options:

- `--dnet <DNET>`: Ask only for the router to this network number.
- `--wait <WAIT>`: Seconds to wait for responses. Default: `3`.

### bacnet ack-alarm

Acknowledge an alarm.

Aliases: `ack`.

```text
Usage: bacnet ack-alarm [OPTIONS] --state <STATE> --timestamp <TIMESTAMP> --ack-time <ACK_TIME> <TARGET> <OBJECT>
```

Arguments:

- `<TARGET>`: Target device. Required.
- `<OBJECT>`: Object specifier (e.g., ai:1). Required.

Options:

- `--state <STATE>`: Event state to acknowledge (0=normal, 1=fault, etc.). Required.
- `--source <SOURCE>`: Acknowledgment source string. Default: `bacnet-cli`.
- `--timestamp <TIMESTAMP>`: Exact timestamp from the original event notification. Required.
- `--ack-time <ACK_TIME>`: Caller-selected time of acknowledgment. Required.

### bacnet read-range

Read a range of items from a list or log buffer.

Aliases: `rr`.

```text
Usage: bacnet read-range [OPTIONS] <TARGET> <OBJECT> [PROPERTY]
```

Arguments:

- `<TARGET>`: Target device. Required.
- `<OBJECT>`: Object specifier (e.g., trend-log:1). Required.
- `[PROPERTY]`: Property (default: log-buffer). Default: `log-buffer`.

Options:

- `--position <POSITION>`: Read by position, from this one-based index.
- `--sequence <SEQUENCE>`: Read by sequence number, from this record.
- `--time <TIME>`: Read the records logged after this time, in the device's local time: YYYY-MM-DDTHH:MM\[:SS\[.hh\]\].
- `--count <COUNT>`: Items to read (default 100); negative reads backward from the reference. With --all, the page size.
- `--all`: Page through the log from the oldest record, or from --position, --sequence or --time, until the newest.
- `--strict`: Refuse an answer that breaks a ReadRange rule, instead of showing it with the rules it broke.

### bacnet create-object

Create an object on a remote device.

```text
Usage: bacnet create-object [OPTIONS] <TARGET> <OBJECT>
```

Arguments:

- `<TARGET>`: Target device. Required.
- `<OBJECT>`: Object specifier (type:instance, e.g., av:100). Required.

### bacnet delete-object

Delete an object on a remote device.

```text
Usage: bacnet delete-object [OPTIONS] <TARGET> <OBJECT>
```

Arguments:

- `<TARGET>`: Target device. Required.
- `<OBJECT>`: Object specifier (type:instance). Required.

### bacnet time-sync

Synchronize time with a device.

Aliases: `ts`.

```text
Usage: bacnet time-sync [OPTIONS] <TARGET>
```

Arguments:

- `<TARGET>`: Target device. Required.

Options:

- `--utc`: Use UTC time synchronization.

### bacnet capture

Capture and decode BACnet packets.

Needs the `pcap` feature.

```text
Usage: bacnet capture [OPTIONS]
```

Options:

- `--read <READ>`: Read from a pcap file instead of live capture.
- `--save <SAVE>`: Save captured packets to a pcap file.
- `--quiet`: Suppress decoded output (use with --save).
- `--decode`: Full protocol decode (BVLC/NPDU/APDU/service details).
- `--device <DEVICE>`: Network interface name for live capture (e.g., en0, eth0).
- `--filter <FILTER>`: Additional BPF filter expression, joined with `and` to "ip and udp port PORT", PORT being -p (47808 by default).
- `--count <COUNT>`: Stop after capturing N packets.
- `--snaplen <SNAPLEN>`: Maximum bytes to capture per packet. Default: `65535`.
<!-- generated:end -->
