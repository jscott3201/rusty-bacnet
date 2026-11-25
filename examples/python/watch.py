"""Watching points: COV with renewal, polling where COV is refused.

Demonstrates:
- `Device.watch()`: each point read, then subscribed for COV and renewed
  before each lifetime ends, as an async iterator of `WatchUpdate`s
- telling a value that changed from a point the watch gave up
- `async with`, which cancels the subscriptions on the way out
"""

import asyncio

from rusty_bacnet import BACnetClient, BACnetServer, EngineeringUnits, ObjectIdentifier
from rusty_bacnet import ObjectType, PropertyValue

ZONE_TEMP = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)


async def change_values(server: BACnetServer) -> None:
    """The zone warming up, a degree a second."""
    for step in range(1, 4):
        await asyncio.sleep(1)
        await server.set_present_value_local(ZONE_TEMP, PropertyValue.real(72.5 + step))


async def main():
    server = BACnetServer(
        device_instance=1234,
        device_name="Example HVAC Controller",
        interface="127.0.0.1",
        port=0,
        broadcast_address="127.0.0.1",
    )
    server.add_analog_input(1, "Zone Temp", EngineeringUnits.DEGREES_FAHRENHEIT, 72.5)
    server.add_binary_value(1, "Occupied")
    await server.start()
    address = await server.local_address()

    async with BACnetClient(interface="127.0.0.1", port=0,
                            broadcast_address="127.0.0.1") as client:
        ahu = await client.device(address)
        changes = asyncio.ensure_future(change_values(server))
        seen = 0
        # Five-minute subscriptions by default; a point the device won't
        # subscribe is polled every `poll_interval` seconds instead.
        async with await ahu.watch(["ai:1", "bv:1"], lifetime=60) as watch:
            async for update in watch:
                if update.state == "gone":
                    # Another device took the address ("replaced"), or the
                    # device answers elsewhere now ("moved").
                    print(f"{update.object_id} given up: {update.why}")
                    break
                if update.why is not None:
                    print(f"{update.object_id} {update.state}: {update.why}")
                if update.value is None:
                    continue
                print(f"{update.object_id}  {update.value.text:12} via {update.via:5} "
                      f"flags={update.status_flags.names} ({update.state})")
                seen += update.object_id == ZONE_TEMP and update.via == "cov"
                if seen >= 4:
                    break
        # Leaving `async with` cancelled the subscriptions.
        await changes

    await server.stop()


if __name__ == "__main__":
    asyncio.run(main())
