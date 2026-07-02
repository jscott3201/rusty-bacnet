# Network samples

Standalone BACnet/IP **client and server binaries** that complement the single-file examples in this directory. Each crate is self-contained with its own `Cargo.toml` and uses **path dependencies** on the workspace `bacnet-*` crates.

## Samples

| Crate | Role |
|-------|------|
| [`mini-device-revisited/`](mini-device-revisited/) | BACnet **server** — 4-point test device (BACpypes3 port) |
| [`whois-scan/`](whois-scan/) | **Who-Is** scanner — list I-Am responses, exit |
| [`point-discover/`](point-discover/) | **Point discovery** — object-list, present-value, priority arrays |
| [`bacnet-write/`](bacnet-write/) | **WriteProperty** — write, verify, relinquish |
| [`rpm-read/`](rpm-read/) | **ReadPropertyMultiple** — bulk sensor read |

Helper script:

```bash
./run-with-logs.sh          # run mini-device in foreground with debug logging
```

Pass `--replace-existing` to the mini-device binary (via the script) if UDP `:47808` is already in use and you intend to take over the port.

## Typical workflow

```bash
# Terminal 1 — local test server
./run-with-logs.sh

# Terminal 2 — scan subnet (set your NIC IP / broadcast)
cd whois-scan && ./run.sh

# Terminal 3 — enumerate a device (defaults target instance 5007)
cd point-discover && ./run-5007.sh

# Terminal 4 — RPM read three sensors in one request
cd rpm-read && ./run-5007.sh

# Terminal 5 — WriteProperty demo (writes then reverts)
cd bacnet-write && ./run-5007.sh
```

Override bench defaults with environment variables (`BACNET_BIND_ADDRESS`, `BACNET_BROADCAST`, etc.) — see each crate's README.

## Same-host caveat

rusty-bacnet BIP drops frames where the source MAC equals the local bind MAC. A client on the same IP as a local server may not discover it; scan from another host when testing discovery.

Only one process should bind UDP `:47808` on a host at a time.

## Build

Each subfolder is its own Cargo crate:

```bash
cd mini-device-revisited && cargo build --release
cd whois-scan           && cargo build --release
cd point-discover       && cargo build --release
```

The `run.sh` / `run-5007.sh` wrappers build automatically on first use. Build artifacts go to each crate's `target/` directory (gitignored).
