# BACnet CLI Reference

The `bacnet` command-line tool provides interactive and scripted access to BACnet networks for device discovery, property reading/writing, diagnostics, and packet analysis. `bacnet tui` opens a full-screen terminal UI in builds with the opt-in `tui` feature; see [Terminal UI](#terminal-ui).

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
```

The features combine (`--features sc-tls,pcap`). Packet capture needs libpcap
and its headers (`libpcap-dev` on Debian and Ubuntu).

## Global Options

| Flag | Default | Description |
|------|---------|-------------|
| `-i, --interface <IP>` | (see below) | Network interface IP to bind |
| `-p, --port <PORT>` | `47808` | BACnet UDP port |
| `-b, --broadcast <IP>` | `255.255.255.255` | Broadcast address for WhoIs |
| `-t, --timeout <MS>` | `6000` | APDU timeout in milliseconds |
| `--min-interval-ms <MS>` | `0` | Least time from the latest confirmed request to a device finishing (or being sent, while it is still outstanding) to the next, at most 3600000, so paging or polling leaves a slow device room for others |
| `--ipv6` | | Use BACnet/IPv6 transport |
| `--ipv6-interface <IP>` | | IPv6 interface address |
| `--device-instance <N>` | | Device instance for BIP6 VMAC derivation |
| `--sc` | | Use BACnet/SC transport |
| `--sc-url <URL>` | | SC hub WebSocket URL |
| `--sc-ca <FILE>` | required for SC | Trusted site CA certificate(s) in PEM; no system-root fallback |
| `--sc-cert <FILE>` | | SC TLS certificate PEM |
| `--sc-key <FILE>` | | SC TLS private key PEM |
| `--sc-vmac <HEX>` | | SC local VMAC as 12 hex digits or separated bytes |
| `--sc-device-uuid <UUID>` | | SC device UUID as 32 hex digits or hyphenated text |
| `--format <FMT>` | auto | Output format: `table` or `json` |
| `--json` | | JSON output shorthand |
| `-v` | | Verbosity (`-v`, `-vv`, `-vvv`) |

Output auto-detects: tables in TTY, JSON when piped.

**Interface selection:** When launching the interactive shell without `--interface` on BACnet/IP, the CLI lists available network interfaces and prompts you to select one; `bacnet tui` shows the same list as a dialog. For one-shot commands without `--interface`, it defaults to `0.0.0.0`.

## Target Resolution

Commands that take a `<target>` argument accept several formats:

| Format | Example | Description |
|--------|---------|-------------|
| IPv4 address | `192.168.1.100` | Direct BIP target (default port 47808) |
| IPv4:port | `10.0.1.5:47809` | Direct BIP target with explicit port |
| IPv6 bracket | `[fe80::1]` | Direct BIP6 target (default port 47808) |
| IPv6:port | `[fe80::1]:47809` | Direct BIP6 target with explicit port |
| Device instance | `1234` | Looks up address from discovered device cache |
| DNET:instance | `2:1234` | Routed device (network:instance) |

When using a device instance number, the device must have been previously found via `discover`. In the shell, you can set a default target with the `target` command to omit the target argument from subsequent commands.

## Commands

### Interactive Shell

```bash
bacnet              # launch interactive REPL
bacnet shell        # same as above
```

The shell provides:

- **Tab completion** for commands, object types, and property names
- **Command history** (saved to `~/.bacnet_history`) with history-based hints
- **Default target** via the `target` command (omit target from subsequent commands)
- **BBMD auto-renewal** at 80% of TTL when registered via `register`
- **Colored output** (green for success/values, red for errors, cyan for addresses, dimmed for labels)
- **Quoted string support** in arguments (e.g., `write 10.0.1.5 av:1 on "Zone Temp"`)

Shell-only commands:

```bash
target 192.168.1.100        # set default target
target 1234                 # set default target by device instance
target clear                # clear default target
target                      # show current default target

status                      # show session state (transport, local address,
                            # default target, BBMD registration, device count)

help                        # list all commands
exit                        # exit the shell (also: quit, Ctrl-D)
```

**Command aliases in shell:** `whois`=discover, `whohas`=find, `rp`=read, `rpm`=readm, `rr`=read-range, `wp`=write, `wpm`=writem, `cov`=subscribe, `dcc`=control, `ack`=ack-alarm, `ts`=time-sync

### Terminal UI

```bash
bacnet tui                               # full-screen UI on BACnet/IP
bacnet tui -i 10.0.1.5                   # bind this interface, skip the picker
bacnet --ipv6 tui                        # BACnet/IPv6
bacnet --sc --sc-url wss://hub:443 --sc-ca ca.pem --sc-cert me.pem \
  --sc-key me.key --sc-vmac 02:00:00:00:00:09 \
  --sc-device-uuid 00112233-4455-6677-8899-aabbccddeeff tui   # BACnet/SC
bacnet tui --fps 5 --log-file tui.log    # lower redraw rate, keep a log file
```

`bacnet tui` opens a full-screen terminal UI on the transport chosen by the
global flags. This first version has one screen, **Devices**: a live table of
the devices that answer Who-Is, built from the client's I-Am notifications. It
is **read-only**: it sends discovery requests and nothing that changes a remote
device, and the status bar says `READ-ONLY`. The design and the planned screens
(browse, watch, BBMD, SC, MS/TP, capture) are in
[docs/design/tui.md](design/tui.md).

**Tui flags:**

| Flag | Default | Description |
|------|---------|-------------|
| `--fps <N>` | `20` | Redraws per second, 1 to 60. Idle screens are not redrawn |
| `--log-file <FILE>` | | Append log lines to this file as well as the in-app log pane |

`-v` and `-vv` raise the log level from info to debug and trace.

**Requirements.** stdin and stdout must be a terminal and `TERM` must not be
`dumb`; otherwise `bacnet tui` exits 1 with a hint on stderr and writes nothing
to stdout. The same hint appears, with exit status 1, when the terminal refuses
raw mode (as MSYS and mintty terminals on Windows do). Use the one-shot
commands (`bacnet discover --json`) in scripts and pipes. 80x24 is the smallest
supported terminal and 120x40 shows the full table; below 80x24 the TUI shows a
notice until the window is resized. Colour follows `NO_COLOR`. On Windows use
Windows Terminal or conhost; mintty and Git Bash are not supported.

**Exit status.** 0 after `q` or a second Ctrl-C. 1 for an error, such as a
failed connection or an internal error, with the reason on stderr. When a
signal ends it, the terminal is restored first; on Unix the status then follows
the shell convention of 128 plus the signal number (130 for SIGINT, 129 for
SIGHUP, 143 for SIGTERM), and on Windows a console close or Ctrl-Break exits 1.

**Interface selection.** On BACnet/IP without `-i`, a dialog lists the IPv4
interfaces (one interface is used without asking, as in the shell).

**Devices screen.** Press `d` for the Who-Is form:

| Field | Values |
|---|---|
| Scope | Local broadcast, global broadcast, directed (one address), or remote network |
| Target | For directed: `IP`, `IP:port`, `[IPv6]:port`, or an SC VMAC as 12 hex digits. For remote network: the network number |
| Range | Blank for every instance, `N`, or `LOW-HIGH` (0 to 4194303) |
| Listen | Seconds to show the request as running while replies arrive (1 to 60, default 3) |

A global or unbounded Who-Is can draw a reply from every device on a site, so
the first Enter shows a one-line warning with the number of devices already
known, and a second Enter sends it. The table shows instance, address (for a
routed device, its remote MAC and the router), network, vendor, max APDU,
segmentation and time since the last I-Am. When two addresses answer for the
same instance, a `DUPLICATE` banner names both. If the UI falls behind a burst
of I-Am traffic, events are dropped rather than queued without limit; the
status bar's `drop` counter shows how many, and the table is refreshed from the
client's discovery table afterwards.

**Keys:**

| Key | Action |
|---|---|
| `d` | Who-Is form |
| `/` | Filter rows by instance, address, network or vendor; Enter keeps the filter, Esc clears it |
| `s` / `S` | Next sort column / reverse the order |
| arrows, `j`/`k`, `PgUp`/`PgDn`, `Home`/`End` | Move |
| `L` | Show or hide the log pane |
| `?` | Help |
| `Ctrl-C` | Close the open dialog or cancel the running Who-Is; press again within 2 s to quit |
| `q` | Quit |

The `tui` cargo feature is off by default until the terminal UI ships in
0.13.0 (#975), so the release binaries and a plain `cargo install bacnet-cli`
leave it out. Build it with `cargo install bacnet-cli --features tui`, or with
`--features bacnet-cli/tui` in a workspace checkout. Without it, ratatui and
crossterm aren't built and `bacnet tui` prints that rebuild advice. The
one-shot commands and their JSON output are the same with or without the
feature.

### Device Discovery

```bash
bacnet discover                          # discover all devices
bacnet discover 1000-2000                # discover devices in instance range
bacnet discover --wait 5                 # wait 5 seconds for responses (default: 3)
bacnet discover --target 192.168.1.100   # directed (unicast) WhoIs
bacnet discover --dnet 2                 # target a specific remote network
bacnet discover --bbmd 10.0.0.1          # register as foreign device before discovering
bacnet discover --bbmd 10.0.0.1 --ttl 300  # BBMD registration with TTL (default: 300s)
```

**Discover flags:**

| Flag | Default | Description |
|------|---------|-------------|
| `--wait <N>` | `3` | Seconds to wait for responses |
| `--target <ADDR>` | | Send directed WhoIs to a specific IP address |
| `--dnet <N>` | | Target a specific remote network number |
| `--bbmd <ADDR>` | | Register as foreign device with BBMD before discovering (BIP only) |
| `--ttl <N>` | `300` | TTL in seconds for BBMD foreign device registration |

```bash
bacnet find --name "Zone Temp"           # find objects by name (WhoHas)
bacnet find --name "Zone Temp" --wait 5  # wait 5 seconds for responses
```

```bash
bacnet devices                           # list cached discovered devices
bacnet whois-router                      # send Who-Is-Router-To-Network
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
bacnet write 192.168.1.100 av:1 pv 72.5              # write a float value
bacnet write 192.168.1.100 av:1 pv 72.5 --priority 8 # with priority (1-16)
bacnet write 192.168.1.100 bv:1 pv true               # boolean
bacnet write 192.168.1.100 bv:1 pv active              # enumerated (active=1)
bacnet write 192.168.1.100 av:1 pv null --priority 8   # relinquish
bacnet write 192.168.1.100 av:1 on "\"Zone Temp\""    # character string
bacnet write 192.168.1.100 av:1 pv 72.5@8             # inline priority syntax
bacnet write 192.168.1.100 av:1 pv enumerated:3       # explicit enumerated
bacnet write 192.168.1.100 sc:1 pv date:2024-03-15    # date value
bacnet write 192.168.1.100 sc:1 pv time:14:30:00      # time value
bacnet write 192.168.1.100 nc:1 pv object:ai:1        # object identifier value
```

**Write multiple properties (shell only):**

```bash
writem 192.168.1.100 av:1 pv=72.5,desc="Zone Temp" av:2 pv=68.0
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
bacnet subscribe 192.168.1.100 ai:1                     # unconfirmed COV
bacnet subscribe 192.168.1.100 ai:1 --confirmed          # confirmed COV
bacnet subscribe 192.168.1.100 ai:1 --lifetime 300       # 5-minute subscription
```

Subscribes and then watches for COV notifications in real time. Press Ctrl+C to stop watching.

**Alias:** `cov` = subscribe

### Alarms and Events

```bash
bacnet alarms 192.168.1.100                              # get event/alarm summary

bacnet ack-alarm 192.168.1.100 ai:1 --state 1 \
  --timestamp sequence:417 --ack-time time:14,30,00,00
bacnet ack-alarm 192.168.1.100 ai:1 --state 1 --source "operator" \
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
bacnet control 192.168.1.100 disable-initiation --duration 5
bacnet control 192.168.1.100 disable-initiation
bacnet control 192.168.1.100 enable
bacnet control 192.168.1.100 disable-initiation --password secret

# Reinitialize
bacnet reinit 192.168.1.100 coldstart
bacnet reinit 192.168.1.100 warmstart --password secret
bacnet reinit 192.168.1.100 start-backup
bacnet reinit 192.168.1.100 activate-changes
```

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
bacnet time-sync 192.168.1.100
bacnet time-sync 192.168.1.100 --utc

# Create/delete objects
bacnet create-object 192.168.1.100 av:100
bacnet delete-object 192.168.1.100 av:100
```

**Alias:** `ts` = time-sync

### File Transfer

```bash
bacnet file-read 192.168.1.100 1                             # stream payload as hex
bacnet file-read 192.168.1.100 1 --count 4096 --output data.bin
bacnet file-read 192.168.1.100 1 --access record --output records
bacnet file-write 192.168.1.100 1 firmware.bin               # write file
bacnet file-write 192.168.1.100 1 firmware.bin --start 0     # with offset
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
concurrent-writer no-clobber guarantee. The same flags and behavior apply in
the interactive shell.

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
bacnet whois-router                       # send Who-Is-Router-To-Network
bacnet devices                            # list cached discovered devices
```

### BBMD Management

These commands are BACnet/IP only.

```bash
bacnet bdt 192.168.1.1              # read broadcast distribution table
bacnet fdt 192.168.1.1              # read foreign device table
bacnet register 192.168.1.1 --ttl 300   # register as foreign device
bacnet unregister 192.168.1.1       # unregister from BBMD
```

In the interactive shell, `register` also starts a background auto-renewal task that re-registers at 80% of the TTL (e.g., every 240 seconds for a 300-second TTL). The renewal runs silently in the background and prints a dimmed confirmation on each renewal. Use `unregister` or `status` to check registration state.

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
| `--filter <EXPR>` | Additional BPF filter (appended to `udp port 47808`) |
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
bacnet --ipv6 read [fe80::1]:47808 ai:1 pv

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
it cannot corrupt JSON stdout. Peer trust and certificate validity dates are
checked during TLS, not by an eager local date/issuer check. Help/version, capture
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
| `bacnet-linux-amd64` | Linux x86_64 | pcap, sc-tls |
| `bacnet-linux-arm64` | Linux aarch64 | pcap, sc-tls |
| `bacnet-macos-amd64` | macOS Intel | sc-tls |
| `bacnet-macos-arm64` | macOS Apple Silicon | sc-tls |
| `bacnet-windows-amd64.exe` | Windows x86_64 | sc-tls |

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
