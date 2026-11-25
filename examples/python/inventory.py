"""A device's points as records.

Demonstrates:
- `Device.inventory()`: every object's name, units or state texts and
  whether it takes commands, in one result that keeps a partial read
- `Device.points()`: the same records as an async iterator, read a batch
  at a time, for a large device
"""

import asyncio

from rusty_bacnet import BACnetClient, BACnetServer, EngineeringUnits


async def main():
    server = BACnetServer(
        device_instance=1234,
        device_name="Example HVAC Controller",
        interface="127.0.0.1",
        port=0,
        broadcast_address="127.0.0.1",
    )
    server.add_analog_input(1, "Zone Temp", EngineeringUnits.DEGREES_FAHRENHEIT, 72.5)
    server.add_analog_output(1, "Damper Position", EngineeringUnits.PERCENT)
    server.add_binary_value(1, "Occupied")
    server.add_multistate_value(1, "Operating Mode", 4)
    for n in range(2, 40):
        server.add_analog_input(n, f"Room {n} Temp", EngineeringUnits.DEGREES_FAHRENHEIT, 70.0)
    await server.start()
    address = await server.local_address()

    async with BACnetClient(interface="127.0.0.1", port=0,
                            broadcast_address="127.0.0.1") as client:
        ahu = await client.device(address)

        # The whole device, with the values: one summary, the Object_List,
        # then the objects ten or so a request.
        inventory = await ahu.inventory(values=True)
        print(f"{len(inventory)} points on {inventory.summary.name!r} "
              f"(COV {inventory.cov_support})")
        for point in inventory.points[:6]:
            units = point.units_text or ", ".join(point.state_texts) or ""
            value = point.present_value.text if point.present_value else ""
            command = "commandable" if point.commandable else ""
            # Names are the device's own text: repr() shows any control
            # characters instead of printing them.
            print(f"  {point.object_id}  {point.name!r:24} {units:20} {value:12} {command}")
        if inventory.stopped is not None:
            print(f"stopped early: {inventory.stopped}; {len(inventory.unread)} not read")

        # The analog inputs alone, handed out as each batch is read.
        walk = await ahu.points(types=["ai"])
        print(f"\nwalking {len(walk)} analog inputs")
        async for point in walk:
            if point.object_id.instance > 3:
                break  # nothing more is read
            print(f"  {point.name}: {point.units_text} ({point.units!r})")

    await server.stop()


if __name__ == "__main__":
    asyncio.run(main())
