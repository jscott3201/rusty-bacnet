# BACnet CLI Reference

The `bacnet` command-line tool provides interactive and scripted access to BACnet networks for device discovery, property reading/writing, diagnostics, and packet analysis. Plain `bacnet` on a terminal, or `bacnet tui`, opens a full-screen terminal UI; see [Terminal UI](#terminal-ui). Every other command is a one-shot command that prints its result and exits, as a table at a terminal or as JSON with `--json` or through a pipe.

**Screen readers.** The terminal UI redraws a whole screen at a time, which
screen readers follow poorly. The one-shot commands cover the same work
(`discover`, `read`, `readm`, `write`, `subscribe` and the rest below) and
print plain lines: use them with `--json` for output that reads the same every
time, or with `--format table`.

## Installation

Download a release binary (see [Pre-built Binaries](#pre-built-binaries)), or
build it with Cargo.

`bacnet-cli` is published on crates.io with each release from 0.12.0:

```bash
cargo install bacnet-cli --locked --features sc-tls
```

For changes merged after the release, install from a checkout:

```bash
# From a checkout of this repository
cargo install --path crates/bacnet-cli --locked

# With packet capture support (requires libpcap)
cargo install --path crates/bacnet-cli --locked --features pcap

# With BACnet/SC support
cargo install --path crates/bacnet-cli --locked --features sc-tls

# With MS/TP over a serial port
cargo install --path crates/bacnet-cli --locked --features serial
```

The features combine (`--features sc-tls,pcap,serial`). Packet capture needs libpcap
and its headers (`libpcap-dev` on Debian and Ubuntu). The `serial` feature
builds on Linux, macOS and Windows; the Linux release binary has it, and the
macOS and Windows ones don't yet (see [MS/TP](#mstp)).

## Global Options

| Flag | Default | Description |
|------|---------|-------------|
| `-i, --interface <IP>` | (see below) | Network interface IP to bind |
| `-p, --port <PORT>` | `47808` | BACnet UDP port |
| `-b, --broadcast <IP>` | `255.255.255.255` | Broadcast address for WhoIs |
| `-t, --timeout <MS>` | `6000` | APDU timeout in milliseconds |
| `--min-interval-ms <MS>` | `0` | Least time from the latest confirmed request to a device finishing (or being sent, while it is still outstanding) to the next, at most 3600000, so paging or polling leaves a slow device room for others |
| `--foreign-bbmd <ADDR>` | | Run as a foreign device registered with this BBMD (`ip` or `ip:port`), so broadcasts reach its network; BACnet/IP only. See [Foreign-device mode](#foreign-device-mode) |
| `--ttl <N>` | `300` | Time-to-live in seconds of a foreign-device registration, for `--foreign-bbmd`, `register` and `discover --bbmd` |
| `--renewal <SECS>` | half the TTL | Seconds between `--foreign-bbmd` registrations, less than `--ttl` |
| `--bip-multicast <GROUP>` | | Join this IPv4 multicast group (class D) as a B/IP-M node: broadcasts go to the group and only the group's are taken. Needs `--interface`; BACnet/IP only, not with `--foreign-bbmd`. See [Multicast mode](#multicast-mode) |
| `--bip-multicast-ttl <TTL>` | `1` | Multicast TTL for `--bip-multicast`, 1 to 255; raise it for a group that spans IP routers |
| `--bip-multicast-port <PORT>` | `--port` | UDP port of the `--bip-multicast` group; needed with `--port 0`, and for several stacks on one host at one address |
| `--ipv6` | | Use BACnet/IPv6 transport |
| `--ipv6-interface <IP>` | | IPv6 interface address |
| `--device-instance <N>` | | Device instance whose VMAC the CLI uses on `--ipv6`; needed where the link has no multicast, such as Linux loopback |
| `--sc` | | Use BACnet/SC transport |
| `--sc-url <URL>` | | SC hub WebSocket URL |
| `--sc-failover-url <URL>` | | SC failover hub, used when the primary can't be reached or its reconnects run out; turns on `--sc-reconnect` |
| `--sc-reconnect` | off (a profile: on) | Redial a lost SC hub with backoff: 10 s doubling to 10 min, 10 attempts |
| `--sc-connect-timeout <MS>` | 10000 | Bound on each SC hub dial and handshake, 1 to 600000 ms |
| `--sc-ca <FILE>` | required for SC | Trusted site CA certificate(s) in PEM; no system-root fallback |
| `--sc-cert <FILE>` | | SC TLS certificate PEM |
| `--sc-key <FILE>` | | SC TLS private key PEM |
| `--sc-vmac <HEX>` | | SC local VMAC as 12 hex digits or separated bytes |
| `--sc-device-uuid <UUID>` | | SC device UUID as 32 hex digits or hyphenated text |
| `--mstp <PORT>` | | Join an MS/TP trunk as a master station on this serial port; needs `--mstp-mac` and the `serial` feature. See [MS/TP](#mstp) |
| `--mstp-mac <MAC>` | required with `--mstp` | This station's MAC, 0 to 127; no other device on the trunk may use it |
| `--mstp-baud <BAUD>` | `38400` | 9600, 19200, 38400, 57600, 76800 or 115200 (Clause 9.2.3) |
| `--mstp-max-master <MAC>` | `127` | Highest master MAC on the trunk (Max_Master), 0 to 127, at least `--mstp-mac` |
| `--mstp-max-info-frames <N>` | `1` | Frames this station sends each time it holds the token (Max_Info_Frames), 1 to 255 |
| `--mstp-rt-priority <N>` | | Linux only: run the station's token handling under `SCHED_FIFO` at priority N, 1 to 99. See [MS/TP](#mstp) |
| `--mstp-cpus <CPUS>` | | Linux only: pin the station's token handling to these CPUs, such as `2,3` |
| `--format <FMT>` | auto | Output format: `table` or `json` |
| `--json` | | JSON output shorthand |
| `-v` | | Verbosity (`-v`, `-vv`, `-vvv`) |
| `--allow-write` | | Let a command that changes a device run; see [Write guard](#write-guard) |
| `--no-tui` | | With no command, print the usage instead of opening the terminal UI; see [Plain `bacnet`](#plain-bacnet) |
| `--yes` | | Confirm communication control and reinitialize without typing the target; needed when stdin is not a terminal |

Output auto-detects: tables in TTY, JSON when piped.

**Interface selection:** A one-shot command without `--interface` binds `0.0.0.0`. The terminal UI on BACnet/IP without `-i` lists the available network interfaces in a dialog; named B/IP profiles use their saved interface.

## Target Resolution

Commands that take a `<target>` argument accept several formats:

| Format | Example | Description |
|--------|---------|-------------|
| IPv4 address | `192.168.1.100` | Direct BIP target (default port 47808) |
| IPv4:port | `10.0.1.5:47809` | Direct BIP target with explicit port |
| B/IPv6 device | `bip6:1234` | The VMAC of device instance 1234 (0 to 4194303), with `--ipv6` |
| B/IPv6 VMAC | `bip6:0x40ABCD` | Any 3-octet VMAC, as the CLI prints B/IPv6 addresses |
| IPv6 bracket | `[2001:db8::10]` | B/IPv6 address (port 47808), resolved to the VMAC of the node there |
| IPv6:port | `[fe80::1%3]:47809` | The same with a port; a link-local address takes a numeric zone |
| 6-octet MAC or SC VMAC | `02:00:00:00:00:01`, `020000000001` | A MAC as six hex octets, such as a BACnet/SC VMAC; not on B/IPv6 or MS/TP |
| MS/TP station | `mstp:5`, `mstp:0x05` | A station on the trunk, 0 to 254, with `--mstp`; device tables print MS/TP addresses this way |
| Device instance | `1234` | Looks up address from discovered device cache |
| DNET:instance | `2:1234` | Routed device (network:instance) |
| DNET:0xMAC | `2:0x05` | Routed station by its MAC on that network (MS/TP station 5 here) |

When using a device instance number, the device must have been previously found via `discover` in the same process. A one-shot command starts with an empty device table, so give it an address, a `bip6:` or `mstp:` target, or a routed `DNET:instance` target, which it looks up itself.

**Routed targets.** `read`, `readm`, `write`, `writem` and `subscribe` take
`DNET:instance` and `DNET:0xMAC`. When the device isn't in the discovery table
yet, the CLI sends a Who-Is to that network and waits up to three seconds for
the device to answer, then sends the request through the router that answered
for it. Other commands refuse a routed target, since the client has routed
calls only for those services. On BACnet/SC, device tables show a VMAC as hex
octets (`02:00:00:00:00:01`), which every target argument takes back.

**B/IPv6 targets are VMACs.** A B/IPv6 node is addressed by its 3-octet VMAC,
not its IPv6 address (Clause U.1.3), and a device's VMAC is its device instance
(Clause H.7.2), so `bip6:1234` reaches device 1234 without discovery. The CLI
turns an `[IPv6]` or `[IPv6]:port` target into a VMAC before the command runs,
asking the node there with a unicast Virtual-Address-Resolution (Clause U.5);
it fails after three seconds when nothing answers. Device tables, COV output
and the TUI show B/IPv6 addresses as `bip6:0x0004D2`, which every target
argument takes back. A `bip6:` target without `--ipv6` is refused.

`--device-instance` gives the CLI the VMAC of that instance on `--ipv6`.
Without it the CLI picks a random VMAC at each start, and must first ask the
link whether another node already has it, by multicast. A link with no
multicast, such as Linux loopback (`--ipv6-interface ::1`), can't carry that
question, so pass `--device-instance` there.

## Write Guard

A one-shot command that changes a device is refused unless the command line
has `--allow-write`. Communication control and reinitialize also need `--yes`
when stdin is not a terminal; at a terminal without `--yes` they ask you to
type the device instance (or, for an address target, the target as you gave
it) and send nothing unless it matches. Both flags are global, so they go on
either side of the subcommand. A refused command sends nothing: it stops
before the CLI binds a socket.

| Class | Commands (aliases) | Needs |
|---|---|---|
| read | `discover` (`whois`), `find` (`whohas`), `read` (`rp`), `readm` (`rpm`), `read-range` (`rr`), `alarms`, `file-read`, `bdt`, `fdt`, `whois-router`, `subscribe` (`cov`) with a finite lifetime (the default), `register`, `mstp-ports`, `capture`, `sc-check`, `tui`, `help` | nothing |
| write | `write` (`wp`), `writem` (`wpm`), `create-object`, `delete-object`, `file-write`, `ack-alarm` (`ack`), `time-sync` (`ts`), `unregister`, `subscribe --lifetime 0` | `--allow-write` |
| device management | `control` (`dcc`), `reinit` | `--allow-write`, plus `--yes` or the typed confirmation |

`subscribe` and `register` stay read commands: each leaves an entry on the
remote device only so that it can deliver what the CLI watches (a COV
subscription, a foreign-device registration), the entry lapses on its own, and
the TUI makes both from its read-only level. `subscribe` asks for a 300-second
subscription unless told otherwise, renews it while it watches and cancels it
when it stops (see [COV Subscriptions](#cov-subscriptions)). `subscribe
--lifetime 0` asks for one that never expires and stays after the CLI exits, so
it is a write. `unregister` is a write: the BBMD drops whichever foreign-device
entry has the address the request names (the CLI gives its own, but the BBMD
can't tell). The TUI's session levels use the same classes: a read-only
session runs reads, an armed write session runs writes, and device management
will need a session the profile marks `dangerous`.

A refused one-shot exits with status 4 and prints nothing on stdout. With
table output the reason goes to stderr as `ERROR: ...`. With JSON output
(`--json`, `--format json`, or stdout not a terminal) stderr gets one line of
JSON in this shape, which scripts can rely on:

```json
{"error":"refused","reason":"allow-write-required","command":"reinit","class":"device-management","missing":["--allow-write","--yes"],"message":"reinit is a device-management command and needs --allow-write and --yes (stdin is not a terminal, so it can't be confirmed by typing); nothing was sent"}
```

| Field | Values |
|---|---|
| `error` | always `refused` |
| `reason` | `allow-write-required`, `yes-required`, `not-confirmed` (the typed text didn't match, or input ended), `unclassified` |
| `command` | the subcommand's full name, also when an alias was typed (`wp` is `write`) |
| `class` | `write` or `device-management` (`null` for `unclassified`) |
| `missing` | the flags that would have let it run, possibly empty |
| `message` | the same reason as a sentence, for people; its wording may change |

**The TUI** ignores both flags: its writes need an armed write session (see
[Write sessions](#write-sessions)). Profiles apply only to the TUI, so there
is no profile setting for one-shot commands.

### Exit status

| Status | Meaning |
|---|---|
| 0 | The command completed |
| 1 | It failed while running: a timeout, an error or reject from the device, a bad value, a transport that wouldn't start |
| 2 | Usage error from the argument parser (an unknown flag, a missing argument, or no command where the terminal UI can't open); nothing is sent |
| 3 | `sc-check` could not complete (see [the SC check guide](sc-check.md)) |
| 4 | The write guard refused the command; nothing is sent |

## Commands

### Offline SC credential inspection

`bacnet sc-check --sc-cert node.pem [--sc-ca site-ca.pem] [--sc-key node.key]`
inspects local files without creating a client or connecting to a hub. It needs
the `sc-tls` feature and is new in 0.14.0; 0.13.0 binaries don't have it. `--at-unix <SECONDS>` selects a reproducible signed Unix
time. The identity file is required; trust and key inputs are optional.

Output is human-readable in a terminal and JSON through a pipe, with explicit
`--format table|json` and `--json` overrides. Exit 0 permits warnings, 1 reports
local inspection errors, 2 rejects usage, and 3 means inspection/output could
not complete. Transport flags such as `--sc` and `--sc-url` are rejected even
though credential paths use the existing global flags. With `--sc-ca`, it also
reports whether a CA in that file issued the certificate directly. Read the
[command contract and examples](sc-check.md) for diagnostics, JSON fields, the
same checks before every SC dial, and an OpenSSL recipe for a lab PKI.

### Terminal UI

```bash
bacnet                                   # on a terminal: the same as `bacnet tui`
bacnet tui                               # create, select or preview a profile
bacnet tui --profile plant-a             # preview this saved connection
bacnet tui --profile plant-a --port 47808 # explicit flags override saved settings
bacnet tui --no-profile                  # flags-only connection, no profile I/O
bacnet tui -i 10.0.1.5                   # bind this interface, skip the picker
bacnet --ipv6 tui                        # BACnet/IPv6
bacnet --sc --sc-url wss://hub:443 --sc-ca ca.pem --sc-cert me.pem \
  --sc-key me.key --sc-vmac 02:00:00:00:00:09 \
  --sc-device-uuid 00112233-4455-6677-8899-aabbccddeeff tui   # BACnet/SC
bacnet --mstp /dev/ttyUSB0 --mstp-mac 3 tui   # MS/TP (serial feature); confirm the join with y
bacnet --mstp /dev/ttyUSB0 --mstp-mac 3       # the same, without `tui`
bacnet tui --mstp-monitor /dev/ttyUSB0   # listen to an MS/TP trunk; J finds a MAC to join as
bacnet tui --fps 5 --log-file tui.log    # lower redraw rate, keep a log file
bacnet tui -i 10.0.1.5 --capture-device en0   # also capture the wire (pcap feature)
```

`bacnet tui` opens a profile dialog (B/IP, BACnet/SC or MS/TP) before connecting. With no saved
profiles it opens creation; with one it previews that connection; with several
it shows a picker. `--profile NAME` selects one directly. The Devices screen is
a live table of I-Am answers, with a Who-Is form. The status bar shows the
session level: `READ-ONLY` by default (discovery, reads, finite COV
subscriptions and foreign-device registrations), a red `WRITE mm:ss` while a
write session is armed (see [Write sessions](#write-sessions)), or `PASSIVE`.
Screens: `1` devices, `3` watch list, `4` network, `5` SC, `6` MS/TP and `7`
[capture](#capture-screen). The design is in
[docs/design/tui.md](design/tui.md).

Explicit global network flags without `--profile`, or `--no-profile`, start
the existing flags-only connection flow and do not read or write profiles or
workspaces. B/IPv6 uses that flow only; named profiles support B/IP,
BACnet/SC and MS/TP. Global flags can appear before or after `tui`. With `--profile`, a
flag for the other transport (say `--port` on an SC profile) is an error.

**Tui flags:**

| Flag | Default | Description |
|------|---------|-------------|
| `--profile <NAME>` | | Preview a named B/IP, SC or MS/TP profile; explicit global flags override saved values |
| `--no-profile` | | Use flags only, without profile/workspace I/O; conflicts with `--profile` |
| `--passive` | | Transmit nothing: no client is built, so no discovery, reads, subscriptions, registrations or writes. The ledger can still be viewed |
| `--mstp-monitor <PORT>` | | Listen to an MS/TP trunk on this serial port without transmitting (the `serial` feature). Takes `--mstp-baud` and `--mstp-max-info-frames`, and on Linux `--mstp-rt-priority` and `--mstp-cpus`, which only a station joined from it gets (the monitor itself runs normally); not `--mstp`, `--mstp-mac`, `--mstp-max-master`, another transport's flags or `--profile`. See [MS/TP monitor](#mstp-monitor-and-safe-join) |
| `--fps <N>` | `20` | Redraws per second, 1 to 60. Idle screens are not redrawn |
| `--log-file <FILE>` | | Append log lines to this file as well as the in-app log pane |
| `--max-rate <N>` | `20` | Confirmed requests a second across the browser and the watch list; 0 for no cap |
| `--cov-lifetime <SECS>` | `300` | Lifetime of the watch list's COV subscriptions, 10 to 28800 |
| `--poll-interval <SECS>` | `10` | Least time between two polls of a watched point, 1 to 3600 |
| `--capture-ring <N>` | `50000` | Frames the Capture screen keeps, 1 to 1000000, at about 0.5 to 1 KB each; the oldest go first |
| `--capture-device <DEVICE>` | | Also capture B/IP datagrams on the `-p` port from this network device with libpcap (the `pcap` feature; capture rights needed) |

`-v` and `-vv` raise the log level from info to debug and trace.

#### Plain `bacnet`

`bacnet` with no command opens the terminal UI when stdin and stdout are both
terminals and `TERM` isn't `dumb`. Its global flags carry over exactly as they
do before `tui`: `bacnet --sc --sc-url ...` opens an SC session, and `bacnet
--mstp /dev/ttyUSB0 --mstp-mac 3` asks before it joins the trunk, as `bacnet
... tui` does. The `tui` flags (`--profile`, `--fps` and the rest) take their
defaults; name `tui` to set them.

Otherwise it prints the usage to stderr, prints nothing on stdout, and exits
with status 2, as for any other usage error. That happens in a pipe or a
script, when `--no-tui` is given, when the `BACNET_NO_TUI` environment variable
is set to anything but empty or `0`, and in a build without the `tui` feature.
`--no-tui` and `BACNET_NO_TUI` matter only when no command is given: every
other command ignores them.

**Requirements.** stdin and stdout must be a terminal and `TERM` must not be
`dumb`; otherwise `bacnet tui` exits 1 with a hint on stderr and writes nothing
to stdout. The same hint appears, with exit status 1, when the terminal refuses
raw mode (as MSYS and mintty terminals on Windows do). Use the one-shot
commands (`bacnet discover --json`) in scripts and pipes. 80x24 is the smallest
supported terminal and 120x40 shows the full table; below 80x24 the TUI shows a
notice until the window is resized. Colour follows `NO_COLOR`. On Windows use
Windows Terminal or conhost; mintty and Git Bash are not supported.

**Exit status.** 0 after `q` or a second Ctrl-C. Connection failures remain in
the UI so settings can be corrected and retried. Terminal/internal failures
exit 1, with the reason on stderr. When a
signal ends it, the terminal is restored first; on Unix the status then follows
the Unix shell convention of 128 plus the signal number (130 for SIGINT, 129 for
SIGHUP, 143 for SIGTERM), and on Windows a console close or Ctrl-Break exits 1.

**Interface selection.** In flags-only B/IP sessions without `-i`, a dialog
lists the IPv4 interfaces (a single one is used without asking). An
explicit broadcast value survives either choice. Profiles use their saved
interface; `0.0.0.0` binds all interfaces.

**Profiles and connection lifecycle.** Press `p` to open the profile dialog,
`x` to disconnect or cancel a connection attempt, and `c` to reconnect. A
profile reconnect opens its preview again. Disconnect must finish before
another profile can be loaded or edited. Failed connection attempts keep the
TUI open. Rows, duplicates and running operations are reset between sessions;
late events from a previous session cannot repopulate the new one. If a
Who-Is cancellation cannot enter a command queue, its status shows the
failure; Ctrl-C retries immediately and `x` disconnects. The queued Who-Is
may still run until cancellation is accepted or the connection is retired.

In the profile picker, Up/Down choose, Enter previews, `n`, `s` or `m`
creates a B/IP, BACnet/SC or MS/TP profile and `r` reloads the list. In a
preview, `e` edits, `r` reloads the file and re-checks it, `p` returns to the
list, `n`, `s` or `m` creates another profile, and `x` disconnects a live session
while the dialog stays open. The editor uses Tab/Shift-Tab and Backspace;
Enter saves without connecting. Once an SC profile has a VMAC and device UUID,
the editor shows them read-only; change them only by editing the file. Save errors retain the draft.
Esc explicitly discards it and returns to the saved preview (or the picker
for a new profile), where `r` reloads from disk. If a file changed externally,
the refused save leaves that file untouched; discard, reload and edit again.
A blank BBMD selects Normal mode and clears the saved renewal; a BBMD selects
Foreign Device mode. Blank renewal uses half the TTL. Port `0` requests an ephemeral local UDP port.

An SC profile names a primary hub URL (`wss://`, with a host and a nonzero
port if one is given), an optional failover hub URL, and the paths of the CA,
certificate and key PEM files. Only the paths are saved, never PEM text, key
bytes or passwords; relative paths are made absolute on save. A blank VMAC or
device UUID is generated when the profile is first saved (or first connected,
for a hand-written file) and kept from then on, so the device UUID stays the
same across restarts. An SC profile reconnects with backoff after it loses its
hub, primary-only profiles included, so a long session survives a hub restart:
by default the first retry comes after 10 s, the delay doubles up to 10 min,
and 10 attempts are made before a failover hub, if one is set, is tried.
`reconnect = false` turns this off (a lost hub then stays down) and is refused
with a failover URL, since the failover hub is reached by reconnecting. The
backoff (`reconnect_initial_delay_ms`, `reconnect_max_delay_ms`,
`reconnect_max_retries`) and the connect timeout (`connect_timeout_ms`, 1 to
600000, default 10000) can be set in the file; the editor offers the connect
timeout and reconnect on/off. `--sc-failover-url`, `--sc-reconnect` and
`--sc-connect-timeout` override a profile like the other flags, and give a
flags-only session the same settings (reconnect stays off there unless one of
the first two is given).

Before a connection is attempted, the preview checks that each named file
opens for reading, and on Unix warns when the key file is readable by group or
others. With `sc-tls` it also runs the credential lint of
[`bacnet sc-check`](sc-check.md#before-a-dial) on them: an expired or
not-yet-valid operational certificate, or a key that does not match it, is an
error; expiry within 30 days, or a certificate no CA in the CA file issued
directly, is a warning. An error names the problem and refuses the connection;
nothing is sent until it passes. Warnings hold the connection until `y`
accepts the ones shown, and the check runs again just before the dial.

The preview shows the settings after CLI overrides. Explicit values win even
when equal to defaults, such as `--port 47808`, `--timeout 6000` or
`--min-interval-ms 0`. They are not written to the profile by a connection or
an authorization save. `--renewal` may use a named profile's saved BBMD.

On first use, `a` explicitly attests that you are authorized to use BACnet on
the displayed network. Its timestamp must be saved successfully before the
client is built. Enter alone does not attest. Subsequent connections use Enter
or `c`; editing saved network settings clears the old attestation. The safety
policy is `writes = "session"` (the default: `W` may arm a write session) or
`"never"` (`W` says why and arms nothing), and `dangerous = true`, which needs
`writes = "session"`, allows dangerous operations inside an armed session
(today, commanding priority 6).

The config root is the platform directory returned by
`directories::ProjectDirs::from("", "", "bacnet").config_dir()`. The dialog
shows its path. `BACNET_CONFIG_DIR` replaces it completely, including all
per-profile workspaces. Profiles are `profiles/NAME.toml`; workspaces are
`workspaces/NAME/` and hold the session ledger (`ledger.json`), the audit log
(`audit.jsonl`) and watch-list CSV logs.
Names use 1–64 ASCII letters, digits, `_` or `-`, excluding Windows device
names. New-profile saves never overwrite an existing name. Failed saves keep
the previous file; errors stay visible. Reload after changing a file outside
the TUI. A profile uses this versioned schema:

```toml
version = 1
transport = "bip"
# authorized_at_unix is added only after explicit first-use attestation.

[bip]
interface = "127.0.0.1"
port = 0
broadcast = "127.0.0.255"
timeout_ms = 6000
min_interval_ms = 0
ttl = 300
# bbmd = "127.0.0.2:47808"
# renewal_secs = 30

[safety]
writes = "session"
dangerous = false
```

An SC profile has an `[sc]` table instead of `[bip]`:

```toml
version = 1
transport = "sc"

[sc]
hub_url = "wss://hub.example.com:443/bacnet"
# failover_hub_url = "wss://hub2.example.com:443/bacnet"
ca = "/etc/bacnet/site/ca.pem"
cert = "/etc/bacnet/site/node.pem"
key = "/etc/bacnet/site/node.key"
vmac = "22:10:20:30:40:50"                           # generated when blank
device_uuid = "6f1c2a9e-4407-4b3d-9a51-0e7c8821d40b" # generated when blank
timeout_ms = 6000
min_interval_ms = 0
connect_timeout_ms = 10000
reconnect = true                  # false: a lost hub stays down (no failover then)
reconnect_initial_delay_ms = 10000
reconnect_max_delay_ms = 600000
reconnect_max_retries = 10

[safety]
writes = "session"
dangerous = false
```

An MS/TP profile has an `[mstp]` table. `port` and `mac` have no default;
the rest default as shown:

```toml
version = 1
transport = "mstp"

[mstp]
port = "/dev/serial/by-id/usb-FTDI_USB-RS485-if00-port0"
mac = 3
baud = 38400
max_master = 127
max_info_frames = 1
timeout_ms = 6000
min_interval_ms = 0

[safety]
writes = "session"
dangerous = false
```

Local connection does not establish that a BBMD accepted registration. See
`L` for the first outcome; refusal or timeout is logged while renewal retries
continue. Disconnect stops local renewals and releases the client; it does
not promise immediate deletion of the remote FDT entry. The remote entry
expires according to its TTL plus grace (J.5.2.2–J.5.2.3).

**Devices screen.** Press `d` for the Who-Is form:

| Field | Values |
|---|---|
| Scope | Local broadcast, global broadcast, directed (one address), or remote network |
| Target | For directed: `IP` or `IP:port` on BACnet/IP; `bip6:N`, `bip6:0xVMAC` or `[IPv6]:port` on BACnet/IPv6, where the address is resolved to the VMAC of the node there before the Who-Is goes; an SC VMAC as 12 hex digits on BACnet/SC. For remote network: the network number |
| Range | Blank for every instance, `N`, or `LOW-HIGH` (0 to 4194303) |
| Listen | Seconds to show the request as running while replies arrive (1 to 60, default 3) |

A global or unbounded Who-Is can draw a reply from every device on a site, so
the first Enter shows a one-line warning with the number of devices already
known, and a second Enter sends it. The table shows instance, address,
network, vendor ID, max APDU, segmentation and time since the last I-Am. A
routed device's address is its station in the CLI's target syntax,
`NET:0xMAC` (`2:0x7F000001C335`, or `2:0x05` for an MS/TP station), which
the one-shot commands and the command bar take back as a target. The router
it answered through is in a `Via` column when the whole column fits, and
otherwise left out. Each column is as wide as its header or its widest cell,
so a wide terminal leaves the spare room at the right; instance and address
are never cut. The filter also matches the router. When two addresses answer
for the same instance, a `DUPLICATE` banner names both (a routed one as
`2:0x05 via ROUTER`). If the UI falls behind a burst of I-Am traffic, events
are dropped rather than queued without limit; the status bar's `drop` counter shows how many, and the
table is refreshed from the client's discovery table afterwards.

**Keys:**

| Key | Action |
|---|---|
| `p` | Profile picker/editor/preview (profile-enabled sessions only) |
| `c` | Reconnect; named profiles return to the resolved preview |
| `x` | Disconnect or cancel connection; wait for resources to be released |
| `d` | Who-Is form |
| `/` | Filter rows by instance, address, network or vendor; Enter keeps the filter, Esc clears it |
| `Enter` | Browse the selected device |
| `1` / `2` / `3` / `4` / `5` / `6` / `7` | The devices / Browse, the open device's browser / the watch list / the Network screen (BBMD tables on B/IP; the Networks tab on every transport) / the SC screen / the MS/TP screen / Capture. The status bar's strip numbers those that apply to the session (Network once connected, SC on SC, MS/TP with a serial port), with the one shown in brackets. `1` keeps an open browser as it was, and `2` shows it again; with no device open, `2` stays on the devices and says so |
| `:` | Command bar: `watch DEVICE OBJECT [PROPERTY]`, `csv on`, `csv off`, `writem DEVICE OBJECT PROPERTY VALUE[@P] ...`, `bbmd [ADDRESS]`, `find NAME`, `mstp rotation MS`, `mstp hold MS`, `mstp listen SECS`; the footer shows the syntax of the commands matching what is typed |
| `W` | Arm a write session, or disarm the armed one |
| `w` / `n` | On an open object, in a write session: write a property / relinquish a priority slot |
| `u` | The session ledger |
| `s` / `S` | Next sort column / reverse the order |
| arrows, `j`/`k`, `PgUp`/`PgDn`, `Home`/`End` | Move |
| `L` | Show or hide the log pane |
| `?` | Help: the global keys, this screen's, and the commands; the movement keys scroll it |
| `Ctrl-C` | Close a dialog or cancel the running Who-Is/connection attempt; press again within 2 s to quit |
| `q` | Quit; with changes still in the ledger, the exit guard asks first |

**Device browser.** Enter on a device opens its card (vendor, model, firmware,
protocol revision, supported services and object types, Max APDU,
segmentation, Database_Revision), its Object_List grouped by object type, and
the selected object's properties. From 120 columns the panes sit side by side;
below that only the focused pane is shown and Tab switches between them. Enter
on an object reads it; `/` filters the objects, `r` reads again and Esc goes
back to the devices. A commandable object also shows its 16 command
priorities with the one in control marked `>`, Relinquish_Default and
Present_Value. Status is shown as text badges as well as colour: `[ALM]` in
alarm, `[FLT]` fault, `[OVR]` overridden and `[OOS]` out of service.

The browser reads with one ReadPropertyMultiple per object, falling back to
ReadProperty for each property when a device refuses it, and reads an
Object_List too large for one answer element by element. A device never has
more than one of the browser's requests outstanding, and `--max-rate N` caps
the requests a second across all devices (default 20; 0 for no cap). Writes
go through a write session (below).

**Watch list.** `a` on an object in the browser watches its Present_Value;
`:watch 1234 ai:1 [property]` on the command bar watches any property of a
device in the table, named by its instance, as `dnet:1234` for a routed one,
or by the routed station the table shows, `2:0x05`. A network typed that the
table disagrees with is replaced by the table's, and the footer says so
("2001 is on network 2, not 3"). `3` shows the list: device
(`2:1234` when routed), object, the object's name, property, value, units,
status flags, source, the age of the last value and a sparkline. Columns fit
their content and the device and object are never cut. Below 120 columns the
property shows only when some row watches another property than
Present_Value, and units are left to the value's symbol; from 120 columns
the property always shows, and from 160 the units column too, with more room
for names and the sparkline. `x` stops watching the selected row.

Each row subscribes for COV with a finite lifetime (`--cov-lifetime`, default
300 s) that is renewed before it runs out: SubscribeCOV for Present_Value,
SubscribeCOVProperty for anything else. Removing a row, and quitting, cancel
its subscription on the device; the status bar's `COV` counter is how many
subscriptions the session holds. A renewal that fails turns the row to
polling and it subscribes again later, waiting longer each time it fails.
A device that refuses COV, or a routed one, is polled instead: one
ReadPropertyMultiple per device for up to 16 rows, a point at most every
`--poll-interval` seconds (default 10) counted from the end of its last
poll, and within the shared `--max-rate`; a device that stops answering is
polled again after a wait that doubles up to a minute. The watch list keeps
at most one of its own requests outstanding per device. The browser keeps
its own one-per-device limit, so a device you browse while watching can
have one of each, and the renewals are sent by the client outside both
limits and `--max-rate`, once per lifetime. The list's title shows the
request rate over the last 10 s. Notifications the TUI couldn't
keep up with count in the `drop` counter. With a profile, `:csv on` logs every
value to `watch-<time>.csv` in the profile's workspace and `:csv off` stops.
Its columns are `unix_ms,device,object,property,value,status_flags,source,network`:
`device` is the plain instance and `network` a routed device's network,
empty on the local network.

**Network screen.** Its BBMD tabs are for B/IP, and its Networks tab
(below) works on every transport. `4` shows the foreign-device panel and one BBMD's
tables. With `--foreign-bbmd` (or a profile with a BBMD) the panel shows the
BBMD, the TTL, the last BVLC-Result, a countdown to the next registration and
the lease left; a refused registration raises a red banner on every screen.
Once the BBMD accepts, the TUI reads its FDT and shows the entry the BBMD
recorded for us, the address it forwards broadcasts to; when no entry has our
address, a NAT router may have rewritten it. `:bbmd ADDRESS` reads any other
BBMD, and `:bbmd` alone goes back to ours, as the footer says meanwhile. Tab
switches between the BDT, with what each entry's mask means for
forwarding, the FDT, with each entry's seconds left, and the consistency
matrix; `r` reads again. `m` reads the BDT of every BBMD the tables lead to (up
to 16) and shows which lists which: a peer listed one way only, a BDT without
its own entry, a BBMD listed with different masks, and BBMDs that refuse the
read or don't answer are flagged. Every read is a BVLC request; nothing is
written. The matrix matches BBMDs by address, so a BBMD reached at an address
other than its own BDT entry (behind NAT, its global address in peers' tables
and its private one in its own) shows as two rows, one flagged with no entry
for itself.

**Who-Has and the Networks tab.** `:find NAME` broadcasts a Who-Has for the
object named NAME (the rest of the line, exactly as typed, apostrophes and
runs of spaces included; a name that is one quoted word, `"Zone  Temp"`, is
unwrapped, and `:find ""` closes the panel like `:find` alone) and lists the
devices whose I-Have answers it in a panel below the device table (beside
it from 160 columns, when both fit at their content widths): the
device, the object as the browser labels it (`analog-input 3`), the object's
name (cut with `…` when it doesn't fit, control characters shown as spaces),
the address it answered from as the device table writes it, the router in a
`Via` column, and the network. The name has the room first: the `Via` column
is shown only when it fits beside every name in full. `:find` alone closes
the panel. The Network screen's last tab, Networks, works on every
transport: `r` clears the map and broadcasts a Who-Is-Router-To-Network, and
each router that announces itself, asked or not, is listed against the
networks it reaches. A network number that more than one router claims is
flagged DUPLICATE: two networks may share the number, the routers form a
loop, or the route moved since the last search. The tab's title shows the
local network's number once a Network-Number-Is is heard, and a router's
Reject-Message-To-Network for a message the TUI sent is shown in the footer
with the network and the reason, in kebab-case (`not-directly-connected`).

**SC screen.** `5` shows the BACnet/SC hub connector: its state, the hub in
use (primary or failover) and its VMAC, the latest reconnect attempt and its
backoff, and how long ago the hub was last heard with the latest heartbeat
round trip. Beside it are the local VMAC, the profile's device UUID, the hub
VMAC and any duplicate-VMAC refusals. Tab and Shift-Tab switch the middle
panel between the connected hub's certificate chain (subject, issuer, expiry),
the connection doctor, and **My credentials**: this node's own certificate,
chain and CA file as the pre-dial lint read them (subject, issuer, key
algorithm, SAN, EKU, days to expiry with a warning at 30 and an alarm at 7),
the key match, whether a CA in the CA file issued the certificate directly,
and every finding, with a note on where the hub name check is stricter than
the Annex AB.7.4 default. A dial the lint refused opens that view. The doctor walks a failed dial through its phases (URL,
DNS, TCP, TLS, the WebSocket upgrade with the hub subprotocol, and
Connect-Request/Accept), marks the one that failed and says what to try: an
untrusted CA, an expired or future-dated certificate (with clock skew when it
is close), a key mismatch, a missing client-authentication key usage, a proxy
that drops the upgrade or refuses it with an HTTP status (which the hint
names), a dial that stalls (placed on the step it stalled in), a hub that
never answers the Connect-Request, or a duplicate VMAC. A failed connect opens
the doctor. Connects, losses, reconnect attempts, failover, each failed probe
of the primary while on the failover hub, and the primary's return go on the
screen's timeline and in the log pane, and the status bar
names the hub in use. The one-shot `--sc` commands report errors as before.

**Joining an MS/TP trunk.** An MS/TP session adds a master to the trunk's
token ring, which every station on it notices, so the TUI asks before it opens
the port: a dialog states the station, baud rate, Max_Master and
Max_Info_Frames, lists the serial ports (the one given first, then the
others `mstp-ports` lists; a profile's own port only), and joins on `y`. Like
the write session's T2 changes, nothing but `y` goes ahead; Esc goes back to
the profile, or quits a flags-only session before its first join. Every later
join (`c` after a disconnect or a failed join) shows the dialog again, and
declining it then leaves the session disconnected. On a profile session each
confirmed join is written to the workspace's `audit.jsonl` (event `mstp-join`,
with the port, station, baud, Max_Master and Max_Info_Frames) before the port
opens. Unlike a write, whose audit record failing is only logged, a join that
can't be recorded is not made. `--passive` joins nothing. The
one-shot commands take `--mstp` itself as the request.

**MS/TP screen.** `6` shows this station (port, MAC, baud rate, Max_Master,
Max_Info_Frames), whether it holds the token, the master state (Clause 9.5.6
names), the next station and the one it polls, and a warning when it believes
it is the only master. Beside it are the transport's counters, read once a
second: frames received and sent, NPDUs delivered, tokens passed, retried and
regenerated, invalid and aborted frames, reply timeouts, postponed and late
replies, frames from another station with this MAC, queue refusals, serial
errors, and how late the station's timers fired: the count 5 ms or more late
(the transport's threshold), the largest lateness and, where the pane is tall
enough, how many fell in each lateness band (under 1 ms, 1-2, 2-5, 5-10, 10-25
and 25 ms or more). A Tokio timer fires up to about 2 ms late by its own
rounding, so the first two bands are ordinary. `z` counts from now (the
transport keeps its totals; the screen shows the change since), and Space
pauses the screen or follows the counters again. After `z` the largest
lateness is exact once a later one comes; until then it shows a bound, `<N ms`
(the top of the highest band that moved) or `≤N ms` (the largest before).
Frames themselves are on the Capture screen (`7`).

#### MS/TP monitor and safe join

`bacnet tui --mstp-monitor PORT` opens the port to listen only and runs at the
`PASSIVE` level: no client is built, the port's write side is dropped as it
opens, and the status bar reads `listen only`. The MS/TP screen (`6`, shown
first) opens with a summary (port, baud rate, uptime, stations heard, timestamp
resolution), which goes on to a second line on a narrow terminal rather than
being cut, and has four views, Tab to change:

- **Stations**: a map of every address, masters 0 to 127 and slaves 128 to
  254, each marked `M` master (passes the token or polls), `S` slave (only
  answers), `R` router (sends routed or network-layer messages) or `x` silent
  (heard before, or sent requests, but quiet for 10 s); then a row per station
  with its role in words, the device instance once its I-Am is seen, its frames
  and when it was last heard. From 160 columns the map and the table sit side
  by side; a table with more stations than rows ends with `+N more`.
- **Token ring**: the successor chain, the rotation time (shortest, mean and
  longest, with a sparkline of recent rotations), and per master the tokens it
  got, how long it held each, the frames it sent per token, token retries and
  its Max_Master as its polls show it: `N` once its polls wrap there, `≥N` the
  highest it has polled so far, `?` before any poll.
- **Findings**: a Max_Master that leaves a station out (only when the polls
  show the exact value), a Max_Master more than 8 above the highest station,
  a gap of more than 8 empty addresses between two masters, a rotation over
  2 s and a hold over 500 ms, each with a suggestion. The two 8s are rules of
  thumb, not values from the standard. `:mstp rotation MS` and
  `:mstp hold MS` move the time thresholds.
- **Errors**: header and data CRC errors, malformed or oversize frames,
  partial frames, noise, lost tokens with when and who started the ring again,
  and extended (COBS, Clause 9.10) frames, which are decoded but not
  interpreted. `z` counts from now and Space pauses. The timestamps are only
  as good as the adapter: the screen shows the 16 ms a USB adapter adds.

`J` starts the safe join: the monitor listens for 10 s (`:mstp listen SECS`
changes that), then proposes a free MAC that a master polls and that is within
every Max_Master the line shows, with a Max_Master that reaches every station
heard and `--mstp-max-info-frames`. `+` and `-` pick another free MAC; Enter
opens the same join confirmation as `--mstp`, and `y` there stops the monitor,
closes the port and joins on it, so only one of them holds the port at a time.
Enter and `y` each check the proposal against the newest reading first: if
a station has turned up at that MAC, the dialog closes and the wizard
proposes again. Until the participant session starts the screen stays on the
monitor; a join the worker refuses leaves it listening, with the reason
shown. From then on the session is a participant (`READ-ONLY`), and a later
join is confirmed again. If the join fails once the monitor has closed the
port (the port can't be opened, say), `c` confirms a join again; to listen
again, quit and restart with `--mstp-monitor`. With `--passive` the wizard proposes but never joins. The
monitor is the place to start on an unfamiliar trunk; like the participant, it
has hardware evidence only on Linux, and macOS and Windows are experimental.

#### Capture screen

`7` opens a live list of frames and starts capturing on first use. The
sources:

- **This node's own traffic**, from the transport's frame tap: every B/IP or
  B/IPv6 BVLL message, every BACnet/SC message (in plaintext, since the tap
  sits inside TLS: something no wire capture can show), and on MS/TP every
  frame the station sends or hears, tokens and polls included. Each new
  session subscribes again; the tap starts when the screen is first opened,
  so an SC session's Connect-Request has gone by then.
- **Every frame on an MS/TP trunk**, from the monitor (`--mstp-monitor`),
  line errors included.
- **The wire** with `--capture-device DEVICE` (the `pcap` feature): IPv4
  B/IP datagrams on the `-p` port, between any nodes, for the whole run.
  It needs capture rights rather than running the TUI as root: on macOS
  the ChmodBPF launch daemon that Wireshark installs, on Linux
  `sudo setcap cap_net_raw,cap_net_admin=eip "$(command -v bacnet)"`. With
  the tap running too, this node's own datagrams appear twice, once from
  each source. If libpcap stops (no rights, the device gone), the list's
  bottom line says why.

The list shows, from 120 columns, the frame number, the time (UTC), the
link, the direction (`in`, `out`, or `--` for the monitor's frames), source
and destination as the device table writes them, the link message, the APDU
type, the invoke ID, the service, and any error, reject, abort or NAK. Below
120 columns a one-character mark (`→` out, `←` in, `-` heard on the wire)
replaces the link and direction, and the rest folds into one `Info` column
that starts with the invoke ID (`#7 conf-req read-property`), so a cut keeps
it. A routed NPDU names its routed ends in the CLI's target syntax
(`2:0x05`, or `2:*` for a broadcast on network 2); from 150 columns a `Via`
column shows the link peer it went through. Names are kebab-case
(`read-property`, `original-unicast-npdu`, `bvlc-result`). Errors are
highlighted. Below the list, the selected frame decodes into a tree, layer by
layer down to the service's tags, beside a hex view that marks the octets of
the field selected in the tree (below 120 columns the pane shows the tree,
or the hex view once Tab reaches it). Only the selected frame is decoded.

The ring keeps the newest 50,000 frames (`--capture-ring`); each takes
about 0.5 to 1 KB, its octets and summary, so the default ring holds up to
about 50 MB. Frames captured before a session ended stay. The title counts
frames, the dropped ones (lost before the ring: a full tap queue, a lagging
monitor stream or a busy screen; exact while a source runs) and `LIVE`, or
`PAUSED +N` with the frames the filter would show that arrived since. Keys:

| Key | Action |
|---|---|
| Space | pause the list, or follow the newest frame again; moving the selection pauses too |
| Tab, Shift-Tab | list, tree, hex view; going to the tree or hex view pauses |
| arrows, `j`/`k`, PgUp/PgDn, Home/End | move in the focused pane |
| Enter | in the list, go to the tree (and pause); in the tree, open or close the node |
| Left, Right | in the tree or hex view: close the node (or go to its parent), open it |
| `/` | filter; Enter applies it, Esc on the screen clears it |
| `e`, `E` | export the frames the filter keeps to pcap: B/IP as Ethernet, or as raw IPv4 |
| `z` | forget every frame |
| Esc | clear the filter, then back to the devices |

Filter terms, space-separated, must all match: `dev:N` (from or to device
N's address in the device table, a routed device's as its `NET:0xMAC`
station, or its I-Am; looked up again as the table learns devices),
`mac:TEXT` (in the source or destination), `net:N` (SNET or DNET),
`svc:NAME` (a service's requests, ACKs and errors; `svc:ReadProperty`,
`svc:read-property` and `svc:READ_PROPERTY` are the same), `pdu:TYPE` (the
start or end of the APDU type: `pdu:confirmed`, `pdu:ack`, `pdu:error`;
`pdu:network`, `pdu:link`), `errors` (an Error, Reject or Abort PDU, a BVLC
or SC NAK, a frame that doesn't decode, and the monitor's line errors),
`bcast`, and any other word as text in the row. So `svc:ReadProperty
errors` keeps only Error responses to ReadProperty.

`e` and `E` export the frames the filter keeps into the profile's
workspace or, without a profile, the working directory:
`capture-<UTC time of the last frame>-bip.pcap` (`e`, Ethernet, LINKTYPE 1)
or `-ipv4.pcap` (`E`, raw IPv4, LINKTYPE 228), and `-mstp.pcap` (LINKTYPE
165, each frame from its preamble), for the links it has frames of. A name
that is taken gets `-1`, `-2` and so on; no file is replaced, a file that
can't be finished is removed, and the footer lists what was written. For
tapped B/IP frames the IPv4 and UDP headers are made up from the endpoints
(the UDP checksum is zero, which IPv4 allows); a broadcast the tap took in
is written with this node's address as its destination, since that is
where the tap saw it arrive. Wireshark and `tshark -r` read the files as
BVLC, BACnet and BACnet-APDU, or MS/TP. B/IPv6 and BACnet/SC frames are not
exported yet.

#### Write sessions

The TUI is read-only until `W` arms a write session. Arming needs a profile
whose policy allows it (`writes = "session"`), because the profile's
workspace holds the ledger and the audit log; a flags-only session can't arm.
The arm dialog shows the site and network, takes an optional reason or ticket,
and a length in minutes (default 15, at most 480). While armed the status bar
is red with a countdown; at zero the session disarms itself, and `W` disarms
it early. Only one session at a time may change a profile's ledger: a second
`bacnet tui` on the same profile can't arm, and says which process holds it.
Raising a profile's write policy (`never` to `session`, or `dangerous` to
true), or using a profile saved before write policies existed, asks for the
authorization attestation again before connecting.

In the browser, with an object open, `w` writes a property (Present_Value by
default, at priority 16 by default for a commandable object) and `n`
relinquishes a priority slot (writes NULL to it). Values use the one-shot
`write` syntax (`42`, `72.5`, `true`, `active`, `"text"`, `null`); a number is
sent as the datatype the property already holds, and a typed `null` is a
relinquish. `:writem DEVICE OBJECT PROPERTY VALUE[@P] ...` sends several writes
to one device as one WritePropertyMultiple; each property may appear only once
in a batch, because a failed batch names the failing write without its
priority.

The command bar splits its line into words itself (all but `:find`, which
takes the rest of the line). Single or double quotes group words, and a
backslash makes the quote, whitespace character or backslash after it plain
text; a quote left open is an error. A quoted or escaped VALUE is a
CharacterString, with any priority after the last quote or escape:
`:writem 1001 av:1 description "two words"`, `... Zone\ 2` or
`... "Zone 2"@12`. A quoted number, such as `'72.5'`, is a string too, so a
numeric property refuses it.

Before every write the TUI reads the object's priority array, Present_Value,
Relinquish_Default and Out_Of_Service, then shows a preview: the old and new
slot or property value, who holds the slot (empty, our earlier write, or
another commander, whose value the write replaces without telling them), and
whether Present_Value will change; a write at a weaker priority than the one in
control won't change it. The preview then asks for a confirmation that depends
on the tier:

| Tier | Changes | Confirm with |
|---|---|---|
| T1 | Priorities 9 to 16, non-commandable properties | Enter |
| T2 | Priorities 3, 4, 7 and 8; Out_Of_Service; relinquishing another commander's slot; any `writem` batch | `y` |
| T3 | Priorities 1, 2 and 5 (life safety, critical equipment) | typing the device instance, then Enter |
| Blocked | Priority 6, which the standard keeps for minimum on and off times | refused, unless the profile sets `dangerous = true` (then T3) |

Enter and `y` are ignored for 300 ms after a preview opens, so a repeating key
can't confirm a write unseen, and a preview left unconfirmed for two minutes is
declined. Just before sending, the TUI reads the targets again; if a pre-image
or a slot's holder changed since the preview, nothing is sent and the result
says to review it again. A priority array that was read but isn't 16 values
refuses the write. Esc declines; nothing is sent and the audit log records it.
The result shows
each write's outcome; a device whose write policy refuses it
(SERVICE_REQUEST_DENIED) is shown as `DENIED`, apart from other errors.

**Ledger.** Every change still in effect is kept with its pre-image in
`ledger.json`, saved before each request goes out and after each answer, so it
outlives a crash. A slot counts as ours only while it still holds what we
wrote; one someone has overwritten since is treated as another commander's.
`u` lists the ledger, and `R` then `y` reverts everything, latest write first:
our slots are relinquished, or given back the value another commander had
there; other properties (Out_Of_Service included) get their old value back. A
write to one array element (`state-text[2]`) keeps that element's old value and
reverts just that element; if the element couldn't be read before the write,
revert leaves the entry and says the old value is unknown. Writing index 0 (the
array's length) is a resize, which is never reverted. Element and whole writes
to one array revert to the array as it was before the first of them. A
device not yet discovered (after a restart, say) is looked for first. Reverting
needs a connection but not an armed session. Quitting with entries in the
ledger, or with a write session request still out, opens the exit guard: `r`
reverts all and quits, `k` keeps them, records that in the audit log and quits
(also when passive or disconnected), Esc cancels. Entries left by an earlier
run, whether kept or left by a crash, are listed when the next session on the
profile starts; one a crash cut off mid-request is recorded as unknown.

**Audit log.** `audit.jsonl` in the workspace gets one JSON object a line,
flushed and synced: UTC time, OS user, host, profile, transport and local
address, session level, the event, and for a write its target, service,
parameters, priority, pre-image and outcome (ack, error class and code,
denied, reject, abort, timeout or not sent). A `sending` record goes in before
each request. Arming, disarming, refusals, declined confirmations, reverts and
kept entries are recorded too; a session that ran out shows as
`write-session (expired)`. Fields
named like passwords are written as `[redacted]`. Reads are not logged. This
is a client-side record, separate from BACnet's own audit reporting.

**Passive.** `--passive` builds no client at all, so nothing is transmitted;
discovery, browsing, watching and arming are refused with a notice. The
status bar shows `PASSIVE`.

**Footprint.** The status bar's `COV n` and `FD` show the state this session
holds on other devices: COV subscriptions, and our entry in a BBMD's
foreign-device table. `FD` appears only with a registration and says
where it stands (`held`, `pending`, `refused`, `no answer` or `lapsed`), from
the same state the Network screen's banner reads. `ledger n` counts changes
still in effect.

**Status bar and footer.** Both fit their width by priority: at 80 columns
the least useful fields and keys go first, while the link state, the
session level, `? help` and `q quit` always stay. A notice sits at the
footer's left beside the keys, and goes when the screen changes or a dialog
opens. A failed connection's footer shows its cause in a few words with the
recovery keys (`p settings`, `c retry`); the device screen and the log pane
keep the whole error. Before a profile connects the status bar reads
`no profile`. Each screen's footer lists only keys that work there, so the
SC and MS/TP screens of a session on another transport offer just the way
back (and `p` for profiles). Below 120 columns the browser's priority array
is a pane of its own, after Properties in the Tab order, so the properties
keep the whole width.

The `tui` cargo feature is on by default from 0.14.0, so the release binaries
and a plain `cargo install bacnet-cli` include it. `--no-default-features`
builds a lean CLI without it (add back what you need, such as
`--no-default-features --features sc-tls`): ratatui and crossterm aren't built,
`bacnet tui` prints rebuild advice, and plain `bacnet` prints the usage. The
one-shot commands and their JSON output are the same with or without the
feature.

### Device Discovery

```bash
bacnet discover                          # discover all devices
bacnet discover 1000-2000                # discover devices in instance range
bacnet discover --wait 5                 # wait 5 seconds for responses (default: 3)
bacnet discover --target 192.168.1.100   # directed (unicast) WhoIs
bacnet discover --dnet 2                 # target a specific remote network
bacnet discover --bbmd 10.0.0.1          # register once as a foreign device, then discover
bacnet --foreign-bbmd 10.0.0.1 discover  # discover through the BBMD as a foreign device
```

**Discover flags:**

| Flag | Default | Description |
|------|---------|-------------|
| `--wait <N>` | `3` | Seconds to wait for responses |
| `--target <ADDR>` | | Send directed WhoIs to a specific IP address |
| `--dnet <N>` | | Target a specific remote network number |
| `--bbmd <ADDR>` | | Register once as a foreign device with this BBMD, for the global `--ttl`, before discovering (BIP only). The Who-Is still goes out as a local broadcast; `--foreign-bbmd` sends it through the BBMD |

```bash
bacnet find "Zone Temp"           # find objects by name (WhoHas)
bacnet find "Zone Temp" --wait 5  # wait 5 seconds for responses
```

The name is one word: quote one with spaces. `--name "Zone Temp"` says the
same; giving both, or neither, is a usage error (exit 2).

`find` sends its Who-Has as a global broadcast, so routers pass it to every
network, and prints every I-Have that answers within the wait, one row per device
and object. With `--json` it prints an array, empty when nothing answered:

```json
[{ "device": 1234, "object": "ANALOG_INPUT:7", "name": "Zone Temp" }]
```

```bash
bacnet whois-router                      # routers and the networks they reach
```

### Reading Properties

```bash
bacnet read 192.168.1.100 ai:1 pv              # read present-value
bacnet read 192.168.1.100 analog-input:1 present-value  # full names work too
bacnet read 192.168.1.100 device:1234 object-name
bacnet read 192.168.1.100 ai:1 ol[3]            # read array index (object-list[3])
bacnet read 192.168.1.100 ai:1 all              # read ALL properties via RPM

# Read multiple properties (ReadPropertyMultiple)
bacnet readm 192.168.1.100 ai:1 pv,object-name ao:1 pv

# Read range (trend logs, lists)
bacnet read-range 192.168.1.100 trend-log:1 log-buffer
bacnet read-range 192.168.1.100 trend-log:1     # defaults to log-buffer
bacnet read-range 192.168.1.100 trend-log:1 --position 1 --count 50
bacnet read-range 192.168.1.100 trend-log:1 --sequence 1200 --count -20   # backward
bacnet read-range 192.168.1.100 trend-log:1 --time 2026-10-05T09:00 --count 50
bacnet read-range 192.168.1.100 trend-log:1 --all --count 100            # the whole log
bacnet --min-interval-ms 50 read-range 192.168.1.100 trend-log:1 --all --sequence 1201
```

`read` and `readm` interpret Tags as named values: a whole array looks like
`["exhaust", "floor"=3]`, an indexed element like `"floor"=3`, and index zero
prints the count. An empty array prints `[]`. Names and text values are quoted
and escaped; a semantic tag has no equals sign, while a valued NULL prints
`"name"=null`. Invalid Tags bytes produce one `[invalid Tags; raw: ...]`
marker for the complete payload. JSON keeps the existing `object`, `property`
and string `value` fields, with an array index included in the property name.
Other properties and `read-range` retain their existing formatting.

`--position`, `--sequence` and `--time` pick the range, each with `--count`
(default 100; negative reads backward). `--time` takes the device's local time
as `YYYY-MM-DDTHH:MM[:SS[.hh]]`. The heading shows the result flags
(FIRST_ITEM, LAST_ITEM, MORE_ITEMS) and the first sequence number, so a page
that left items out says so. A read shows an answer that breaks a ReadRange
rule anyway and names the rules under `violations` (in the heading and the
JSON); `--strict` refuses such an answer instead. `--all` pages through a
log's Log_Buffer from the oldest record, or from the start given, `--count`
records a page, and ends with `pages=` and `next:`, the flags that resume the
read later; JSON adds `pages`, `next` and `gaps`. When an error stops it part
way, such as a timeout, it prints what it read and `next:` first. A device
whose sequence numbers are inconsistent (it answers from before the record
asked for) fails; read it with `--all --position 1`.

`read-range` decodes the Log_Buffer of a Trend Log, Event Log, Trend Log
Multiple or Audit Log record by record, showing each record's timestamp, datum
(a value, log status, time change, event notification or audit notification)
and, for a Trend Log, status flags (blank when none is set). JSON output
lists them under `records`, with `result_flags` and `first_sequence_number`,
and the hex of anything from the first record that doesn't decode under
`undecoded`. Other properties decode as application
values under `items`, where anything that doesn't decode ends the list as
`[raw: ..]` hex.

**Aliases:** `rp` = read, `rpm` = readm, `rr` = read-range

### Writing Properties

```bash
bacnet --allow-write write 192.168.1.100 av:1 pv 72.5              # write a float value
bacnet --allow-write write 192.168.1.100 av:1 pv 72.5 --priority 8 # with priority (1-16)
bacnet --allow-write write 192.168.1.100 bv:1 pv true               # boolean
bacnet --allow-write write 192.168.1.100 bv:1 pv active              # enumerated (active=1)
bacnet --allow-write write 192.168.1.100 av:1 pv null --priority 8   # relinquish
bacnet --allow-write write 192.168.1.100 av:1 on "\"Zone Temp\""    # character string
bacnet --allow-write write 192.168.1.100 av:1 pv 72.5@8             # inline priority syntax
bacnet --allow-write write 192.168.1.100 av:1 pv enumerated:3       # explicit enumerated
bacnet --allow-write write 192.168.1.100 sc:1 pv date:2024-03-15    # date value
bacnet --allow-write write 192.168.1.100 sc:1 pv time:14:30:00      # time value
bacnet --allow-write write 192.168.1.100 nc:1 pv object:ai:1        # object identifier value
```

Every `write` needs `--allow-write` (see [Write guard](#write-guard)).

**Write multiple properties (WritePropertyMultiple):** each object is followed
by its comma-separated `property=value` writes, each value with an optional
`@priority`. Like `write`, it needs `--allow-write` and takes routed targets.
A value can't contain a comma, since the comma separates the writes. The
TUI's `:writem` command takes a different grammar, `OBJECT PROPERTY VALUE`
groups separated by spaces; the one-shot takes `OBJECT prop=value,...`.

```bash
bacnet --allow-write writem 192.168.1.100 av:1 pv=72.5@8,desc='"Zone Temp"' av:2 pv=68.0
```

**Aliases:** `wp` = write, `wpm` = writem

**Value formats:**

| Format | Example | BACnet Type |
|--------|---------|-------------|
| `null` | `null` | Null |
| `true` / `false` | `true` | Boolean |
| `active` / `inactive` | `active` | Enumerated (1/0) |
| Integer | `42`, `-5` | Unsigned / Signed |
| Float | `72.5`, `1e10` | Real |
| Quoted string | `"hello"` | CharacterString |
| `enumerated:N` | `enumerated:3` | Enumerated |
| `date:YYYY-MM-DD` | `date:2024-03-15` | Date (use `*` for unspecified) |
| `time:HH:MM:SS[.hh]` | `time:14:30:00` | Time (use `*` for unspecified) |
| `object:type:inst` | `object:ai:1` | ObjectIdentifier |

Inline priority: append `@N` to any value (e.g., `72.5@8`, `null@16`).

### COV Subscriptions

```bash
bacnet subscribe 192.168.1.100 ai:1                     # unconfirmed COV, 300 s, renewed
bacnet subscribe 192.168.1.100 ai:1 --confirmed          # confirmed COV
bacnet subscribe 192.168.1.100 ai:1 --lifetime 60        # 60 s, renewed
bacnet --allow-write subscribe 192.168.1.100 ai:1 --lifetime 0   # never expires
```

Subscribes and then watches for COV notifications in real time. Press Ctrl+C
(or send SIGTERM, on Unix) to stop watching. The subscription asks for a
lifetime of 300 seconds unless `--lifetime` says otherwise; the CLI renews it 30
seconds before it would run out (for a short lifetime, halfway through) and
cancels it when the watch stops, so nothing is left on the device. A renewal
that fails ends the watch with an error, after the cancel. `--lifetime 0` asks
for a subscription that never expires (Clause 13.14.1.1.4): it is not renewed
or cancelled, it stays after the CLI exits, and it needs `--allow-write`. A
process killed outright (SIGKILL) can't cancel; its subscription runs out with
its lifetime.

**Alias:** `cov` = subscribe

### Alarms and Events

```bash
bacnet alarms 192.168.1.100                              # get event/alarm summary

bacnet --allow-write ack-alarm 192.168.1.100 ai:1 --state 1 \
  --timestamp sequence:417 --ack-time time:14,30,00,00
bacnet --allow-write ack-alarm 192.168.1.100 ai:1 --state 1 --source "operator" \
  --timestamp time:14,29,58,25 \
  --ack-time "datetime:2026,9,2,3;14,30,00,00"
```

**ack-alarm flags:**

| Flag | Default | Description |
|------|---------|-------------|
| `--state <N>` | (required) | Event state to acknowledge (0=normal, 1=fault, etc.) |
| `--timestamp <SPEC>` | (required) | Exact timestamp from the original event notification |
| `--ack-time <SPEC>` | (required) | Caller-selected time of acknowledgment |
| `--source <S>` | `bacnet-cli` | Acknowledgment source string |

Both timestamp flags use the same strict grammar:

- `sequence:<0..65535>`
- `time:<hour>,<minute>,<second>,<hundredths>`
- `datetime:<full-year>,<month>,<day>,<day-of-week>;<hour>,<minute>,<second>,<hundredths>`

Date years are `1900..2154`; months are `1..14`, days are `1..34`,
days-of-week are `1..7`, and Time uses hours `0..23`, minutes/seconds
`0..59`, and hundredths `0..99`. Any Date/Time component may be `255`
when BACnet's unspecified value is intended (use full-year `255` for an
unspecified year). Values are preserved exactly and neither timestamp is
inferred from a clock. Quote `datetime:` values in command shells because the
grammar contains a semicolon. `--timestamp` must come from the original event
notification; `--ack-time` is explicitly chosen by the caller. The current
raw `alarms` response is not a guided source for these values.

**Alias:** `ack` = ack-alarm

### Device Management

```bash
# Communication control
bacnet --allow-write control 192.168.1.100 disable-initiation --duration 5
bacnet --allow-write control 192.168.1.100 disable-initiation
bacnet --allow-write control 192.168.1.100 enable
bacnet --allow-write control 192.168.1.100 disable-initiation --password secret

# Reinitialize
bacnet --allow-write reinit 192.168.1.100 coldstart
bacnet --allow-write reinit 192.168.1.100 warmstart --password secret
bacnet --allow-write reinit 192.168.1.100 start-backup
bacnet --allow-write --yes reinit 1234 activate-changes   # in a script
```

Communication control and reinitialize need `--allow-write`. At a terminal they
then ask you to type the device instance (or the address target as given)
before anything is sent; add `--yes` to skip that, which a script must do
because its stdin is not a terminal. See [Write guard](#write-guard).

**Control actions:** `enable`, `disable`, `disable-initiation`. `disable` is
deprecated, and a rusty-bacnet server refuses it under every DCC policy.

**Control flags:**

| Flag | Description |
|------|-------------|
| `--duration <M>` | Duration in minutes |
| `--password <P>` | Password string |

**Aliases:** `dcc` = control

**Reinit states:** `coldstart`, `warmstart`, `start-backup`, `end-backup`, `start-restore`, `end-restore`, `abort-restore`, `activate-changes`

**Reinit flags:**

| Flag | Description |
|------|-------------|
| `--password <P>` | Password string |

```bash
# Time synchronization
bacnet --allow-write time-sync 192.168.1.100
bacnet --allow-write time-sync 192.168.1.100 --utc

# Create/delete objects
bacnet --allow-write create-object 192.168.1.100 av:100
bacnet --allow-write delete-object 192.168.1.100 av:100
```

**Alias:** `ts` = time-sync

### File Transfer

```bash
bacnet file-read 192.168.1.100 1                             # stream payload as hex
bacnet file-read 192.168.1.100 1 --count 4096 --output data.bin
bacnet file-read 192.168.1.100 1 --access record --output records
bacnet --allow-write file-write 192.168.1.100 1 firmware.bin           # write file
bacnet --allow-write file-write 192.168.1.100 1 firmware.bin --start 0 # with offset
```

`file-read` decodes each AtomicReadFile ACK and keeps requesting windows until
the peer returns `End Of File = TRUE`. Stream mode writes or displays only the
returned file-data octets, never the encoded ACK. Record mode never
concatenates records: it requires an output directory and writes each record,
including a zero-length record, to
`record-{absolute-index:010}.bin` (for example,
`record-0000000007.bin`). Each ACK's returned start must exactly match the
cursor requested for that window; a gap or overlap fails before that window is
written. The actual returned octet or record count advances the next cursor.

`--start` must be non-negative and `--count` must be greater than zero. The
count is the window size for each request, not a total transfer limit. Stream
display without `--output` is limited to 1 MiB of cumulative payload; use
`--output FILE` for larger files. Stream and record output refuse an existing
final target. New output is staged in a sibling file or directory and published
only after authoritative EOF; remote, decode, cursor, or write failures remove
staging and leave the final target absent. If cleanup fails, the error reports
the retained staging path. This is not a crash journal or a hostile
concurrent-writer no-clobber guarantee.

**file-read flags:**

| Flag | Default | Description |
|------|---------|-------------|
| `--access <MODE>` | `stream` | `stream` or `record` access |
| `--start <N>` | `0` | Initial octet position or record index (non-negative) |
| `--count <N>` | `1024` | Positive per-request octet or record window size |
| `--output <PATH>` | | Stream output file or required record output directory |

**file-write flags:**

| Flag | Default | Description |
|------|---------|-------------|
| `--start <N>` | `0` | Start position in file |

### Network and Routing

```bash
bacnet whois-router                       # routers and the networks they reach
bacnet whois-router --dnet 2 --wait 5     # only the router to network 2
```

`whois-router` broadcasts a Who-Is-Router-To-Network and lists each router
that answers within `--wait` seconds (default 3), at its address on the
local network, with every network its I-Am-Router-To-Network messages
named. With `--json` it prints an array, empty when no router answered:

```json
[{ "router": "192.168.1.1:47808", "networks": [2, 3] }]
```

A device on one of those networks is then a routed target, such as
`bacnet read 2:1234 ai:1 pv` for device 1234 on network 2.

### BBMD Management

These commands are BACnet/IP only.

```bash
bacnet bdt 192.168.1.1              # read broadcast distribution table
bacnet fdt 192.168.1.1              # read foreign device table
bacnet register 192.168.1.1 --ttl 300   # register as foreign device
bacnet --allow-write unregister 192.168.1.1   # unregister from BBMD (a write)
```

`register` sends one registration and exits; it is not renewed. To stay
registered, use foreign-device mode below, or the TUI's Network screen.

#### Foreign-device mode

`register` sends one registration from an otherwise normal node, so the
CLI's own broadcasts stay on the local subnet. With the global
`--foreign-bbmd ADDR`, the CLI's transport is a foreign device instead: it
registers with that BBMD when it starts, renews on its own (every `--renewal`
seconds, half the `--ttl` by default) and sends its broadcasts through the
BBMD, so `discover` reaches the BBMD's network (#938).

```bash
bacnet --foreign-bbmd 203.0.113.7 discover                 # Who-Is through the BBMD
bacnet --foreign-bbmd 203.0.113.7:47808 --ttl 600 --renewal 20 discover 1000-2000
```

A one-shot command waits for the BBMD to answer the first registration and
fails if it refuses or doesn't answer. The TUI reports the outcome, keeps
trying either way, and shows the registration on its Network screen.
Behind NAT, keep `--renewal` short so the mapping the BBMD's replies come back
on stays open; the TTL can stay long. While the transport manages the
registration, `register` and `unregister` to that BBMD are refused; use
another BBMD or a normal-mode CLI. With `--foreign-bbmd`, log lines go to
stderr so they can't corrupt JSON output.

#### Multicast mode

With `--bip-multicast GROUP`, the CLI's transport joins that IPv4 multicast
group on `--interface` as a B/IP-M node (Annex J.8, #941): `discover` sends
its Who-Is to the group, and only broadcasts sent to the group are taken,
not the subnet's. The interface must be explicit. The multicast TTL is 1 by
default, which keeps the group on the local network; raise it with
`--bip-multicast-ttl` when routers forward the group between subnets. The
group's UDP port is `--port` unless `--bip-multicast-port` names another.
Several stacks on one host at one address can't share a port, so each takes
its own `--port` (0 picks one) and they share `--bip-multicast-port`. The
group can't be in 224.0.0.0/24, and `--bip-multicast` can't be combined
with `--foreign-bbmd`. A named TUI profile keeps the same settings as
`multicast`, `multicast_ttl` and `multicast_port` in its `[bip]` table, and
the profile editor has Group and Group TTL fields (an empty Group clears
multicast).

```bash
bacnet --interface 192.0.2.20 --bip-multicast 239.255.66.1 discover
# Two stacks on one host at one address: their own ports, one group port.
bacnet --interface 127.0.0.1 --port 0 --bip-multicast 239.255.66.1 --bip-multicast-port 47809 discover
```

### Packet Capture

Requires the `pcap` feature (included in Linux pre-built binaries). Live capture requires root/sudo on most systems.

```bash
# Live capture (summary mode)
bacnet capture
bacnet capture --device en0

# Full protocol decode
bacnet capture --decode
bacnet capture --device eth0 --decode

# Save to pcap file
bacnet capture --save traffic.pcap
bacnet capture --save traffic.pcap --quiet      # headless recording

# Read pcap file (offline analysis)
bacnet capture --read traffic.pcap
bacnet capture --read traffic.pcap --decode

# Filtering
bacnet capture --filter "host 192.168.1.100"
bacnet capture --filter "host 10.0.0.0/24"

# Limit capture
bacnet capture --count 100 --save sample.pcap

# Combine: filter, decode, and save
bacnet capture --device eth0 --filter "host 10.0.0.1" --decode --save filtered.pcap
```

**Capture flags:**

| Flag | Description |
|------|-------------|
| `--read <FILE>` | Read from pcap file (offline mode) |
| `--save <FILE>` | Save packets to pcap file |
| `--quiet` | Suppress output (use with `--save`) |
| `--decode` | Full protocol decode (BVLC/NPDU/APDU/service) |
| `--device <NAME>` | Network interface name (e.g., `en0`, `eth0`) |
| `--filter <EXPR>` | Additional BPF filter (appended to `ip and udp port PORT`, where PORT is the global `-p`, 47808 by default) |
| `--count <N>` | Stop after N packets |
| `--snaplen <N>` | Max bytes per packet (default: 65535) |

Note: `--read` and `--device` are mutually exclusive; `--quiet` requires `--save`.

**Output example (summary):**
```
12:34:56.789  192.168.1.100:47808 -> 192.168.1.255:47808  ORIGINAL_BROADCAST_NPDU  WHO_IS
12:34:56.812  192.168.1.50:47808  -> 192.168.1.100:47808  ORIGINAL_UNICAST_NPDU    I_AM
12:34:57.001  192.168.1.100:47808 -> 192.168.1.50:47808   ORIGINAL_UNICAST_NPDU    READ_PROPERTY
```

**Output example (full decode with `--decode`):**
```
12:34:57.001  192.168.1.100:47808 -> 192.168.1.50:47808  ORIGINAL_UNICAST_NPDU  READ_PROPERTY
  BVLC: ORIGINAL_UNICAST_NPDU (0x0a), length=25
  NPDU: version=1, no-routing
  APDU: Confirmed-Request, invoke-id=1, seg=no
  Service: READ_PROPERTY
```

### Transport Variants

```bash
# BACnet/IPv6
bacnet --ipv6 discover
bacnet --ipv6 read bip6:1234 ai:1 pv                # device 1234's VMAC
bacnet --ipv6 read [2001:db8::10]:47808 ai:1 pv     # resolved to its VMAC first

# BACnet/SC (requires sc-tls feature)
bacnet --sc --sc-url wss://hub:443 --sc-ca site-ca.pem --sc-cert cert.pem --sc-key key.pem --sc-vmac 22:01:02:03:04:05 --sc-device-uuid 00112233-4455-6677-8899-aabbccddeeff discover
```

**SC trust migration:** Existing `bacnet --sc` client invocations must now add
`--sc-ca <FILE>`, naming a nonempty, usable site CA PEM file. Only certificates
in that file become trust anchors: system roots and environment trust settings
are not fallback sources, and there is no insecure opt-in. Keep supplying the
operational `--sc-cert` and matching `--sc-key`, hub URL, non-reserved local VMAC,
and nonzero device UUID. Global flags can appear before or after the subcommand;
quote paths containing spaces in a shell. TLS 1.3-only remains local policy.

SC construction rejects missing/empty CA paths, unreadable/empty/malformed or
unusable CA PEM, and invalid or mismatched local cert/key before DNS/TCP dialing.
Failures retain nonzero exit status and stderr diagnostics, not a successful JSON
result. SC tracing (including connection-close warnings) also goes to stderr so
it cannot corrupt JSON stdout. Before any DNS or TCP, the credential lint of
[`bacnet sc-check`](sc-check.md#before-a-dial) refuses an expired or
not-yet-valid operational certificate, an unreadable one, and a key that does
not match it, with the reason on stderr; its warnings (expiry within 30 days, a
certificate no `--sc-ca` CA issued directly) are logged on stderr and the
command goes on. The hub's certificate is still verified during TLS. Help/version, capture
paths that do not construct a client, and non-SC transports do not load SC files.
A build without `sc-tls` still reports its rebuild advice without opening them.

The CLI tests run the built executable against an ephemeral Rust mTLS SC hub and
BACnet server, parse a known ReadProperty JSON value, and cover explicit/wrong
site trust, credential failures, TLS 1.2 rejection, and pre-dial listener checks.
This is bounded native CLI evidence, not full Annex AB security-profile or
external-device interoperability certification. The CLI now delegates local
policy construction to `ScNodeTlsConfig`; CLI flags and error phases stay compatible
despite the [Rust source break](rust-api.md#strict-local-node-tls-configuration).
Credentials are offered when requested and compatible, not proof of remote hub
verification. A trusted server without CertificateRequest can complete, and normal
TLS resumption may not retransmit certificates. That work is still incomplete.

### MS/TP

With the `serial` feature, `--mstp PORT` joins an RS-485 trunk as a master
station, and every command runs over it. Joining takes part in token passing,
so it is never implicit: `--mstp` is the request, and `--mstp-mac` has no
default. Pick a MAC that no other device on the trunk uses (two stations with
one MAC break the token ring for everyone), and a baud rate the trunk already
runs at.

```bash
# The serial ports to choose from: a JSON array of names, possibly empty.
bacnet mstp-ports --json

# Find the devices on the trunk as station 3, then read one.
bacnet --mstp /dev/ttyUSB0 --mstp-mac 3 --mstp-baud 38400 discover
bacnet --mstp /dev/ttyUSB0 --mstp-mac 3 --json read mstp:5 ai:1 pv

# Or stay on the trunk in the terminal UI, which asks before it joins.
bacnet --mstp /dev/ttyUSB0 --mstp-mac 3
```

The JSON from a read is the same on every transport:

```json
{
  "object": "ANALOG_INPUT:1",
  "property": "PRESENT_VALUE",
  "value": "72.5"
}
```

A station is addressed as `mstp:N` (or `mstp:0xNN`). A station behind a
router is still `DNET:0xMAC`, as on any other transport. `mstp-ports` opens no
port and needs no `--mstp`. It puts the likely choice first. On Linux it lists
the stable names under `/dev/serial/by-id/`, then USB adapters (`ttyUSB*`,
`ttyACM*`), then the rest; an adapter with a by-id name is listed once, by that
name, not again as the `ttyUSB*` device it points to. On macOS it lists only the call-out devices
(`/dev/cu.*`), which open without waiting for a carrier signal the way their
`/dev/tty.*` twins do, leaves out Bluetooth ports and the debug console, and
puts USB adapters first. On Windows it lists USB adapters first, each group in
`COMn` number order.

Flag mistakes are usage errors: the CLI exits with status 2 before opening the
port and prints nothing on stdout, and the message on stderr names the values
the flag takes. That includes an empty port name and, on either side of the
subcommand, any B/IP, B/IPv6 or BACnet/SC flag (`-i`, `-p`, `-b`,
`--foreign-bbmd`, `--ttl`, `--renewal`, `--ipv6*`, `--device-instance`,
`--sc*`), which MS/TP would otherwise ignore. A port that can't be opened exits with status 1.

The station runs its token handling on a dedicated thread. Its timing defaults
suit a USB adapter: it waits up to 100 ms for the rest of a frame, and its
usage and reply timeouts are the longest Clause 9.5 allows.

**Real-time scheduling (Linux).** Two flags schedule that thread, for any
station the process joins as: with `--mstp`, or in the terminal UI from a
profile or the monitor's join. The passive monitor (`tui --mstp-monitor`)
listens on its own, normally scheduled; only a station joined from it gets the
settings. They are host settings, so profiles don't keep them, and a terminal
UI session whose transport isn't MS/TP logs a warning that they don't apply.

| Flag | Effect |
|---|---|
| `--mstp-rt-priority N` | Run the thread under `SCHED_FIFO` at priority N, 1 to 99. Needs `CAP_SYS_NICE` or an `RLIMIT_RTPRIO` of at least N (`ulimit -r`), and real-time budget in the process's cgroup where the kernel has real-time group scheduling. |
| `--mstp-cpus CPUS` | Pin the thread to these CPUs, such as `2` or `2,3`. |

```bash
bacnet --mstp /dev/ttyUSB0 --mstp-mac 3 --mstp-rt-priority 40 --mstp-cpus 3 discover
bacnet tui --mstp-monitor /dev/ttyUSB0 --mstp-rt-priority 40
```

A setting the kernel won't apply fails the join with the reason (exit status 1
for a one-shot command; the terminal UI shows the error), never a station that
quietly runs without it. On macOS and Windows both flags are usage errors. The
join confirmation shows the thread settings asked for, the MS/TP screen's
`Thread` row shows what the kernel applied (such as `FIFO 40, CPU 3`, or
`normal`), and a profile's `mstp-join` audit record has `realtime_priority` and
`cpu_affinity`. They lower scheduling latency; they don't guarantee timing. Compare the MS/TP
screen's timer lateness with and without them, and see the
[Rust API guide](rust-api.md#real-time-scheduling-and-cpu-affinity-linux) for
PREEMPT_RT, interrupt-thread priorities and memory locking.

**Platforms.** The Linux release binary includes MS/TP. On macOS and Windows,
`serial` builds and runs in CI with in-memory and pseudo-terminal tests, but no
USB RS-485 adapter has been tested there yet, so their release binaries leave
it out; build from source with `--features serial` to try it. Only the Rust
library's kernel RS-485 mode and `serial-gpio` direction control are
Linux-only, and the CLI uses neither: it expects an adapter that switches
direction by itself.

**Linux permissions.** Serial devices belong to the `dialout` group on Debian,
Ubuntu and Raspberry Pi OS (`uucp` on Arch). Add yourself once and log in again:

```bash
sudo usermod -aG dialout "$USER"
```

**USB adapter latency.** A USB adapter hands received octets to the host in
batches, and an FTDI chip waits up to its latency timer, 16 ms by default,
before sending a partial batch. That delay counts against the time a station
has to answer a token or a poll (Clause 9.5), so set it to 1 ms on Linux:

```bash
cat /sys/bus/usb-serial/devices/ttyUSB0/latency_timer      # 16 by default
echo 1 | sudo tee /sys/bus/usb-serial/devices/ttyUSB0/latency_timer
```

The setting lasts until the adapter is unplugged; a udev rule can make it
permanent. Other chips (CH340, CP210x) have no such timer. At 9600 and 19200
baud, field reports of USB adapters show confirmed requests timing out while
the token still circulates (#707); prefer 38400 or 76800 where the trunk
allows. On-wire timing is not qualified on any adapter yet (#502).

## Object Type Shorthand

| Short | Full Name |
|-------|-----------|
| `ai` | analog-input |
| `ao` | analog-output |
| `av` | analog-value |
| `bi` | binary-input |
| `bo` | binary-output |
| `bv` | binary-value |
| `msi` | multi-state-input |
| `mso` | multi-state-output |
| `msv` | multi-state-value |
| `dev` | device |
| `sc` | schedule |
| `cal` | calendar |
| `nc` | notification-class |
| `trn` | trend-log |
| `lo` | loop |
| `lp` | life-safety-point |
| `lsp` | life-safety-point |
| `acc` | accumulator |
| `pi` | pulse-converter |
| `prg` | program |
| `cmd` | command |

All BACnet object types are also accepted by full name in kebab-case (e.g., `analog-input`, `notification-forwarder`, `color-temperature`) or by numeric value.

## Property Shorthand

| Short | Full Name |
|-------|-----------|
| `pv` | present-value |
| `on` | object-name |
| `ot` | object-type |
| `desc` | description |
| `sf` | status-flags |
| `es` | event-state |
| `oos` | out-of-service |
| `pa` | priority-array |
| `rd` | relinquish-default |
| `ol` | object-list |
| `all` | ALL (reads all properties via RPM) |

All BACnet properties are also accepted by full name in kebab-case (e.g., `present-value`, `reliability`, `notification-class`) or by numeric value. Array indices use bracket syntax: `ol[3]`, `pa[8]`.

## Pre-built Binaries

Available from [GitHub Releases](https://github.com/jscott3201/rusty-bacnet/releases):

| Binary | OS | Features |
|--------|-----|----------|
| `bacnet-linux-amd64` | Linux x86_64 | tui, pcap, sc-tls, serial |
| `bacnet-linux-arm64` | Linux aarch64 | tui, pcap, sc-tls, serial |
| `bacnet-macos-amd64` | macOS Intel | tui, sc-tls |
| `bacnet-macos-arm64` | macOS Apple Silicon | tui, sc-tls |
| `bacnet-windows-amd64.exe` | Windows x86_64 | tui, sc-tls |

Rename the downloaded file to `bacnet` (`bacnet.exe` on Windows), make it
executable and put it on your `PATH`. Linux binaries include packet capture
support out of the box. macOS/Windows users who need capture can build from
source with `--features pcap`.

The Linux binaries need glibc 2.17 or newer, so they run on RHEL/CentOS 7,
Debian 8, Ubuntu 14.04 and later. They link libpcap statically, so no libpcap
package is needed (live capture still needs root, as
[Packet Capture](#packet-capture) says). The macOS binaries need macOS 10.12
(Intel) or 11.0 (Apple Silicon) or later and aren't notarized. The Windows
binary links the C runtime statically. Each release also has a `SHA256SUMS`
file and a `THIRD-PARTY-NOTICES` file listing the third-party code in the
binaries; the [installation guide](https://jscott3201.github.io/rusty-bacnet/start/installation/)
shows how to check a download.

Before 0.12.0, the Linux binaries needed glibc 2.39 or newer and the system's
libpcap.
