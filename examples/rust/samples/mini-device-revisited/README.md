# mini-device-revisited

100% Rust port of the BACpypes3 [`mini-device-revisited.py`](https://github.com/JoelBender/bacpypes3) example. Runs a minimal BACnet/IP **server** with four points on a single device object.

## Points exposed

| Object | Instance | Access | Behavior |
|--------|----------|--------|----------|
| `analogInput` | 1 | Read-only | Simulated temperature ramp (°F) |
| `binaryInput` | 1 | Read-only | Simulated active/inactive toggle |
| `analogValue` | 2 | Commandable | Priority array (write at priority 8–16) |
| `binaryValue` | 2 | Commandable | Priority array (write at priority 8–16) |

The device object includes all four points in **`object-list`** so Yabe, BACnet Discovery Tool, and `point-discover` can enumerate them.

## Defaults

| Setting | Value |
|---------|-------|
| Device name | `BensServerTest` |
| Device instance | `3456` |
| Vendor ID | `999` |
| UDP port | `47808` |
| Bind address | `0.0.0.0` (all interfaces) |
| Advertised IP | `--address` or auto-detected `enp3s0` |
| I-Am interval | 60 s (startup I-Am + periodic) |

## Quick start

From repo root (recommended):

```bash
../run-with-logs.sh
```

Or directly:

```bash
cargo run --release -- \
  --address 192.168.204.55 \
  --broadcast 192.168.204.255 \
  --debug
```

Release binary:

```bash
cargo build --release
./target/release/mini-device-revisited --address 192.168.204.55 --debug
```

## Useful flags

| Flag | Purpose |
|------|---------|
| `--name` | BACnet device object-name |
| `--instance` | Device instance number |
| `--address` | NIC IPv4 advertised in I-Am |
| `--broadcast` | Subnet directed broadcast |
| `--announce-interval` | Periodic I-Am seconds (`0` = off) |
| `--debug` | Stack + discovery logging |
| `--trace` | Full UDP/NPDU/APDU trace |
| `--skip-self-check` | Skip startup Who-Is self-check |
| `--replace-existing` | Kill other listeners on UDP port before bind (destructive) |

**Do not use `--log-packets`** — it binds a second socket with `SO_REUSEPORT` and steals Who-Is from the server.

## Discovery notes

- Binds **`0.0.0.0:47808`** with directed broadcast (rusty-bacnet-mcp style) so subnet Who-Is reaches the socket on Linux.
- Sends startup + periodic **I-Am** via the server transport (correct BIP MAC), so network scanners find the device without a Who-Is.
- Use **`--replace-existing`** only when you intend to free UDP `:47808` (kills other listeners on that port).
- Exits immediately if UDP bind fails when the port is already in use.
- Same-host clients on the bind IP may not see this device (see root README).

## Extra binary

```bash
cargo run --release --bin discover_probe -- 192.168.204.55 192.168.204.255 3456
```

One-shot unicast + Who-Is probe for sanity checks.

## Dependencies

`bacnet-server`, `bacnet-objects`, `bacnet-client`, `bacnet-transport`, `bacnet-services`, `bacnet-types`, `bacnet-encoding` — all `0.9`.
