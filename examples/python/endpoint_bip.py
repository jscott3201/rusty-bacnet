"""B/IP endpoint example: one transport that both receives and initiates.

Demonstrates RB-19 migration from separately-constructed client/server
objects (two connections) to one endpoint (one socket):

- Old: BACnetClient(port=X) + BACnetServer(port=Y) = two sockets/ports.
- New: BipEndpoint(port=P) serves AND initiates through one socket.

Run with an installed rusty_bacnet build (dev: maturin develop):
    python endpoint_bip.py
"""

import asyncio

from rusty_bacnet import (
    BipEndpoint,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
)


async def main() -> None:
    # Two endpoints on distinct loopback ports (explicit so local_address is exact).
    first = BipEndpoint(
        device_instance=1001,
        device_name="Endpoint A",
        vendor_id=42,
        interface="127.0.0.1",
        port=47901,
        broadcast_address="127.255.255.255",
    )
    first.add_analog_input(instance=1, name="A-Temp", units=62, present_value=21.5)

    second = BipEndpoint(
        device_instance=1002,
        device_name="Endpoint B",
        vendor_id=42,
        interface="127.0.0.1",
        port=47902,
        broadcast_address="127.255.255.255",
    )
    second.add_analog_input(instance=1, name="B-Temp", units=62, present_value=99.0)

    async with first:
        async with second:
            addr_first = await first.local_address()
            addr_second = await second.local_address()
            print(f"first at {addr_first}, second at {addr_second}")

            first_client = await first.client()
            second_client = await second.client()
            first_server = await first.server()
            print(f"first server alive: {first_server.is_session_alive()}")

            oid = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)

            # Initiate both directions concurrently through each single transport.
            a_reads_b, b_reads_a = await asyncio.gather(
                first_client.read_property(
                    "127.0.0.1:47902", oid, PropertyIdentifier.PRESENT_VALUE
                ),
                second_client.read_property(
                    "127.0.0.1:47901", oid, PropertyIdentifier.PRESENT_VALUE
                ),
            )
            print(f"first reads second: {a_reads_b.value} ({a_reads_b.tag})")
            print(f"second reads first: {b_reads_a.value} ({b_reads_a.tag})")

            # Identity agreement: Device readback matches the composed identity.
            device_b = ObjectIdentifier(ObjectType.DEVICE, 1002)
            name = await first_client.read_property(
                "127.0.0.1:47902", device_b, PropertyIdentifier.OBJECT_NAME
            )
            vendor = await first_client.read_property(
                "127.0.0.1:47902", device_b, PropertyIdentifier.VENDOR_IDENTIFIER
            )
            print(f"second device: name={name.value!r} vendor={vendor.value}")

            print(await first.status())

    print("Done (both endpoints closed; role handles now fail closed).")


if __name__ == "__main__":
    asyncio.run(main())
