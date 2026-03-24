# BTL Compliance Test Harness

## Overview

BACnet Testing Laboratories (BTL) is the certification arm of BACnet International. BTL certification verifies that a BACnet device correctly implements the protocol per ASHRAE Standard 135. The certification process follows the **BTL Test Plan**, a document that specifies hundreds of individual tests organized by functional area.

The `bacnet-btl` crate implements an automated test harness for **BTL Test Plan 26.1**, the current revision of the test specification. It provides 3,808 tests covering 100% of all 13 BTL sections. The harness can:

- **Self-test** the Rusty BACnet stack by spinning up an in-process server and client on loopback, running the full suite in under one second.
- **Test external devices** over BIP (UDP/IP) or BACnet/SC (WebSocket/TLS) transports.
- **Serve** as a BTL-compliant BACnet device with all 64 standard object types, for use as an IUT (Implementation Under Test) by external test tools.

The binary is called `bacnet-test`.

## Building

### Default build (BIP only)

```bash
cargo build -p bacnet-btl
```

### With BACnet/SC support

The `sc-tls` feature enables BACnet/SC (Secure Connect) transport for both client and server modes. This pulls in `tokio-rustls`, `rcgen`, and `rand` for TLS and self-signed certificate generation.

```bash
cargo build -p bacnet-btl --features sc-tls
```

### All available features

| Feature    | Description                                            |
|------------|--------------------------------------------------------|
| `sc-tls`   | BACnet/SC transport (WebSocket over TLS)               |
| `serial`   | MS/TP serial transport (Linux only)                    |
| `ethernet` | BACnet Ethernet 802.3 transport (Linux only)           |
| `pcap`     | Packet capture support (requires libpcap)              |

```bash
# Full-featured build (Linux)
cargo build -p bacnet-btl --features sc-tls,serial,ethernet
```

### Check without building (no linker required)

```bash
cargo check -p bacnet-btl
cargo check -p bacnet-btl --features sc-tls
```

## Commands Reference

### Global Options

These options apply before the subcommand and affect transport configuration:

```
bacnet-test [OPTIONS] <COMMAND>
```

| Option          | Default           | Description                         |
|-----------------|-------------------|-------------------------------------|
| `--interface`   | `0.0.0.0`         | Bind interface address (IPv4)       |
| `--port`        | `47808`            | BACnet UDP port                     |
| `--broadcast`   | `255.255.255.255`  | Broadcast address (BIP only)        |

### `self-test` -- Test the built-in BACnet server

Starts an in-process BACnet server on an ephemeral loopback port with all 64 standard object types, then runs the full BTL test suite against it via a loopback BIP client.

```
bacnet-test self-test [OPTIONS]
```

| Option          | Default       | Description                                  |
|-----------------|---------------|----------------------------------------------|
| `--mode`        | `in-process`  | Self-test mode (`in-process`, `subprocess`)  |
| `--section`     |               | Filter by section number (e.g., `2`, `3`)   |
| `--tag`         |               | Filter by tag (e.g., `cov`, `negative`)     |
| `--test`        |               | Run a single test by ID (e.g., `2.1.1`)     |
| `--fail-fast`   |               | Stop on first failure                        |
| `--dry-run`     |               | Show which tests would run without executing |
| `--report`      |               | Save JSON report to file path                |
| `--format`      | `terminal`    | Output format: `terminal` or `json`          |
| `--verbose`     |               | Show step-level details for each test        |

```bash
# Run the full suite
./target/debug/bacnet-test self-test

# Run only Section 5 (Alarm & Event) tests
./target/debug/bacnet-test self-test --section 5

# Run a single test with verbose output
./target/debug/bacnet-test self-test --test 2.1.1 --verbose

# Dry run to see what would execute
./target/debug/bacnet-test self-test --dry-run

# Save results to JSON
./target/debug/bacnet-test self-test --report results.json
```

### `run` -- Test an external BACnet device

Connects to an external IUT (Implementation Under Test) via BIP or BACnet/SC and runs the test suite against it.

```
bacnet-test [GLOBAL OPTIONS] run --target <TARGET> [OPTIONS]
```

| Option          | Default       | Description                                         |
|-----------------|---------------|-----------------------------------------------------|
| `--target`      | *(required)*  | IUT address: `IP:port` for BIP, `VMAC` hex for SC  |
| `--sc-hub`      |               | SC hub WebSocket URL (enables SC transport)         |
| `--sc-no-verify`|               | Skip TLS certificate verification (testing only)    |
| `--section`     |               | Filter by section number                            |
| `--tag`         |               | Filter by tag                                       |
| `--test`        |               | Run a single test by ID                             |
| `--fail-fast`   |               | Stop on first failure                               |
| `--dry-run`     |               | Show which tests would run without executing        |
| `--report`      |               | Save JSON report to file path                       |
| `--format`      | `terminal`    | Output format: `terminal` or `json`                 |

```bash
# BIP: test a device at 192.168.1.100 on the default port
./target/debug/bacnet-test run --target 192.168.1.100

# BIP: test a device on a non-standard port
./target/debug/bacnet-test run --target 192.168.1.100:47809

# BIP: bind to a specific interface and broadcast address
./target/debug/bacnet-test --interface 192.168.1.50 --broadcast 192.168.1.255 \
    run --target 192.168.1.100

# SC: test a device via hub, target specified as VMAC hex
./target/debug/bacnet-test run --target aa:bb:cc:dd:ee:ff \
    --sc-hub wss://hub.example.com:47809 --sc-no-verify
```

For BIP targets, the `--target` value is parsed as `IP:port` (port defaults to 47808 if omitted). The IP and port are combined into a 6-byte BACnet MAC address.

For SC targets, the `--target` value must be a 6-byte VMAC in colon-separated hex notation (e.g., `8f:36:1c:d4:97:c7`). The `--sc-hub` option is required and specifies the WebSocket URL of the BACnet/SC hub.

### `serve` -- Run a BTL-compliant BACnet server

Starts a standalone BACnet server with the full BTL test object database (all 64 standard object types). Useful for Docker testing or as an IUT target for external test tools.

```
bacnet-test [GLOBAL OPTIONS] serve [OPTIONS]
```

| Option              | Default  | Description                                     |
|---------------------|----------|-------------------------------------------------|
| `--device-instance` | `99999`  | Device instance number                          |
| `--sc-hub`          |          | SC hub WebSocket URL (enables SC transport)     |
| `--sc-no-verify`    |          | Skip TLS certificate verification               |

```bash
# BIP server on the default port
./target/debug/bacnet-test serve

# BIP server on a specific interface
./target/debug/bacnet-test --interface 192.168.1.50 --port 47808 \
    --broadcast 192.168.1.255 serve --device-instance 12345

# SC server connected to a hub
./target/debug/bacnet-test serve --sc-hub wss://hub:47809 --sc-no-verify
```

The server runs until Ctrl+C. On startup it prints the transport type, bind address, device instance, MAC/VMAC, and object count.

### `list` -- List available tests

Prints all registered tests matching the given filters. No server or client is started.

```
bacnet-test list [OPTIONS]
```

| Option      | Description                              |
|-------------|------------------------------------------|
| `--section` | Filter by section number (e.g., `3`)     |
| `--tag`     | Filter by tag (e.g., `cov`)             |
| `--search`  | Search test names and references         |

```bash
# List all tests
./target/debug/bacnet-test list

# List Section 2 tests
./target/debug/bacnet-test list --section 2

# Search for COV-related tests
./target/debug/bacnet-test list --search cov

# Filter by tag
./target/debug/bacnet-test list --tag negative
```

Output columns: ID, Name, Reference.

### `shell` -- Interactive REPL

Launches an interactive shell with readline support for exploring and running tests.

```
bacnet-test shell
```

Shell commands:

| Command                            | Description                    |
|------------------------------------|--------------------------------|
| `list [--section N] [--tag TAG]`   | List available tests           |
| `self-test [--section N] [--tag TAG]` | Run self-test               |
| `help`                             | Show available commands        |
| `exit` / `quit`                    | Exit the shell                 |

## Test Coverage

All 13 sections of BTL Test Plan 26.1 are fully implemented. The harness includes parameterized tests that apply common test patterns (Out_Of_Service, Command Prioritization, COV, etc.) across all applicable object types.

| Section | Name                            | BTL Refs | Tests | Status |
|---------|---------------------------------|----------|-------|--------|
| 2       | Basic BACnet Functionality      | 27       | 27    | 100%   |
| 3       | Objects                         | 701      | 701   | 100%   |
| 4       | Data Sharing BIBBs              | 799      | 801   | 100%   |
| 5       | Alarm and Event Management      | 456      | 472   | 100%   |
| 6       | Scheduling                      | 222      | 227   | 100%   |
| 7       | Trending                        | 219      | 219   | 100%   |
| 8       | Device Management               | 591      | 592   | 100%   |
| 9       | Data Link Layer                 | 494      | 494   | 100%   |
| 10      | Network Management              | 96       | 96    | 100%   |
| 11      | Gateway                         | 5        | 5     | 100%   |
| 12      | Network Security                | 0        | 9     | 100%   |
| 13      | Audit Reporting                 | 80       | 80    | 100%   |
| 14      | BACnet Web Services             | 0        | 2     | 100%   |
| **Total** |                               | **~3,690** | **3,808** | **100%** |

The "Tests" column exceeds "BTL Refs" in some sections because parameterized cross-cutting tests (e.g., OOS/Status_Flags applied to 32 object types) are counted per instantiation, and some sections include smoke tests or supplementary coverage beyond the minimum BTL references.

## Test Architecture

### Engine Components

The test engine is composed of five modules under `crates/bacnet-btl/src/engine/`:

- **`registry.rs`** -- `TestRegistry` stores all `TestDef` entries. Each test has an ID, name, BTL reference, section, tags, conditionality rules, optional timeout override, and an async test function.
- **`selector.rs`** -- `TestSelector` evaluates IUT capabilities and user-supplied filters to determine which tests to run. Tests can be unconditional (`MustExecute`), capability-gated (`RequiresCapability`), revision-gated (`MinProtocolRevision`), or use a custom predicate.
- **`runner.rs`** -- `TestRunner` iterates the selected tests, executes each with a per-test timeout (default 30 seconds), and collects `TestResult` entries. Supports fail-fast and dry-run modes.
- **`context.rs`** -- `TestContext` is the central runtime type passed to every test function. It wraps a transport-erased BACnet client (`ClientHandle` enum supporting BIP, BIP6, and SC variants), the IUT address, capabilities, and step tracking. Provides high-level helpers: `read_real`, `write_bool`, `verify_readable`, `subscribe_cov`, `read_property_multiple`, `write_expect_error`, etc.
- **`make.rs`** -- `MakeAction` enum for MAKE steps: `Direct` (in-process DB manipulation), `WriteOrPrompt` (BACnet write with interactive fallback), `ManualOnly` (human-required actions).

### Test Definition

Each test is a `TestDef` struct:

```rust
TestDef {
    id: "2.1.1",                          // BTL Test Plan ID
    name: "Processing Remote Network Messages",
    reference: "135.1-2025 - 10.1.1",     // ASHRAE 135.1 clause
    section: Section::BasicFunctionality,
    tags: &["basic", "network", "remote"],
    conditionality: Conditionality::MustExecute,
    timeout: None,                         // Uses runner default (30s)
    run: |ctx| Box::pin(test_fn(ctx)),     // Async test function
}
```

Test functions receive `&mut TestContext` and return `Result<(), TestFailure>`. Each BACnet operation (read, write, subscribe) is automatically recorded as a numbered step with timestamps and pass/fail status.

### Test Organization

Tests are organized in directories matching BTL Test Plan sections:

```
crates/bacnet-btl/src/tests/
  smoke.rs              Engine pipeline validation (3 tests)
  parameterized.rs      Cross-cutting tests applied per object type
  helpers.rs            Reusable test patterns (OOS, command priority, COV, etc.)
  s02_basic/            Section 2: Basic BACnet Functionality
  s03_objects/          Section 3: Objects (one file per object type, ~40 files)
  s04_data_sharing/     Section 4: Data Sharing BIBBs
  s05_alarm/            Section 5: Alarm and Event Management
  s06_scheduling/       Section 6: Scheduling
  s07_trending/         Section 7: Trending
  s08_device_mgmt/      Section 8: Device Management
  s09_data_link/        Section 9: Data Link Layer
  s10_network_mgmt/     Section 10: Network Management
  s11_gateway/          Section 11: Gateway
  s12_security/         Section 12: Network Security
  s13_audit/            Section 13: Audit Reporting
  s14_web_services/     Section 14: BACnet Web Services
```

All tests are registered via `register_all()` in `tests/mod.rs`, which calls each section's `register()` function plus the parameterized test generator.

## Test Filtering

Tests can be filtered by four criteria. All filters are ANDed together.

### By section number

Matches either by section number (all tests in that section) or by test ID prefix:

```bash
# All Section 3 (Objects) tests
bacnet-test self-test --section 3

# Tests starting with "3.1" (subsection prefix)
bacnet-test self-test --section 3.1
```

Section numbers correspond to BTL Test Plan chapters:

| Number | Section                    |
|--------|----------------------------|
| 2      | Basic BACnet Functionality |
| 3      | Objects                    |
| 4      | Data Sharing BIBBs        |
| 5      | Alarm and Event Management |
| 6      | Scheduling                 |
| 7      | Trending                   |
| 8      | Device Management          |
| 9      | Data Link Layer            |
| 10     | Network Management         |
| 11     | Gateway                    |
| 12     | Network Security           |
| 13     | Audit Reporting            |
| 14     | BACnet Web Services        |

### By tag

Tests are tagged for cross-cutting filtering. Some commonly used tags:

| Tag                  | Description                                    |
|----------------------|------------------------------------------------|
| `smoke`              | Engine pipeline validation tests               |
| `basic`              | Section 2 base requirements                    |
| `negative`           | Error/rejection cases                          |
| `network`            | Network layer tests                            |
| `segmentation`       | Segmentation support tests                     |
| `cov`                | Change of Value tests                          |
| `subscribe`          | COV subscription tests                         |
| `oos`                | Out_Of_Service tests                           |
| `status-flags`       | Status_Flags interaction tests                 |
| `command-priority`   | Command prioritization tests                   |
| `commandable`        | Commandable object tests                       |
| `relinquish-default` | Relinquish_Default tests                       |
| `parameterized`      | Cross-object-type parameterized tests          |
| `rei`                | Reliability_Evaluation_Inhibit tests           |
| `value-source`       | Value Source mechanism tests                   |
| `event-state`        | Event reporting state tests                    |
| `data-sharing`       | Data sharing BIBB tests                        |
| `rp-b`               | ReadProperty-B tests                           |
| `data-link`          | Data link layer tests                          |
| `ipv4`, `ipv6`       | IP transport-specific tests                    |
| `mstp`               | MS/TP transport tests                          |
| `sc`                 | BACnet/SC transport tests                      |
| `ethernet`           | BACnet Ethernet tests                          |
| `bbmd`               | BBMD tests                                     |
| `device-mgmt`        | Device management tests                        |
| `time-sync`          | Time synchronization tests                     |
| `gateway`            | Gateway tests                                  |
| `security`           | Network security tests                         |
| `audit`              | Audit reporting tests                          |
| `web-services`       | BACnet web services tests                      |
| `life-safety`        | Life safety domain tests                       |
| `lighting`           | Lighting domain tests                          |
| `elevator`           | Elevator domain tests                          |
| `access-control`     | Access control domain tests                    |

```bash
# All negative tests
bacnet-test self-test --tag negative

# All COV-related tests
bacnet-test self-test --tag cov

# All parameterized tests
bacnet-test self-test --tag parameterized
```

### By test ID

Run exactly one test:

```bash
bacnet-test self-test --test 2.1.5
bacnet-test self-test --test P1.1    # Parameterized test IDs start with "P"
```

Test IDs follow these patterns:
- `2.1.1` through `14.x.y` -- BTL Test Plan section IDs
- `0.0.x` -- Smoke tests
- `P1.x`, `P2.x`, etc. -- Parameterized cross-cutting tests

### By search string

Case-insensitive substring match against test names and BTL references:

```bash
# Find tests related to AnalogInput
bacnet-test list --search "analog"

# Find tests referencing a specific 135.1 clause
bacnet-test list --search "13.4.3"
```

### Combined filters

Filters are ANDed:

```bash
# Section 2 negative tests only
bacnet-test self-test --section 2 --tag negative
```

## Running Against External Devices

### BIP (UDP/IP)

The `run` command creates a BIP client, connects to the target device, and runs the test suite. The harness assumes the target device runs a BTL-compliant server with the standard test object database.

```bash
# Test device on default BACnet port (47808)
bacnet-test run --target 192.168.1.100

# Test device on a custom port
bacnet-test run --target 192.168.1.100:47809

# Specify local interface and broadcast address
bacnet-test --interface 192.168.1.50 --broadcast 192.168.1.255 \
    run --target 192.168.1.100
```

### BACnet/SC

Requires the `sc-tls` feature. The client connects to an SC hub and tests the target device identified by its VMAC.

```bash
# Build with SC support
cargo build -p bacnet-btl --features sc-tls

# Connect to hub and test device by VMAC
./target/release/bacnet-test run \
    --target aa:bb:cc:dd:ee:ff \
    --sc-hub wss://hub.example.com:47809 \
    --sc-no-verify
```

The `--sc-no-verify` flag disables TLS certificate verification, which is necessary when using self-signed certificates in test environments.

### Capability-based test selection

When running against an external device, the harness uses `IutCapabilities` to determine which tests apply. Tests gated by `RequiresCapability` (e.g., requiring a specific object type or service) are automatically skipped if the IUT does not support that capability.

### Exit codes

The `run` and `self-test` commands exit with code 0 if all executed tests pass, or code 1 if any test fails or errors.

## Self-Test Mode

Self-test mode is the primary way to verify the Rusty BACnet stack itself. It is a closed-loop test: the harness starts its own BACnet server and client, both on loopback, and runs the full suite.

### What happens

1. An `InProcessServer` starts a `BACnetServer` bound to `127.0.0.1` on an ephemeral port (port 0).
2. The server is populated with the full BTL test database: all 64 standard BACnet object types with representative instances (Analog I/O/V, Binary I/O/V, MultiState I/O/V, Schedule, Calendar, TrendLog, EventEnrollment, NotificationClass, File, all value types, all access control types, all lighting types, elevator types, color types, audit types, etc.).
3. A `BACnetClient` is created on a separate ephemeral port, also on loopback.
4. A `TestContext` is built with the client, server MAC address, and capabilities derived from the database.
5. All 3,808 registered tests are run sequentially against the in-process server.
6. Results are displayed (terminal or JSON) and the process exits with the appropriate code.

### Test topologies verified

| Topology                          | Transport        | Tests  | Status     |
|-----------------------------------|------------------|--------|------------|
| In-process loopback               | BIP              | 3,808  | All pass   |
| External server (localhost)       | BIP (UDP)        | 3,808  | All pass   |
| SC hub + SC server (localhost)    | SC (WSS/TLS)     | 3,808  | All pass   |
| Docker container (self-test)      | BIP (loopback)   | 3,808  | All pass   |

### Expected output

```
BTL Compliance Test Run
IUT: Device 99999 (Rusty BACnet) via bip [self-test (in-process)]
════════════════════════════════════════════════════════════════════════
  [checkmark] 0.0.1   Read Device Object_Identifier                     0.00s
  [checkmark] 0.0.2   Read Device Object_Name                           0.00s
  [checkmark] 0.0.3   Read AI Present_Value                             0.00s
  [checkmark] 2.1.1   Processing Remote Network Messages                0.00s
  ...
  [checkmark] 14.1.2  RESTful API Endpoint Test                         0.00s
════════════════════════════════════════════════════════════════════════
TOTAL: 3808 tests -- 3808 passed, 0 failed, 0 skipped, 0 manual, 0 errors
Duration: 0.8s
```

Terminal output uses colored markers: green checkmark for pass, red X for fail, yellow circle for skip, blue question mark for manual, and red exclamation for error.

## Serve Mode

The `serve` command runs a standalone BACnet server using the same full BTL test object database as self-test mode. This is useful for:

- **Docker testing** -- run the server in one container and the tester in another.
- **External test tools** -- expose a BTL-compliant device for third-party testers.
- **Manual exploration** -- interact with the server using `bacnet-cli` or any BACnet client.

### BIP server

```bash
bacnet-test --interface 0.0.0.0 --port 47808 --broadcast 255.255.255.255 \
    serve --device-instance 99999
```

Output:
```
BTL server (BIP) listening on 0.0.0.0:47808 -- instance=99999, mac=..., objects=65
Press Ctrl+C to stop.
```

### SC server

Requires `sc-tls` feature. Connects to an SC hub as a node:

```bash
bacnet-test serve --sc-hub wss://hub:47809 --sc-no-verify --device-instance 99999
```

Output:
```
BTL server (SC) connected to wss://hub:47809 -- instance=99999, vmac=..., objects=65
Press Ctrl+C to stop.
```

### Object database contents

The BTL test database includes one instance of every standard BACnet object type (64 types, 65 objects including the Device object):

- **Analog**: AnalogInput:1, AnalogOutput:1, AnalogValue:1
- **Binary**: BinaryInput:1, BinaryOutput:1, BinaryValue:1
- **MultiState**: MultiStateInput:1, MultiStateOutput:1, MultiStateValue:1
- **Infrastructure**: Calendar:1, Schedule:1, TrendLog:1, EventEnrollment:1, NotificationClass:1, Averaging:1, Command:1, Loop:1, Group:1
- **Value types**: IntegerValue:1, PositiveIntegerValue:1, LargeAnalogValue:1, CharacterStringValue:1, OctetStringValue:1, BitStringValue:1, DateValue:1, TimeValue:1, DateTimeValue:1, DatePatternValue:1, TimePatternValue:1, DateTimePatternValue:1
- **Structured**: GlobalGroup:1, StructuredView:1
- **Logging**: EventLog:1, TrendLogMultiple:1, AuditLog:1
- **Specialty**: Accumulator:1, PulseConverter:1, Program:1, LifeSafetyPoint:1, LifeSafetyZone:1, AccessDoor:1, LoadControl:1
- **Access control**: AccessPoint:1, AccessZone:1, AccessUser:1, AccessRights:1, AccessCredential:1, CredentialDataInput:1
- **Notification**: NotificationForwarder:1, AlertEnrollment:1
- **Lighting**: Channel:1, LightingOutput:1, BinaryLightingOutput:1
- **Network**: NetworkPort:1
- **Timer**: Timer:1
- **Elevator**: ElevatorGroup:1, Escalator:1, Lift:1
- **File**: File:1
- **Staging**: Staging:1
- **Audit**: AuditReporter:1
- **Color**: Color:1, ColorTemperature:1

## Docker Testing

The `examples/docker/` directory provides a multi-stage Dockerfile and Docker Compose file for containerized BTL testing.

### Dockerfile

`examples/docker/Dockerfile.btl` builds `bacnet-test` with SC support plus supporting binaries (`bacnet-sc-hub`, `bacnet-device`, `bacnet-router`, `bacnet-bbmd`) in a two-stage Alpine build:

```dockerfile
# Builder: Rust 1.93 on Alpine, builds bacnet-btl with sc-tls
FROM rust:1.93-alpine AS builder
RUN apk add --no-cache musl-dev cmake make perl gcc g++
WORKDIR /src
COPY . .
RUN cargo build --release -p bacnet-btl --features sc-tls && \
    cargo build --release -p bacnet-benchmarks \
        --bin bacnet-sc-hub --bin bacnet-device \
        --bin bacnet-router --bin bacnet-bbmd

# Runtime: minimal Alpine with just the binaries
FROM alpine:3.21 AS runtime
RUN apk add --no-cache ca-certificates
COPY --from=builder /src/target/release/bacnet-test    /usr/local/bin/
COPY --from=builder /src/target/release/bacnet-sc-hub  /usr/local/bin/
COPY --from=builder /src/target/release/bacnet-device  /usr/local/bin/
COPY --from=builder /src/target/release/bacnet-router  /usr/local/bin/
COPY --from=builder /src/target/release/bacnet-bbmd    /usr/local/bin/
CMD ["bacnet-test", "self-test"]
```

### Docker Compose topologies

`examples/docker/docker-compose.btl.yml` defines several test topologies:

#### Standalone self-test (no networking needed)

```bash
docker compose -f docker-compose.btl.yml up btl-self-test
```

Runs the in-process self-test inside a container.

#### BACnet/SC topology (hub + server + tester)

```bash
docker compose -f docker-compose.btl.yml up btl-sc-test
```

Three containers on a `172.21.0.0/24` bridge network:
- `sc-hub` -- BACnet/SC hub listening on port 47809 with self-signed TLS
- `btl-sc-server` -- BTL server connected to the hub as an SC node
- `btl-sc-test` -- BTL tester running self-test (in-process, not against the SC server)

#### BIP topology (server + tester)

```bash
docker compose -f docker-compose.btl.yml up btl-bip-test
```

Two containers on a `172.21.1.0/24` bridge network:
- `btl-bip-server` -- BTL server on `172.21.1.10:47808`
- `btl-bip-test` -- BTL tester running self-test

#### Multi-network routing topology

A router bridging two BIP subnets (`172.21.1.0/24` and `172.21.2.0/24`), with a remote device on subnet B:
- `btl-router` -- bridges subnet A and B
- `btl-remote-device` -- device on subnet B (instance 20000, 10 objects)

### Building and running

```bash
cd examples/docker

# Build the image
docker compose -f docker-compose.btl.yml build

# Run self-test
docker compose -f docker-compose.btl.yml up btl-self-test

# Run SC topology
docker compose -f docker-compose.btl.yml up btl-sc-test

# Tear down
docker compose -f docker-compose.btl.yml down
```

## Report Formats

### Terminal output

The default output format. Provides colored, human-readable results with a summary line.

Each test result shows:
- Status marker (colored: green pass, red fail, yellow skip, blue manual, red bold error)
- Test ID
- Test name
- Duration

With `--verbose`, each test also shows its individual steps (VERIFY, WRITE, TRANSMIT, RECEIVE, MAKE, WAIT) with pass/fail status.

Failure details include the failing step number and error message. The summary line shows total, passed, failed, skipped, manual, and error counts plus total duration.

### JSON output

Use `--format json` to get machine-readable output on stdout, or `--report <path>` to save to a file. Both can be used together.

The JSON report structure:

```json
{
  "id": "uuid-v4",
  "timestamp": "2026-03-19T12:00:00Z",
  "duration": 0.82,
  "iut": {
    "device_instance": 99999,
    "vendor_name": "Rusty BACnet",
    "vendor_id": 555,
    "model_name": "BTL Self-Test",
    "firmware_revision": "0.7.0",
    "protocol_revision": 24,
    "address": "[127, 0, 0, 1, ...]"
  },
  "transport": {
    "transport_type": "bip",
    "local_address": "",
    "details": ""
  },
  "mode": "SelfTestInProcess",
  "suites": [
    {
      "section": "all",
      "name": "BTL Test Run",
      "tests": [
        {
          "id": "2.1.1",
          "name": "Processing Remote Network Messages",
          "reference": "135.1-2025 - 10.1.1",
          "status": "Pass",
          "steps": [
            {
              "step_number": 1,
              "action": {
                "Verify": {
                  "object": "Device:99999",
                  "property": "ObjectIdentifier",
                  "value": "4 bytes"
                }
              },
              "expected": null,
              "actual": "4 bytes",
              "pass": true,
              "timestamp": "2026-03-19T12:00:00.001Z",
              "duration": 0.001
            }
          ],
          "duration": 0.002,
          "notes": []
        }
      ],
      "duration": 0.82,
      "summary": {
        "total": 3808,
        "passed": 3808,
        "failed": 0,
        "skipped": 0,
        "manual": 0,
        "errors": 0,
        "duration": 0.82
      }
    }
  ],
  "capture_file": null,
  "summary": {
    "total": 3808,
    "passed": 3808,
    "failed": 0,
    "skipped": 0,
    "manual": 0,
    "errors": 0,
    "duration": 0.82
  }
}
```

Durations are serialized as floating-point seconds. Timestamps are RFC 3339 UTC. The `mode` field is one of `SelfTestInProcess`, `SelfTestSubprocess`, `SelfTestDocker`, or `External`. The `status` field on each test is one of `Pass`, `Fail` (with `message` and optional `step`), `Skip` (with `reason`), `Manual` (with `description`), or `Error` (with `message`).

### Saving reports

```bash
# JSON file only (terminal still shows colored output)
bacnet-test self-test --report results.json

# JSON to stdout (no terminal formatting)
bacnet-test self-test --format json

# Both: JSON file + JSON stdout
bacnet-test self-test --format json --report results.json
```

## Conditionality System

Not all tests apply to all devices. The harness uses a conditionality system to skip tests that do not apply to the IUT:

| Conditionality           | Description                                           |
|--------------------------|-------------------------------------------------------|
| `MustExecute`            | Always runs, regardless of IUT capabilities           |
| `RequiresCapability`     | Skipped if the IUT lacks a specific capability        |
| `MinProtocolRevision`    | Skipped if the IUT protocol revision is too low       |
| `Custom`                 | Evaluated by a user-defined predicate function        |

Capabilities include:
- **Service support** -- by Protocol_Services_Supported bit position (e.g., ReadProperty=12, SubscribeCOV=5)
- **Object type support** -- by BACnet object type number
- **Segmentation** -- whether the IUT supports segmented messages
- **COV** -- whether the IUT supports Change of Value notifications
- **Intrinsic reporting** -- whether any object supports intrinsic event reporting
- **Command prioritization** -- whether any object is commandable
- **Transport** -- BIP, BIP6, MS/TP, or SC

## Parameterized Tests

The `parameterized.rs` module generates tests by applying common test patterns across all applicable object types. This avoids writing repetitive per-type test functions. The patterns are:

| Pattern | Name                          | BTL Reference         | Applicable Types |
|---------|-------------------------------|-----------------------|------------------|
| P1      | Out_Of_Service / Status_Flags | BTL 7.3.1.1.1        | ~32 types        |
| P2      | Command Prioritization        | 135.1-2025 7.3.1.3   | ~15 types        |
| P3      | Relinquish Default            | 135.1-2025 7.3.1.2   | ~15 types        |
| P4      | COV Subscription              | 135.1-2025 8.2.x     | ~9 types         |
| P5      | Event State                   | BTL event reporting   | ~6 types         |
| P6      | Reliability_Evaluation_Inhibit| 135.1-2025 7.3.1.21.3| ~57 types        |
| P7      | OOS + Commandable             | 135.1-2025 7.3.1.1.2 | ~15 types        |
| P8      | Value Source                  | BTL 7.3.1.28.x       | ~29 types        |

Parameterized test IDs use the prefix `P` followed by the pattern number and a sequential index (e.g., `P1.1` for OOS/Status_Flags on AnalogInput, `P2.3` for Command Prioritization on BinaryOutput).

## Environment Variables

| Variable       | Description                                              |
|----------------|----------------------------------------------------------|
| `RUST_LOG`     | Controls tracing output level (e.g., `RUST_LOG=debug`)  |

The harness uses `tracing-subscriber` with `EnvFilter`, so standard `RUST_LOG` syntax applies (e.g., `RUST_LOG=bacnet_btl=debug,bacnet_server=trace`).
