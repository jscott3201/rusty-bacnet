---
title: "Your first read"
description: "Confirm a device, read a point, and understand the result before changing anything."
---

**Outcome:** read one known property from one authorized BACnet/IP device. You will not write to the device or change its configuration.

The examples use `192.168.1.100`, device instance `1234`, and analog input `1`. **Replace these example identifiers with values from your own lab or site.** Analog input 1 is not guaranteed to exist on every device.

No known controller yet? [Start with the local lab](/rusty-bacnet/start/local-lab/) instead. This guide is for a specific device on a network you are authorized to use.

## 1. Check the local tool

```sh
bacnet --version
bacnet --help
```

Confirm that you are using the intended binary. These guides target v0.11.0; a newer or source-built version may have different options.

## 2. Confirm one known target

A directed Who-Is keeps the first discovery bounded to the address you supply:

```sh
bacnet discover --target 192.168.1.100 --wait 3
```

Look for an I-Am response and record the device instance. The displayed device count and values depend on your network; this guide does not promise that a particular controller is present.

If you already know the device identity and object list, a direct property read can be useful even when discovery does not return a response. No discovery response is not proof that no device exists.

## 3. Read the device name

Use the device instance returned by your target, not the example `1234`:

```sh
bacnet read 192.168.1.100 device:1234 object-name
```

A property response tells you more than a ping: the BACnet request reached a responding endpoint, and that endpoint answered the selected object/property request.

## 4. Read a known point

```sh
bacnet read 192.168.1.100 ai:1 pv
bacnet readm 192.168.1.100 ai:1 pv,units,object-name
```

`ai:1` means analog input instance 1. `pv` means `present-value`. Read units and object name before attaching engineering meaning to a number. A value such as `72.5` alone is not a universal indication of temperature or a particular unit system.

For a script-friendly response:

```sh
bacnet --json read 192.168.1.100 ai:1 pv
```

The CLI also chooses JSON when output is piped. Explicit `--json` makes intent visible. Inspect the actual output before treating it as a long-term schema contract.

## If the read fails

| Observation | First thing to check |
|---|---|
| No I-Am response | Target address, interface, firewall, transport, and discovery scope |
| Address already in use | Another local BACnet tool is using the same socket |
| Unknown object | The selected object instance exists on a different device, or does not exist |
| Unknown property | The property is not supported on this particular object |
| Timeout on a large request | Try a single property before investigating APDU size or segmentation |

[Follow the troubleshooting guide](/rusty-bacnet/help/troubleshooting/) rather than increasing every timeout or repeatedly broadcasting across the network.

## Understand addresses before scripting

`--port` selects the local BACnet UDP port; a target such as `192.168.1.100:47809` specifies a remote port. They are different settings.

A standalone `bacnet discover` process does not establish a documented persistent discovery cache for a later process. Use explicit IP/port targets for one-shot commands. In the interactive shell, discovery and subsequent commands can share the session cache.

## Continue

[Configure the network deliberately](/rusty-bacnet/guides/configuration/), [automate the read in Python](/rusty-bacnet/start/python/), or [learn about subscriptions](/rusty-bacnet/guides/observe-changes/). Keep writes separate until you have a reviewed operational procedure.

## Sources and release scope

These instructions target **v0.11.0**. Source review is not a claim of hardware qualification.

[CLI guide](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/docs/CLI.md) · [CLI argument definitions](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/crates/bacnet-cli/src/args.rs) · [One-shot target resolution](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/crates/bacnet-cli/src/main.rs).
