"""Change of Value (COV) subscription example.

Demonstrates:
- Subscribing to COV notifications on an analog input
- Receiving real-time value changes via async iterator
- Unsubscribing when done
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
    # Start a server
    server = BACnetServer(
        device_instance=1000, device_name="Sensor Controller", port=0
    )
    server.add_analog_input(instance=1, name="Zone Temp", units=62, present_value=72.0)
    await server.start()
    addr = await server.local_address()
    print(f"Server at {addr}")

    ai1 = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)

    async with BACnetClient(port=0) as client:
        # Subscribe to COV (confirmed, 60 second lifetime)
        await client.subscribe_cov(
            addr,
            subscriber_process_identifier=1,
            monitored_object_identifier=ai1,
            confirmed=True,
            lifetime=60,
        )
        print("Subscribed to COV on AI:1")

        # Start a listener task
        async def listen_for_changes():
            async for notif in client.cov_notifications():
                obj = notif.monitored_object_identifier
                print(f"\n  COV from {obj}:")
                for v in notif.values:
                    pid = v["property_id"]
                    val = v["value"]
                    print(f"    {pid}: {val.value if val else 'N/A'}")

        listener = asyncio.create_task(listen_for_changes())

        # Simulate value changes on the server
        for temp in [73.0, 74.5, 71.0, 75.0]:
            await asyncio.sleep(0.3)
            await server.write_property_local(
                ai1,
                PropertyIdentifier.PRESENT_VALUE,
                PropertyValue.real(temp),
            )
            print(f"Server wrote: {temp}")

        await asyncio.sleep(0.5)

        # Unsubscribe
        await client.unsubscribe_cov(
            addr,
            subscriber_process_identifier=1,
            monitored_object_identifier=ai1,
        )
        print("\nUnsubscribed from COV")
        listener.cancel()

    await server.stop()


if __name__ == "__main__":
    asyncio.run(main())
