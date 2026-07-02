# whois-scan

One-shot BACnet **Who-Is** scanner. Sends local-subnet and global Who-Is, waits for I-Am responses, prints discovered devices, exits.

Built for the bench where rusty-bacnet servers reply with **broadcast I-Am** — the client must bind UDP **`47808`** on the NIC to receive them.

## Quick start

```bash
./run.sh
```

Build + run manually:

```bash
cargo build --release
./target/release/whois-scan \
  --interface 192.168.204.55 \
  --broadcast 192.168.204.255
```

## Environment variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `BACNET_BIND_ADDRESS` | `192.168.204.55` | Local NIC IPv4 |
| `BACNET_BROADCAST` | `192.168.204.255` | Directed broadcast |
| `BACNET_SCAN_TIMEOUT` | `3` | Seconds to wait for I-Am |

## Flags

| Flag | Purpose |
|------|---------|
| `-i, --interface` | Bind NIC (auto-detects `enp3s0` if omitted) |
| `-b, --broadcast` | Subnet broadcast (default: /24 from interface) |
| `-t, --timeout` | Wait time after Who-Is |
| `--low` / `--high` | Optional device instance range |
| `--port` | UDP bind port (default `47808`) |
| `--ephemeral` | Random port if `47808` busy (may miss broadcast I-Am) |

## Examples

Scan entire subnet:

```bash
./run.sh
```

Scan for one device:

```bash
./run.sh --low 5007 --high 5007
```

## Sample output

```
Who-Is scan: bind=192.168.204.55:47808 broadcast=192.168.204.255 timeout=3s
Sending local-subnet Who-Is...
Sending global Who-Is (DNET=0xFFFF)...

Found 4 device(s):

  device   5007  addr 192.168.204.200:47808  vendor 5  max_apdu 480
  device   3456  addr 192.168.204.55:47808   vendor 999 max_apdu 1476
  ...
```

## Notes

- Stop `mini-device-revisited` first if it holds `:47808`, or use `--ephemeral`.
- Same-host scan on `.55` may not list the local mini-device on `.55`; scan from another PC (e.g. `.11`) to verify it.
- Sends both **local broadcast** and **global Who-Is** (DNET `0xFFFF`).
