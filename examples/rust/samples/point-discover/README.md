# point-discover

Discover a BACnet device by **Who-Is**, read its **object-list**, fetch each point's **object-name** and **present-value**, then scan **priority arrays** on commandable points. Prints results and exits.

Tuned for the bench Niagara box at device **5007** (`192.168.204.200`).

## Quick start

```bash
./run-5007.sh
```

Build + run manually:

```bash
cargo build --release
./target/release/point-discover \
  --device 5007 \
  --interface 192.168.204.55 \
  --broadcast 192.168.204.255
```

## What it does

1. **Who-Is** for the target device instance (default `5007`, 3 s wait)
2. Read device **object-name**
3. Read **object-list** (array-indexed fallback for field devices)
4. **ReadPropertyMultiple** in batches of 10 for name + present-value
5. **Priority-array scan** on commandable points (AO, BO, MSO, AV, BV, MSV)
   - Probes `priority-array` support per point
   - Reads all 16 slots (full list, batched RPM, or per-slot fallback)
   - Shows active slots and inferred `cmd@Pn` winning priority
   - Filters Niagara metadata AVs (`Priority`, `I1`, `O1`, …) with empty arrays from output
   - Still shows relinquished **outputs** (AO/BO) so you know they're writable

## Environment variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `BACNET_DEVICE_INSTANCE` | `5007` | Target device instance |
| `BACNET_BIND_ADDRESS` | `192.168.204.55` | Local NIC IPv4 |
| `BACNET_BROADCAST` | `192.168.204.255` | Directed broadcast |
| `BACNET_DEVICE_ADDRESS` | *(unset)* | Skip Who-Is; use fixed device IP |

## Flags

| Flag | Purpose |
|------|---------|
| `-d, --device` | Device instance to find |
| `-a, --address` | Skip Who-Is; use known device IP |
| `-i, --interface` | Bind NIC (auto-detects `enp3s0`) |
| `-b, --broadcast` | Subnet broadcast |
| `-t, --timeout` | Who-Is wait seconds |
| `--skip-priority` | Point list only; skip priority-array scan |
| `--ephemeral` | Random UDP port if `:47808` busy |

## Examples

Full discover + priority scan (default):

```bash
./run-5007.sh
```

Points only (faster):

```bash
./run-5007.sh --skip-priority
```

Known IP, skip Who-Is:

```bash
BACNET_DEVICE_ADDRESS=192.168.204.200 ./run-5007.sh
```

Different device:

```bash
./run-5007.sh --device 3456
```

## Sample output

```
Device 5007 at 192.168.204.200:47808  name "BENS BENCHTEST BOX"

Points (65):

  ANALOG_INPUT:1173             OA-T                              pv=71.58
  ANALOG_OUTPUT:2466            ACTUATOR-0                        pv=55
  ...

Scanning priority arrays on 50 commandable candidate(s)...

Commandable points — priority arrays (10 shown, 3 with active slot(s)):

  ANALOG_OUTPUT:2466            ACTUATOR-0                        pv=55  cmd@P8
    P8   55

  ANALOG_OUTPUT:10032           C06-0-10VDC-O                     pv=11  cmd@P1
    P1   11

  ANALOG_VALUE:10011            STAT ZN WC-ADJ                    pv=67.59  cmd@P16
    P16  67.59
```

This matches expected Niagara behavior: real overrides at P1/P8/P16, relinquished outputs listed separately, sensor inputs skipped.

## Notes

- Device **5007** has `max_apdu 480` — RPM is batched to stay within APDU limits (~10 s for full scan, ~65 s with priority arrays).
- Prefer **Who-Is discovery** over `--address`; fixed-address mode skips I-Am and may mis-size APDUs on some stacks.
- Stop `mini-device-revisited` before running if it holds `:47808`.
