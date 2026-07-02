# bacnet-write

Who-Is a BACnet device, **WriteProperty** at a priority, then fully verify the write was taken before relinquishing.

## Verification flow

1. **Baseline** — read `present-value`, `priority-array[Pn]`, `current-command-priority`
2. **Write** — WriteProperty real value @ priority Pn
3. **Verify write** — read-back confirms:
   - `present-value` matches written value
   - `priority-array[Pn]` matches written value
4. **Relinquish** — WriteProperty `Null` @ Pn
5. **Verify relinquish** — read-back confirms:
   - `priority-array[Pn]` is null
   - `present-value` matches baseline (restored)

Exits non-zero if any verify step fails.

## Quick start

```bash
chmod +x run-5007.sh
./run-5007.sh
```

Default target on device **5007**: `analog-output:10035` (`C07-0-10VDC-O`, relinquished — no higher priority blocking), value `5.0` @ **P8**, then auto-revert.

Use `analog-output:10032` only if you understand P1 may hold present-value while P8 still receives the write in the priority array.

## Example output

```
=== Baseline ===
Before: ANALOG_OUTPUT:10032  pv=11  P8=null  cmd@P1

=== Write @ P8 = 12 ===
WriteProperty ACK

=== Verify write (read-back + priority array) ===
After write: ANALOG_OUTPUT:10032  pv=12  P8=12  cmd@P8
OK: write taken at P8 (present-value + priority-array match)

=== Relinquish P8 (Null write) ===
WriteProperty Null ACK

=== Verify relinquish (read-back + priority array) ===
After revert: ANALOG_OUTPUT:10032  pv=11  P8=null  cmd@P1
OK: P8 relinquished (priority-array null, present-value restored to baseline)

Done — full write cycle verified.
```

## Flags

| Flag | Purpose |
|------|---------|
| `-d, --device` | Device instance (default `5007`) |
| `-p, --point` | Point as `type:instance` |
| `-v, --value` | Real value to write |
| `--priority` | Write priority 1–16 (default `8`) |
| `--tolerance` | Float compare tolerance (default `0.05`) |
| `--no-revert` | Stop after write verify (leave slot active) |
| `--ephemeral` | Random UDP port if `:47808` busy |
