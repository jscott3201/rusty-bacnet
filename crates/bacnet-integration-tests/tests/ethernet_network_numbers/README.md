# Opt-in Ethernet Number wire tests

These tests exercise the Linux AF_PACKET transport on an isolated virtual
Ethernet link. They require raw-socket permission, so ordinary CI leaves them
ignored. They do not qualify a physical LAN or an Ethernet shared endpoint.

Provide an existing Linux image with the repository's pinned Rust toolchain,
Cargo, Python 3, and the locked dependency sources. No package/image download,
host networking or physical interface is needed. Create a task-owned Docker
bridge with `docker network create --internal <network>` and attach two
containers to it. Use `--cap-drop ALL --cap-add NET_RAW`; NET_ADMIN is unnecessary.
Mount the repository read-only at `/src`, a writable task directory at `/task`,
and a read-only Cargo registry under a writable task Cargo home. Set
`CARGO_HOME=/task/cargo`, `CARGO_TARGET_DIR=/task/target`, and the pinned
`RUSTUP_TOOLCHAIN`. Build with no network and no capabilities first:

```sh
cargo test --offline --locked -p bacnet-integration-tests --features ethernet \
  --test ethernet_network_numbers -- --list
```

For each run, create a fresh shared directory such as `/task/wire-01`. Both
runtime containers need `BACNET_ETHERNET_INTERFACE=eth0` and
`BACNET_ETHERNET_FIXTURE_DIR=/task/wire-01`. Start the commands together; the peer
waits for the owner's ready file and each phase has a 30-second progress guard.
The owner command, from `/src`, is:

```sh
cargo test --offline --locked -p bacnet-integration-tests --features ethernet \
  --test ethernet_network_numbers ethernet_number_full_server_and_client_wire \
  -- --ignored --nocapture
```

The independent peer command, in the second container, is:

```sh
python3 /src/crates/bacnet-integration-tests/tests/ethernet_network_numbers/peer.py
```

The peer independently constructs and parses 802.3/LLC bytes, including declared
length and padding. It records injected and captured hex frames. The owner runs
full-server stop/drop and client stop/drop sequentially, checking its own packet
FDs through `/proc/self/fd` and `/proc/net/packet`. Each Number negative is followed
by an exact same-worker positive response; XID/TEST uses its separate receive-loop
fence. The final bounded receive window supplements the positive FD-release
check; silence alone is not the cleanup proof.

Run direct transport checks in one isolated container with the same interface
setting. Besides the lifecycle checks, `ethernet_transport_drops_group_source_frames`
sends XID, TEST and data frames from group source MACs (#1492). A Linux bridge,
such as Docker's, drops a frame with a group source before the other side sees
it, so the peer container cannot deliver one. The test sends them from its own
raw socket on the same interface instead: the kernel hands each outgoing frame
to the transport's socket too. It checks that the first frame the transport
sends answers a station's TEST, that the first NPDU handed up is the station's,
and that all six group-source frames are counted.

```sh
cargo test --offline --locked -p bacnet-integration-tests --features ethernet \
  --test ethernet_network_numbers ethernet_transport \
  -- --ignored --nocapture --test-threads=1
```

Inspect both command exits and logs. A skipped test or missing peer is not a pass.
Use disposable `--rm` containers and remove only the task-owned internal network
when finished. Preserve logs and use ordinary Cargo cleanup in the creating Linux
environment when the task cache is no longer needed.
