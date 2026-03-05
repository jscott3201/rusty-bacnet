"""BACnet/IPv6 client and server example.

Demonstrates:
- Running a server on IPv6 with UDP multicast
- Connecting a client over IPv6
- Same API as BIP, just different transport parameter
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
    # Start an IPv6 server
    server = BACnetServer(
        device_instance=5000,
        device_name="IPv6 Device",
        transport="ipv6",
        ipv6_interface="::",
        port=0,  # auto-assign
    )
    server.add_analog_input(instance=1, name="Temp", units=62, present_value=21.0)
    await server.start()
    addr = await server.local_address()
    print(f"IPv6 server at {addr}")

    # Connect an IPv6 client
    async with BACnetClient(transport="ipv6", ipv6_interface="::", port=0) as client:
        value = await client.read_property(
            addr,
            ObjectIdentifier(ObjectType.ANALOG_INPUT, 1),
            PropertyIdentifier.PRESENT_VALUE,
        )
        print(f"Read over IPv6: {value.value}")

        # Write and verify
        await client.write_property(
            addr,
            ObjectIdentifier(ObjectType.ANALOG_INPUT, 1),
            PropertyIdentifier.PRESENT_VALUE,
            PropertyValue.real(22.5),
        )
        value = await client.read_property(
            addr,
            ObjectIdentifier(ObjectType.ANALOG_INPUT, 1),
            PropertyIdentifier.PRESENT_VALUE,
        )
        print(f"After write: {value.value}")

    await server.stop()
    print("Done.")


if __name__ == "__main__":
    asyncio.run(main())
