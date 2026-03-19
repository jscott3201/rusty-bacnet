# BACnet CLI Reference

The `bacnet` command-line tool provides interactive and scripted access to BACnet networks for device discovery, property reading/writing, diagnostics, and packet analysis.

## Installation

```bash
# From source
cargo install bacnet-cli

# With packet capture support (requires libpcap)
cargo install bacnet-cli --features pcap

# With BACnet/SC support
cargo install bacnet-cli --features sc-tls

# Pre-built binaries (from GitHub Releases)
# Linux builds include pcap support by default
```

## Global Options

| Flag | Default | Description |
|------|---------|-------------|
| `-i, --interface <IP>` | (see below) | Network interface IP to bind |
| `-p, --port <PORT>` | `47808` | BACnet UDP port |
| `-b, --broadcast <IP>` | `255.255.255.255` | Broadcast address for WhoIs |
| `-t, --timeout <MS>` | `6000` | APDU timeout in milliseconds |
| `--ipv6` | | Use BACnet/IPv6 transport |
| `--ipv6-interface <IP>` | | IPv6 interface address |
| `--device-instance <N>` | | Device instance for BIP6 VMAC derivation |
| `--sc` | | Use BACnet/SC transport |
| `--sc-url <URL>` | | SC hub WebSocket URL |
| `--sc-cert <FILE>` | | SC TLS certificate PEM |
| `--sc-key <FILE>` | | SC TLS private key PEM |
| `--format <FMT>` | auto | Output format: `table` or `json` |
| `--json` | | JSON output shorthand |
| `-v` | | Verbosity (`-v`, `-vv`, `-vvv`) |

Output auto-detects: tables in TTY, JSON when piped.

**Interface selection:** When launching the interactive shell without `--interface` on BACnet/IP, the CLI lists available network interfaces and prompts you to select one. For one-shot commands without `--interface`, it defaults to `0.0.0.0`.

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
bacnet find "Zone Temp"                  # find objects by name (WhoHas)
bacnet find --name "Zone Temp"           # same, explicit flag
bacnet find "Zone Temp" --wait 5         # wait 5 seconds for responses
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
```

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

bacnet ack-alarm 192.168.1.100 ai:1 --state 1            # acknowledge an alarm
bacnet ack-alarm 192.168.1.100 ai:1 --state 1 --source "operator"  # custom source
```

**ack-alarm flags:**

| Flag | Default | Description |
|------|---------|-------------|
| `--state <N>` | (required) | Event state to acknowledge (0=normal, 1=fault, etc.) |
| `--source <S>` | `bacnet-cli` | Acknowledgment source string |

**Alias:** `ack` = ack-alarm

### Device Management

```bash
# Communication control
bacnet control 192.168.1.100 disable --duration 5
bacnet control 192.168.1.100 disable-initiation
bacnet control 192.168.1.100 enable
bacnet control 192.168.1.100 disable --password secret

# Reinitialize
bacnet reinit 192.168.1.100 coldstart
bacnet reinit 192.168.1.100 warmstart --password secret
bacnet reinit 192.168.1.100 start-backup
bacnet reinit 192.168.1.100 activate-changes
```

**Control actions:** `enable`, `disable`, `disable-initiation`

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
bacnet file-read 192.168.1.100 1 --output data.bin          # save to file
bacnet file-read 192.168.1.100 1 --start 0 --count 4096     # with range
bacnet file-write 192.168.1.100 1 firmware.bin               # write file
bacnet file-write 192.168.1.100 1 firmware.bin --start 0     # with offset
```

**file-read flags:**

| Flag | Default | Description |
|------|---------|-------------|
| `--start <N>` | `0` | Start position in file |
| `--count <N>` | `1024` | Byte count to read |
| `--output <PATH>` | | Save data to file (otherwise hex-dumps to stdout) |

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
bacnet --sc --sc-url wss://hub:443 --sc-cert cert.pem --sc-key key.pem read 00:01:02:03:04:05 ai:1 pv
```

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

Linux binaries include packet capture support out of the box. macOS/Windows users who need capture can build from source with `--features pcap`.
