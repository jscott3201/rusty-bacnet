"""Device management example.

Demonstrates:
- DeviceCommunicationControl (disable initiation, then enable)
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
    # The server refuses DeviceCommunicationControl unless a policy allows it.
    server = BACnetServer(
        device_instance=3000,
        device_name="Managed Device",
        port=0,
        dcc_policy="require_password",
        dcc_password="dcc-secret",
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
        # Only some object types can be created over the network; Analog
        # Output is one of them. The server refuses a Units initial value
        # (WRITE_ACCESS_DENIED), so only the name is given.
        print("\n=== CreateObject ===")
        requested = ObjectIdentifier(ObjectType.ANALOG_OUTPUT, 100)
        created = await client.create_object(
            addr,
            requested,
            initial_values=[
                (
                    PropertyIdentifier.OBJECT_NAME,
                    PropertyValue.character_string("Dynamic AO"),
                    None,
                    None,
                ),
            ],
        )
        print(f"Created {created}")

        # --- DeviceCommunicationControl ---
        print("\n=== DeviceCommunicationControl ===")
        # The deprecated DISABLE is always refused; DISABLE_INITIATION stops
        # what the server starts but leaves it answering requests.
        await client.device_communication_control(
            addr,
            EnableDisable.DISABLE_INITIATION,
            time_duration=1,  # 1 minute
            password="dcc-secret",
        )
        print("Device initiation disabled")

        state = await server.comm_state()
        print(f"Server comm_state: {state!r}")  # EnableDisable.DISABLE_INITIATION

        # Re-enable
        await client.device_communication_control(
            addr, EnableDisable.ENABLE, password="dcc-secret"
        )
        print("Device communication re-enabled")

        # --- Delete the object we created ---
        print("\n=== DeleteObject ===")
        try:
            await client.delete_object(addr, created)
            print("Object deleted")
        except BacnetError as e:
            print(f"Delete failed: {e}")

    await server.stop()
    print("\nDone.")


if __name__ == "__main__":
    asyncio.run(main())
