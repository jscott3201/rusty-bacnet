"""Device management example.

Demonstrates:
- DeviceCommunicationControl (enable/disable)
- CreateObject / DeleteObject
- Error handling with typed exceptions
"""

import asyncio

from rusty_bacnet import (
    BACnetClient,
    BACnetServer,
    BacnetError,
    BacnetProtocolError,
    BacnetTimeoutError,
    EnableDisable,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
)


async def main():
    server = BACnetServer(
        device_instance=3000, device_name="Managed Device", port=0
    )
    server.add_analog_input(instance=1, name="Temp", units=62, present_value=72.0)
    await server.start()
    addr = await server.local_address()

    async with BACnetClient(port=0) as client:
        ai1 = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)

        # --- Error handling ---
        print("=== Error Handling ===")
        try:
            # Try reading a non-existent property
            await client.read_property(
                addr, ai1, PropertyIdentifier.from_raw(9999)
            )
        except BacnetProtocolError as e:
            print(f"Protocol error (expected): {e}")
        except BacnetTimeoutError:
            print("Timeout (device not responding)")
        except BacnetError as e:
            print(f"General BACnet error: {e}")

        # --- Create an object remotely ---
        print("\n=== CreateObject ===")
        raw = await client.create_object(
            addr,
            ObjectType.ANALOG_VALUE,
            initial_values=[
                (
                    PropertyIdentifier.OBJECT_NAME,
                    PropertyValue.character_string("Dynamic AV"),
                    None,
                    None,
                ),
                (PropertyIdentifier.UNITS, PropertyValue.enumerated(62), None, None),
            ],
        )
        print(f"Created object (raw ACK: {len(raw)} bytes)")

        # --- DeviceCommunicationControl ---
        print("\n=== DeviceCommunicationControl ===")
        await client.device_communication_control(
            addr,
            EnableDisable.DISABLE,
            time_duration=1,  # 1 minute
        )
        print("Device communication disabled")

        state = await server.comm_state()
        print(f"Server comm_state: {state} (1 = disabled)")

        # Re-enable
        await client.device_communication_control(addr, EnableDisable.ENABLE)
        print("Device communication re-enabled")

        # --- Delete the object we created ---
        print("\n=== DeleteObject ===")
        try:
            await client.delete_object(
                addr, ObjectIdentifier(ObjectType.ANALOG_VALUE, 1)
            )
            print("Object deleted")
        except BacnetError as e:
            print(f"Delete failed: {e}")

    await server.stop()
    print("\nDone.")


if __name__ == "__main__":
    asyncio.run(main())
