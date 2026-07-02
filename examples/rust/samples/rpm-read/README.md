# rpm-read

Who-Is a BACnet device, send a single **ReadPropertyMultiple** request for several points, print object-name + present-value + units, exit.

## Quick start

```bash
chmod +x run-5007.sh
./run-5007.sh
```

Default points on device **5007** (one RPM round-trip):

| Point | Name (typical) |
|-------|----------------|
| `analog-input:1173` | OA-T |
| `analog-input:10014` | STAT ZN-T |
| `analog-input:1192` | DUCT-T |

## Example output

```
ReadPropertyMultiple results:

  ANALOG_INPUT:1173             OA-T                  pv=71.58  units=62
  ANALOG_INPUT:10014            STAT ZN-T             pv=72.48  units=62
  ANALOG_INPUT:1192             DUCT-T                pv=68.72  units=62

Done.
```

## Flags

| Flag | Purpose |
|------|---------|
| `-d, --device` | Device instance (default `5007`) |
| `--points` | Comma-separated `type:instance` list |
| `-i, --interface` | Local NIC IPv4 |
| `-b, --broadcast` | Subnet broadcast |
| `-t, --timeout` | Who-Is wait seconds |
| `--ephemeral` | Random UDP port if `:47808` busy |

## Environment variables

`BACNET_DEVICE_INSTANCE`, `BACNET_BIND_ADDRESS`, `BACNET_BROADCAST`, `BACNET_RPM_POINTS`

Custom batch:

```bash
./run-5007.sh --points analog-input:1173,analog-input:9334
```
