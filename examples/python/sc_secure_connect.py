"""BACnet/SC (Secure Connect) example.

Demonstrates:
- Running a BACnet/SC Hub (TLS WebSocket relay)
- Connecting a server to the hub
- Connecting a client to the hub
- Reading properties via VMAC addressing

Prerequisites:
    Generate TLS certificates first:
        openssl ecparam -genkey -name prime256v1 -out ca-key.pem
        openssl req -new -x509 -key ca-key.pem -out ca-cert.pem -days 365 -subj "/CN=BACnet CA"
        openssl ecparam -genkey -name prime256v1 -out hub-key.pem
        openssl req -new -key hub-key.pem -out hub.csr -subj "/CN=localhost"
        openssl x509 -req -in hub.csr -CA ca-cert.pem -CAkey ca-key.pem -out hub-cert.pem -days 365
        # Repeat for server-key.pem/server-cert.pem and client-key.pem/client-cert.pem
"""

import asyncio

from rusty_bacnet import (
    BACnetClient,
    BACnetServer,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
    ScHub,
)


async def main():
    # 1. Start the SC Hub
    hub = ScHub(
        listen="127.0.0.1:0",
        cert="hub-cert.pem",
        key="hub-key.pem",
        vmac=b"\xff\x00\x00\x00\x00\x01",
        ca_cert="ca-cert.pem",  # enable mTLS
    )
    await hub.start()
    hub_url = await hub.url()
    print(f"SC Hub running at {hub_url}")

    # 2. Start a server connected to the hub
    server = BACnetServer(
        device_instance=2000,
        device_name="Secure Device",
        transport="sc",
        sc_hub=hub_url,
        sc_vmac=b"\x00\x01\x02\x03\x04\x05",
        sc_ca_cert="ca-cert.pem",
        sc_client_cert="server-cert.pem",
        sc_client_key="server-key.pem",
    )
    server.add_analog_input(instance=1, name="Secure Temp", units=62, present_value=68.5)
    server.add_binary_value(instance=1, name="Alarm Status")
    await server.start()
    print("Server connected to hub")

    # 3. Connect a client to the same hub
    async with BACnetClient(
        transport="sc",
        sc_hub=hub_url,
        sc_vmac=b"\x00\x02\x03\x04\x05\x06",
        sc_ca_cert="ca-cert.pem",
        sc_client_cert="client-cert.pem",
        sc_client_key="client-key.pem",
    ) as client:
        # Address the server by its VMAC (hex-colon notation)
        server_vmac = "00:01:02:03:04:05"

        # Read a property over SC
        value = await client.read_property(
            server_vmac,
            ObjectIdentifier(ObjectType.ANALOG_INPUT, 1),
            PropertyIdentifier.PRESENT_VALUE,
        )
        print(f"\nSecure read: {value.value} °F")

        # Write and read back
        await client.write_property(
            server_vmac,
            ObjectIdentifier(ObjectType.ANALOG_INPUT, 1),
            PropertyIdentifier.PRESENT_VALUE,
            PropertyValue.real(70.0),
        )
        value = await client.read_property(
            server_vmac,
            ObjectIdentifier(ObjectType.ANALOG_INPUT, 1),
            PropertyIdentifier.PRESENT_VALUE,
        )
        print(f"After write: {value.value} °F")

        # Read multiple
        results = await client.read_property_multiple(
            server_vmac,
            [
                (
                    ObjectIdentifier(ObjectType.ANALOG_INPUT, 1),
                    [
                        (PropertyIdentifier.PRESENT_VALUE, None),
                        (PropertyIdentifier.OBJECT_NAME, None),
                    ],
                ),
            ],
        )
        for obj in results:
            for prop in obj["results"]:
                if prop["value"]:
                    print(f"  {prop['property_id']}: {prop['value'].value}")

    await server.stop()
    await hub.stop()
    print("\nAll stopped.")


if __name__ == "__main__":
    asyncio.run(main())
