---
title: "Write and relinquish deliberately"
description: "Keep control-changing operations separate from read-only onboarding."
---

A BACnet write can change real equipment behavior. Use this page only for an authorized, identified, commandable point with a reviewed site procedure. Prefer an isolated lab device while learning.

## Before the write

Confirm the device identity, object name, units, datatype, valid range, and whether the object is commandable. Check the priority array and the site's ownership of the selected priority slot. Do not overwrite someone else's command to make a tutorial succeed.

Do not copy a write to `PRESENT_VALUE` on an arbitrary analog input. This guide uses an **existing commandable analog value** as a lab example; not every analog value is configured for commandable behavior.

## Inspect the target

```sh
bacnet readm 192.168.1.100 av:1 object-name,units,pv
bacnet read 192.168.1.100 av:1 priority-array
bacnet read 192.168.1.100 av:1 relinquish-default
```

A missing or rejected property is information about the target. Resolve it before proceeding rather than treating all points as the same type.

## Write only at an approved priority

For a lab point whose approved test value is `72.5` and whose approved free slot is priority `8`:

```sh
bacnet write 192.168.1.100 av:1 pv 72.5 --priority 8
bacnet read 192.168.1.100 av:1 pv
```

The value, units, target, and priority are examples, not universal operating defaults. A higher-priority command may still determine the effective present value. A successful write response is not proof that equipment physically moved to the intended state.

## Relinquish your command

To release the same priority slot that your test used:

```sh
bacnet write 192.168.1.100 av:1 pv null --priority 8
bacnet read 192.168.1.100 av:1 pv
```

Relinquishing removes that slot's command. It does **not** promise a return to the previously observed number: another active priority or the relinquish default may now determine the effective value. If the slot was already occupied, clearing it is not a valid restoration of the prior command. Use the site's restoration procedure and retain the needed pre-change state.

## More consequential operations

Alarm acknowledgments, time synchronization, communication control, object creation/deletion, and file writes are not read-only diagnostics. They need distinct guidance and operator intent. In particular, v0.11.0 alarm acknowledgment requires the original event timestamp and an explicit acknowledgment timestamp; do not manufacture those from an unrelated current clock reading.

Record what was requested, the response, readback, and cleanup outcome in the approved operational record. Keep customer data out of public issues and site examples.

## Sources and release scope

These instructions target **v0.11.0**. Source review is not a claim of hardware qualification.

[Write and relinquish grammar](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/docs/CLI.md) · [CLI arguments and acknowledgment inputs](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/crates/bacnet-cli/src/args.rs) · [Python typed values](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/docs/python-api.md).
