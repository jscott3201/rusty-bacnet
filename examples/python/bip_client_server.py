"""BACnet/IP client and server example.

Demonstrates:
- Starting a BACnet/IP server with multiple object types
- Reading and writing properties from a client
- ReadPropertyMultiple for efficient bulk reads
- Device discovery via WhoIs/IAm
"""

import asyncio

from rusty_bacnet import (
    BACnetClient,
    BACnetServer,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
)


async def main():
    # --- Server Setup ---
    server = BACnetServer(
        device_instance=1234,
        device_name="Example HVAC Controller",
        port=0,  # auto-assign port
    )

    # Add various object types
    server.add_analog_input(instance=1, name="Zone Temp", units=62, present_value=72.5)
    server.add_analog_output(instance=1, name="Damper Position", units=98)
    server.add_analog_value(instance=1, name="Cooling Setpoint", units=62)
    server.add_binary_input(instance=1, name="Occupancy Sensor")
    server.add_binary_output(instance=1, name="Fan Enable")
    server.add_binary_value(instance=1, name="Override Mode")
    server.add_multistate_value(instance=1, name="Operating Mode", number_of_states=4)

    await server.start()
    addr = await server.local_address()
    print(f"Server running at {addr}")

    # --- Client Setup ---
    async with BACnetClient(port=0) as client:
        ai1 = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
        ao1 = ObjectIdentifier(ObjectType.ANALOG_OUTPUT, 1)

        # Read a single property
        value = await client.read_property(
            addr, ai1, PropertyIdentifier.PRESENT_VALUE
        )
        print(f"\nZone Temp: {value.value} ({value.tag})")

        # Write a property with priority
        await client.write_property(
            addr,
            ao1,
            PropertyIdentifier.PRESENT_VALUE,
            PropertyValue.real(65.0),
            priority=8,
        )
        print("Wrote damper position: 65.0 @ priority 8")

        # Read it back
        value = await client.read_property(
            addr, ao1, PropertyIdentifier.PRESENT_VALUE
        )
        print(f"Damper Position readback: {value.value}")

        # Read multiple properties from multiple objects
        results = await client.read_property_multiple(
            addr,
            [
                (
                    ai1,
                    [
                        (PropertyIdentifier.PRESENT_VALUE, None),
                        (PropertyIdentifier.OBJECT_NAME, None),
                        (PropertyIdentifier.UNITS, None),
                    ],
                ),
                (
                    ao1,
                    [
                        (PropertyIdentifier.PRESENT_VALUE, None),
                        (PropertyIdentifier.OBJECT_NAME, None),
                    ],
                ),
            ],
        )

        print("\nReadPropertyMultiple results:")
        for obj in results:
            print(f"  Object: {obj['object_id']}")
            for prop in obj["results"]:
                if prop["value"] is not None:
                    print(f"    {prop['property_id']}: {prop['value'].value}")

        # Discover devices
        await client.who_is()
        await asyncio.sleep(0.5)
        devices = await client.discovered_devices()
        print(f"\nDiscovered {len(devices)} device(s):")
        for dev in devices:
            print(
                f"  Device {dev.object_identifier.instance}"
                f" vendor={dev.vendor_id}"
                f" max_apdu={dev.max_apdu_length}"
            )

    await server.stop()
    print("\nDone.")


if __name__ == "__main__":
    asyncio.run(main())
