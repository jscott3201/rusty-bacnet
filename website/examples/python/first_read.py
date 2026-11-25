from __future__ import annotations

import argparse
import asyncio
import sys

from rusty_bacnet import BACnetClient, BacnetError


async def read_one(address: str, instance: int) -> None:
    # An ephemeral local port is suitable for this directed-read example.
    # This example does not validate broadcast discovery or COV delivery.
    async with BACnetClient(port=0) as client:
        # A handle pinned to the address: making it sends nothing.
        device = await client.device(address)
        # "ai:1" is analog input 1; a read with no property is its present value.
        value = await device.read(f"ai:{instance}")
        print(f"{value.value.tag}: {value.text}")


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
