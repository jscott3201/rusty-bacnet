---
title: "Read with Python"
description: "Use the asynchronous Python client for a small, read-only integration."
---

Install `rusty-bacnet==0.11.0` in a virtual environment first. The distribution name uses a hyphen; the import name uses an underscore.

For a self-contained first run with no external target, use the [local client/server lab](/rusty-bacnet/start/local-lab/). Continue here when you have a known device address and object.

## A complete first-read script

Save the following as `first_read.py`. It reads analog input 1 by default and makes no property writes.

```python
from __future__ import annotations

import argparse
import asyncio
import sys

from rusty_bacnet import (
    BACnetClient,
    BacnetError,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
)


async def read_one(address: str, instance: int) -> None:
    # An ephemeral local port is suitable for this directed-read example.
    # This example does not validate broadcast discovery or COV delivery.
    async with BACnetClient(port=0) as client:
        object_id = ObjectIdentifier(ObjectType.ANALOG_INPUT, instance)
        value = await client.read_property(
            address, object_id, PropertyIdentifier.PRESENT_VALUE
        )
        print(f"{value.tag}: {value.value}")


def main() -> int:
    parser = argparse.ArgumentParser(description="Read one authorized BACnet/IP point")
    parser.add_argument("address", help="Target IPv4:port, for example 192.168.1.100:47808")
    parser.add_argument("--instance", type=int, default=1, help="Analog input instance")
    args = parser.parse_args()
    if not 0 <= args.instance <= 4194303:
        parser.error("--instance must be in 0..4194303")
    try:
        asyncio.run(read_one(args.address, args.instance))
    except (BacnetError, ValueError) as error:
        print(f"BACnet read failed: {error}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

```

Run it against a device and object that you are authorized to read:

```sh
python first_read.py 192.168.1.100:47808 --instance 1
```

An example result shape is `real: 72.5`; that is illustrative, not a captured reading from your site.

## Why the local port is zero

`port=0` asks for an ephemeral local UDP port. It avoids making the directed-read example contend with another process for the default local BACnet port. The remote target still explicitly uses `47808`.

Do not generalize this setup to a broadcast-discovery, server, foreign-device, or subscription deployment without checking its network requirements. A successful direct read is not a discovery or COV acceptance test.

## Add context to the value

Read `OBJECT_NAME` and `UNITS` as well as `PRESENT_VALUE` before assigning meaning to a result. For several properties on the same device, `read_property_multiple` is available, but individual results can contain property-level errors. A completed call does not mean every requested property succeeded.

Keep requests bounded. Avoid starting one task per point across an entire building before measuring the device and network behavior.

## Lifecycle and errors

The async context manager starts the client and stops it when the block exits. The Python API includes typed protocol, timeout, reject, and abort exceptions beneath the library error surface. Record the target, object, property, transport, and exception type when troubleshooting; sanitize site identifiers before sharing logs.

## Next steps

Use [the API entry points](/rusty-bacnet/reference/api/) for complete signatures and type stubs. The repository also includes a client/server example, but review its binds and writes before running it: a demonstration server is not automatically an isolated or production-ready deployment.

## Sources and release scope

These instructions target **v0.11.0**. Source review is not a claim of hardware qualification.

[Python API](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/docs/python-api.md) · [Release client/server example](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/examples/python/bip_client_server.py) · [Type stubs](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/crates/rusty-bacnet/rusty_bacnet.pyi).
