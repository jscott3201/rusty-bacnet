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
| `-i, --interface <IP>` | `0.0.0.0` | Network interface IP to bind |
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

## Commands

### Interactive Shell

```bash
bacnet              # launch interactive REPL
bacnet shell        # same as above
```

The shell supports command history, tab completion, and all commands below.

### Device Discovery

```bash
bacnet discover                 # discover all devices
bacnet discover 1000-2000       # discover devices in instance range
bacnet discover --wait 5        # wait 5 seconds for responses

bacnet find --name "Zone Temp"  # find objects by name (WhoHas)
bacnet devices                  # list cached discovered devices
```

### Reading Properties

```bash
bacnet read 192.168.1.100 ai:1 pv              # read present-value
bacnet read 192.168.1.100 analog-input:1 present-value  # full names work too
bacnet read 192.168.1.100 device:1234 object-name

# Read multiple properties
bacnet readm 192.168.1.100 ai:1 pv,object-name ao:1 pv

# Read range (trend logs, lists)
bacnet read-range 192.168.1.100 trend-log:1 log-buffer
```

### Writing Properties

```bash
bacnet write 192.168.1.100 av:1 pv 72.5              # write a value
bacnet write 192.168.1.100 av:1 pv 72.5 --priority 8 # with priority
bacnet write 192.168.1.100 bv:1 pv true               # boolean
bacnet write 192.168.1.100 av:1 pv null --priority 8   # relinquish
```

### COV Subscriptions

```bash
bacnet subscribe 192.168.1.100 ai:1                     # unconfirmed COV
bacnet subscribe 192.168.1.100 ai:1 --confirmed          # confirmed COV
bacnet subscribe 192.168.1.100 ai:1 --lifetime 300       # 5-minute subscription
```

### Device Management

```bash
# Communication control
bacnet control 192.168.1.100 disable --duration 5
bacnet control 192.168.1.100 enable

# Reinitialize
bacnet reinit 192.168.1.100 coldstart --password secret

# Time synchronization
bacnet time-sync 192.168.1.100
bacnet time-sync 192.168.1.100 --utc

# Create/delete objects
bacnet create-object 192.168.1.100 av:100
bacnet delete-object 192.168.1.100 av:100

# Alarms and events
bacnet alarms 192.168.1.100
bacnet ack-alarm 192.168.1.100 ai:1 --state 1
```

### File Transfer

```bash
bacnet file-read 192.168.1.100 1 --count 4096 --output data.bin
bacnet file-write 192.168.1.100 1 firmware.bin --start 0
```

### BBMD Management

These commands are BACnet/IP only.

```bash
bacnet bdt 192.168.1.1              # read broadcast distribution table
bacnet fdt 192.168.1.1              # read foreign device table
bacnet register 192.168.1.1 --ttl 300   # register as foreign device
bacnet unregister 192.168.1.1       # unregister from BBMD
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
| `--filter <EXPR>` | Additional BPF filter (appended to `udp port 47808`) |
| `--count <N>` | Stop after N packets |
| `--snaplen <N>` | Max bytes per packet (default: 65535) |

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
| `mi` | multi-state-input |
| `mo` | multi-state-output |
| `mv` | multi-state-value |
| `dev` | device |
| `file` | file |
| `schedule` | schedule |
| `calendar` | calendar |
| `tl` | trend-log |
| `nc` | notification-class |
| `loop` | loop |

## Property Shorthand

| Short | Full Name |
|-------|-----------|
| `pv` | present-value |
| `on` | object-name |
| `ot` | object-type |
| `sf` | status-flags |
| `oos` | out-of-service |
| `desc` | description |
| `units` | units |

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
