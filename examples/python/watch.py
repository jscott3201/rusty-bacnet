"""Watching points: COV with renewal, polling where COV is refused.

Demonstrates:
- `Device.watch()`: each point read, then subscribed for COV and renewed
  before each lifetime ends, as an async iterator of `WatchUpdate`s
- a watch of a handle made from an instance, which follows the device to a
  new address (`update.moved`), and telling a value that changed from a
  point the watch gave up
- the client's watch defaults (`watch_lifetime=`), which a watch may override
- `Watch.add()` and `Watch.remove()`, changing the points while iterating
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
    server.add_analog_input(2, "Supply Temp", EngineeringUnits.DEGREES_FAHRENHEIT, 55.0)
    await server.start()
    address = await server.local_address()

    # Every watch of this client takes one-minute subscriptions unless it
    # names its own; watches with equal settings share their subscriptions.
    async with BACnetClient(interface="127.0.0.1", port=0, broadcast_address="127.0.0.1",
                            watch_lifetime=60) as client:
        # A handle made from the device's instance follows it: a watch of it
        # moves with the device once its own reads find it at a new address.
        # One made from an address (`client.device(address)`) watches there.
        await client.discover(scope=address, wait=0.5)
        ahu = await client.device(1234)
        changes = asyncio.ensure_future(change_values(server))
        seen = 0
        changed = False
        # A point the device won't subscribe is polled every `poll_interval`
        # seconds instead (`subscribe=False` polls every point).
        async with await ahu.watch(["ai:1", "bv:1"]) as watch:
            async for update in watch:
                if update.moved is not None and update.state == "starting":
                    # The watch followed the device; values come from there.
                    print(f"device {update.instance} moved from {update.moved.from_} "
                          f"to {update.moved.to}")
                if update.state == "gone":
                    # A watch of one address only: another device took it
                    # ("replaced"), or the device answers elsewhere ("moved").
                    print(f"{update.object_id} given up: {update.why}")
                    break
                if update.why is not None:
                    print(f"{update.object_id} {update.state}: {update.why}")
                if update.value is None:
                    continue
                # Status_Flags come with the value, when the device sent them.
                flags = update.status_flags.text if update.status_flags else "-"
                print(f"{update.object_id}  {update.value.text:12} via {update.via:5} "
                      f"flags={flags} ({update.state})")
                seen += update.object_id == ZONE_TEMP and update.via == "cov"
                if seen == 1 and not changed:
                    changed = True
                    # Change the points while iterating: no update of bv:1
                    # comes once `remove` has returned.
                    await watch.add(["ai:2"])
                    await watch.remove(["bv:1"])
                    print(f"now watching {len(watch)} points")
                if seen >= 4:
                    break
        # Leaving `async with` cancelled the subscriptions.
        await changes

    await server.stop()


if __name__ == "__main__":
    asyncio.run(main())
