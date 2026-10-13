"""BACnet/IPv6 client and server example.

Demonstrates:
- Running a server on IPv6; its VMAC is its device instance
- Finding a server's VMAC from its IPv6 address with resolve_vmac
- Naming a B/IPv6 peer by VMAC: "bip6:5000" or "bip6:0x001388"

Both ends use the loopback address and a device instance, so the example runs
on one host, Linux included.
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

SERVER_PORT = 47820


async def main():
    # Start an IPv6 server. A B/IPv6 node is addressed by its VMAC, and a
    # server's VMAC is its device instance: 5000 is "bip6:0x001388".
    server = BACnetServer(
        device_instance=5000,
        device_name="IPv6 Device",
        transport="ipv6",
        ipv6_interface="::1",
        port=SERVER_PORT,
    )
    server.add_analog_input(instance=1, name="Temp", units=62, present_value=21.0)
    server.add_analog_value(instance=1, name="Setpoint", units=62)
    await server.start()
    print(f"IPv6 server is {await server.local_address()} at [::1]:{SERVER_PORT}")

    # Connect an IPv6 client on another port. Its own instance gives it a
    # stable VMAC; without one it would pick a random VMAC, which needs a
    # multicast-capable link at start, and Linux loopback has none.
    async with BACnetClient(
        transport="ipv6", ipv6_interface="::1", port=0, ipv6_device_instance=5001
    ) as client:
        # Ask the node at the server's address for its VMAC
        # (Virtual-Address-Resolution). Knowing the device instance,
        # "bip6:5000" names the same node without asking.
        addr = await client.resolve_vmac(f"[::1]:{SERVER_PORT}")
        print(f"Resolved [::1]:{SERVER_PORT} to {addr}")

        value = await client.read_property(
            addr,
            ObjectIdentifier(ObjectType.ANALOG_INPUT, 1),
            PropertyIdentifier.PRESENT_VALUE,
        )
        print(f"Read over IPv6: {value.value}")

        # Write and verify
        await client.write_property(
            "bip6:5000",
            ObjectIdentifier(ObjectType.ANALOG_VALUE, 1),
            PropertyIdentifier.PRESENT_VALUE,
            PropertyValue.real(22.5),
            priority=8,
        )
        value = await client.read_property(
            "bip6:5000",
            ObjectIdentifier(ObjectType.ANALOG_VALUE, 1),
            PropertyIdentifier.PRESENT_VALUE,
        )
        print(f"Setpoint after write: {value.value}")

    await server.stop()
    print("Done.")


if __name__ == "__main__":
    asyncio.run(main())
